use image::RgbaImage;
use image::imageops::{self, FilterType};

use super::element::{Element, Shape};
use crate::geom::RectI;

/// The blurred or pixelated pixels an effect puts over `base`, and where they go.
pub fn patch(base: &RgbaImage, element: &Element) -> Option<(RectI, RgbaImage)> {
    let (rect, pixelate) = match &element.shape {
        Shape::Blur(rect) => (rect, false),
        Shape::Pixelate(rect) => (rect, true),
        _ => return None,
    };
    let full = RectI::new(0, 0, base.width() as i32, base.height() as i32);
    let area = rect.to_pixels(full);
    if area.is_empty() {
        return None;
    }
    let (width, height) = (area.w as u32, area.h as u32);
    let region = imageops::crop_imm(base, area.x as u32, area.y as u32, width, height).to_image();
    let result = if pixelate {
        let block = element.size.max(2.0) as u32;
        let small = imageops::resize(
            &region,
            width.div_ceil(block).max(1),
            height.div_ceil(block).max(1),
            FilterType::Triangle,
        );
        imageops::resize(&small, width, height, FilterType::Nearest)
    } else {
        imageops::fast_blur(&region, element.size.max(1.0))
    };
    Some((area, result))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::element::PALETTE;
    use crate::geom::RectF;
    use image::Rgba;

    fn checker() -> RgbaImage {
        RgbaImage::from_fn(64, 64, |x, y| {
            if (x + y) % 2 == 0 {
                Rgba([255, 255, 255, 255])
            } else {
                Rgba([0, 0, 0, 255])
            }
        })
    }

    fn effect(shape: Shape) -> Element {
        Element {
            id: 1,
            shape,
            color: PALETTE[0],
            fill: false,
            width: 0.0,
            size: 8.0,
        }
    }

    #[test]
    fn pixelate_flattens_each_block() {
        let rect = RectF::new(8.0, 8.0, 32.0, 32.0);
        let (area, patch) = patch(&checker(), &effect(Shape::Pixelate(rect))).unwrap();
        assert_eq!(area, RectI::new(8, 8, 32, 32));
        assert_eq!(patch.get_pixel(0, 0), patch.get_pixel(7, 7));
        let gray = patch.get_pixel(3, 3)[0];
        assert!((100..156).contains(&gray), "{gray}");
    }

    #[test]
    fn blur_mixes_neighbors_and_clips_to_the_image() {
        let rect = RectF::new(40.0, 40.0, 100.0, 100.0);
        let (area, patch) = patch(&checker(), &effect(Shape::Blur(rect))).unwrap();
        assert_eq!(area, RectI::new(40, 40, 24, 24));
        let gray = patch.get_pixel(12, 12)[0];
        assert!((90..166).contains(&gray), "{gray}");
    }

    #[test]
    fn an_effect_off_the_image_has_no_patch() {
        let rect = RectF::new(200.0, 200.0, 10.0, 10.0);
        assert!(patch(&checker(), &effect(Shape::Blur(rect))).is_none());
    }
}
