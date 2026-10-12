use std::fmt::Write as _;

use image::RgbaImage;
use image::imageops;
use resvg::{tiny_skia, usvg};

use super::effects;
use super::element::{Color, Element, HIGHLIGHT_OPACITY, LINE_HEIGHT, Shape, arrow_geometry};
use crate::assets::{MARKUP_FONT, MARKUP_FONT_FAMILY};
use crate::geom::{Pt, RectI};

/// Ascent and descent of the markup font for a font size of one, as the window draws it.
#[derive(Clone, Copy, Debug)]
pub struct TextMetrics {
    pub ascent: f32,
    pub descent: f32,
}

impl TextMetrics {
    /// Where the baseline of the first line sits below the top of a text block.
    pub fn baseline(&self, size: f32) -> f32 {
        let line_height = size * LINE_HEIGHT;
        (line_height - (self.ascent + self.descent) * size) / 2.0 + self.ascent * size
    }
}

/// Draws the markup onto the capture and returns the cropped result.
pub fn flatten(
    base: &RgbaImage,
    crop: RectI,
    elements: &[Element],
    metrics: TextMetrics,
) -> Result<RgbaImage, String> {
    let full = RectI::new(0, 0, base.width() as i32, base.height() as i32);
    let crop = crop.intersect(&full);
    if crop.is_empty() {
        return Err("nothing left after the crop".into());
    }
    let mut working = base.clone();
    for element in elements {
        if let Some((area, patch)) = effects::patch(base, element) {
            imageops::replace(&mut working, &patch, area.x as i64, area.y as i64);
        }
    }
    let cropped = imageops::crop_imm(
        &working,
        crop.x as u32,
        crop.y as u32,
        crop.w as u32,
        crop.h as u32,
    )
    .to_image();
    let (width, height) = cropped.dimensions();
    let size = tiny_skia::IntSize::from_wh(width, height).ok_or("empty image")?;
    let mut pixmap = tiny_skia::Pixmap::from_vec(cropped.into_raw(), size).ok_or("bad pixmap")?;

    let mut options = usvg::Options::default();
    options.fontdb_mut().load_font_data(MARKUP_FONT.to_vec());
    let tree = usvg::Tree::from_str(&svg(crop, elements, metrics), &options)
        .map_err(|err| format!("markup did not parse: {err}"))?;
    resvg::render(
        &tree,
        tiny_skia::Transform::identity(),
        &mut pixmap.as_mut(),
    );

    RgbaImage::from_raw(width, height, pixmap.take()).ok_or_else(|| "bad image".into())
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// The pen path both renderers draw: straight to the first midpoint, then a curve through each point.
pub fn pen_path(points: &[Pt]) -> String {
    let mut path = String::new();
    match points {
        [] => {}
        [only] => {
            let _ = write!(path, "M{} {}L{} {}", only.x, only.y, only.x + 0.01, only.y);
        }
        [first, .., last] => {
            let _ = write!(path, "M{} {}", first.x, first.y);
            for pair in points[1..].windows(2) {
                let mid = pair[0].lerp(pair[1], 0.5);
                let _ = write!(path, "Q{} {} {} {}", pair[0].x, pair[0].y, mid.x, mid.y);
            }
            let _ = write!(path, "L{} {}", last.x, last.y);
        }
    }
    path
}

fn text_lines(
    out: &mut String,
    lines: &str,
    x: f32,
    top: f32,
    size: f32,
    color: Color,
    metrics: TextMetrics,
) {
    for (row, line) in lines.split('\n').enumerate() {
        let baseline = top + metrics.baseline(size) + row as f32 * size * LINE_HEIGHT;
        let _ = write!(
            out,
            r#"<text x="{x}" y="{baseline}" font-family="{MARKUP_FONT_FAMILY}" font-weight="600" font-size="{size}" fill="{}" xml:space="preserve">{}</text>"#,
            color.css(),
            escape(line),
        );
    }
}

fn svg(crop: RectI, elements: &[Element], metrics: TextMetrics) -> String {
    let mut out = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="{x} {y} {w} {h}">"#,
        x = crop.x,
        y = crop.y,
        w = crop.w,
        h = crop.h,
    );
    for element in elements {
        let color = element.color.css();
        let width = element.stroke_width();
        let paint = if element.fill {
            format!(r#"fill="{color}""#)
        } else {
            format!(
                r#"fill="none" stroke="{color}" stroke-width="{width}" stroke-linejoin="round" stroke-linecap="round""#
            )
        };
        match &element.shape {
            Shape::Rect(rect) => {
                let _ = write!(
                    out,
                    r#"<rect x="{}" y="{}" width="{}" height="{}" {paint}/>"#,
                    rect.x, rect.y, rect.w, rect.h
                );
            }
            Shape::Ellipse(rect) => {
                let center = rect.center();
                let _ = write!(
                    out,
                    r#"<ellipse cx="{}" cy="{}" rx="{}" ry="{}" {paint}/>"#,
                    center.x,
                    center.y,
                    rect.w / 2.0,
                    rect.h / 2.0
                );
            }
            Shape::Line(a, b) => {
                let _ = write!(
                    out,
                    r#"<line x1="{}" y1="{}" x2="{}" y2="{}" stroke="{color}" stroke-width="{width}" stroke-linecap="round"/>"#,
                    a.x, a.y, b.x, b.y
                );
            }
            Shape::Arrow(a, b) => {
                let geometry = arrow_geometry(*a, *b, element.width);
                let [tip, left, right] = geometry.head;
                let _ = write!(
                    out,
                    r#"<line x1="{}" y1="{}" x2="{}" y2="{}" stroke="{color}" stroke-width="{width}" stroke-linecap="round"/><polygon points="{},{} {},{} {},{}" fill="{color}"/>"#,
                    a.x,
                    a.y,
                    geometry.shaft_end.x,
                    geometry.shaft_end.y,
                    tip.x,
                    tip.y,
                    left.x,
                    left.y,
                    right.x,
                    right.y
                );
            }
            Shape::Pen(points) | Shape::Highlighter(points) => {
                let opacity = match element.shape {
                    Shape::Highlighter(_) => HIGHLIGHT_OPACITY,
                    _ => 1.0,
                };
                let _ = write!(
                    out,
                    r#"<path d="{}" fill="none" stroke="{color}" stroke-opacity="{opacity}" stroke-width="{width}" stroke-linejoin="round" stroke-linecap="round"/>"#,
                    pen_path(points)
                );
            }
            Shape::Text { origin, text, .. } => {
                let mut ink = element.color;
                if element.fill {
                    let bounds = element.bounds();
                    let _ = write!(
                        out,
                        r#"<rect x="{}" y="{}" width="{}" height="{}" rx="{}" fill="{color}"/>"#,
                        bounds.x,
                        bounds.y,
                        bounds.w,
                        bounds.h,
                        element.size * 0.25
                    );
                    ink = element.color.contrast();
                }
                text_lines(
                    &mut out,
                    text,
                    origin.x,
                    origin.y,
                    element.size,
                    ink,
                    metrics,
                );
            }
            Shape::Step { center, number } => {
                let size = element.size;
                let baseline = center.y - size * LINE_HEIGHT / 2.0 + metrics.baseline(size);
                let _ = write!(
                    out,
                    r#"<circle cx="{}" cy="{}" r="{}" fill="{color}"/><text x="{}" y="{baseline}" text-anchor="middle" font-family="{MARKUP_FONT_FAMILY}" font-weight="600" font-size="{size}" fill="{}">{number}</text>"#,
                    center.x,
                    center.y,
                    element.step_radius(),
                    center.x,
                    element.color.contrast().css(),
                );
            }
            Shape::Blur(_) | Shape::Pixelate(_) => {}
        }
    }
    out.push_str("</svg>");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::element::PALETTE;
    use crate::geom::{RectF, pt};
    use image::Rgba;
    use std::sync::Arc;

    const METRICS: TextMetrics = TextMetrics {
        ascent: 1.025,
        descent: 0.275,
    };

    fn white(width: u32, height: u32) -> RgbaImage {
        RgbaImage::from_pixel(width, height, Rgba([255, 255, 255, 255]))
    }

    fn element(shape: Shape, fill: bool) -> Element {
        Element {
            id: 1,
            shape,
            color: PALETTE[0],
            fill,
            width: 4.0,
            size: 24.0,
        }
    }

    #[test]
    fn a_filled_rect_lands_where_it_was_drawn() {
        let rect = element(Shape::Rect(RectF::new(10.0, 10.0, 20.0, 20.0)), true);
        let image = flatten(&white(60, 40), RectI::new(0, 0, 60, 40), &[rect], METRICS).unwrap();
        assert_eq!(image.get_pixel(20, 20), &Rgba([0xff, 0x4d, 0x4f, 255]));
        assert_eq!(image.get_pixel(40, 20), &Rgba([255, 255, 255, 255]));
    }

    #[test]
    fn the_crop_shifts_the_markup() {
        let rect = element(Shape::Rect(RectF::new(10.0, 10.0, 20.0, 20.0)), true);
        let image = flatten(&white(60, 40), RectI::new(10, 10, 30, 20), &[rect], METRICS).unwrap();
        assert_eq!(image.dimensions(), (30, 20));
        assert_eq!(image.get_pixel(1, 1), &Rgba([0xff, 0x4d, 0x4f, 255]));
        assert_eq!(image.get_pixel(25, 10), &Rgba([255, 255, 255, 255]));
    }

    #[test]
    fn text_is_drawn_with_the_bundled_font() {
        let text = element(
            Shape::Text {
                origin: pt(8.0, 8.0),
                text: Arc::from("Hi <&>"),
                extent: (80.0, 31.0),
            },
            false,
        );
        let image = flatten(&white(120, 50), RectI::new(0, 0, 120, 50), &[text], METRICS).unwrap();
        let inked = image.pixels().filter(|pixel| pixel[1] < 200).count();
        assert!(inked > 40, "only {inked} text pixels");
    }

    #[test]
    fn an_arrow_reaches_its_tip() {
        let arrow = element(Shape::Arrow(pt(10.0, 20.0), pt(90.0, 20.0)), false);
        let image = flatten(
            &white(100, 40),
            RectI::new(0, 0, 100, 40),
            &[arrow],
            METRICS,
        )
        .unwrap();
        assert!(image.get_pixel(86, 20)[1] < 120);
        assert_eq!(image.get_pixel(95, 20), &Rgba([255, 255, 255, 255]));
    }

    #[test]
    fn pen_path_curves_through_the_points() {
        let path = pen_path(&[pt(0.0, 0.0), pt(10.0, 0.0), pt(10.0, 10.0)]);
        assert_eq!(path, "M0 0Q10 0 10 5L10 10");
    }
}
