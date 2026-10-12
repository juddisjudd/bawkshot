use std::sync::Arc;

use gpui_kit::{Hsla, Rgba};

use crate::geom::{Pt, RectF, pt};

pub const HIGHLIGHT_OPACITY: f32 = 0.4;
pub const HIGHLIGHT_WIDTH: f32 = 4.0;
pub const LINE_HEIGHT: f32 = 1.3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Color {
    pub const fn hex(value: u32) -> Self {
        Self {
            r: (value >> 16) as u8,
            g: (value >> 8) as u8,
            b: value as u8,
        }
    }

    pub fn css(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }

    pub fn hsla(self) -> Hsla {
        Rgba {
            r: self.r as f32 / 255.0,
            g: self.g as f32 / 255.0,
            b: self.b as f32 / 255.0,
            a: 1.0,
        }
        .into()
    }

    /// Black or white, whichever reads better on top of this color.
    pub fn contrast(self) -> Color {
        let luma = 0.299 * self.r as f32 + 0.587 * self.g as f32 + 0.114 * self.b as f32;
        if luma > 150.0 {
            Color::hex(0x151313)
        } else {
            Color::hex(0xffffff)
        }
    }
}

pub const PALETTE: [Color; 8] = [
    Color::hex(0xff4d4f),
    Color::hex(0xff9a3d),
    Color::hex(0xffd43b),
    Color::hex(0x37d67a),
    Color::hex(0x3d9bff),
    Color::hex(0xb07cff),
    Color::hex(0xffffff),
    Color::hex(0x151313),
];

#[derive(Clone, Debug, PartialEq)]
pub enum Shape {
    Rect(RectF),
    Ellipse(RectF),
    Line(Pt, Pt),
    Arrow(Pt, Pt),
    Pen(Arc<Vec<Pt>>),
    Highlighter(Arc<Vec<Pt>>),
    /// `extent` is the measured size of the text block, kept so hit tests need no text system.
    Text {
        origin: Pt,
        text: Arc<str>,
        extent: (f32, f32),
    },
    Step {
        center: Pt,
        number: u32,
    },
    Blur(RectF),
    Pixelate(RectF),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handle {
    North,
    South,
    East,
    West,
    NorthEast,
    NorthWest,
    SouthEast,
    SouthWest,
    Start,
    End,
}

/// One piece of markup. `size` is the font size for text and steps, and the strength of an effect.
#[derive(Clone, Debug, PartialEq)]
pub struct Element {
    pub id: u64,
    pub shape: Shape,
    pub color: Color,
    pub fill: bool,
    pub width: f32,
    pub size: f32,
}

pub struct ArrowGeometry {
    pub shaft_end: Pt,
    pub head: [Pt; 3],
}

/// The shaft stops inside the head so the round cap never pokes past the tip.
pub fn arrow_geometry(a: Pt, b: Pt, width: f32) -> ArrowGeometry {
    let length = a.distance(b).max(0.001);
    let head = (width * 4.5).max(10.0).min(length * 0.7);
    let (dx, dy) = ((b.x - a.x) / length, (b.y - a.y) / length);
    let base = pt(b.x - dx * head, b.y - dy * head);
    let half = head * 0.5;
    ArrowGeometry {
        shaft_end: pt(b.x - dx * head * 0.8, b.y - dy * head * 0.8),
        head: [
            b,
            pt(base.x - dy * half, base.y + dx * half),
            pt(base.x + dy * half, base.y - dx * half),
        ],
    }
}

fn points_bounds(points: &[Pt]) -> RectF {
    let Some(first) = points.first() else {
        return RectF::default();
    };
    let (mut left, mut top, mut right, mut bottom) = (first.x, first.y, first.x, first.y);
    for p in points {
        left = left.min(p.x);
        top = top.min(p.y);
        right = right.max(p.x);
        bottom = bottom.max(p.y);
    }
    RectF::new(left, top, right - left, bottom - top)
}

fn polyline_distance(points: &[Pt], p: Pt) -> f32 {
    match points {
        [] => f32::MAX,
        [only] => p.distance(*only),
        _ => points
            .windows(2)
            .map(|pair| p.distance_to_segment(pair[0], pair[1]))
            .fold(f32::MAX, f32::min),
    }
}

impl Element {
    pub fn is_effect(&self) -> bool {
        matches!(self.shape, Shape::Blur(_) | Shape::Pixelate(_))
    }

