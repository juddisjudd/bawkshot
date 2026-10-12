use std::borrow::Cow;

use gpui_kit::component::{Theme, ThemeMode, ThemeRegistry};
use gpui_kit::{App, Global, Hsla, Window, rgb, rgba};
use serde::{Deserialize, Serialize};

use crate::assets::{MARKUP_FONT, UI_FONTS};

const BAWK: &str = include_str!("../assets/themes/bawk.json");

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    System,
    #[default]
    Dark,
    Light,
}

impl Mode {
    pub const ALL: [Mode; 3] = [Mode::System, Mode::Dark, Mode::Light];

    pub fn label(self) -> &'static str {
        match self {
            Mode::System => "system",
            Mode::Dark => "dark",
            Mode::Light => "light",
        }
    }
}

struct Active(Palette);

impl Global for Active {}

pub fn init(cx: &mut App) {
    let fonts = UI_FONTS.iter().chain([&MARKUP_FONT]);
    cx.text_system()
        .add_fonts(fonts.map(|bytes| Cow::Borrowed(*bytes)).collect())
        .expect("bundled fonts are valid");
    ThemeRegistry::global_mut(cx)
        .load_themes_from_str(BAWK)
        .expect("bundled theme is valid");
    cx.set_global(Active(Palette::dark()));
    apply(Mode::Dark, None, cx);
}

pub fn apply(mode: Mode, window: Option<&mut Window>, cx: &mut App) {
    let appearance = window
        .as_ref()
        .map(|window| window.appearance())
        .unwrap_or_else(|| cx.window_appearance());
    let dark = match mode {
        Mode::System => ThemeMode::from(appearance).is_dark(),
        Mode::Dark => true,
        Mode::Light => false,
    };
    let themes = ThemeRegistry::global(cx).themes().clone();
    cx.global_mut::<Active>().0 = if dark {
        Palette::dark()
    } else {
        Palette::light()
    };
    Theme::update(cx, |theme| {
        if let Some(config) = themes.get("Bawk Light") {
            theme.light_theme = config.clone();
        }
        if let Some(config) = themes.get("Bawk Dark") {
            theme.dark_theme = config.clone();
        }
    });
    let mode = if dark {
        ThemeMode::Dark
    } else {
        ThemeMode::Light
    };
    Theme::change(mode, window, cx);
}

/// bawkterm's color tokens, for the places the component theme has no slot for.
#[derive(Clone, Copy)]
pub struct Palette {
    pub bg: Hsla,
    pub bg_weak: Hsla,
    pub bg_hover: Hsla,
    pub text: Hsla,
    pub text_weak: Hsla,
    pub text_weaker: Hsla,
    pub text_strong: Hsla,
    pub border_weak: Hsla,
    pub yolk: Hsla,
    pub yolk_dim: Hsla,
    pub danger: Hsla,
    pub canvas: Hsla,
}

impl Palette {
    fn dark() -> Self {
        Self {
            bg: rgb(0x13100f).into(),
            bg_weak: rgb(0x1a1616).into(),
            bg_hover: rgb(0x241f1f).into(),
            text: rgb(0xc3bcbb).into(),
            text_weak: rgb(0x9d9696).into(),
            text_weaker: rgb(0x797372).into(),
            text_strong: rgb(0xf3efee).into(),
            border_weak: rgb(0x2c2827).into(),
            yolk: rgb(0xf6d56b).into(),
            yolk_dim: rgba(0xf6d56b24).into(),
            danger: rgb(0xf8767a).into(),
            canvas: rgb(0x0b0909).into(),
        }
    }

    fn light() -> Self {
        Self {
            bg: rgb(0xfdfbfb).into(),
            bg_weak: rgb(0xf8f5f5).into(),
            bg_hover: rgb(0xefeceb).into(),
            text: rgb(0x403c3b).into(),
            text_weak: rgb(0x676261).into(),
            text_weaker: rgb(0x8a8584).into(),
            text_strong: rgb(0x1b1717).into(),
            border_weak: rgb(0xe1dddd).into(),
            yolk: rgb(0xa28200).into(),
            yolk_dim: rgba(0x84720024).into(),
            danger: rgb(0xc2272d).into(),
            canvas: rgb(0xe9e5e4).into(),
        }
    }
}

pub fn palette(cx: &App) -> Palette {
    cx.global::<Active>().0
}
