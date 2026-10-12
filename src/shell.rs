use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::path::PathBuf;
use std::rc::Rc;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use global_hotkey::hotkey::HotKey;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use gpui_kit::*;
use image::RgbaImage;
use image::imageops;
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{
    Icon, MouseButton as TrayButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
};

use crate::config::{CaptureMode, Config, RecordFormat, Trigger, data_dir};
use crate::geom::RectI;
use crate::overlay::{RegionEvent, RegionOverlay};
use crate::pill::{PILL_HEIGHT, PILL_WIDTH, PillEvent, RecordingPill};
use crate::platform::{Outline, WindowState};
use crate::recorder::{self, Ffmpeg, Health, Recording, ScreenFrames};
use crate::updates::{UpdateFound, UpdateState, Updates};
use crate::{editor, imaging, platform, settings};

/// An open tray menu is still fading out when its item fires, so a capture waits for it to clear.
const TRAY_DELAY: Duration = Duration::from_millis(300);
/// How often a running recording is checked on, and its outline moved after a followed window.
const WATCH: Duration = Duration::from_millis(100);
const SAVED_TAG: &str = "bawkshot-saved";
const UPDATE_TAG: &str = "bawkshot-update";

#[derive(Clone, Debug)]
pub enum Command {
    Capture {
        mode: CaptureMode,
        from_tray: bool,
    },
    /// `target` skips the picker; `seconds` stops the recording by itself.
    Record {
        format: RecordFormat,
        from_tray: bool,
        target: Option<RecordTarget>,
        seconds: Option<u64>,
    },
    StopRecording,
    TrayClick,
    OpenFile,
    OpenPath(PathBuf),
    EditClipboard,
    OpenFolder,
    Settings,
    Quit,
}

impl Command {
    fn from_trigger(trigger: Trigger, from_tray: bool) -> Self {
        match trigger {
            Trigger::Capture(mode) => Command::Capture { mode, from_tray },
            Trigger::Record(format) => Command::Record {
                format,
                from_tray,
                target: None,
                seconds: None,
            },
        }
    }
}

/// What a recording shows: a fixed screen area, or the area a window covers, wherever it moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordTarget {
    Area(RectI),
    Window(isize),
    ActiveWindow,
}

/// What the picked area is for.
#[derive(Clone, Copy)]
enum Purpose {
    Capture,
    Record {
        format: RecordFormat,
        seconds: Option<u64>,
    },
}

struct RegionSession {
    shot: RgbaImage,
    area: RectI,
    purpose: Purpose,
    windows: Vec<AnyWindowHandle>,
    _subscriptions: Vec<Subscription>,
}

struct ActiveRecording {
    id: u64,
    recording: Recording,
    /// Where the capture is now; it moves with `window`.
    area: RectI,
    window: Option<isize>,
    stop_at: Option<Instant>,
    outline: Outline,
    pill: Option<WindowHandle<RecordingPill>>,
    pill_at: (i32, i32),
    _subscription: Option<Subscription>,
}
struct Tray {
    stop: MenuItem,
    settings: MenuItem,
    _icon: TrayIcon,
}

/// The part of the app that outlives every window: tray icon, hotkeys, captures and recordings.
pub struct Shell {
    pub config: Config,
    pub hotkey_errors: Vec<(Trigger, String)>,
    updates: Entity<Updates>,
    hotkeys: Option<GlobalHotKeyManager>,
    registered: Vec<HotKey>,
    triggers: Arc<Mutex<HashMap<u32, Trigger>>>,
    region: Option<RegionSession>,
    recording: Option<ActiveRecording>,
    /// True from the moment a recording is asked for until its file is written, apart from the capture itself.
    recorder_busy: bool,
    recordings: u64,
    ffmpeg: Option<Ffmpeg>,
    last_saved: Option<PathBuf>,
    settings: Option<AnyWindowHandle>,
    tray: Option<Tray>,
    _subscriptions: [Subscription; 3],
}

struct ShellHandle(Entity<Shell>);

impl Global for ShellHandle {}

pub fn shell(cx: &App) -> Entity<Shell> {
    cx.global::<ShellHandle>().0.clone()
}