    pub fn is_text(&self) -> bool {
        matches!(self.shape, Shape::Text { .. })
    }

    pub fn step_radius(&self) -> f32 {
        self.size * 0.9
    }

    pub fn text_padding(&self) -> (f32, f32) {
        (self.size * 0.4, self.size * 0.12)
    }

    /// How wide the stroke is drawn, which for a highlighter is several times its nominal width.
    pub fn stroke_width(&self) -> f32 {
        match self.shape {
            Shape::Highlighter(_) => self.width * HIGHLIGHT_WIDTH,
            _ => self.width,
        }
    }

    /// The area the element covers, with its stroke.
    pub fn bounds(&self) -> RectF {
        let half = self.stroke_width() / 2.0;
        match &self.shape {
            Shape::Rect(rect) | Shape::Ellipse(rect) if self.fill => *rect,
            Shape::Rect(rect) | Shape::Ellipse(rect) => rect.inflate(half),
            Shape::Blur(rect) | Shape::Pixelate(rect) => *rect,
            Shape::Line(a, b) => RectF::from_points(*a, *b).inflate(half),
            Shape::Arrow(a, b) => {
                let geometry = arrow_geometry(*a, *b, self.width);
                points_bounds(&geometry.head)
                    .union(&RectF::from_points(*a, *b))
                    .inflate(half)
            }
            Shape::Pen(points) | Shape::Highlighter(points) => points_bounds(points).inflate(half),
            Shape::Text { origin, extent, .. } => {
                let rect = RectF::new(origin.x, origin.y, extent.0, extent.1);
                if self.fill {
                    let (pad_x, pad_y) = self.text_padding();
                    RectF::new(
                        rect.x - pad_x,
                        rect.y - pad_y,
                        rect.w + pad_x * 2.0,
                        rect.h + pad_y * 2.0,
                    )
                } else {
                    rect
                }
            }
            Shape::Step { center, .. } => {
                let radius = self.step_radius();
                RectF::new(
                    center.x - radius,
                    center.y - radius,
                    radius * 2.0,
                    radius * 2.0,
                )
            }
        }
    }

    pub fn hit(&self, p: Pt, slop: f32) -> bool {
        let reach = self.stroke_width() / 2.0 + slop;
        match &self.shape {
            Shape::Rect(rect) if !self.fill => {
                rect.inflate(reach).contains(p)
                    && !(rect.w > reach * 2.0
                        && rect.h > reach * 2.0
                        && rect.inflate(-reach).contains(p))
            }
            Shape::Ellipse(rect) => {
                let (rx, ry) = ((rect.w / 2.0).max(0.001), (rect.h / 2.0).max(0.001));
                let center = rect.center();
                let radial = ((p.x - center.x) / rx).hypot((p.y - center.y) / ry);
                if self.fill {
                    radial <= 1.0 + slop / rx.min(ry)
                } else {
                    (radial - 1.0).abs() * rx.min(ry) <= reach
                }
            }
            Shape::Line(a, b) | Shape::Arrow(a, b) => p.distance_to_segment(*a, *b) <= reach,
            Shape::Pen(points) | Shape::Highlighter(points) => {
                polyline_distance(points, p) <= reach
            }
            Shape::Step { center, .. } => p.distance(*center) <= self.step_radius() + slop,
            Shape::Rect(_) | Shape::Text { .. } | Shape::Blur(_) | Shape::Pixelate(_) => {
                self.bounds().inflate(slop).contains(p)
            }
        }
    }

    pub fn translate(&mut self, dx: f32, dy: f32) {
        match &mut self.shape {
            Shape::Rect(rect)
            | Shape::Ellipse(rect)
            | Shape::Blur(rect)
            | Shape::Pixelate(rect) => *rect = rect.translate(dx, dy),
            Shape::Line(a, b) | Shape::Arrow(a, b) => {
                *a = a.offset(dx, dy);
                *b = b.offset(dx, dy);
            }
            Shape::Pen(points) | Shape::Highlighter(points) => {
                *points = Arc::new(points.iter().map(|p| p.offset(dx, dy)).collect());
            }
            Shape::Text { origin, .. } => *origin = origin.offset(dx, dy),
            Shape::Step { center, .. } => *center = center.offset(dx, dy),
        }
    }

