use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::*;

use crate::geom::RectI;
use crate::platform::{Monitor, SnapTarget};
use crate::theme::palette;

/// A press that moves less than this is a click on the window under the pointer, not a drag.
const CLICK_SLOP: f32 = 4.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RegionEvent {
    /// window is set when the area came from a click on a window, not from a drag.
    Picked {
        rect: RectI,
        window: Option<isize>,
    },
    Cancelled,
}

/// One monitor's frozen frame, where the user drags a region or clicks a window.
pub struct RegionOverlay {
    frame: Arc<RenderImage>,
    monitor: Monitor,
    targets: Rc<Vec<SnapTarget>>,
    anchor: Option<Point<Pixels>>,
    pointer: Point<Pixels>,
    hovered: bool,
    focus: FocusHandle,
}

impl EventEmitter<RegionEvent> for RegionOverlay {}

impl RegionOverlay {
    pub fn new(
        frame: Arc<RenderImage>,
        monitor: Monitor,
        targets: Rc<Vec<SnapTarget>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        Self {
            frame,
            monitor,
            targets,
            anchor: None,
            pointer: point(px(-1.), px(-1.)),
            hovered: false,
            focus,
        }
    }

    fn scale(&self) -> f32 {
        self.monitor.scale
    }

    /// Window pixels to screen pixels.
    fn to_screen(&self, position: Point<Pixels>) -> (i32, i32) {
        (
            self.monitor.rect.x + (position.x.as_f32() * self.scale()).round() as i32,
            self.monitor.rect.y + (position.y.as_f32() * self.scale()).round() as i32,
        )
    }

    /// Screen pixels to window pixels.
    fn to_window(&self, rect: RectI) -> Bounds<Pixels> {
        let scale = self.scale();
        Bounds::new(
            point(
                px((rect.x - self.monitor.rect.x) as f32 / scale),
                px((rect.y - self.monitor.rect.y) as f32 / scale),
            ),
            size(px(rect.w as f32 / scale), px(rect.h as f32 / scale)),
        )
    }

    fn dragged(&self) -> Option<RectI> {
        let anchor = self.anchor?;
        let moved = (self.pointer.x - anchor.x)
            .as_f32()
            .hypot((self.pointer.y - anchor.y).as_f32());
        if moved < CLICK_SLOP {
            return None;
        }
        let (ax, ay) = self.to_screen(anchor);
        let (bx, by) = self.to_screen(self.pointer);
        let rect = RectI::new(ax.min(bx), ay.min(by), (ax - bx).abs(), (ay - by).abs());
        Some(rect.intersect(&self.monitor.rect))
    }

    fn snapped(&self) -> SnapTarget {
        let (x, y) = self.to_screen(self.pointer);
        let whole = SnapTarget {
            rect: self.monitor.rect,
            window: None,
        };
        self.targets
            .iter()
            .find(|target| target.rect.contains(x, y))
            .map(|target| SnapTarget {
                rect: target.rect.intersect(&self.monitor.rect),
                window: target.window,
            })
            .filter(|target| !target.rect.is_empty())
            .unwrap_or(whole)
    }

    fn selection(&self) -> Option<RectI> {
        if !self.hovered && self.anchor.is_none() {
            return None;
        }
        Some(self.dragged().unwrap_or_else(|| self.snapped().rect))
    }

    fn on_mouse_down(&mut self, event: &MouseDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.pointer = event.position;
        self.anchor = Some(event.position);
        cx.notify();
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.pointer = event.position;
        self.hovered = true;
        cx.notify();
    }

    fn on_mouse_up(&mut self, event: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.anchor.is_none() {
            return;
        }
        self.pointer = event.position;
        let (rect, window) = match self.dragged() {
            Some(rect) => (rect, None),
            None => {
                let target = self.snapped();
                (target.rect, target.window)
            }
        };
        self.anchor = None;
        if rect.is_empty() {
            cx.notify();
        } else {
            cx.emit(RegionEvent::Picked { rect, window });
        }
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        if event.keystroke.key == "escape" {
            cx.emit(RegionEvent::Cancelled);
        }
    }
}

impl Render for RegionOverlay {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let frame = self.frame.clone();
        let selection = self.selection();
        let picked = selection.map(|rect| self.to_window(rect));
        let pointer = self.pointer;
        let crosshair = self.hovered && self.anchor.is_none();
        let shade = hsla(0., 0., 0., 0.45);
        let line = hsla(0., 0., 1., 0.35);
        let accent = p.yolk;

        let label = selection.map(|rect| {
            div()
                .absolute()
                .left(pointer.x + px(16.))
                .top(pointer.y + px(20.))
                .px(px(8.))
                .py(px(3.))
                .rounded(px(4.))
                .bg(hsla(0., 0., 0.07, 0.92))
                .text_size(px(12.))
                .text_color(rgb(0xf3efee))
                .child(format!("{} × {}", rect.w, rect.h))
        });