/// Starts the tray and hotkeys, then runs the commands given on the command line.
pub fn init(startup: Vec<Command>, cx: &mut App) {
    let (commands, inbox) = async_channel::unbounded::<Command>();
    for command in startup {
        let _ = commands.try_send(command);
    }
    let shell = cx.new(|cx| Shell::new(commands.clone(), cx));
    cx.set_global(ShellHandle(shell.clone()));
    let (revealer, opener) = (shell.downgrade(), commands.clone());
    cx.on_system_notification_response(move |response, cx| match response.tag.as_ref() {
        SAVED_TAG => {
            let saved = revealer
                .read_with(cx, |shell, _| shell.last_saved.clone())
                .ok()
                .flatten();
            if let Some(path) = saved {
                cx.reveal_path(&path);
            }
        }
        UPDATE_TAG => {
            let _ = opener.try_send(Command::Settings);
        }
        _ => {}
    });
    cx.spawn(async move |cx| {
        while let Ok(command) = inbox.recv().await {
            shell.update(cx, |shell, cx| shell.run(command, cx));
        }
    })
    .detach();
}

fn menu_id(trigger: Trigger) -> &'static str {
    match trigger {
        Trigger::Capture(CaptureMode::Region) => "capture-region",
        Trigger::Capture(CaptureMode::Fullscreen) => "capture-fullscreen",
        Trigger::Capture(CaptureMode::Monitor) => "capture-monitor",
        Trigger::Capture(CaptureMode::Window) => "capture-window",
        Trigger::Capture(CaptureMode::LastRegion) => "capture-last-region",
        Trigger::Record(RecordFormat::Video) => "record-video",
        Trigger::Record(RecordFormat::Gif) => "record-gif",
    }
}

fn menu_command(id: &str) -> Option<Command> {
    if let Some(trigger) = Trigger::ALL
        .into_iter()
        .find(|trigger| menu_id(*trigger) == id)
    {
        return Some(Command::from_trigger(trigger, true));
    }
    match id {
        "stop-recording" => Some(Command::StopRecording),
        "open-file" => Some(Command::OpenFile),
        "edit-clipboard" => Some(Command::EditClipboard),
        "open-folder" => Some(Command::OpenFolder),
        "settings" => Some(Command::Settings),
        "quit" => Some(Command::Quit),
        _ => None,
    }
}

fn build_tray(commands: &async_channel::Sender<Command>) -> Option<Tray> {
    let menu = Menu::new();
    let add = |id: &str, label: &str| {
        let _ = menu.append(&MenuItem::with_id(id, label, true, None));
    };
    for trigger in Trigger::ALL {
        if trigger == Trigger::Record(RecordFormat::Video) {
            let _ = menu.append(&PredefinedMenuItem::separator());
        }
        match trigger {
            Trigger::Capture(mode) => add(menu_id(trigger), &format!("capture {}", mode.label())),
            Trigger::Record(_) => add(menu_id(trigger), trigger.label()),
        }
    }
    let stop = MenuItem::with_id("stop-recording", "stop recording", false, None);
    let _ = menu.append(&stop);
    let _ = menu.append(&PredefinedMenuItem::separator());
    add("open-file", "open an image…");
    add("edit-clipboard", "edit the clipboard image");
    add("open-folder", "open the screenshots folder");
    let _ = menu.append(&PredefinedMenuItem::separator());
    let settings = MenuItem::with_id("settings", SETTINGS_LABEL, true, None);
    let _ = menu.append(&settings);
    add("quit", "quit bawkshot");

    let sender = commands.clone();
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        if let Some(command) = menu_command(event.id().as_ref()) {
            let _ = sender.try_send(command);
        }
    }));
    let sender = commands.clone();
    TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
        if let TrayIconEvent::Click {
            button: TrayButton::Left,
            button_state: MouseButtonState::Up,
            ..
        } = event
        {
            let _ = sender.try_send(Command::TrayClick);
        }
    }));

    let pixels = imaging::brand_icon(64)?;
    let icon = Icon::from_rgba(pixels.into_raw(), 64, 64).ok()?;
    let icon = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .with_tooltip("bawkshot")
        .with_icon(icon)
        .build()
        .map_err(|err| log::error!("could not create the tray icon: {err}"))
        .ok()?;
    Some(Tray {
        stop,
        settings,
        _icon: icon,
    })
}