    /// The box that resize handles sit on, for the shapes that have one.
    fn frame(&self) -> Option<RectF> {
        match &self.shape {
            Shape::Rect(rect)
            | Shape::Ellipse(rect)
            | Shape::Blur(rect)
            | Shape::Pixelate(rect) => Some(*rect),
            Shape::Pen(points) | Shape::Highlighter(points) => Some(points_bounds(points)),
            _ => None,
        }
    }

    pub fn handles(&self) -> Vec<(Handle, Pt)> {
        if let Shape::Line(a, b) | Shape::Arrow(a, b) = &self.shape {
            return vec![(Handle::Start, *a), (Handle::End, *b)];
        }
        let Some(frame) = self.frame() else {
            return Vec::new();
        };
        let (left, top, right, bottom) = (frame.x, frame.y, frame.right(), frame.bottom());
        let center = frame.center();
        vec![
            (Handle::NorthWest, pt(left, top)),
            (Handle::North, pt(center.x, top)),
            (Handle::NorthEast, pt(right, top)),
            (Handle::East, pt(right, center.y)),
            (Handle::SouthEast, pt(right, bottom)),
            (Handle::South, pt(center.x, bottom)),
            (Handle::SouthWest, pt(left, bottom)),
            (Handle::West, pt(left, center.y)),
        ]
    }

    /// Moves one handle of `original` to `to` and stores the result in `self`.
    pub fn drag_handle(&mut self, original: &Element, handle: Handle, to: Pt, constrain: bool) {
        match (&original.shape, handle) {
            (Shape::Line(a, b), Handle::Start | Handle::End)
            | (Shape::Arrow(a, b), Handle::Start | Handle::End) => {
                let (anchor, start) = if handle == Handle::Start {
                    (*b, true)
                } else {
                    (*a, false)
                };
                let to = if constrain {
                    snap_angle(anchor, to)
                } else {
                    to
                };
                let (a, b) = if start { (to, anchor) } else { (anchor, to) };
                self.shape = match original.shape {
                    Shape::Line(..) => Shape::Line(a, b),
                    _ => Shape::Arrow(a, b),
                };
            }
            _ => {
                let Some(frame) = original.frame() else {
                    return;
                };
                let resized = resize_frame(frame, handle, to, constrain);
                self.shape = match &original.shape {
                    Shape::Rect(_) => Shape::Rect(resized),
                    Shape::Ellipse(_) => Shape::Ellipse(resized),
                    Shape::Blur(_) => Shape::Blur(resized),
                    Shape::Pixelate(_) => Shape::Pixelate(resized),
                    Shape::Pen(points) => Shape::Pen(remap(points, frame, resized)),
                    Shape::Highlighter(points) => Shape::Highlighter(remap(points, frame, resized)),
                    other => other.clone(),
                };
            }
        }
    }

