//! Redraws `assets/brand/bawkshot.ico` and `.png` from `icon.svg`. Run from the repo root: `cargo run --example brand`.

use image::ExtendedColorType;
use image::codecs::ico::{IcoEncoder, IcoFrame};
use resvg::{tiny_skia, usvg};

fn main() {
    let svg = std::fs::read("assets/brand/icon.svg").expect("read assets/brand/icon.svg");
    let tree = usvg::Tree::from_data(&svg, &usvg::Options::default()).expect("parse icon.svg");
    let draw = |size: u32| {
        let mut pixmap = tiny_skia::Pixmap::new(size, size).expect("icon sizes are not zero");
        let scale = size as f32 / tree.size().width();
        resvg::render(
            &tree,
            tiny_skia::Transform::from_scale(scale, scale),
            &mut pixmap.as_mut(),
        );
        let mut pixels = pixmap.take();
        for pixel in pixels.as_chunks_mut::<4>().0 {
            let alpha = pixel[3] as u32;
            if alpha > 0 && alpha < 255 {
                for channel in &mut pixel[..3] {
                    *channel = (*channel as u32 * 255 / alpha).min(255) as u8;
                }
            }
        }
        pixels
    };

    let frames: Vec<IcoFrame> = [16, 24, 32, 48, 64, 128, 256]
        .into_iter()
        .map(|size| {
            IcoFrame::as_png(&draw(size), size, size, ExtendedColorType::Rgba8)
                .expect("encode an icon size")
        })
        .collect();
    let file = std::fs::File::create("assets/brand/bawkshot.ico").expect("create bawkshot.ico");
    IcoEncoder::new(file)
        .encode_images(&frames)
        .expect("write bawkshot.ico");
    image::save_buffer(
        "assets/brand/bawkshot.png",
        &draw(256),
        256,
        256,
        ExtendedColorType::Rgba8,
    )
    .expect("write bawkshot.png");
}