const SETTINGS_LABEL: &str = "settings";

/// The tray has no badge, so its settings entry says when an update is waiting there.
fn settings_label(state: &UpdateState) -> &'static str {
    match state {
        UpdateState::Available(_) => "settings · update available",
        UpdateState::Ready { .. } => "settings · restart to update",
        _ => SETTINGS_LABEL,
    }
}

impl Shell {
    fn new(commands: async_channel::Sender<Command>, cx: &mut Context<Self>) -> Self {
        let config = Config::load();
        crate::theme::apply(config.mode, None, cx);

        let triggers: Arc<Mutex<HashMap<u32, Trigger>>> = Arc::default();
        let hotkeys = GlobalHotKeyManager::new()
            .map_err(|err| log::error!("global hotkeys are unavailable: {err}"))
            .ok();
        let (sender, pressed) = (commands.clone(), triggers.clone());
        GlobalHotKeyEvent::set_event_handler(Some(move |event: GlobalHotKeyEvent| {
            if event.state() != HotKeyState::Pressed {
                return;
            }
            let trigger = pressed
                .lock()
                .ok()
                .and_then(|triggers| triggers.get(&event.id()).copied());
            if let Some(trigger) = trigger {
                let _ = sender.try_send(Command::from_trigger(trigger, false));
            }
        }));
        let quit = cx.on_app_quit(|this, _| {
            if let Some(active) = this.recording.take() {
                let _ = active.recording.finish(true);
            }
            async {}
        });
        let updates = cx.new(|cx| Updates::new(config.check_updates, cx));
        let found = cx.subscribe(&updates, |_, _, found: &UpdateFound, cx| {
            let title = format!("bawkshot {} is out", found.0);
            let body = "click here, or open settings from the tray icon, to update.";
            notify_tagged(UPDATE_TAG, &title, body, cx);
        });
        let changed = cx.observe(&updates, |this, updates, cx| {
            let label = settings_label(&updates.read(cx).state);
            if let Some(tray) = this
                .tray
                .as_ref()
                .filter(|tray| tray.settings.text() != label)
            {
                tray.settings.set_text(label);
            }
        });

        let mut this = Self {
            tray: build_tray(&commands),
            config,
            hotkey_errors: Vec::new(),
            updates,
            hotkeys,
            registered: Vec::new(),
            triggers,
            region: None,
            recording: None,
            recorder_busy: false,
            recordings: 0,
            ffmpeg: None,
            last_saved: None,
            settings: None,
            _subscriptions: [quit, found, changed],
        };
        this.register_hotkeys();
        if !this.hotkey_errors.is_empty() {
            let count = this.hotkey_errors.len();
            let body = format!(
                "{count} of the hotkeys could not be set. open settings from the tray icon to change them."
            );
            notify("some hotkeys are not active", &body, cx);
        }
        this
    }

    fn register_hotkeys(&mut self) {
        self.hotkey_errors.clear();
        let Some(manager) = &self.hotkeys else {
            return;
        };
        for hotkey in self.registered.drain(..) {
            let _ = manager.unregister(hotkey);
        }
        let Ok(mut triggers) = self.triggers.lock() else {
            return;
        };
        triggers.clear();
        for trigger in Trigger::ALL {
            let keys = self.config.hotkeys.get(trigger).trim();
            if keys.is_empty() {
                continue;
            }
            let Ok(hotkey) = HotKey::from_str(keys) else {
                self.hotkey_errors
                    .push((trigger, format!("\"{keys}\" is not a key combination")));
                continue;
            };
            let problem = match triggers.entry(hotkey.id()) {
                Entry::Occupied(_) => "is already used above",
                Entry::Vacant(slot) if manager.register(hotkey).is_ok() => {
                    slot.insert(trigger);
                    self.registered.push(hotkey);
                    continue;
                }
                Entry::Vacant(_) => "is taken by another app",
            };
            self.hotkey_errors
                .push((trigger, format!("\"{keys}\" {problem}")));
        }
    }

