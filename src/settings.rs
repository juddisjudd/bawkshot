use std::path::PathBuf;

use gpui_kit::assets::IconName;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{Icon, Sizable, TitleBar};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::assets::MARK;
use crate::config::{Config, GIF_RATES, Trigger, VIDEO_RATES};
use crate::shell::{Shell, shell};
use crate::theme::{self, Mode, Palette, palette};
use crate::updater::VERSION;
use crate::updates::{UpdateState, Updates};
use crate::{autostart, platform, recorder};

/// Keeps its own copy of the settings: the shell can be mid-update, and so unreadable, while this window draws.
pub struct SettingsView {
    shell: Entity<Shell>,
    config: Config,
    errors: Vec<(Trigger, String)>,
    ffmpeg: Option<PathBuf>,
    autostart: bool,
    autostart_error: Option<String>,
    updates: Entity<Updates>,
    update: UpdateState,
    save_dir: Entity<InputState>,
    ffmpeg_path: Entity<InputState>,
    hotkeys: Vec<(Trigger, Entity<InputState>)>,
    _subscriptions: Vec<Subscription>,
}

pub fn open(
    config: Config,
    errors: Vec<(Trigger, String)>,
    updates: Entity<Updates>,
    cx: &mut App,
) -> Option<AnyWindowHandle> {
    let bounds = Bounds::centered(None, size(px(640.), px(760.)), cx);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        window_min_size: Some(size(px(540.), px(420.))),
        app_id: Some("bawkshot".into()),
        ..TitleBar::window_options()
    };
    let opened = open_window(options, cx, |window, cx| {
        window.set_window_title("bawkshot settings");
        cx.new(|cx| SettingsView::new(config, errors, updates, window, cx))
    });
    let (handle, _) = opened
        .map_err(|err| log::error!("could not open settings: {err}"))
        .ok()?;
    let _ = handle.update(cx, |_, window, cx| {
        window.activate_window();
        platform::bring_to_front(window, cx);
    });
    cx.activate(true);
    Some(handle)
}

fn committed(event: &InputEvent) -> bool {
    matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur)
}