        div()
            .id("region")
            .size_full()
            .relative()
            .track_focus(&self.focus)
            .cursor(CursorStyle::Crosshair)
            .font_family("IBM Plex Mono")
            .on_key_down(cx.listener(Self::on_key_down))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|_, _, _, cx| cx.emit(RegionEvent::Cancelled)),
            )
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                this.hovered = *hovered;
                cx.notify();
            }))
            .child(
                canvas(
                    |_, _, _| (),
                    move |bounds, _, window, _| {
                        let _ = window.paint_image(
                            bounds,
                            bounds,
                            Corners::default(),
                            frame.clone(),
                            0,
                            false,
                        );
                        let Some(picked) = picked else {
                            window.paint_quad(fill(bounds, shade));
                            return;
                        };
                        let (left, top) = (picked.left(), picked.top());
                        let (right, bottom) = (picked.right(), picked.bottom());
                        let band = |x: Pixels, y: Pixels, right: Pixels, bottom: Pixels| {
                            Bounds::from_corners(point(x, y), point(right, bottom))
                        };
                        for quad in [
                            band(bounds.left(), bounds.top(), bounds.right(), top),
                            band(bounds.left(), bottom, bounds.right(), bounds.bottom()),
                            band(bounds.left(), top, left, bottom),
                            band(right, top, bounds.right(), bottom),
                        ] {
                            window.paint_quad(fill(quad, shade));
                        }
                        if crosshair {
                            window.paint_quad(fill(
                                band(bounds.left(), pointer.y, bounds.right(), pointer.y + px(1.)),
                                line,
                            ));
                            window.paint_quad(fill(
                                band(pointer.x, bounds.top(), pointer.x + px(1.), bounds.bottom()),
                                line,
                            ));
                        }
                        window.paint_quad(quad(
                            picked,
                            px(0.),
                            transparent_black(),
                            px(2.),
                            accent,
                            BorderStyle::Solid,
                        ));
                    },
                )
                .absolute()
                .size_full(),
            )
            .children(label)
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use gpui_kit::{Entity, Modifiers, MouseButton, TestAppContext, VisualTestContext, point, px};
    use image::RgbaImage;

    use super::{RegionEvent, RegionOverlay};
    use crate::geom::RectI;
    use crate::platform::{Monitor, SnapTarget};

    type Events = Rc<RefCell<Vec<RegionEvent>>>;

    fn overlay(
        monitor: Monitor,
        targets: Vec<SnapTarget>,
        cx: &mut TestAppContext,
    ) -> (Entity<RegionOverlay>, Events, &mut VisualTestContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::theme::init(cx);
        });
        let frame = crate::imaging::render_image(RgbaImage::new(8, 8));
        let (view, cx) = cx.add_window_view(|window, cx| {
            RegionOverlay::new(frame, monitor, Rc::new(targets), window, cx)
        });
        let events = Events::default();
        let seen = events.clone();
        cx.update(|_, cx| {
            cx.subscribe(&view, move |_, event: &RegionEvent, _| {
                seen.borrow_mut().push(*event)
            })
            .detach()
        });
        (view, events, cx)
    }

    fn monitor(x: i32, scale: f32) -> Monitor {
        Monitor {
            id: 1,
            rect: RectI::new(x, 0, 1920, 1080),
            scale,
        }
    }

    fn picked(rect: RectI, window: Option<isize>) -> RegionEvent {
        RegionEvent::Picked { rect, window }
    }

    fn drag(cx: &mut VisualTestContext, from: (f32, f32), to: (f32, f32)) {
        let (from, to) = (point(px(from.0), px(from.1)), point(px(to.0), px(to.1)));
        cx.simulate_mouse_down(from, MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_move(to, MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_up(to, MouseButton::Left, Modifiers::none());
    }

    #[gpui_kit::test]
    fn a_drag_picks_the_dragged_area(cx: &mut TestAppContext) {
        let (_view, events, cx) = overlay(monitor(0, 1.0), Vec::new(), cx);
        drag(cx, (300.0, 250.0), (100.0, 100.0));
        assert_eq!(
            events.borrow().as_slice(),
            &[picked(RectI::new(100, 100, 200, 150), None)]
        );
    }

    #[gpui_kit::test]
    fn a_drag_on_a_scaled_second_monitor_lands_in_screen_pixels(cx: &mut TestAppContext) {
        let (_view, events, cx) = overlay(monitor(1920, 2.0), Vec::new(), cx);
        drag(cx, (10.0, 10.0), (110.0, 60.0));
        assert_eq!(
            events.borrow().as_slice(),
            &[picked(RectI::new(1940, 20, 200, 100), None)]
        );
    }

    #[gpui_kit::test]
    fn a_click_picks_the_frontmost_window_under_the_pointer(cx: &mut TestAppContext) {
        let front = SnapTarget {
            rect: RectI::new(50, 50, 400, 300),
            window: Some(11),
        };
        let behind = SnapTarget {
            rect: RectI::new(0, 0, 900, 700),
            window: Some(22),
        };
        let (_view, events, cx) = overlay(monitor(0, 1.0), vec![front, behind], cx);
        cx.simulate_click(point(px(60.), px(60.)), Modifiers::none());
        cx.simulate_click(point(px(600.), px(600.)), Modifiers::none());
        cx.simulate_click(point(px(1500.), px(900.)), Modifiers::none());
        drag(cx, (60.0, 60.0), (200.0, 160.0));
        assert_eq!(
            events.borrow().as_slice(),
            &[
                picked(front.rect, Some(11)),
                picked(behind.rect, Some(22)),
                picked(RectI::new(0, 0, 1920, 1080), None),
                picked(RectI::new(60, 60, 140, 100), None),
            ]
        );
    }

    #[gpui_kit::test]
    fn escape_and_right_click_cancel(cx: &mut TestAppContext) {
        let (_view, events, cx) = overlay(monitor(0, 1.0), Vec::new(), cx);
        cx.simulate_keystrokes("escape");
        cx.simulate_mouse_down(
            point(px(40.), px(40.)),
            MouseButton::Right,
            Modifiers::none(),
        );
        assert_eq!(
            events.borrow().as_slice(),
            &[RegionEvent::Cancelled, RegionEvent::Cancelled]
        );
    }
}