    /// Stores changed settings and applies the ones that take effect outside the settings window.
    pub fn update_config(&mut self, change: impl FnOnce(&mut Config), cx: &mut Context<Self>) {
        let before = self.config.hotkeys.clone();
        change(&mut self.config);
        if self.config.hotkeys != before {
            self.register_hotkeys();
        }
        let automatic = self.config.check_updates;
        self.updates
            .update(cx, |updates, cx| updates.set_automatic(automatic, cx));
        self.save();
        cx.notify();
    }

    fn save(&self) {
        if let Err(err) = self.config.save() {
            log::error!("could not save settings: {err}");
        }
    }

    fn run(&mut self, command: Command, cx: &mut Context<Self>) {
        match command {
            Command::Capture { mode, from_tray } => {
                if self.region.is_some() {
                    return;
                }
                cx.spawn(async move |this, cx| {
                    if from_tray {
                        cx.background_executor().timer(TRAY_DELAY).await;
                    }
                    let _ = this.update(cx, |this, cx| this.capture(mode, cx));
                })
                .detach();
            }
            Command::Record {
                format,
                from_tray,
                target,
                seconds,
            } => {
                if self.recording.is_some() {
                    self.stop_recording(true, cx);
                    return;
                }
                if self.region.is_some() || self.recorder_busy {
                    return;
                }
                cx.spawn(async move |this, cx| {
                    if from_tray {
                        cx.background_executor().timer(TRAY_DELAY).await;
                    }
                    let _ = this.update(cx, |this, cx| this.record(format, target, seconds, cx));
                })
                .detach();
            }
            Command::StopRecording => self.stop_recording(true, cx),
            Command::TrayClick if self.recording.is_some() => self.stop_recording(true, cx),
            Command::TrayClick => self.run(
                Command::Capture {
                    mode: CaptureMode::Region,
                    from_tray: false,
                },
                cx,
            ),
            Command::OpenFile => self.open_file(cx),
            Command::OpenPath(path) => self.open_path(path, cx),
            Command::EditClipboard => match imaging::clipboard_image() {
                Some(image) => editor::open(Arc::new(image), None, cx),
                None => notify("nothing to edit", "the clipboard holds no image.", cx),
            },
            Command::OpenFolder => {
                let _ = std::fs::create_dir_all(&self.config.save_dir);
                cx.open_with_system(&self.config.save_dir);
            }
            Command::Settings => self.open_settings(cx),
            Command::Quit => cx.quit(),
        }
    }