impl SettingsView {
    fn new(
        config: Config,
        errors: Vec<(Trigger, String)>,
        updates: Entity<Updates>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let shell = shell(cx);
        let save_dir = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(config.save_dir.to_string_lossy().into_owned())
        });
        let ffmpeg_path = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("found automatically")
                .default_value(config.ffmpeg_path.clone())
        });
        let mut subscriptions = vec![
            cx.observe(&shell, |this, shell, cx| {
                let state = shell.read(cx);
                if state.config.ffmpeg_path != this.config.ffmpeg_path {
                    this.ffmpeg = recorder::locate(&state.config.ffmpeg_path);
                }
                this.config = state.config.clone();
                this.errors = state.hotkey_errors.clone();
                cx.notify();
            }),
            cx.observe(&updates, |this, updates, cx| {
                this.update = updates.read(cx).state.clone();
                cx.notify();
            }),
            cx.subscribe_in(
                &save_dir,
                window,
                |this, input, event: &InputEvent, _, cx| {
                    let value = input.read(cx).value().trim().to_string();
                    if committed(event) && !value.is_empty() {
                        this.change(cx, move |config| config.save_dir = PathBuf::from(value));
                    }
                },
            ),
            cx.subscribe_in(
                &ffmpeg_path,
                window,
                |this, input, event: &InputEvent, _, cx| {
                    if committed(event) {
                        let value = input.read(cx).value().trim().to_string();
                        this.change(cx, move |config| config.ffmpeg_path = value);
                    }
                },
            ),
        ];
        let hotkeys: Vec<(Trigger, Entity<InputState>)> = Trigger::ALL
            .into_iter()
            .map(|trigger| {
                let input = cx.new(|cx| {
                    InputState::new(window, cx)
                        .placeholder("not set")
                        .default_value(config.hotkeys.get(trigger).to_string())
                });
                subscriptions.push(cx.subscribe_in(
                    &input,
                    window,
                    move |this, input, event: &InputEvent, _, cx| {
                        if committed(event) {
                            let keys = input.read(cx).value().trim().to_lowercase();
                            this.change(cx, move |config| config.hotkeys.set(trigger, keys));
                        }
                    },
                ));
                (trigger, input)
            })
            .collect();
        Self {
            shell,
            ffmpeg: recorder::locate(&config.ffmpeg_path),
            autostart: autostart::enabled(),
            autostart_error: None,
            update: updates.read(cx).state.clone(),
            updates,
            config,
            errors,
            save_dir,
            ffmpeg_path,
            hotkeys,
            _subscriptions: subscriptions,
        }
    }

    fn change(&mut self, cx: &mut Context<Self>, change: impl FnOnce(&mut Config)) {
        self.shell
            .update(cx, |shell, cx| shell.update_config(change, cx));
    }

    fn set_autostart(&mut self, on: bool, cx: &mut Context<Self>) {
        self.autostart_error = autostart::set(on).err();
        self.autostart = autostart::enabled();
        cx.notify();
    }

    /// Asks for a folder or a file, then puts it in `input` and stores it with `store`.
    fn browse(
        &mut self,
        input: Entity<InputState>,
        folder: bool,
        store: fn(&mut Config, PathBuf),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: !folder,
            directories: folder,
            multiple: false,
            prompt: Some(
                if folder {
                    "use this folder"
                } else {
                    "use this file"
                }
                .into(),
            ),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = paths.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let _ = this.update_in(cx, |this, window, cx| {
                input.update(cx, |state, cx| {
                    state.set_value(path.to_string_lossy().into_owned(), window, cx)
                });
                this.change(cx, move |config| store(config, path));
            });
        })
        .detach();
    }

    fn toggle(
        id: &'static str,
        label: &'static str,
        detail: &'static str,
        on: bool,
        set: fn(&mut Config, bool),
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> Div {
        row(label, detail, p).child(Switch::new(id).checked(on).on_click(cx.listener(
            move |this, checked: &bool, _, cx| {
                let checked = *checked;
                this.change(cx, move |config| set(config, checked));
            },
        )))
    }

    fn rates(
        group: &'static str,
        options: [u32; 3],
        current: u32,
        set: fn(&mut Config, u32),
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> Div {
        segmented(p).children(options.map(|rate| {
            segment(
                (group, rate as usize),
                format!("{rate} fps"),
                rate == current,
                p,
            )
            .on_click(
                cx.listener(move |this, _, _, cx| this.change(cx, move |config| set(config, rate))),
            )
        }))
    }

    fn path_field(
        &self,
        id: &'static str,
        input: &Entity<InputState>,
        folder: bool,
        store: fn(&mut Config, PathBuf),
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> Div {
        let target = input.clone();
        div()
            .flex()
            .items_center()
            .gap_2()
            .child(div().flex_1().child(Input::new(input)))
            .child(button(id, "browse", false, p).on_click(cx.listener(
                move |this, _, window, cx| this.browse(target.clone(), folder, store, window, cx),
            )))
    }

    /// This build's version, what the last check found, and the one step that comes next.
    fn update_row(&self, p: &Palette) -> Div {
        let (icon, color, text): (IconName, Hsla, String) = match &self.update {
            UpdateState::Idle => (IconName::RefreshCw, p.text_weak, "not checked yet".into()),
            UpdateState::Checking => (
                IconName::RefreshCw,
                p.text_weak,
                "checking for updates…".into(),
            ),
            UpdateState::Current => (
                IconName::CircleCheck,
                p.text_weak,
                "you have the newest version".into(),
            ),
            UpdateState::Available(release) => (
                IconName::ArrowDownToLine,
                p.yolk,
                format!("bawkshot {} is out", release.version),
            ),
            UpdateState::Downloading {
                version,
                done,
                total,
            } => (
                IconName::ArrowDownToLine,
                p.yolk,
                format!(
                    "downloading {version}… {} of {} MB",
                    megabytes(*done),
                    megabytes(*total)
                ),
            ),
            UpdateState::Ready { version, .. } => (
                IconName::CircleCheck,
                p.yolk,
                format!(
                    "{version} is installed. restart to use it. a running recording is saved first."
                ),
            ),
            UpdateState::Failed(error) => (IconName::CircleAlert, p.danger, error.clone()),
        };
        let updates = self.updates.clone();
        let action = match &self.update {
            UpdateState::Idle | UpdateState::Current => Some(
                button("check-now", "check now", false, p)
                    .on_click(move |_, _, cx| updates.update(cx, |updates, cx| updates.check(cx))),
            ),
            UpdateState::Failed(_) => Some(
                button("check-now", "try again", false, p)
                    .on_click(move |_, _, cx| updates.update(cx, |updates, cx| updates.check(cx))),
            ),
            UpdateState::Available(_) => Some(
                button("install-update", "update", true, p).on_click(move |_, _, cx| {
                    updates.update(cx, |updates, cx| updates.install(cx))
                }),
            ),
            UpdateState::Ready { .. } => Some(
                button("restart", "restart now", true, p).on_click(move |_, _, cx| {
                    updates.update(cx, |updates, cx| updates.restart(cx))
                }),
            ),
            UpdateState::Checking | UpdateState::Downloading { .. } => None,
        };
        let notes = match &self.update {
            UpdateState::Available(release) => Some(release.page.clone()),
            _ => None,
        };
        let fraction = match &self.update {
            UpdateState::Downloading { done, total, .. } => {
                Some((*done as f32 / (*total).max(1) as f32).min(1.0))
            }
            _ => None,
        };
        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_4()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .min_w_0()
                            .child(
                                div()
                                    .text_color(p.text_strong)
                                    .child(format!("bawkshot {VERSION}")),
                            )
                            .child(
                                div()
                                    .flex()
                                    .gap(px(6.))
                                    .child(
                                        div()
                                            .flex_none()
                                            .pt(px(3.))
                                            .child(Icon::new(icon).xsmall().text_color(color)),
                                    )
                                    .child(note(text, color).flex_1().min_w_0()),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .gap_2()
                            .children(notes.map(|page| {
                                button("release-notes", "what's new", false, p)
                                    .on_click(move |_, _, cx| cx.open_url(&page))
                            }))
                            .children(action),
                    ),
            )
            .children(fraction.map(|fraction| {
                div()
                    .h(px(3.))
                    .w_full()
                    .rounded_full()
                    .bg(p.border_weak)
                    .child(
                        div()
                            .h_full()
                            .rounded_full()
                            .bg(p.yolk)
                            .w(relative(fraction)),
                    )
            }))
    }
}

fn megabytes(bytes: u64) -> String {
    format!("{:.1}", bytes as f64 / (1024.0 * 1024.0))
}

fn button(id: &'static str, label: &'static str, primary: bool, p: &Palette) -> Stateful<Div> {
    div()
        .id(id)
        .h(px(32.))
        .px(px(12.))
        .flex()
        .flex_none()
        .items_center()
        .rounded(px(4.))
        .cursor_pointer()
        .child(label)
        .map(|this| {
            if primary {
                this.bg(p.yolk)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(p.bg)
                    .hover(|style| style.opacity(0.88))
            } else {
                this.border_1()
                    .border_color(p.border_weak)
                    .text_color(p.text_strong)
                    .hover(|style| style.bg(p.bg_hover))
            }
        })
}

fn section(title: &'static str, p: &Palette) -> Div {
    div().flex().flex_col().gap(px(14.)).child(
        div()
            .text_size(px(11.))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(p.text_weaker)
            .child(title),
    )
}

fn row(label: &'static str, detail: &'static str, p: &Palette) -> Div {
    div().flex().items_center().justify_between().gap_4().child(
        div()
            .flex()
            .flex_col()
            .gap(px(2.))
            .min_w_0()
            .child(div().text_color(p.text_strong).child(label))
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(p.text_weak)
                    .child(detail),
            ),
    )
}

