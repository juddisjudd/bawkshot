use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui_kit::*;

use crate::updater::{self, Release};

/// A first check soon after start, out of the way of the tray and the hotkeys coming up.
const FIRST_CHECK: Duration = Duration::from_secs(10);
/// bawkshot can sit in the tray for weeks, so it looks again once a day.
const NEXT_CHECK: Duration = Duration::from_secs(24 * 60 * 60);
const PROGRESS_EVERY: Duration = Duration::from_millis(250);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpdateState {
    Idle,
    Checking,
    Current,
    Available(Release),
    Downloading {
        version: String,
        done: u64,
        total: u64,
    },
    Ready {
        version: String,
        exe: PathBuf,
    },
    Failed(String),
}

pub struct UpdateFound(pub String);

/// Whether a newer bawkshot exists, and installing it.
pub struct Updates {
    pub state: UpdateState,
    automatic: bool,
    _schedule: Option<Task<()>>,
    _task: Option<Task<()>>,
}

impl EventEmitter<UpdateFound> for Updates {}

impl Updates {
    pub fn new(automatic: bool, cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            state: UpdateState::Idle,
            automatic: false,
            _schedule: None,
            _task: None,
        };
        this.set_automatic(automatic, cx);
        this
    }

    pub fn set_automatic(&mut self, on: bool, cx: &mut Context<Self>) {
        if on == self.automatic {
            return;
        }
        self.automatic = on;
        self._schedule = on.then(|| {
            cx.spawn(async move |this, cx| {
                let mut wait = FIRST_CHECK;
                loop {
                    cx.background_executor().timer(wait).await;
                    wait = NEXT_CHECK;
                    let checked = this.update(cx, |this, cx| {
                        if !matches!(this.state, UpdateState::Available(_)) {
                            this.check(cx);
                        }
                    });
                    if checked.is_err() {
                        break;
                    }
                }
            })
        });
    }

    pub fn check(&mut self, cx: &mut Context<Self>) {
        if matches!(
            self.state,
            UpdateState::Checking | UpdateState::Downloading { .. } | UpdateState::Ready { .. }
        ) {
            return;
        }
        self.state = UpdateState::Checking;
        cx.notify();
        self._task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async { updater::check() })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.state = match result {
                    Ok(Some(release)) => {
                        cx.emit(UpdateFound(release.version.clone()));
                        UpdateState::Available(release)
                    }
                    Ok(None) => UpdateState::Current,
                    Err(error) => UpdateState::Failed(error),
                };
                cx.notify();
            });
        }));
    }

    pub fn install(&mut self, cx: &mut Context<Self>) {
        let UpdateState::Available(release) = self.state.clone() else {
            return;
        };
        self.state = UpdateState::Downloading {
            version: release.version.clone(),
            done: 0,
            total: release.size,
        };
        cx.notify();
        let done = Arc::new(AtomicU64::new(0));
        let result: Arc<Mutex<Option<Result<PathBuf, String>>>> = Arc::default();
        let (worker_done, worker_result) = (done.clone(), result.clone());
        let worker = release.clone();
        cx.background_executor()
            .spawn(async move {
                let outcome =
                    updater::install(&worker, |bytes| worker_done.store(bytes, Ordering::Relaxed));
                *worker_result.lock().expect("update result lock") = Some(outcome);
            })
            .detach();
        self._task = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(PROGRESS_EVERY).await;
                let finished = result.lock().expect("update result lock").take();
                let bytes = done.load(Ordering::Relaxed);
                let stop = this
                    .update(cx, |this, cx| {
                        this.state = match finished {
                            None => UpdateState::Downloading {
                                version: release.version.clone(),
                                done: bytes,
                                total: release.size,
                            },
                            Some(Ok(exe)) => UpdateState::Ready {
                                version: release.version.clone(),
                                exe,
                            },
                            Some(Err(error)) => UpdateState::Failed(error),
                        };
                        cx.notify();
                        !matches!(this.state, UpdateState::Downloading { .. })
                    })
                    .unwrap_or(true);
                if stop {
                    break;
                }
            }
        }));
    }

    pub fn restart(&mut self, cx: &mut Context<Self>) {
        let UpdateState::Ready { exe, .. } = &self.state else {
            return;
        };
        match updater::relaunch(exe) {
            Ok(()) => cx.quit(),
            Err(error) => {
                self.state = UpdateState::Failed(error);
                cx.notify();
            }
        }
    }
}