    fn open_file(&mut self, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("open".into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = paths.await else {
                return;
            };
            if let Some(path) = paths.into_iter().next() {
                let _ = this.update(cx, |this, cx| this.open_path(path, cx));
            }
        })
        .detach();
    }

    fn open_path(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        cx.spawn(async move |_, cx| {
            let loading = path.clone();
            let loaded = cx
                .background_executor()
                .spawn(async move { imaging::load(&loading) })
                .await;
            cx.update(|cx| match loaded {
                Ok(image) => editor::open(Arc::new(image), Some(path), cx),
                Err(err) => notify("could not open the image", &err, cx),
            });
        })
        .detach();
    }

    fn open_settings(&mut self, cx: &mut Context<Self>) {
        let reused = self.settings.is_some_and(|handle| {
            handle
                .update(cx, |_, window, cx| {
                    window.activate_window();
                    platform::bring_to_front(window, cx);
                })
                .is_ok()
        });
        if !reused {
            let (config, errors) = (self.config.clone(), self.hotkey_errors.clone());
            self.settings = settings::open(config, errors, self.updates.clone(), cx);
        }
    }

    fn capture(&mut self, mode: CaptureMode, cx: &mut Context<Self>) {
        let screen = platform::virtual_screen();
        match mode {
            CaptureMode::Region => self.pick_region(Purpose::Capture, cx),
            CaptureMode::Fullscreen => self.grab(screen, cx),
            CaptureMode::Monitor => {
                let (x, y) = platform::cursor_position();
                let rect = platform::monitor_at(x, y).map_or(screen, |monitor| monitor.rect);
                self.grab(rect, cx);
            }
            CaptureMode::Window => {
                let rect = platform::foreground_window().unwrap_or(screen);
                self.grab(rect, cx);
            }
            CaptureMode::LastRegion => {
                let last = self.config.last_region.map(|rect| rect.intersect(&screen));
                match last.filter(|rect| !rect.is_empty()) {
                    Some(rect) => self.grab(rect, cx),
                    None => self.pick_region(Purpose::Capture, cx),
                }
            }
        }
    }

    fn grab(&mut self, rect: RectI, cx: &mut Context<Self>) {
        match platform::grab(rect, self.config.capture_cursor) {
            Some(image) => self.finish(image, cx),
            None => notify("capture failed", "windows refused to copy the screen.", cx),
        }
    }

    /// Freezes the screen and opens one overlay per monitor to choose an area on.
    fn pick_region(&mut self, purpose: Purpose, cx: &mut Context<Self>) {
        let area = platform::virtual_screen();
        let Some(shot) = platform::grab(area, self.config.capture_cursor) else {
            notify("capture failed", "windows refused to copy the screen.", cx);
            return;
        };
        let targets = Rc::new(platform::snap_targets());
        let displays = cx.displays();
        let mut windows = Vec::new();
        let mut subscriptions = Vec::new();
        for monitor in platform::monitors() {
            let Some(display) = displays
                .iter()
                .find(|display| u64::from(display.id()) == monitor.id)
            else {
                continue;
            };
            let local = RectI::new(
                monitor.rect.x - area.x,
                monitor.rect.y - area.y,
                monitor.rect.w,
                monitor.rect.h,
            )
            .intersect(&RectI::new(0, 0, area.w, area.h));
            if local.is_empty() {
                continue;
            }
            let frame = imaging::render_image(
                imageops::crop_imm(
                    &shot,
                    local.x as u32,
                    local.y as u32,
                    local.w as u32,
                    local.h as u32,
                )
                .to_image(),
            );
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Fullscreen(display.bounds())),
                titlebar: None,
                kind: WindowKind::PopUp,
                is_movable: false,
                is_resizable: false,
                is_minimizable: false,
                display_id: Some(display.id()),
                ..Default::default()
            };
            let targets = targets.clone();
            let opened = cx.open_window(options, |window, cx| {
                platform::prepare_overlay(window, cx);
                cx.new(|cx| RegionOverlay::new(frame, monitor, targets, window, cx))
            });
            let Ok(handle) = opened else {
                continue;
            };
            if let Ok(view) = handle.entity(cx) {
                subscriptions.push(cx.subscribe(&view, Self::on_region));
            }
            windows.push(handle.into());
        }
        if windows.is_empty() {
            if let Purpose::Capture = purpose {
                self.finish(shot, cx);
            }
            return;
        }
        cx.activate(true);
        self.region = Some(RegionSession {
            shot,
            area,
            purpose,
            windows,
            _subscriptions: subscriptions,
        });
    }

    fn on_region(&mut self, _: Entity<RegionOverlay>, event: &RegionEvent, cx: &mut Context<Self>) {
        let Some(session) = self.region.take() else {
            return;
        };
        let windows = session.windows.clone();
        cx.defer(move |cx| {
            for window in windows {
                let _ = window.update(cx, |_, window, _| window.remove_window());
            }
        });
        let RegionEvent::Picked { rect, window } = event else {
            return;
        };
        if let Purpose::Record { format, seconds } = session.purpose {
            let target = window.map_or(RecordTarget::Area(*rect), RecordTarget::Window);
            self.record(format, Some(target), seconds, cx);
            return;
        }
        let local = RectI::new(
            rect.x - session.area.x,
            rect.y - session.area.y,
            rect.w,
            rect.h,
        )
        .intersect(&RectI::new(0, 0, session.area.w, session.area.h));
        if local.is_empty() {
            return;
        }
        self.config.last_region = Some(*rect);
        self.save();
        let image = imageops::crop_imm(
            &session.shot,
            local.x as u32,
            local.y as u32,
            local.w as u32,
            local.h as u32,
        )
        .to_image();
        self.finish(image, cx);
    }

    /// Runs the after-capture steps the settings turn on.
    fn finish(&mut self, image: RgbaImage, cx: &mut Context<Self>) {
        let image = Arc::new(image);
        let path = self
            .config
            .save_to_disk
            .then(|| self.config.next_file("png"));
        let copy = self.config.copy_to_clipboard;
        let open_editor = self.config.open_editor;
        if copy || path.is_some() {
            let (image, path) = (image.clone(), path.clone());
            cx.spawn(async move |_, cx| {
                let saving = path.clone();
                let outcome = cx
                    .background_executor()
                    .spawn(async move { store(&image, copy, saving.as_deref()) })
                    .await;
                cx.update(|cx| match outcome {
                    Err(err) => notify("capture not stored", &err, cx),
                    Ok(()) if !open_editor => notify("captured", &stored_message(copy, path), cx),
                    Ok(()) => {}
                });
            })
            .detach();
        }
        if open_editor {
            editor::open(image, path, cx);
        }
    }

    /// Finds ffmpeg, then either asks what to record or starts on the target given.
    fn record(
        &mut self,
        format: RecordFormat,
        target: Option<RecordTarget>,
        seconds: Option<u64>,
        cx: &mut Context<Self>,
    ) {
        if self.recording.is_some() || self.recorder_busy {
            return;
        }
        let Some(path) = recorder::locate(&self.config.ffmpeg_path) else {
            let body = "recording needs ffmpeg. install it, or set its path in settings.";
            notify("ffmpeg was not found", body, cx);
            return;
        };
        let window = match target {
            None => {
                self.pick_region(Purpose::Record { format, seconds }, cx);
                return;
            }
            Some(RecordTarget::Area(_)) => None,
            Some(RecordTarget::Window(window)) => Some(window),
            Some(RecordTarget::ActiveWindow) => platform::foreground_handle(),
        };
        let area = match (target, window.map(platform::window_state)) {
            (Some(RecordTarget::Area(area)), _) => area.intersect(&platform::virtual_screen()),
            (_, Some(WindowState::At(frame))) => frame,
            _ => {
                notify(
                    "nothing to record",
                    "that window is closed or minimized.",
                    cx,
                );
                return;
            }
        };
        let area = recorder::even(area);
        if area.w < 16 || area.h < 16 {
            notify("nothing to record", "that area is too small.", cx);
            return;
        }
        let known = self.ffmpeg.clone().filter(|ffmpeg| ffmpeg.path == path);
        self.recorder_busy = true;
        cx.spawn(async move |this, cx| {
            let ffmpeg = match known {
                Some(ffmpeg) => Ok(ffmpeg),
                None => {
                    cx.background_executor()
                        .spawn(async move { Ffmpeg::probe(path) })
                        .await
                }
            };
            let _ = this.update(cx, |this, cx| {
                this.recorder_busy = false;
                match ffmpeg {
                    Ok(ffmpeg) => this.start_recording(ffmpeg, area, window, format, seconds, cx),
                    Err(err) => notify("ffmpeg did not run", &err, cx),
                }
            });
        })
        .detach();
    }

    fn start_recording(
        &mut self,
        ffmpeg: Ffmpeg,
        area: RectI,
        window: Option<isize>,
        format: RecordFormat,
        seconds: Option<u64>,
        cx: &mut Context<Self>,
    ) {
        self.ffmpeg = Some(ffmpeg.clone());
        let fps = match format {
            RecordFormat::Video => self.config.video_fps,
            RecordFormat::Gif => self.config.gif_fps,
        }
        .clamp(1, 120);
        let cursor = self.config.record_cursor;
        let output = self.config.next_file(format.extension());
        let log = data_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join("ffmpeg.log");
        let open = move || ScreenFrames::open(area, window, cursor);
        let size = (area.w, area.h);
        let recording = match Recording::start(ffmpeg, format, size, fps, open, output, log) {
            Ok(recording) => recording,
            Err(err) => {
                notify("the recording did not start", &err, cx);
                return;
            }
        };
        self.recordings += 1;
        let id = self.recordings;
        let monitor = platform::monitor_at(area.x + area.w / 2, area.y + area.h / 2);
        let scale = monitor.map_or(1.0, |monitor| monitor.scale);
        let pill_at = monitor.map_or((area.x, area.bottom()), |monitor| {
            pill_position(area, monitor.rect, monitor.scale)
        });
        let pill = monitor.and_then(|monitor| open_pill(pill_at, monitor, format, cx));
        let subscription = pill
            .and_then(|pill| pill.entity(cx).ok())
            .map(|view| cx.subscribe(&view, Self::on_pill));
        self.recording = Some(ActiveRecording {
            id,
            recording,
            area,
            window,
            stop_at: seconds.map(|seconds| Instant::now() + Duration::from_secs(seconds)),
            outline: Outline::around(area, (3.0 * scale).round() as i32),
            pill,
            pill_at,
            _subscription: subscription,
        });
        self.show_recording_state(true);

        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(WATCH).await;
                if !this
                    .update(cx, |this, cx| this.watch_recording(id, cx))
                    .unwrap_or(false)
                {
                    break;
                }
            }
        })
        .detach();
    }

    fn show_recording_state(&self, recording: bool) {
        if let Some(tray) = &self.tray {
            tray.stop.set_enabled(recording);
        }
    }

    fn on_pill(&mut self, _: Entity<RecordingPill>, event: &PillEvent, cx: &mut Context<Self>) {
        self.stop_recording(*event == PillEvent::Stop, cx);
    }

    /// Looks after recording `id`: ends it when ffmpeg, the source or its time is up, and keeps the outline on a moving window. False once it is over.
    fn watch_recording(&mut self, id: u64, cx: &mut Context<Self>) -> bool {
        let Some(active) = self.recording.as_mut().filter(|active| active.id == id) else {
            return false;
        };
        let due = active.stop_at.is_some_and(|at| Instant::now() >= at);
        match active.recording.health() {
            Health::Failed(reason) => {
                if let Some(active) = self.recording.take() {
                    close_pill(active.pill, cx);
                    let _ = active.recording.finish(false);
                }
                self.show_recording_state(false);
                notify("the recording stopped", &reason, cx);
                return false;
            }
            Health::Ended => {
                self.stop_recording(true, cx);
                return false;
            }
            Health::Running if due => {
                self.stop_recording(true, cx);
                return false;
            }
            Health::Running => {}
        }
        let moved = active
            .window
            .and_then(|window| match platform::window_state(window) {
                WindowState::At(frame) => Some((frame.x, frame.y)),
                _ => None,
            })
            .filter(|origin| *origin != (active.area.x, active.area.y));
        if let Some((x, y)) = moved {
            active.area = RectI::new(x, y, active.area.w, active.area.h);
            active.outline.move_to(active.area);
            let center = (x + active.area.w / 2, y + active.area.h / 2);
            if let (Some(pill), Some(monitor)) =
                (active.pill, platform::monitor_at(center.0, center.1))
            {
                let at = pill_position(active.area, monitor.rect, monitor.scale);
                let (dx, dy) = (at.0 - active.pill_at.0, at.1 - active.pill_at.1);
                active.pill_at = at;
                let _ = pill.update(cx, |_, window, cx| platform::nudge(window, dx, dy, cx));
            }
        }
        true
    }
    /// Stops the capture and writes the file off the UI thread; `keep` false throws the recording away.
    fn stop_recording(&mut self, keep: bool, cx: &mut Context<Self>) {
        let Some(active) = self.recording.take() else {
            return;
        };
        self.show_recording_state(false);
        let ActiveRecording {
            recording, pill, ..
        } = active;
        if let (true, Some(pill)) = (keep, pill) {
            let _ = pill.update(cx, |pill, _, cx| pill.set_saving(cx));
        } else {
            close_pill(pill, cx);
        }
        let copy = self.config.copy_to_clipboard;
        self.recorder_busy = true;
        cx.spawn(async move |this, cx| {
            let outcome = cx
                .background_executor()
                .spawn(async move {
                    let saved = recording.finish(keep)?;
                    if let (true, Some(path)) = (copy, &saved) {
                        let _ = imaging::copy_file_to_clipboard(path);
                    }
                    Ok::<_, String>(saved)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.recorder_busy = false;
                close_pill(pill, cx);
                match outcome {
                    Ok(Some(path)) => {
                        let body = format!("{}. click to show it in its folder.", path.display());
                        this.last_saved = Some(path);
                        notify_tagged(SAVED_TAG, "recording saved", &body, cx);
                    }
                    Ok(None) => {}
                    Err(err) => notify("the recording was not saved", &err, cx),
                }
            });
        })
        .detach();
    }
}

