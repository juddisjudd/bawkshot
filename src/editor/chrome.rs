use gpui_kit::assets::IconName;
use gpui_kit::component::input::Input;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Icon, Sizable, TitleBar};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::element::PALETTE;
use super::view::{Editor, Export, StyleChange, Tool};
use crate::assets::MARK;
use crate::theme::{Palette, palette};

const SIZE_LABELS: [&str; 4] = ["S", "M", "L", "XL"];

fn hint(label: impl Into<SharedString>) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    let label: SharedString = label.into();
    move |window, cx| Tooltip::new(label.clone()).build(window, cx)
}

/// The floating panel every toolbar row sits in; it swallows clicks so they never reach the canvas.
fn pill(p: &Palette) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(2.))
        .p(px(5.))
        .rounded(px(12.))
        .border_1()
        .border_color(p.border_weak)
        .bg(p.bg_weak)
        .shadow_lg()
        .occlude()
}

fn divider(p: &Palette) -> Div {
    div()
        .w(px(1.))
        .h(px(20.))
        .mx(px(5.))
        .flex_none()
        .bg(p.border_weak)
}

fn icon_button(
    id: impl Into<ElementId>,
    icon: IconName,
    tip: impl Into<SharedString>,
    active: bool,
    enabled: bool,
    p: &Palette,
) -> Stateful<Div> {
    let color = match (active, enabled) {
        (true, _) => p.yolk,
        (false, true) => p.text,
        (false, false) => p.text_weaker.opacity(0.5),
    };
    div()
        .id(id)
        .size(px(32.))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded(px(8.))
        .when(active, |this| this.bg(p.yolk_dim))
        .when(enabled && !active, |this| {
            this.cursor_pointer().hover(|style| style.bg(p.bg_hover))
        })
        .child(Icon::new(icon).small().text_color(color))
        .tooltip(hint(tip))
}

fn chip(id: impl Into<ElementId>, label: &'static str, active: bool, p: &Palette) -> Stateful<Div> {
    div()
        .id(id)
        .h(px(26.))
        .min_w(px(28.))
        .px(px(7.))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded(px(6.))
        .text_size(px(12.))
        .cursor_pointer()
        .map(|this| {
            if active {
                this.bg(p.yolk_dim).text_color(p.text_strong)
            } else {
                this.text_color(p.text_weak)
                    .hover(|style| style.bg(p.bg_hover).text_color(p.text_strong))
            }
        })
        .child(label)
}

fn chips(
    group: &'static str,
    count: usize,
    current: usize,
    change: fn(usize) -> StyleChange,
    p: &Palette,
    cx: &mut Context<Editor>,
) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(2.))
        .children((0..count).map(|index| {
            chip((group, index), SIZE_LABELS[index], index == current, p).on_click(
                cx.listener(move |this, _, window, cx| this.apply_style(change(index), window, cx)),
            )
        }))
}

fn options(editor: &Editor, cx: &mut Context<Editor>) -> Option<Div> {
    let p = palette(cx);
    let caps = editor.caps();
    let style = editor.shown_style();
    let selecting = editor.has_selection();
    let cropping = editor.tool == Tool::Crop;
    if !caps.any() && !selecting && !cropping {
        return None;
    }
    let mut row = pill(&p).gap(px(4.));
    let mut first = true;
    let mut section = |row: Div, content: Div| {
        let row = if first { row } else { row.child(divider(&p)) };
        first = false;
        row.child(content)
    };

    if caps.color {
        let swatches = div().flex().items_center().gap(px(5.)).px(px(4.)).children(
            PALETTE.into_iter().enumerate().map(|(index, color)| {
                let active = color == style.color;
                div()
                    .id(("swatch", index))
                    .size(px(22.))
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .cursor_pointer()
                    .border_2()
                    .border_color(if active { p.yolk } else { transparent_black() })
                    .child(
                        div()
                            .size(px(14.))
                            .rounded_full()
                            .border_1()
                            .border_color(p.border_weak)
                            .bg(color.hsla()),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.apply_style(StyleChange::Color(color), window, cx)
                    }))
            }),
        );
        row = section(row, swatches);
    }
    if caps.width {
        row = section(
            row,
            chips("width", 4, style.width, StyleChange::Width, &p, cx),
        );
    }
    if caps.size {
        row = section(row, chips("size", 4, style.size, StyleChange::Size, &p, cx));
    }
    if caps.strength {
        row = section(
            row,
            chips("strength", 4, style.strength, StyleChange::Strength, &p, cx),
        );
    }
    if caps.fill {
        let fill = style.fill;
        let toggle = icon_button("fill", IconName::PaintBucket, "fill", fill, true, &p).on_click(
            cx.listener(move |this, _, window, cx| {
                this.apply_style(StyleChange::Fill(!fill), window, cx)
            }),
        );
        row = section(row, div().child(toggle));
    }
    if selecting {
        let actions = div()
            .flex()
            .items_center()
            .gap(px(2.))
            .child(
                icon_button(
                    "front",
                    IconName::BringToFront,
                    "bring to front · ]",
                    false,
                    true,
                    &p,
                )
                .on_click(cx.listener(|this, _, _, cx| this.reorder(true, cx))),
            )
            .child(
                icon_button(
                    "back",
                    IconName::SendToBack,
                    "send to back · [",
                    false,
                    true,
                    &p,
                )
                .on_click(cx.listener(|this, _, _, cx| this.reorder(false, cx))),
            )
            .child(
                icon_button("delete", IconName::Trash, "delete · del", false, true, &p)
                    .on_click(cx.listener(|this, _, _, cx| this.delete_selection(cx))),
            );
        row = section(row, actions);
    }
    if cropping {
        let cropped = editor.is_cropped();
        let reset = div()
            .id("reset-crop")
            .h(px(26.))
            .px(px(10.))
            .flex()
            .items_center()
            .rounded(px(6.))
            .text_size(px(12.))
            .map(|this| {
                if cropped {
                    this.cursor_pointer()
                        .text_color(p.text)
                        .hover(|style| style.bg(p.bg_hover).text_color(p.text_strong))
                } else {
                    this.text_color(p.text_weaker)
                }
            })
            .child(if cropped {
                "reset crop"
            } else {
                "drag over the part to keep"
            })
            .on_click(cx.listener(|this, _, _, cx| this.reset_crop(cx)));
        row = section(row, div().child(reset));
    }
    Some(row)
}

