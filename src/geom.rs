use serde::{Deserialize, Serialize};

/// A point in image pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Pt {
    pub x: f32,
    pub y: f32,
}

pub fn pt(x: f32, y: f32) -> Pt {
    Pt { x, y }
}

impl Pt {
    pub fn distance(self, other: Pt) -> f32 {
        (self.x - other.x).hypot(self.y - other.y)
    }

    pub fn offset(self, dx: f32, dy: f32) -> Pt {
        pt(self.x + dx, self.y + dy)
    }

    pub fn lerp(self, other: Pt, t: f32) -> Pt {
        pt(
            self.x + (other.x - self.x) * t,
            self.y + (other.y - self.y) * t,
        )
    }

    /// Distance from this point to the segment `a`-`b`.
    pub fn distance_to_segment(self, a: Pt, b: Pt) -> f32 {
        let (dx, dy) = (b.x - a.x, b.y - a.y);
        let length_squared = dx * dx + dy * dy;
        if length_squared <= f32::EPSILON {
            return self.distance(a);
        }
        let t = (((self.x - a.x) * dx + (self.y - a.y) * dy) / length_squared).clamp(0.0, 1.0);
        self.distance(a.lerp(b, t))
    }
}

/// A rectangle in image pixels, never with a negative size.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RectF {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl RectF {
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    pub fn from_points(a: Pt, b: Pt) -> Self {
        Self {
            x: a.x.min(b.x),
            y: a.y.min(b.y),
            w: (a.x - b.x).abs(),
            h: (a.y - b.y).abs(),
        }
    }

    pub fn right(&self) -> f32 {
        self.x + self.w
    }

    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }

    pub fn center(&self) -> Pt {
        pt(self.x + self.w / 2.0, self.y + self.h / 2.0)
    }

    pub fn contains(&self, p: Pt) -> bool {
        p.x >= self.x && p.x <= self.right() && p.y >= self.y && p.y <= self.bottom()
    }

    pub fn inflate(&self, by: f32) -> Self {
        Self::new(
            self.x - by,
            self.y - by,
            self.w + by * 2.0,
            self.h + by * 2.0,
        )
    }

    pub fn intersects(&self, other: &RectF) -> bool {
        self.x < other.right()
            && other.x < self.right()
            && self.y < other.bottom()
            && other.y < self.bottom()
    }

    pub fn union(&self, other: &RectF) -> Self {
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        Self::new(
            x,
            y,
            self.right().max(other.right()) - x,
            self.bottom().max(other.bottom()) - y,
        )
    }

    pub fn translate(&self, dx: f32, dy: f32) -> Self {
        Self::new(self.x + dx, self.y + dy, self.w, self.h)
    }

    /// The whole pixels this rectangle covers, clipped to `bounds`.
    pub fn to_pixels(self, bounds: RectI) -> RectI {
        let rect = RectI {
            x: self.x.round() as i32,
            y: self.y.round() as i32,
            w: self.w.round() as i32,
            h: self.h.round() as i32,
        };
        rect.intersect(&bounds)
    }
}

/// A rectangle in whole pixels, used for screen areas and crops.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RectI {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl RectI {
    pub fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x, y, w, h }
    }

    pub fn right(&self) -> i32 {
        self.x + self.w
    }

    pub fn bottom(&self) -> i32 {
        self.y + self.h
    }

    pub fn is_empty(&self) -> bool {
        self.w <= 0 || self.h <= 0
    }

    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x && x < self.right() && y >= self.y && y < self.bottom()
    }

    pub fn intersect(&self, other: &RectI) -> RectI {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let right = self.right().min(other.right());
        let bottom = self.bottom().min(other.bottom());
        RectI::new(x, y, (right - x).max(0), (bottom - y).max(0))
    }

    pub fn to_f32(self) -> RectF {
        RectF::new(self.x as f32, self.y as f32, self.w as f32, self.h as f32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segment_distance_clamps_to_the_ends() {
        let (a, b) = (pt(0.0, 0.0), pt(10.0, 0.0));
        assert_eq!(pt(5.0, 3.0).distance_to_segment(a, b), 3.0);
        assert_eq!(pt(-4.0, 0.0).distance_to_segment(a, b), 4.0);
        assert_eq!(pt(13.0, 4.0).distance_to_segment(a, b), 5.0);
    }

    #[test]
    fn rect_from_points_is_normalized() {
        let rect = RectF::from_points(pt(10.0, 2.0), pt(4.0, 8.0));
        assert_eq!(rect, RectF::new(4.0, 2.0, 6.0, 6.0));
    }

    #[test]
    fn to_pixels_clips_to_bounds() {
        let rect = RectF::new(-5.4, 2.2, 20.0, 10.0).to_pixels(RectI::new(0, 0, 10, 8));
        assert_eq!(rect, RectI::new(0, 2, 10, 6));
    }
}