fn close_pill(pill: Option<WindowHandle<RecordingPill>>, cx: &mut App) {
    if let Some(pill) = pill {
        let _ = pill.update(cx, |_, window, _| window.remove_window());
    }
}

/// Puts the control just under the recorded area, or above or inside it when there is no room.
fn pill_position(area: RectI, monitor: RectI, scale: f32) -> (i32, i32) {
    let (width, height) = (
        (PILL_WIDTH * scale).round() as i32,
        (PILL_HEIGHT * scale).round() as i32,
    );
    let gap = (10.0 * scale).round() as i32;
    let left = monitor.x + gap;
    let x = area.x.min(monitor.right() - width - gap).max(left);
    let (below, above) = (area.bottom() + gap, area.y - gap - height);
    let y = if below + height + gap <= monitor.bottom() {
        below
    } else if above >= monitor.y + gap {
        above
    } else {
        area.bottom() - gap - height
    };
    (x, y)
}

fn open_pill(
    at: (i32, i32),
    monitor: platform::Monitor,
    format: RecordFormat,
    cx: &mut App,
) -> Option<WindowHandle<RecordingPill>> {
    let bounds = Bounds::new(
        point(
            px(at.0 as f32 / monitor.scale),
            px(at.1 as f32 / monitor.scale),
        ),
        size(px(PILL_WIDTH), px(PILL_HEIGHT)),
    );
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: None,
        focus: false,
        kind: WindowKind::PopUp,
        is_movable: false,
        is_resizable: false,
        is_minimizable: false,
        display_id: Some(DisplayId::new(monitor.id)),
        ..Default::default()
    };
    cx.open_window(options, |window, cx| {
        platform::hide_from_capture(window, cx);
        cx.new(|cx| RecordingPill::new(format, cx))
    })
    .map_err(|err| log::error!("could not open the recording control: {err}"))
    .ok()
}
fn store(image: &RgbaImage, copy: bool, path: Option<&std::path::Path>) -> Result<(), String> {
    if copy {
        imaging::copy_to_clipboard(image)?;
    }
    if let Some(path) = path {
        imaging::save_png(image, path)?;
    }
    Ok(())
}

