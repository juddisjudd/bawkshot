use gpui_kit::*;
use lyon::tessellation::{LineCap, LineJoin};

use super::element::{Color, Element, HIGHLIGHT_OPACITY, LINE_HEIGHT, Shape, arrow_geometry};
use super::render::TextMetrics;
use crate::assets::MARKUP_FONT_FAMILY;
use crate::geom::{Pt, RectF};

/// Maps image pixels onto the window: `origin` is where the image's top-left corner lands.
#[derive(Clone, Copy)]
pub struct Space {
    pub origin: Point<Pixels>,
    pub zoom: f32,
}

impl Space {
    pub fn point(&self, p: Pt) -> Point<Pixels> {
        point(
            self.origin.x + px(p.x * self.zoom),
            self.origin.y + px(p.y * self.zoom),
        )
    }

    pub fn rect(&self, rect: RectF) -> Bounds<Pixels> {
        Bounds::new(
            self.point(Pt {
                x: rect.x,
                y: rect.y,
            }),
            size(px(rect.w * self.zoom), px(rect.h * self.zoom)),
        )
    }

    pub fn length(&self, value: f32) -> Pixels {
        px(value * self.zoom)
    }

    pub fn to_image(self, position: Point<Pixels>) -> Pt {
        Pt {
            x: (position.x - self.origin.x).as_f32() / self.zoom,
            y: (position.y - self.origin.y).as_f32() / self.zoom,
        }
    }
}

pub fn markup_font() -> Font {
    Font {
        weight: FontWeight::SEMIBOLD,
        ..font(MARKUP_FONT_FAMILY)
    }
}

