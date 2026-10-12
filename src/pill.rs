use std::time::{Duration, Instant};

use gpui_kit::assets::IconName;
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::config::RecordFormat;
use crate::theme::palette;

pub const PILL_WIDTH: f32 = 216.0;
pub const PILL_HEIGHT: f32 = 40.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PillEvent {
    Stop,
    Discard,
}

/// The small always-on-top control shown while a recording runs: elapsed time, stop and discard.
pub struct RecordingPill {
    format: RecordFormat,
    started: Instant,
    saving: bool,
    _tick: Task<()>,
}

impl EventEmitter<PillEvent> for RecordingPill {}

impl RecordingPill {
    pub fn new(format: RecordFormat, cx: &mut Context<Self>) -> Self {
        let tick = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(250))
                    .await;
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        });
        Self {
            format,
            started: Instant::now(),
            saving: false,
            _tick: tick,
        }
    }

    /// Shown from the moment the capture stops until its file is written, which for a gif takes a while.
    pub fn set_saving(&mut self, cx: &mut Context<Self>) {
        self.saving = true;
        cx.notify();
    }

    fn label(&self) -> String {
        if self.saving {
            return format!("saving {}…", self.format.label());
        }
        let seconds = self.started.elapsed().as_secs();
        format!(
            "{} {}:{:02}",
            self.format.label(),
            seconds / 60,
            seconds % 60
        )
    }
}

impl Render for RecordingPill {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let blink = self.saving || (self.started.elapsed().as_millis() / 500).is_multiple_of(2);
        let button = |id: &'static str, icon: IconName, color: Hsla| {
            div()
                .id(id)
                .size(px(28.))
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .rounded(px(6.))
                .cursor_pointer()
                .hover(|style| style.bg(p.bg_hover))
                .child(Icon::new(icon).small().text_color(color))
                .debug_selector(move || format!("pill-{id}"))
        };
        div()
            .size_full()
            .flex()
            .items_center()
            .gap(px(8.))
            .pl(px(14.))
            .pr(px(6.))
            .bg(p.bg_weak)
            .border_1()
            .border_color(p.border_weak)
            .font_family("IBM Plex Mono")
            .text_size(px(13.))
            .text_color(p.text_strong)
            .child(
                div()
                    .size(px(9.))
                    .flex_none()
                    .rounded_full()
                    .bg(if self.saving { p.yolk } else { p.danger })
                    .opacity(if blink { 1.0 } else { 0.25 }),
            )
            .child(div().flex_1().child(self.label()))
            .when(!self.saving, |this| {
                this.child(
                    button("stop", IconName::Square, p.text_strong)
                        .on_click(cx.listener(|_, _, _, cx| cx.emit(PillEvent::Stop))),
                )
                .child(
                    button("discard", IconName::Trash, p.text_weak)
                        .on_click(cx.listener(|_, _, _, cx| cx.emit(PillEvent::Discard))),
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use gpui_kit::{Modifiers, TestAppContext};

    use super::{PillEvent, RecordingPill};
    use crate::config::RecordFormat;

    #[gpui_kit::test]
    fn the_buttons_stop_or_discard_and_go_away_while_saving(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::theme::init(cx);
        });
        let (view, cx) = cx.add_window_view(|_, cx| RecordingPill::new(RecordFormat::Gif, cx));
        let events: Rc<RefCell<Vec<PillEvent>>> = Rc::default();
        let seen = events.clone();
        cx.update(|_, cx| {
            cx.subscribe(&view, move |_, event: &PillEvent, _| {
                seen.borrow_mut().push(*event)
            })
            .detach()
        });
        for name in ["pill-stop", "pill-discard"] {
            let button = cx.debug_bounds(name).expect("the button is drawn");
            cx.simulate_click(button.center(), Modifiers::none());
        }
        assert_eq!(
            events.borrow().as_slice(),
            &[PillEvent::Stop, PillEvent::Discard]
        );

        view.update(cx, |pill, cx| pill.set_saving(cx));
        cx.run_until_parked();
        assert!(cx.debug_bounds("pill-stop").is_none());
        assert_eq!(view.read_with(cx, |pill, _| pill.label()), "saving gif…");
    }
}