fn stored_message(copied: bool, path: Option<PathBuf>) -> String {
    let saved = path.map(|path| format!("saved to {}", path.display()));
    match (copied, saved) {
        (true, Some(saved)) => format!("copied to the clipboard and {saved}."),
        (true, None) => "copied to the clipboard.".into(),
        (false, Some(saved)) => format!("{saved}."),
        (false, None) => String::new(),
    }
}

fn notify_tagged(tag: &'static str, title: &str, body: &str, cx: &App) {
    cx.show_system_notification(SystemNotification {
        tag: tag.into(),
        title: title.to_string().into(),
        body: body.to_string().into(),
        actions: Vec::new(),
    });
}

pub fn notify(title: &str, body: &str, cx: &App) {
    notify_tagged("bawkshot", title, body, cx);
}

#[cfg(test)]
mod tests {
    use super::pill_position;
    use crate::geom::RectI;

    const MONITOR: RectI = RectI {
        x: 0,
        y: 0,
        w: 1920,
        h: 1080,
    };

    #[test]
    fn the_control_sits_under_the_area_when_there_is_room() {
        let area = RectI::new(400, 200, 640, 360);
        assert_eq!(pill_position(area, MONITOR, 1.0), (400, 570));
    }

    #[test]
    fn the_control_moves_above_an_area_at_the_bottom_edge() {
        let area = RectI::new(400, 700, 640, 380);
        assert_eq!(pill_position(area, MONITOR, 1.0), (400, 650));
    }

    #[test]
    fn the_control_goes_inside_a_full_monitor_area_and_stays_on_screen() {
        assert_eq!(pill_position(MONITOR, MONITOR, 1.0), (10, 1030));
        let right_edge = RectI::new(1800, 200, 120, 300);
        assert_eq!(pill_position(right_edge, MONITOR, 1.0).0, 1694);
    }
}