    /// True for a shape too small to see, such as one left by a click without a drag.
    pub fn is_degenerate(&self) -> bool {
        match &self.shape {
            Shape::Rect(rect)
            | Shape::Ellipse(rect)
            | Shape::Blur(rect)
            | Shape::Pixelate(rect) => rect.w < 3.0 || rect.h < 3.0,
            Shape::Line(a, b) | Shape::Arrow(a, b) => a.distance(*b) < 3.0,
            Shape::Pen(points) | Shape::Highlighter(points) => points.is_empty(),
            Shape::Text { text, .. } => text.trim().is_empty(),
            Shape::Step { .. } => false,
        }
    }
}

/// Snaps the segment from `anchor` to `to` onto the nearest 45 degree direction.
pub fn snap_angle(anchor: Pt, to: Pt) -> Pt {
    let (dx, dy) = (to.x - anchor.x, to.y - anchor.y);
    let length = dx.hypot(dy);
    let step = std::f32::consts::FRAC_PI_4;
    let angle = (dy.atan2(dx) / step).round() * step;
    pt(
        anchor.x + angle.cos() * length,
        anchor.y + angle.sin() * length,
    )
}

/// Makes the box from `anchor` to `to` square, keeping the drag direction.
pub fn snap_square(anchor: Pt, to: Pt) -> Pt {
    let (dx, dy) = (to.x - anchor.x, to.y - anchor.y);
    let side = dx.abs().max(dy.abs());
    pt(anchor.x + side.copysign(dx), anchor.y + side.copysign(dy))
}

fn resize_frame(frame: RectF, handle: Handle, to: Pt, constrain: bool) -> RectF {
    let (mut left, mut top, mut right, mut bottom) =
        (frame.x, frame.y, frame.right(), frame.bottom());
    let corner = match handle {
        Handle::NorthWest => Some(pt(right, bottom)),
        Handle::NorthEast => Some(pt(left, bottom)),
        Handle::SouthEast => Some(pt(left, top)),
        Handle::SouthWest => Some(pt(right, top)),
        _ => None,
    };
    if let Some(anchor) = corner {
        let to = if constrain {
            snap_square(anchor, to)
        } else {
            to
        };
        return RectF::from_points(anchor, to);
    }
    match handle {
        Handle::North => top = to.y,
        Handle::South => bottom = to.y,
        Handle::East => right = to.x,
        Handle::West => left = to.x,
        _ => {}
    }
    RectF::from_points(pt(left, top), pt(right, bottom))
}

fn remap(points: &[Pt], from: RectF, to: RectF) -> Arc<Vec<Pt>> {
    let scale = |value: f32, origin: f32, span: f32, new_origin: f32, new_span: f32| {
        if span <= f32::EPSILON {
            new_origin + new_span / 2.0
        } else {
            new_origin + (value - origin) / span * new_span
        }
    };
    Arc::new(
        points
            .iter()
            .map(|p| {
                pt(
                    scale(p.x, from.x, from.w, to.x, to.w),
                    scale(p.y, from.y, from.h, to.y, to.h),
                )
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn element(shape: Shape) -> Element {
        Element {
            id: 1,
            shape,
            color: PALETTE[0],
            fill: false,
            width: 4.0,
            size: 20.0,
        }
    }

    #[test]
    fn hollow_rect_is_hit_on_its_border_only() {
        let rect = element(Shape::Rect(RectF::new(10.0, 10.0, 100.0, 60.0)));
        assert!(rect.hit(pt(10.0, 40.0), 2.0));
        assert!(!rect.hit(pt(60.0, 40.0), 2.0));
        let mut filled = rect.clone();
        filled.fill = true;
        assert!(filled.hit(pt(60.0, 40.0), 2.0));
    }

    #[test]
    fn line_is_hit_near_the_segment() {
        let line = element(Shape::Line(pt(0.0, 0.0), pt(100.0, 0.0)));
        assert!(line.hit(pt(50.0, 3.0), 2.0));
        assert!(!line.hit(pt(50.0, 9.0), 2.0));
    }

    #[test]
    fn corner_drag_resizes_from_the_opposite_corner() {
        let original = element(Shape::Rect(RectF::new(10.0, 10.0, 20.0, 20.0)));
        let mut resized = original.clone();
        resized.drag_handle(&original, Handle::SouthEast, pt(50.0, 40.0), false);
        assert_eq!(
            resized.shape,
            Shape::Rect(RectF::new(10.0, 10.0, 40.0, 30.0))
        );
        resized.drag_handle(&original, Handle::SouthEast, pt(50.0, 40.0), true);
        assert_eq!(
            resized.shape,
            Shape::Rect(RectF::new(10.0, 10.0, 40.0, 40.0))
        );
    }

    #[test]
    fn pen_points_follow_a_resize() {
        let points = Arc::new(vec![pt(0.0, 0.0), pt(10.0, 10.0)]);
        let original = element(Shape::Pen(points));
        let mut resized = original.clone();
        resized.drag_handle(&original, Handle::SouthEast, pt(20.0, 30.0), false);
        let Shape::Pen(points) = &resized.shape else {
            panic!("still a pen stroke");
        };
        assert_eq!(points.as_slice(), &[pt(0.0, 0.0), pt(20.0, 30.0)]);
    }

    #[test]
    fn angle_snap_lands_on_the_axis() {
        let snapped = snap_angle(pt(0.0, 0.0), pt(100.0, 8.0));
        assert!(snapped.y.abs() < 0.01);
    }
}