fn note(text: impl Into<SharedString>, color: Hsla) -> Div {
    div()
        .text_size(px(12.))
        .text_color(color)
        .child(text.into())
}

fn segmented(p: &Palette) -> Div {
    div()
        .flex()
        .flex_none()
        .p(px(2.))
        .gap(px(2.))
        .rounded(px(4.))
        .border_1()
        .border_color(p.border_weak)
}

fn segment(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    active: bool,
    p: &Palette,
) -> Stateful<Div> {
    div()
        .id(id)
        .px(px(12.))
        .py(px(3.))
        .rounded(px(3.))
        .cursor_pointer()
        .text_color(if active { p.text_strong } else { p.text_weak })
        .when(active, |this| this.bg(p.bg_hover))
        .hover(|style| style.text_color(p.text_strong))
        .child(label.into())
}

impl Render for SettingsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let config = self.config.clone();
        let errors = self.errors.clone();

        let after = section("after a capture", &p)
            .child(Self::toggle(
                "open-editor",
                "open the editor",
                "mark the capture up before it goes anywhere.",
                config.open_editor,
                |config, on| config.open_editor = on,
                &p,
                cx,
            ))
            .child(Self::toggle(
                "copy",
                "copy to the clipboard",
                "an image is copied as pixels, a recording as a file.",
                config.copy_to_clipboard,
                |config, on| config.copy_to_clipboard = on,
                &p,
                cx,
            ))
            .child(Self::toggle(
                "save",
                "save captures to the folder below",
                "every capture is written as a png. recordings always go here.",
                config.save_to_disk,
                |config, on| config.save_to_disk = on,
                &p,
                cx,
            ))
            .child(self.path_field(
                "browse-folder",
                &self.save_dir,
                true,
                |config, path| config.save_dir = path,
                &p,
                cx,
            ));

        let capture = section("capture", &p).child(Self::toggle(
            "cursor",
            "include the mouse pointer",
            "draws the pointer into a screenshot.",
            config.capture_cursor,
            |config, on| config.capture_cursor = on,
            &p,
            cx,
        ));

        let ffmpeg = match &self.ffmpeg {
            Some(path) => note(format!("using {}", path.display()), p.text_weak),
            None if config.ffmpeg_path.trim().is_empty() => note(
                "ffmpeg was not found. install it (for example: winget install ffmpeg) or set its path below.",
                p.danger,
            ),
            None => note("there is no file at that path.", p.danger),
        };
        let recording = section("recording", &p)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(div().text_color(p.text_strong).child("ffmpeg"))
                    .child(ffmpeg),
            )
            .child(self.path_field(
                "browse-ffmpeg",
                &self.ffmpeg_path,
                false,
                |config, path| config.ffmpeg_path = path.to_string_lossy().into_owned(),
                &p,
                cx,
            ))
            .child(
                row("video frame rate", "higher is smoother and larger.", &p).child(Self::rates(
                    "video-rate",
                    VIDEO_RATES,
                    config.video_fps,
                    |config, rate| config.video_fps = rate,
                    &p,
                    cx,
                )),
            )
            .child(
                row("gif frame rate", "gifs grow fast, so keep this low.", &p).child(Self::rates(
                    "gif-rate",
                    GIF_RATES,
                    config.gif_fps,
                    |config, rate| config.gif_fps = rate,
                    &p,
                    cx,
                )),
            )
            .child(Self::toggle(
                "record-cursor",
                "include the mouse pointer",
                "draws the pointer into a recording.",
                config.record_cursor,
                |config, on| config.record_cursor = on,
                &p,
                cx,
            ));

        let hotkeys = section("hotkeys", &p)
            .child(note(
                "written like ctrl+shift+printscreen or alt+f9. press enter to apply, leave empty for none.",
                p.text_weak,
            ))
            .children(self.hotkeys.iter().map(|(trigger, input)| {
                let error = errors
                    .iter()
                    .find(|(failed, _)| failed == trigger)
                    .map(|(_, message)| message.clone());
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(
                        row(trigger.label(), trigger.hint(), &p)
                            .child(div().w(px(220.)).flex_none().child(Input::new(input))),
                    )
                    .children(error.map(|message| note(message, p.danger)))
            }));

        let autostart = self.autostart;
        let app = section("app", &p)
            .child(
                row(
                    "start with windows",
                    "puts bawkshot in the tray when you sign in.",
                    &p,
                )
                .child(Switch::new("autostart").checked(autostart).on_click(
                    cx.listener(|this, checked: &bool, _, cx| this.set_autostart(*checked, cx)),
                )),
            )
            .children(
                self.autostart_error
                    .clone()
                    .map(|message| note(message, p.danger)),
            )
            .child(
                row("theme", "follows windows when set to system.", &p).child(
                    segmented(&p).children(Mode::ALL.map(|mode| {
                        segment(mode.label(), mode.label(), config.mode == mode, &p).on_click(
                            cx.listener(move |this, _, window, cx| {
                                theme::apply(mode, Some(window), cx);
                                this.change(cx, move |config| config.mode = mode);
                            }),
                        )
                    })),
                ),
            );

        let about = section("about", &p)
            .child(self.update_row(&p))
            .child(Self::toggle(
                "check-updates",
                "check for updates",
                "looks on github when bawkshot starts, then once a day.",
                config.check_updates,
                |config, on| config.check_updates = on,
                &p,
                cx,
            ));

        let title = div()
            .flex()
            .items_center()
            .gap(px(7.))
            .child(svg().path(MARK).size(px(16.)).text_color(p.text_strong))
            .child(
                div()
                    .text_size(px(14.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(p.text_strong)
                    .child("settings"),
            );

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(p.bg)
            .text_color(p.text)
            .text_size(px(13.))
            .child(
                TitleBar::new()
                    .h(px(40.))
                    .bg(p.bg)
                    .border_color(p.border_weak)
                    .child(title),
            )
            .child(
                div().flex_1().min_h_0().child(
                    div().size_full().overflow_y_scrollbar().child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(30.))
                            .p(px(28.))
                            .child(after)
                            .child(capture)
                            .child(recording)
                            .child(hotkeys)
                            .child(app)
                            .child(about),
                    ),
                ),
            )
    }
}
