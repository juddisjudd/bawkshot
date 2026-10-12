use std::borrow::Cow;
use std::fs;
use std::path::Path;
use std::sync::Arc;

use gpui_kit::RenderImage;
use image::{Frame, RgbaImage};

/// GPUI draws BGRA, so the red and blue channels trade places on the way in.
pub fn render_image(mut image: RgbaImage) -> Arc<RenderImage> {
    for pixel in image.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
    Arc::new(RenderImage::new(vec![Frame::new(image)]))
}

pub fn save_png(image: &RgbaImage, path: &Path) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)
            .map_err(|err| format!("could not create {}: {err}", dir.display()))?;
    }
    image
        .save(path)
        .map_err(|err| format!("could not write {}: {err}", path.display()))
}

pub fn copy_to_clipboard(image: &RgbaImage) -> Result<(), String> {
    let data = arboard::ImageData {
        width: image.width() as usize,
        height: image.height() as usize,
        bytes: Cow::Borrowed(image.as_raw()),
    };
    arboard::Clipboard::new()
        .and_then(|mut clipboard| clipboard.set_image(data))
        .map_err(|err| format!("could not copy to the clipboard: {err}"))
}

/// Puts the file itself on the clipboard, so it pastes into a chat or a folder.
pub fn copy_file_to_clipboard(path: &Path) -> Result<(), String> {
    arboard::Clipboard::new()
        .and_then(|mut clipboard| clipboard.set().file_list(&[path]))
        .map_err(|err| format!("could not copy the file to the clipboard: {err}"))
}

pub fn clipboard_image() -> Option<RgbaImage> {
    let data = arboard::Clipboard::new().ok()?.get_image().ok()?;
    RgbaImage::from_raw(
        data.width as u32,
        data.height as u32,
        data.bytes.into_owned(),
    )
}

/// The app icon drawn at `size` pixels, with straight alpha as window systems expect.
pub fn brand_icon(size: u32) -> Option<RgbaImage> {
    use resvg::{tiny_skia, usvg};

    let tree = usvg::Tree::from_data(crate::assets::ICON_SVG, &usvg::Options::default()).ok()?;
    let mut pixmap = tiny_skia::Pixmap::new(size, size)?;
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
    RgbaImage::from_raw(size, size, pixels)
}

pub fn load(path: &Path) -> Result<RgbaImage, String> {
    image::open(path)
        .map(|image| image.to_rgba8())
        .map_err(|err| format!("could not open {}: {err}", path.display()))
}