pub fn toolbar(editor: &Editor, _: &mut Window, cx: &mut Context<Editor>) -> impl IntoElement {
    let p = palette(cx);
    let tools = pill(&p).children(Tool::ALL.map(|tool| {
        let tip = format!("{} · {}", tool.label(), tool.key().to_uppercase());
        let button = icon_button(
            tool.label(),
            tool.icon(),
            tip,
            editor.tool == tool,
            true,
            &p,
        )
        .debug_selector(move || format!("tool-{}", tool.key()))
        .on_click(cx.listener(move |this, _, window, cx| this.set_tool(tool, window, cx)));
        div()
            .flex()
            .items_center()
            .when(matches!(tool, Tool::Rect | Tool::Blur), |this| {
                this.child(divider(&p))
            })
            .child(button)
    }));
    div()
        .absolute()
        .bottom(px(18.))
        .left_0()
        .right_0()
        .flex()
        .flex_col()
        .items_center()
        .gap(px(8.))
        .children(options(editor, cx))
        .child(tools)
}

pub fn text_box(editor: &Editor, cx: &mut Context<Editor>) -> Option<Div> {
    let p = palette(cx);
    let (input, at) = editor.text_box()?;
    Some(
        pill(&p)
            .absolute()
            .left(at.x)
            .top(at.y)
            .w(px(280.))
            .child(Input::new(&input).appearance(false)),
    )
}

pub fn title_bar(editor: &Editor, cx: &mut Context<Editor>) -> impl IntoElement {
    let p = palette(cx);
    let (width, height) = editor.image_size();
    let file = editor
        .saved
        .as_ref()
        .and_then(|path| path.file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "not saved".into());

    let wordmark = div()
        .flex()
        .items_center()
        .gap(px(7.))
        .child(svg().path(MARK).size(px(16.)).text_color(p.text_strong))
        .child(
            div()
                .flex()
                .text_size(px(14.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(div().text_color(p.text_strong).child("bawk"))
                .child(div().text_color(p.text_weak).child("shot")),
        )
        .child(
            div()
                .ml(px(10.))
                .text_size(px(12.))
                .text_color(p.text_weaker)
                .child(format!("{file} · {width} × {height}")),
        );

    let zoom = div()
        .id("zoom")
        .h(px(26.))
        .px(px(8.))
        .flex()
        .items_center()
        .rounded(px(6.))
        .text_size(px(12.))
        .text_color(p.text_weak)
        .cursor_pointer()
        .hover(|style| style.bg(p.bg_hover).text_color(p.text_strong))
        .child(format!("{:.0}%", editor.zoom_percent()))
        .debug_selector(|| "zoom".to_string())
        .tooltip(hint("switch between fit and 100% · ctrl 0 / ctrl 1"))
        .on_click(cx.listener(|this, _, _, cx| this.toggle_fit(cx)));

    let save = div()
        .id("save")
        .h(px(26.))
        .px(px(12.))
        .ml(px(6.))
        .flex()
        .items_center()
        .gap(px(6.))
        .rounded(px(6.))
        .bg(p.yolk)
        .text_size(px(12.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(p.bg)
        .cursor_pointer()
        .hover(|style| style.opacity(0.88))
        .child("save")
        .tooltip(hint("save and close · enter"))
        .on_click(cx.listener(|this, _, window, cx| this.export(Export::SaveAndClose, window, cx)));

    // The title bar drags the window from anywhere on it, so the buttons occlude it to get their clicks.
    let actions = div()
        .flex()
        .items_center()
        .gap(px(2.))
        .pr(px(10.))
        .occlude()
        .child(zoom)
        .child(divider(&p))
        .child(
            icon_button(
                "undo",
                IconName::Undo2,
                "undo · ctrl Z",
                false,
                editor.can_undo(),
                &p,
            )
            .on_click(cx.listener(|this, _, window, cx| this.undo(window, cx))),
        )
        .child(
            icon_button(
                "redo",
                IconName::Redo2,
                "redo · ctrl shift Z",
                false,
                editor.can_redo(),
                &p,
            )
            .on_click(cx.listener(|this, _, window, cx| this.redo(window, cx))),
        )
        .child(divider(&p))
        .child(
            icon_button(
                "copy",
                IconName::Copy,
                "copy the image · ctrl C",
                false,
                true,
                &p,
            )
            .on_click(cx.listener(|this, _, window, cx| this.export(Export::Copy, window, cx))),
        )
        .child(save);

    TitleBar::new()
        .h(px(40.))
        .bg(p.bg)
        .border_color(p.border_weak)
        .child(wordmark)
        .child(actions)
}