fn shape_line(text: &str, size: Pixels, color: Hsla, window: &Window) -> ShapedLine {
    let run = TextRun {
        len: text.len(),
        font: markup_font(),
        color,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    window
        .text_system()
        .shape_line(text.to_string().into(), size, &[run], None)
}

/// The size of a text block in image pixels: its widest line by its line count.
pub fn measure_text(text: &str, size: f32, window: &Window) -> (f32, f32) {
    let mut width = size * 0.5;
    let mut lines = 0;
    for line in text.split('\n') {
        lines += 1;
        if !line.is_empty() {
            let shaped = shape_line(line, px(size), Hsla::default(), window);
            width = width.max(shaped.width().as_f32());
        }
    }
    (width, lines as f32 * size * LINE_HEIGHT)
}

/// Reads the font's vertical metrics off a shaped line, so the export puts baselines where the window does.
pub fn text_metrics(window: &Window) -> TextMetrics {
    let shaped = shape_line("Hg", px(100.), Hsla::default(), window);
    TextMetrics {
        ascent: shaped.ascent.as_f32() / 100.0,
        descent: shaped.descent.as_f32().abs() / 100.0,
    }
}

fn stroke(width: Pixels) -> PathBuilder {
    let options = StrokeOptions::default()
        .with_line_width(width.as_f32().max(0.5))
        .with_line_cap(LineCap::Round)
        .with_line_join(LineJoin::Round);
    PathBuilder::stroke(width).with_style(PathStyle::Stroke(options))
}

fn paint(builder: PathBuilder, color: Hsla, window: &mut Window) {
    if let Ok(path) = builder.build() {
        window.paint_path(path, color);
    }
}

fn ellipse(builder: &mut PathBuilder, bounds: Bounds<Pixels>) {
    const KAPPA: f32 = 0.552_284_8;
    let center = bounds.center();
    let (rx, ry) = (bounds.size.width / 2., bounds.size.height / 2.);
    let (kx, ky) = (rx * KAPPA, ry * KAPPA);
    let at = |dx: Pixels, dy: Pixels| point(center.x + dx, center.y + dy);
    builder.move_to(at(rx, px(0.)));
    builder.cubic_bezier_to(at(px(0.), ry), at(rx, ky), at(kx, ry));
    builder.cubic_bezier_to(at(-rx, px(0.)), at(-kx, ry), at(-rx, ky));
    builder.cubic_bezier_to(at(px(0.), -ry), at(-rx, -ky), at(-kx, -ry));
    builder.cubic_bezier_to(at(rx, px(0.)), at(kx, -ry), at(rx, -ky));
    builder.close();
}

fn polygon(builder: &mut PathBuilder, points: &[Point<Pixels>]) {
    let Some((first, rest)) = points.split_first() else {
        return;
    };
    builder.move_to(*first);
    for p in rest {
        builder.line_to(*p);
    }
    builder.close();
}

fn pen(builder: &mut PathBuilder, points: &[Pt], space: &Space) {
    match points {
        [] => {}
        [only] => {
            let at = space.point(*only);
            builder.move_to(at);
            builder.line_to(point(at.x + px(0.01), at.y));
        }
        [first, .., last] => {
            builder.move_to(space.point(*first));
            for pair in points[1..].windows(2) {
                let mid = pair[0].lerp(pair[1], 0.5);
                builder.curve_to(space.point(mid), space.point(pair[0]));
            }
            builder.line_to(space.point(*last));
        }
    }
}

fn text_block(
    text: &str,
    origin: Point<Pixels>,
    size: Pixels,
    color: Color,
    window: &mut Window,
    cx: &mut App,
) {
    let line_height = size * LINE_HEIGHT;
    for (row, line) in text.split('\n').enumerate() {
        if line.is_empty() {
            continue;
        }
        let shaped = shape_line(line, size, color.hsla(), window);
        let at = point(origin.x, origin.y + line_height * row as f32);
        let _ = shaped.paint(at, line_height, TextAlign::Left, None, window, cx);
    }
}

pub fn element(element: &Element, space: &Space, window: &mut Window, cx: &mut App) {
    let color = element.color.hsla();
    let width = space.length(element.stroke_width());
    match &element.shape {
        Shape::Rect(rect) => {
            let bounds = space.rect(*rect);
            if element.fill {
                window.paint_quad(fill(bounds, color));
            } else {
                let mut builder = stroke(width);
                polygon(
                    &mut builder,
                    &[
                        bounds.origin,
                        bounds.top_right(),
                        bounds.bottom_right(),
                        bounds.bottom_left(),
                    ],
                );
                paint(builder, color, window);
            }
        }
        Shape::Ellipse(rect) => {
            let mut builder = if element.fill {
                PathBuilder::fill()
            } else {
                stroke(width)
            };
            ellipse(&mut builder, space.rect(*rect));
            paint(builder, color, window);
        }
        Shape::Line(a, b) => {
            let mut builder = stroke(width);
            builder.move_to(space.point(*a));
            builder.line_to(space.point(*b));
            paint(builder, color, window);
        }
        Shape::Arrow(a, b) => {
            let geometry = arrow_geometry(*a, *b, element.width);
            let mut shaft = stroke(width);
            shaft.move_to(space.point(*a));
            shaft.line_to(space.point(geometry.shaft_end));
            paint(shaft, color, window);
            let mut head = PathBuilder::fill();
            polygon(&mut head, &geometry.head.map(|p| space.point(p)));
            paint(head, color, window);
        }
        Shape::Pen(points) => {
            let mut builder = stroke(width);
            pen(&mut builder, points, space);
            paint(builder, color, window);
        }
        Shape::Highlighter(points) => {
            let mut builder = stroke(width);
            pen(&mut builder, points, space);
            paint(builder, color.opacity(HIGHLIGHT_OPACITY), window);
        }
        Shape::Text { origin, text, .. } => {
            let mut ink = element.color;
            if element.fill {
                window.paint_quad(quad(
                    space.rect(element.bounds()),
                    space.length(element.size * 0.25),
                    color,
                    px(0.),
                    transparent_black(),
                    BorderStyle::Solid,
                ));
                ink = element.color.contrast();
            }
            let size = space.length(element.size);
            text_block(text, space.point(*origin), size, ink, window, cx);
        }
        Shape::Step { center, number } => {
            let mut disc = PathBuilder::fill();
            ellipse(&mut disc, space.rect(element.bounds()));
            paint(disc, color, window);
            let size = space.length(element.size);
            let line_height = size * LINE_HEIGHT;
            let label = number.to_string();
            let shaped = shape_line(&label, size, element.color.contrast().hsla(), window);
            let middle = space.point(*center);
            let at = point(middle.x - shaped.width() / 2., middle.y - line_height / 2.);
            let _ = shaped.paint(at, line_height, TextAlign::Left, None, window, cx);
        }
        Shape::Blur(_) | Shape::Pixelate(_) => {}
    }
}

/// The frame drawn around a selected element, or around an effect so its edge stays findable.
pub fn frame(bounds: Bounds<Pixels>, color: Hsla, dashed: bool, window: &mut Window) {
    let style = if dashed {
        BorderStyle::Dashed
    } else {
        BorderStyle::Solid
    };
    window.paint_quad(quad(
        bounds,
        px(0.),
        transparent_black(),
        px(1.),
        color,
        style,
    ));
}

pub fn handle(center: Point<Pixels>, round: bool, color: Hsla, window: &mut Window) {
    let half = px(4.5);
    let bounds = Bounds::from_corners(
        point(center.x - half, center.y - half),
        point(center.x + half, center.y + half),
    );
    let radius = if round { half } else { px(1.5) };
    window.paint_quad(quad(
        bounds,
        radius,
        gpui_kit::white(),
        px(1.5),
        color,
        BorderStyle::Solid,
    ));
}
