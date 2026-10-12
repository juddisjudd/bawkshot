use std::borrow::Cow;

use gpui_kit::assets::{Assets, icon_assets};
use gpui_kit::{AssetSource, Result, SharedString};

icon_assets!(
    AppIcons,
    [
        ArrowUpRight,
        BringToFront,
        Camera,
        Check,
        Circle,
        Copy,
        Crop,
        Droplet,
        FolderOpen,
        Grid3x3,
        Hash,
        Highlighter,
        Maximize,
        Minus,
        MousePointer2,
        PaintBucket,
        Pencil,
        Redo2,
        Save,
        SendToBack,
        Settings,
        Square,
        Trash,
        Type,
        Undo2,
        X,
    ]
);

pub const MARK: &str = "brand/mark.svg";
pub const ICON_SVG: &[u8] = include_bytes!("../assets/brand/icon.svg");

pub const UI_FONTS: [&[u8]; 4] = [
    include_bytes!("../assets/fonts/IBMPlexMono-Regular.ttf"),
    include_bytes!("../assets/fonts/IBMPlexMono-Medium.ttf"),
    include_bytes!("../assets/fonts/IBMPlexMono-SemiBold.ttf"),
    include_bytes!("../assets/fonts/IBMPlexMono-Bold.ttf"),
];

pub const MARKUP_FONT: &[u8] = include_bytes!("../assets/fonts/IBMPlexSans-SemiBold.ttf");
pub const MARKUP_FONT_FAMILY: &str = "IBM Plex Sans";

pub struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if path == MARK {
            return Ok(Some(Cow::Borrowed(include_bytes!(
                "../assets/brand/mark.svg"
            ))));
        }
        if let Some(bytes) = AppIcons.load(path)? {
            return Ok(Some(bytes));
        }
        Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut paths = Assets.list(path)?;
        paths.extend(AppIcons.list(path)?);
        if MARK.starts_with(path) {
            paths.push(MARK.into());
        }
        paths.sort();
        paths.dedup();
        Ok(paths)
    }
}
