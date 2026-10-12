use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use gpui_kit::assets::IconName;
use gpui_kit::component::WindowExt;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::notification::Notification;
use gpui_kit::*;
use image::RgbaImage;

use super::chrome;
use super::doc::Doc;
use super::effects;
use super::element::{Color, Element, Handle, PALETTE, Shape, snap_angle, snap_square};
use super::paint::{self, Space};
use super::render;
use crate::geom::{Pt, RectF, RectI};
use crate::shell::shell;
use crate::theme::palette;
use crate::{imaging, platform};

pub const WIDTHS: [f32; 4] = [2.0, 4.0, 7.0, 12.0];
pub const SIZES: [f32; 4] = [14.0, 20.0, 30.0, 44.0];
pub const STRENGTHS: [f32; 4] = [4.0, 8.0, 14.0, 22.0];

const HANDLE_REACH: f32 = 8.0;
const HIT_SLOP: f32 = 5.0;
const MIN_ZOOM: f32 = 0.05;
const MAX_ZOOM: f32 = 32.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Select,
    Rect,
    Ellipse,
    Arrow,
    Line,
    Pen,
    Highlighter,
    Text,
    Step,
    Blur,
    Pixelate,
    Crop,
}

impl Tool {
    pub const ALL: [Tool; 12] = [
        Tool::Select,
        Tool::Rect,
        Tool::Ellipse,
        Tool::Arrow,
        Tool::Line,
        Tool::Pen,
        Tool::Highlighter,
        Tool::Text,
        Tool::Step,
        Tool::Blur,
        Tool::Pixelate,
        Tool::Crop,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Tool::Select => "select",
            Tool::Rect => "rectangle",
            Tool::Ellipse => "ellipse",
            Tool::Arrow => "arrow",
            Tool::Line => "line",
            Tool::Pen => "pen",
            Tool::Highlighter => "highlighter",
            Tool::Text => "text",
            Tool::Step => "step number",
            Tool::Blur => "blur",
            Tool::Pixelate => "pixelate",
            Tool::Crop => "crop",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Tool::Select => "v",
            Tool::Rect => "r",
            Tool::Ellipse => "o",
            Tool::Arrow => "a",
            Tool::Line => "l",
            Tool::Pen => "p",
            Tool::Highlighter => "h",
            Tool::Text => "t",
            Tool::Step => "n",
            Tool::Blur => "b",
            Tool::Pixelate => "x",
            Tool::Crop => "c",
        }
    }

    pub fn icon(self) -> IconName {
        match self {
            Tool::Select => IconName::MousePointer2,
            Tool::Rect => IconName::Square,
            Tool::Ellipse => IconName::Circle,
            Tool::Arrow => IconName::ArrowUpRight,
            Tool::Line => IconName::Minus,
            Tool::Pen => IconName::Pencil,
            Tool::Highlighter => IconName::Highlighter,
            Tool::Text => IconName::Type,
            Tool::Step => IconName::Hash,
            Tool::Blur => IconName::Droplet,
            Tool::Pixelate => IconName::Grid3x3,
            Tool::Crop => IconName::Crop,
        }
    }
}

/// Which style controls make sense for a tool or a selection.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Caps {
    pub color: bool,
    pub width: bool,
    pub fill: bool,
    pub size: bool,
    pub strength: bool,
}

impl Caps {
    fn of_tool(tool: Tool) -> Self {
        let mut caps = Caps::default();
        match tool {
            Tool::Select | Tool::Crop => {}
            Tool::Rect | Tool::Ellipse => {
                caps.color = true;
                caps.width = true;
                caps.fill = true;
            }
            Tool::Arrow | Tool::Line | Tool::Pen | Tool::Highlighter => {
                caps.color = true;
                caps.width = true;
            }
            Tool::Text => {
                caps.color = true;
                caps.size = true;
                caps.fill = true;
            }
            Tool::Step => {
                caps.color = true;
                caps.size = true;
            }
            Tool::Blur | Tool::Pixelate => caps.strength = true,
        }
        caps
    }

    fn of_shape(shape: &Shape) -> Self {
        Self::of_tool(match shape {
            Shape::Rect(_) => Tool::Rect,
            Shape::Ellipse(_) => Tool::Ellipse,
            Shape::Line(..) => Tool::Line,
            Shape::Arrow(..) => Tool::Arrow,
            Shape::Pen(_) => Tool::Pen,
            Shape::Highlighter(_) => Tool::Highlighter,
            Shape::Text { .. } => Tool::Text,
            Shape::Step { .. } => Tool::Step,
            Shape::Blur(_) => Tool::Blur,
            Shape::Pixelate(_) => Tool::Pixelate,
        })
    }

    fn union(self, other: Caps) -> Caps {
        Caps {
            color: self.color || other.color,
            width: self.width || other.width,
            fill: self.fill || other.fill,
            size: self.size || other.size,
            strength: self.strength || other.strength,
        }
    }

    pub fn any(self) -> bool {
        self != Caps::default()
    }
}

/// The style new markup gets. Sizes are indexes into the preset tables.
#[derive(Clone, Copy)]
pub struct Style {
    pub color: Color,
    pub width: usize,
    pub size: usize,
    pub strength: usize,
    pub fill: bool,
    pub text_fill: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum StyleChange {
    Color(Color),
    Width(usize),
    Size(usize),
    Strength(usize),
    Fill(bool),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Export {
    Copy,
    Save,
    SaveAs,
    Done,
}

enum Gesture {
    Draw {
        id: u64,
        start: Pt,
    },
    Stroke {
        id: u64,
    },
    Move {
        start: Pt,
        originals: Vec<Element>,
    },
    Resize {
        id: u64,
        handle: Handle,
        original: Element,
    },
    Marquee {
        start: Pt,
        current: Pt,
    },
    Pan {
        from: Point<Pixels>,
        offset: Point<Pixels>,
    },
    Crop {
        start: Pt,
        current: Pt,
    },
}

struct EffectImage {
    key: (RectI, u32, bool),
    image: Arc<RenderImage>,
}

struct TextEdit {
    id: u64,
    input: Entity<InputState>,
    _subscription: Subscription,
}

/// Everything one frame of the canvas draws, copied out of the editor before painting starts.
struct Scene {
    space: Space,
    base: Arc<RenderImage>,
    full: RectF,
    visible: RectF,
    effects: Vec<(RectF, Arc<RenderImage>)>,
    elements: Vec<Element>,
    selected: Vec<RectF>,
    handles: Vec<(Pt, bool)>,
    marquee: Option<RectF>,
    crop: Option<RectF>,
    retired: Vec<Arc<RenderImage>>,
    cursor: CursorStyle,
    accent: Hsla,
    edge: Hsla,
}

pub struct Editor {
    doc: Doc,
    base: Arc<RenderImage>,
    pub(super) tool: Tool,
    pub(super) style: Style,
    selection: Vec<u64>,
    zoom: f32,
    offset: Point<Pixels>,
    fit: bool,
    canvas: Bounds<Pixels>,
    scale: f32,
    gesture: Option<Gesture>,
    space_held: bool,
    effects: HashMap<u64, EffectImage>,
    retired: Vec<Arc<RenderImage>>,
    text_edit: Option<TextEdit>,
    pub(super) saved: Option<PathBuf>,
    busy: bool,
    focus: FocusHandle,
}

impl Editor {
    pub fn new(
        image: Arc<RgbaImage>,
        saved: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        Self {
            base: imaging::render_image((*image).clone()),
            doc: Doc::new(image),
            tool: Tool::Arrow,
            style: Style {
                color: PALETTE[0],
                width: 1,
                size: 1,
                strength: 1,
                fill: false,
                text_fill: true,
            },
            selection: Vec::new(),
            zoom: 1.0,
            offset: Point::default(),
            fit: true,
            canvas: Bounds::default(),
            scale: window.scale_factor(),
            gesture: None,
            space_held: false,
            effects: HashMap::new(),
            retired: Vec::new(),
            text_edit: None,
            saved,
            busy: false,
            focus,
        }
    }

    pub(super) fn can_undo(&self) -> bool {
        self.doc.can_undo()
    }

    pub(super) fn can_redo(&self) -> bool {
        self.doc.can_redo()
    }

    pub(super) fn has_selection(&self) -> bool {
        !self.selection.is_empty()
    }

    pub(super) fn is_cropped(&self) -> bool {
        self.doc.crop != self.doc.full()
    }

    pub(super) fn zoom_percent(&self) -> f32 {
        self.zoom * self.scale * 100.0
    }

    pub(super) fn image_size(&self) -> (i32, i32) {
        (self.doc.crop.w, self.doc.crop.h)
    }

    fn selected(&self) -> impl Iterator<Item = &Element> {
        self.doc
            .elements
            .iter()
            .filter(|el| self.selection.contains(&el.id))
    }

    /// The style controls to show: the selection's when there is one, else the tool's.
    pub(super) fn caps(&self) -> Caps {
        if self.selection.is_empty() {
            return Caps::of_tool(self.tool);
        }
        self.selected()
            .map(|el| Caps::of_shape(&el.shape))
            .fold(Caps::default(), Caps::union)
    }

    /// The style the controls display: the first selected element's, else the tool defaults.
    pub(super) fn shown_style(&self) -> Style {
        let Some(element) = self.selected().next() else {
            let mut style = self.style;
            if self.tool == Tool::Text {
                style.fill = style.text_fill;
            }
            return style;
        };
        let nearest = |table: &[f32], value: f32| {
            let value = value / self.scale;
            (0..table.len())
                .min_by(|a, b| {
                    (table[*a] - value)
                        .abs()
                        .total_cmp(&(table[*b] - value).abs())
                })
                .unwrap_or(0)
        };
        Style {
            color: element.color,
            width: nearest(&WIDTHS, element.width),
            size: nearest(&SIZES, element.size),
            strength: nearest(&STRENGTHS, element.size),
            fill: element.fill,
            text_fill: self.style.text_fill,
        }
    }

    fn space(&self) -> Space {
        let snap = |value: Pixels| px((value.as_f32() * self.scale).round() / self.scale);
        Space {
            origin: point(
                snap(self.canvas.origin.x + self.offset.x),
                snap(self.canvas.origin.y + self.offset.y),
            ),
            zoom: self.zoom,
        }
    }

    fn view_target(&self) -> RectF {
        if self.tool == Tool::Crop {
            self.doc.full().to_f32()
        } else {
            self.doc.crop.to_f32()
        }
    }

    fn fit_view(&mut self) {
        let target = self.view_target();
        let (pad_x, pad_top, pad_bottom) = (40.0, 28.0, 132.0);
        let width = (self.canvas.size.width.as_f32() - pad_x * 2.0).max(60.0);
        let height = (self.canvas.size.height.as_f32() - pad_top - pad_bottom).max(60.0);
        let zoom = (width / target.w.max(1.0))
            .min(height / target.h.max(1.0))
            .min(1.0 / self.scale);
        self.zoom = zoom.max(0.005);
        self.offset = point(
            px(pad_x + (width - target.w * self.zoom) / 2.0 - target.x * self.zoom),
            px(pad_top + (height - target.h * self.zoom) / 2.0 - target.y * self.zoom),
        );
    }

    fn zoom_about(&mut self, anchor: Point<Pixels>, factor: f32) {
        let zoom = (self.zoom * factor).clamp(MIN_ZOOM / self.scale, MAX_ZOOM / self.scale);
        let ratio = zoom / self.zoom;
        let local = anchor - self.canvas.origin;
        self.offset = point(
            local.x - (local.x - self.offset.x) * ratio,
            local.y - (local.y - self.offset.y) * ratio,
        );
        self.zoom = zoom;
        self.fit = false;
    }

    pub(super) fn zoom_step(&mut self, factor: f32, cx: &mut Context<Self>) {
        self.zoom_about(self.canvas.center(), factor);
        cx.notify();
    }

    pub(super) fn toggle_fit(&mut self, cx: &mut Context<Self>) {
        if self.fit {
            let factor = 1.0 / (self.zoom * self.scale);
            self.zoom_about(self.canvas.center(), factor);
        } else {
            self.fit = true;
        }
        cx.notify();
    }

    pub(super) fn set_tool(&mut self, tool: Tool, window: &mut Window, cx: &mut Context<Self>) {
        self.commit_text(window, cx);
        if (self.tool == Tool::Crop) != (tool == Tool::Crop) {
            self.fit = true;
        }
        self.tool = tool;
        self.gesture = None;
        self.selection.clear();
        cx.notify();
    }

    fn new_element(&mut self, shape: Shape) -> Element {
        let style = self.style;
        let (fill, size) = match &shape {
            Shape::Text { .. } => (style.text_fill, SIZES[style.size]),
            Shape::Step { .. } => (true, SIZES[style.size]),
            Shape::Blur(_) | Shape::Pixelate(_) => (false, STRENGTHS[style.strength]),
            Shape::Rect(_) | Shape::Ellipse(_) => (style.fill, 0.0),
            _ => (false, 0.0),
        };
        Element {
            id: self.doc.next_id(),
            shape,
            color: style.color,
            fill,
            width: WIDTHS[style.width] * self.scale,
            size: size * self.scale,
        }
    }

    /// Changes the tool defaults, and the selected elements the change applies to.
    pub(super) fn apply_style(
        &mut self,
        change: StyleChange,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let text_subject = self
            .selected()
            .next()
            .map_or(self.tool == Tool::Text, |el| el.is_text());
        match change {
            StyleChange::Color(color) => self.style.color = color,
            StyleChange::Width(index) => self.style.width = index,
            StyleChange::Size(index) => self.style.size = index,
            StyleChange::Strength(index) => self.style.strength = index,
            StyleChange::Fill(fill) if text_subject => self.style.text_fill = fill,
            StyleChange::Fill(fill) => self.style.fill = fill,
        }
        if !self.selection.is_empty() {
            self.doc.checkpoint();
            let scale = self.scale;
            let ids = self.selection.clone();
            for element in self
                .doc
                .elements
                .iter_mut()
                .filter(|el| ids.contains(&el.id))
            {
                let caps = Caps::of_shape(&element.shape);
                match change {
                    StyleChange::Color(color) if caps.color => element.color = color,
                    StyleChange::Width(index) if caps.width => {
                        element.width = WIDTHS[index] * scale
                    }
                    StyleChange::Size(index) if caps.size => element.size = SIZES[index] * scale,
                    StyleChange::Strength(index) if caps.strength => {
                        element.size = STRENGTHS[index] * scale
                    }
                    StyleChange::Fill(fill) if caps.fill => element.fill = fill,
                    _ => {}
                }
                if let Shape::Text { text, extent, .. } = &mut element.shape {
                    *extent = paint::measure_text(text, element.size, window);
                }
            }
            self.doc.discard_unchanged();
        }
        cx.notify();
    }

    pub(super) fn undo(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.commit_text(window, cx);
        if self.doc.undo() {
            self.after_history(cx);
        }
    }

    pub(super) fn redo(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.commit_text(window, cx);
        if self.doc.redo() {
            self.after_history(cx);
        }
    }

    fn after_history(&mut self, cx: &mut Context<Self>) {
        self.selection
            .retain(|id| self.doc.elements.iter().any(|el| el.id == *id));
        self.gesture = None;
        cx.notify();
    }

    pub(super) fn delete_selection(&mut self, cx: &mut Context<Self>) {
        if self.selection.is_empty() {
            return;
        }
        self.doc.checkpoint();
        self.doc.remove(&self.selection);
        self.selection.clear();
        cx.notify();
    }

    pub(super) fn reorder(&mut self, to_front: bool, cx: &mut Context<Self>) {
        if self.selection.is_empty() {
            return;
        }
        self.doc.checkpoint();
        self.doc.reorder(&self.selection, to_front);
        self.doc.discard_unchanged();
        cx.notify();
    }

    pub(super) fn reset_crop(&mut self, cx: &mut Context<Self>) {
        if !self.is_cropped() {
            return;
        }
        self.doc.checkpoint();
        self.doc.crop = self.doc.full();
        self.fit = true;
        cx.notify();
    }

    fn duplicate(&mut self, cx: &mut Context<Self>) {
        let copies: Vec<Element> = self.selected().cloned().collect();
        if copies.is_empty() {
            return;
        }
        self.doc.checkpoint();
        let shift = 16.0 * self.scale;
        let mut ids = Vec::new();
        for mut copy in copies {
            copy.id = self.doc.next_id();
            copy.translate(shift, shift);
            ids.push(copy.id);
            self.doc.add(copy);
        }
        self.selection = ids;
        cx.notify();
    }

    fn nudge(&mut self, dx: f32, dy: f32, cx: &mut Context<Self>) {
        if self.selection.is_empty() {
            return;
        }
        self.doc.checkpoint();
        let ids = self.selection.clone();
        for element in self
            .doc
            .elements
            .iter_mut()
            .filter(|el| ids.contains(&el.id))
        {
            element.translate(dx, dy);
        }
        cx.notify();
    }

    fn top_hit(&self, p: Pt) -> Option<u64> {
        let slop = HIT_SLOP / self.zoom;
        self.doc
            .elements
            .iter()
            .rev()
            .find(|el| el.hit(p, slop))
            .map(|el| el.id)
    }

    fn handle_hit(&self, position: Point<Pixels>) -> Option<(u64, Handle)> {
        let [id] = self.selection.as_slice() else {
            return None;
        };
        let element = self.doc.get(*id)?;
        let space = self.space();
        element.handles().into_iter().find_map(|(handle, at)| {
            let at = space.point(at);
            let distance = (at.x - position.x)
                .as_f32()
                .hypot((at.y - position.y).as_f32());
            (distance <= HANDLE_REACH).then_some((*id, handle))
        })
    }

    fn begin_text(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Shape::Text { text, .. }) = self.doc.get(id).map(|el| &el.shape) else {
            return;
        };
        let text = text.to_string();
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("type, then press enter")
                .default_value(text)
        });
        let subscription = cx.subscribe_in(
            &input,
            window,
            |this, input, event: &InputEvent, window, cx| match event {
                InputEvent::Change => {
                    let value = input.read(cx).value().to_string();
                    this.set_text(&value, window, cx);
                }
                InputEvent::PressEnter { .. } | InputEvent::Blur => {
                    this.commit_text(window, cx);
                    this.focus.focus(window, cx);
                }
                _ => {}
            },
        );
        input.update(cx, |state, cx| state.focus(window, cx));
        self.selection = vec![id];
        self.text_edit = Some(TextEdit {
            id,
            input,
            _subscription: subscription,
        });
        cx.notify();
    }

    fn set_text(&mut self, value: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.text_edit.as_ref().map(|edit| edit.id) else {
            return;
        };
        if let Some(element) = self.doc.get_mut(id) {
            let size = element.size;
            if let Shape::Text { text, extent, .. } = &mut element.shape {
                *text = Arc::from(value);
                *extent = paint::measure_text(value, size, window);
            }
        }
        cx.notify();
    }

    /// Ends text entry. A text element left empty is removed.
    fn commit_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let Some(edit) = self.text_edit.take() else {
            return;
        };
        if self.doc.get(edit.id).is_some_and(|el| el.is_degenerate()) {
            self.doc.remove(&[edit.id]);
            self.selection.retain(|id| *id != edit.id);
        }
        self.doc.discard_unchanged();
        cx.notify();
    }

    fn mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.commit_text(window, cx);
        window.focus(&self.focus, cx);
        let panning = event.button == MouseButton::Middle
            || (event.button == MouseButton::Left && self.space_held);
        if panning {
            self.gesture = Some(Gesture::Pan {
                from: event.position,
                offset: self.offset,
            });
            self.fit = false;
            cx.notify();
            return;
        }
        if event.button != MouseButton::Left {
            return;
        }
        let p = self.space().to_image(event.position);
        if let Some((id, handle)) = self.handle_hit(event.position)
            && let Some(original) = self.doc.get(id).cloned()
        {
            self.doc.checkpoint();
            self.gesture = Some(Gesture::Resize {
                id,
                handle,
                original,
            });
            cx.notify();
            return;
        }
        match self.tool {
            Tool::Select => self.select_at(p, event, window, cx),
            Tool::Crop => {
                self.gesture = Some(Gesture::Crop {
                    start: p,
                    current: p,
                })
            }
            Tool::Text => {
                self.doc.checkpoint();
                let element = self.new_element(Shape::Text {
                    origin: p,
                    text: Arc::from(""),
                    extent: (0.0, 0.0),
                });
                let id = element.id;
                self.doc.add(element);
                self.set_extent(id, window);
                self.begin_text(id, window, cx);
            }
            Tool::Step => {
                self.doc.checkpoint();
                let number = self.doc.next_step();
                let element = self.new_element(Shape::Step { center: p, number });
                self.selection = vec![element.id];
                self.doc.add(element);
            }
            Tool::Pen | Tool::Highlighter => {
                self.doc.checkpoint();
                let points = Arc::new(vec![p]);
                let element = self.new_element(if self.tool == Tool::Pen {
                    Shape::Pen(points)
                } else {
                    Shape::Highlighter(points)
                });
                self.selection.clear();
                self.gesture = Some(Gesture::Stroke { id: element.id });
                self.doc.add(element);
            }
            Tool::Rect | Tool::Ellipse | Tool::Arrow | Tool::Line | Tool::Blur | Tool::Pixelate => {
                self.doc.checkpoint();
                let element = self.new_element(drawn_shape(self.tool, p, p));
                self.selection = vec![element.id];
                self.gesture = Some(Gesture::Draw {
                    id: element.id,
                    start: p,
                });
                self.doc.add(element);
            }
        }
        cx.notify();
    }

    fn set_extent(&mut self, id: u64, window: &Window) {
        if let Some(element) = self.doc.get_mut(id) {
            let size = element.size;
            if let Shape::Text { text, extent, .. } = &mut element.shape {
                *extent = paint::measure_text(text, size, window);
            }
        }
    }

    fn select_at(
        &mut self,
        p: Pt,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(id) = self.top_hit(p) else {
            if !event.modifiers.shift {
                self.selection.clear();
            }
            self.gesture = Some(Gesture::Marquee {
                start: p,
                current: p,
            });
            return;
        };
        if event.modifiers.shift {
            match self.selection.iter().position(|selected| *selected == id) {
                Some(index) => {
                    self.selection.remove(index);
                }
                None => self.selection.push(id),
            }
            return;
        }
        if !self.selection.contains(&id) {
            self.selection = vec![id];
        }
        if event.click_count >= 2 && self.doc.get(id).is_some_and(|el| el.is_text()) {
            self.doc.checkpoint();
            self.begin_text(id, window, cx);
            return;
        }
        self.doc.checkpoint();
        self.gesture = Some(Gesture::Move {
            start: p,
            originals: self.selected().cloned().collect(),
        });
    }

    fn mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        let p = self.space().to_image(event.position);
        let constrain = event.modifiers.shift;
        let min_step = (1.5 / self.zoom).max(0.75);
        let Some(gesture) = &mut self.gesture else {
            return;
        };
        match gesture {
            Gesture::Pan { from, offset } => {
                self.offset = *offset + (event.position - *from);
            }
            Gesture::Draw { id, start } => {
                let to = match self.tool {
                    Tool::Arrow | Tool::Line if constrain => snap_angle(*start, p),
                    _ if constrain => snap_square(*start, p),
                    _ => p,
                };
                let shape = drawn_shape(self.tool, *start, to);
                if let Some(element) = self.doc.get_mut(*id) {
                    element.shape = shape;
                }
            }
            Gesture::Stroke { id } => {
                if let Some(element) = self.doc.get_mut(*id)
                    && let Shape::Pen(points) | Shape::Highlighter(points) = &mut element.shape
                    && points
                        .last()
                        .is_none_or(|last| last.distance(p) >= min_step)
                {
                    Arc::make_mut(points).push(p);
                }
            }
            Gesture::Move { start, originals } => {
                let (mut dx, mut dy) = (p.x - start.x, p.y - start.y);
                if constrain {
                    if dx.abs() > dy.abs() {
                        dy = 0.0;
                    } else {
                        dx = 0.0;
                    }
                }
                for original in originals.iter() {
                    if let Some(element) = self.doc.get_mut(original.id) {
                        *element = original.clone();
                        element.translate(dx, dy);
                    }
                }
            }
            Gesture::Resize {
                id,
                handle,
                original,
            } => {
                if let Some(element) = self.doc.get_mut(*id) {
                    element.drag_handle(original, *handle, p, constrain);
                }
            }
            Gesture::Marquee { start, current } => {
                *current = p;
                let area = RectF::from_points(*start, p);
                self.selection = self
                    .doc
                    .elements
                    .iter()
                    .filter(|el| el.bounds().intersects(&area))
                    .map(|el| el.id)
                    .collect();
            }
            Gesture::Crop { current, .. } => *current = p,
        }
        cx.notify();
    }

    fn mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some(gesture) = self.gesture.take() else {
            return;
        };
        match gesture {
            Gesture::Draw { id, .. } | Gesture::Stroke { id } => {
                if self.doc.get(id).is_some_and(|el| el.is_degenerate()) {
                    self.doc.remove(&[id]);
                    self.selection.retain(|selected| *selected != id);
                }
                self.doc.discard_unchanged();
            }
            Gesture::Move { .. } | Gesture::Resize { .. } => self.doc.discard_unchanged(),
            Gesture::Crop { start, current } => {
                let crop = RectF::from_points(start, current).to_pixels(self.doc.full());
                if crop.w >= 8 && crop.h >= 8 && crop != self.doc.crop {
                    self.doc.checkpoint();
                    self.doc.crop = crop;
                    self.tool = Tool::Select;
                    self.fit = true;
                }
            }
            Gesture::Marquee { .. } | Gesture::Pan { .. } => {}
        }
        cx.notify();
    }

    fn scroll(&mut self, event: &ScrollWheelEvent, _: &mut Window, cx: &mut Context<Self>) {
        let delta = event.delta.pixel_delta(px(20.)).y.as_f32();
        if delta != 0.0 {
            self.zoom_about(event.position, (delta * 0.0025).exp());
            cx.notify();
        }
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();
        if self.text_edit.is_some() {
            if key == "escape" {
                self.commit_text(window, cx);
                window.focus(&self.focus, cx);
                cx.stop_propagation();
            }
            return;
        }
        if !self.focus.is_focused(window) {
            return;
        }
        let modifiers = event.keystroke.modifiers;
        let (command, shift) = (modifiers.control || modifiers.platform, modifiers.shift);
        let step = if shift { 10.0 } else { 1.0 } * self.scale;
        let mut handled = true;
        match (command, shift, key) {
            (true, false, "z") => self.undo(window, cx),
            (true, true, "z") | (true, false, "y") => self.redo(window, cx),
            (true, false, "c") => self.export(Export::Copy, window, cx),
            (true, false, "s") => self.export(Export::Save, window, cx),
            (true, true, "s") => self.export(Export::SaveAs, window, cx),
            (true, false, "d") => self.duplicate(cx),
            (true, false, "a") => {
                self.selection = self.doc.elements.iter().map(|el| el.id).collect();
                self.tool = Tool::Select;
            }
            (true, _, "0") => self.fit = true,
            (true, _, "1") => {
                self.fit = true;
                self.toggle_fit(cx);
            }
            (true, _, "=" | "+") => self.zoom_step(1.25, cx),
            (true, _, "-") => self.zoom_step(0.8, cx),
            (false, _, "delete" | "backspace") => self.delete_selection(cx),
            (false, _, "enter") => self.export(Export::Done, window, cx),
            (false, _, "escape") => {
                if self.gesture.take().is_some() {
                    self.doc.discard_unchanged();
                } else if !self.selection.is_empty() {
                    self.selection.clear();
                } else {
                    self.set_tool(Tool::Select, window, cx);
                }
            }
            (false, _, "space") => self.space_held = true,
            (false, _, "left") => self.nudge(-step, 0.0, cx),
            (false, _, "right") => self.nudge(step, 0.0, cx),
            (false, _, "up") => self.nudge(0.0, -step, cx),
            (false, _, "down") => self.nudge(0.0, step, cx),
            (false, false, "[") => self.reorder(false, cx),
            (false, false, "]") => self.reorder(true, cx),
            (false, false, key) => match Tool::ALL.into_iter().find(|tool| tool.key() == key) {
                Some(tool) => self.set_tool(tool, window, cx),
                None => handled = false,
            },
            _ => handled = false,
        }
        if handled {
            cx.stop_propagation();
            cx.notify();
        }
    }

    fn on_key_up(&mut self, event: &KeyUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        if event.keystroke.key == "space" {
            self.space_held = false;
            cx.notify();
        }
    }

    /// Flattens the markup off the UI thread, then copies, saves or both.
    pub(super) fn export(&mut self, what: Export, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.commit_text(window, cx);
        let config = shell(cx).read(cx).config.clone();
        let target = match what {
            Export::Copy | Export::SaveAs => None,
            Export::Save => Some(
                self.saved
                    .clone()
                    .unwrap_or_else(|| config.next_file("png")),
            ),
            Export::Done => self
                .saved
                .clone()
                .or_else(|| config.save_to_disk.then(|| config.next_file("png"))),
        };
        let prompt = (what == Export::SaveAs).then(|| {
            let _ = std::fs::create_dir_all(&config.save_dir);
            cx.prompt_for_new_path(&config.save_dir, Some("bawkshot.png"))
        });
        let (base, crop) = (self.doc.base.clone(), self.doc.crop);
        let elements = self.doc.elements.clone();
        let untouched = elements.is_empty() && crop == self.doc.full();
        let metrics = paint::text_metrics(window);
        let copy = matches!(what, Export::Copy | Export::Done);
        self.busy = true;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let target = match prompt {
                None => target,
                Some(prompt) => match prompt.await {
                    Ok(Ok(Some(path))) => Some(path.with_extension("png")),
                    _ => {
                        let _ = this.update(cx, |this, cx| {
                            this.busy = false;
                            cx.notify();
                        });
                        return;
                    }
                },
            };
            let path = target.clone();
            let outcome = cx
                .background_executor()
                .spawn(async move {
                    let image = if untouched {
                        base
                    } else {
                        Arc::new(render::flatten(&base, crop, &elements, metrics)?)
                    };
                    if copy {
                        imaging::copy_to_clipboard(&image)?;
                    }
                    if let Some(path) = &path {
                        imaging::save_png(&image, path)?;
                    }
                    Ok::<(), String>(())
                })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                cx.notify();
                if let Err(err) = outcome {
                    window.push_notification(Notification::warning(err), cx);
                    return;
                }
                let saved = target.as_ref().map(|path| path.display().to_string());
                if target.is_some() {
                    this.saved = target;
                }
                if what == Export::Done {
                    window.remove_window();
                    return;
                }
                let message = match (copy, saved) {
                    (_, Some(path)) => format!("saved to {path}"),
                    _ => "copied to the clipboard".to_string(),
                };
                window.push_notification(Notification::info(message), cx);
            });
        })
        .detach();
    }

    /// Rebuilds the blurred and pixelated patches whose area or strength changed.
    fn sync_effects(&mut self) {
        let full = self.doc.full();
        let mut alive = Vec::new();
        for element in &self.doc.elements {
            let (rect, pixelate) = match &element.shape {
                Shape::Blur(rect) => (rect, false),
                Shape::Pixelate(rect) => (rect, true),
                _ => continue,
            };
            alive.push(element.id);
            let key = (rect.to_pixels(full), element.size.to_bits(), pixelate);
            if self
                .effects
                .get(&element.id)
                .is_some_and(|effect| effect.key == key)
            {
                continue;
            }
            let fresh = effects::patch(&self.doc.base, element).map(|(_, patch)| EffectImage {
                key,
                image: imaging::render_image(patch),
            });
            let stale = match fresh {
                Some(effect) => self.effects.insert(element.id, effect),
                None => self.effects.remove(&element.id),
            };
            self.retired.extend(stale.map(|effect| effect.image));
        }
        let dead: Vec<u64> = self
            .effects
            .keys()
            .filter(|id| !alive.contains(id))
            .copied()
            .collect();
        for id in dead {
            self.retired
                .extend(self.effects.remove(&id).map(|effect| effect.image));
        }
    }

    fn cursor(&self) -> CursorStyle {
        match (&self.gesture, self.tool) {
            (Some(Gesture::Pan { .. }), _) => CursorStyle::ClosedHand,
            _ if self.space_held => CursorStyle::OpenHand,
            (_, Tool::Select) => CursorStyle::Arrow,
            (_, Tool::Text) => CursorStyle::IBeam,
            _ => CursorStyle::Crosshair,
        }
    }

    fn scene(&mut self, bounds: Bounds<Pixels>, cx: &App) -> Scene {
        if bounds != self.canvas {
            self.canvas = bounds;
        }
        if self.fit {
            self.fit_view();
        }
        self.sync_effects();
        let p = palette(cx);
        let cropping = self.tool == Tool::Crop;
        let crop = match &self.gesture {
            Some(Gesture::Crop { start, current }) => Some(RectF::from_points(*start, *current)),
            _ if cropping => Some(self.doc.crop.to_f32()),
            _ => None,
        };
        let single = match self.selection.as_slice() {
            [id] => self.doc.get(*id),
            _ => None,
        };
        let handles = single.map_or(Vec::new(), |element| {
            let round = matches!(element.shape, Shape::Line(..) | Shape::Arrow(..));
            element
                .handles()
                .into_iter()
                .map(|(_, at)| (at, round))
                .collect()
        });
        Scene {
            space: self.space(),
            base: self.base.clone(),
            full: self.doc.full().to_f32(),
            visible: self.view_target(),
            effects: self
                .doc
                .elements
                .iter()
                .filter_map(|el| {
                    let effect = self.effects.get(&el.id)?;
                    Some((effect.key.0.to_f32(), effect.image.clone()))
                })
                .collect(),
            elements: self.doc.elements.clone(),
            selected: self.selected().map(|el| el.bounds()).collect(),
            handles,
            marquee: match &self.gesture {
                Some(Gesture::Marquee { start, current }) => {
                    Some(RectF::from_points(*start, *current))
                }
                _ => None,
            },
            crop,
            retired: std::mem::take(&mut self.retired),
            cursor: self.cursor(),
            accent: p.yolk,
            edge: p.border_weak,
        }
    }

    /// Where the text entry box goes: under the text being edited, in canvas coordinates.
    pub(super) fn text_box(&self) -> Option<(Entity<InputState>, Point<Pixels>)> {
        let edit = self.text_edit.as_ref()?;
        let bounds = self.space().rect(self.doc.get(edit.id)?.bounds());
        let local = bounds.bottom_left() - self.canvas.origin;
        let x = local
            .x
            .clamp(px(8.), (self.canvas.size.width - px(288.)).max(px(8.)));
        let y = (local.y + px(10.)).clamp(px(8.), (self.canvas.size.height - px(48.)).max(px(8.)));
        Some((edit.input.clone(), point(x, y)))
    }
}

fn drawn_shape(tool: Tool, from: Pt, to: Pt) -> Shape {
    let rect = RectF::from_points(from, to);
    match tool {
        Tool::Ellipse => Shape::Ellipse(rect),
        Tool::Arrow => Shape::Arrow(from, to),
        Tool::Line => Shape::Line(from, to),
        Tool::Blur => Shape::Blur(rect),
        Tool::Pixelate => Shape::Pixelate(rect),
        _ => Shape::Rect(rect),
    }
}

impl Scene {
    fn paint(self, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
        for image in self.retired {
            let _ = window.drop_image(image);
        }
        let space = self.space;
        let visible = space.rect(self.visible);
        let clip = visible.intersect(&bounds);
        window.with_content_mask(Some(ContentMask { bounds: clip }), |window| {
            let _ = window.paint_image(
                clip,
                space.rect(self.full),
                Corners::default(),
                self.base.clone(),
                0,
                false,
            );
            for (rect, image) in &self.effects {
                let area = space.rect(*rect);
                let _ = window.paint_image(
                    area.intersect(&clip),
                    area,
                    Corners::default(),
                    image.clone(),
                    0,
                    false,
                );
            }
            for element in &self.elements {
                paint::element(element, &space, window, cx);
            }
        });
        paint::frame(visible.dilate(px(1.)), self.edge, false, window);

        if let Some(crop) = self.crop {
            let kept = space.rect(crop).intersect(&visible);
            let shade = hsla(0., 0., 0., 0.55);
            let band = |left: Pixels, top: Pixels, right: Pixels, bottom: Pixels| {
                Bounds::from_corners(point(left, top), point(right, bottom))
            };
            for quad in [
                band(visible.left(), visible.top(), visible.right(), kept.top()),
                band(
                    visible.left(),
                    kept.bottom(),
                    visible.right(),
                    visible.bottom(),
                ),
                band(visible.left(), kept.top(), kept.left(), kept.bottom()),
                band(kept.right(), kept.top(), visible.right(), kept.bottom()),
            ] {
                window.paint_quad(fill(quad, shade));
            }
            paint::frame(kept, self.accent, false, window);
        }
        for rect in &self.selected {
            paint::frame(space.rect(*rect).dilate(px(3.)), self.accent, false, window);
        }
        for (at, round) in &self.handles {
            paint::handle(space.point(*at), *round, self.accent, window);
        }
        if let Some(marquee) = self.marquee {
            let area = space.rect(marquee);
            window.paint_quad(fill(area, self.accent.opacity(0.12)));
            paint::frame(area, self.accent, true, window);
        }
    }
}

impl Render for Editor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let entity = cx.entity();
        let scene_entity = entity.clone();
        let surface = canvas(
            move |bounds, window, cx| {
                let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
                let scene = scene_entity.update(cx, |this, cx| this.scene(bounds, cx));
                (hitbox, scene)
            },
            move |bounds, (hitbox, scene), window, cx| {
                window.set_cursor_style(scene.cursor, &hitbox);
                scene.paint(bounds, window, cx);
                let (view, hit) = (entity.clone(), hitbox.clone());
                window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
                    if phase == DispatchPhase::Bubble && hit.is_hovered(window) {
                        view.update(cx, |this, cx| this.mouse_down(event, window, cx));
                        window.prevent_default();
                    }
                });
                let view = entity.clone();
                window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                    if phase == DispatchPhase::Bubble {
                        view.update(cx, |this, cx| this.mouse_move(event, window, cx));
                    }
                });
                let view = entity.clone();
                window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
                    if phase == DispatchPhase::Bubble {
                        view.update(cx, |this, cx| this.mouse_up(event, window, cx));
                    }
                });
                window.on_mouse_event(move |event: &ScrollWheelEvent, phase, window, cx| {
                    if phase == DispatchPhase::Bubble && hitbox.is_hovered(window) {
                        entity.update(cx, |this, cx| this.scroll(event, window, cx));
                    }
                });
            },
        )
        .absolute()
        .size_full();

        div()
            .size_full()
            .flex()
            .flex_col()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key_down))
            .on_key_up(cx.listener(Self::on_key_up))
            .bg(p.bg)
            .text_color(p.text)
            .text_size(px(13.))
            .child(chrome::title_bar(self, cx))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .relative()
                    .overflow_hidden()
                    .bg(p.canvas)
                    .child(surface)
                    .children(chrome::text_box(self, cx))
                    .child(chrome::toolbar(self, window, cx)),
            )
    }
}

/// Opens a capture in its own editor window, sized to the image where the screen allows.
pub fn open(image: Arc<RgbaImage>, saved: Option<PathBuf>, cx: &mut App) {
    use gpui_kit::component::TitleBar;

    let (x, y) = platform::cursor_position();
    let scale = platform::monitor_at(x, y).map_or(1.0, |monitor| monitor.scale);
    let room = cx
        .primary_display()
        .map_or(size(px(1600.), px(900.)), |display| {
            display.visible_bounds().size
        });
    let wanted = size(
        px(image.width() as f32 / scale + 120.),
        px(image.height() as f32 / scale + 230.),
    );
    let fitted = size(
        wanted
            .width
            .clamp(px(920.), (room.width * 0.92).max(px(920.))),
        wanted
            .height
            .clamp(px(620.), (room.height * 0.92).max(px(620.))),
    );
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, fitted, cx))),
        window_min_size: Some(size(px(760.), px(480.))),
        app_id: Some("bawkshot".into()),
        ..TitleBar::window_options()
    };
    let opened = open_window(options, cx, |window, cx| {
        window.set_window_title("bawkshot");
        cx.new(|cx| Editor::new(image, saved, window, cx))
    });
    match opened {
        Ok((handle, _)) => {
            let _ = handle.update(cx, |_, window, cx| {
                window.activate_window();
                platform::bring_to_front(window, cx);
            });
            cx.activate(true);
        }
        Err(err) => log::error!("could not open the editor: {err}"),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use gpui_kit::{
        Entity, Modifiers, MouseButton, Pixels, Point, ScrollDelta, ScrollWheelEvent,
        TestAppContext, TouchPhase, VisualTestContext, point, px,
    };
    use image::RgbaImage;

    use super::{Editor, StyleChange, Tool};
    use crate::editor::element::{Element, PALETTE, Shape};
    use crate::geom::{Pt, RectF, RectI, pt};

    fn editor(cx: &mut TestAppContext) -> (Entity<Editor>, &mut VisualTestContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::theme::init(cx);
        });
        cx.add_window_view(|window, cx| {
            Editor::new(Arc::new(RgbaImage::new(400, 300)), None, window, cx)
        })
    }

    fn at(view: &Entity<Editor>, cx: &mut VisualTestContext, p: (f32, f32)) -> Point<Pixels> {
        view.read_with(cx, |editor, _| editor.space().point(pt(p.0, p.1)))
    }

    fn drag(
        view: &Entity<Editor>,
        cx: &mut VisualTestContext,
        path: &[(f32, f32)],
        modifiers: Modifiers,
    ) {
        let first = at(view, cx, path[0]);
        cx.simulate_mouse_down(first, MouseButton::Left, modifiers);
        let mut last = first;
        for step in &path[1..] {
            last = at(view, cx, *step);
            cx.simulate_mouse_move(last, MouseButton::Left, modifiers);
        }
        cx.simulate_mouse_up(last, MouseButton::Left, modifiers);
    }

    fn click(view: &Entity<Editor>, cx: &mut VisualTestContext, p: (f32, f32)) {
        let position = at(view, cx, p);
        cx.simulate_click(position, Modifiers::none());
    }

    fn elements(view: &Entity<Editor>, cx: &mut VisualTestContext) -> Vec<Element> {
        view.read_with(cx, |editor, _| editor.doc.elements.clone())
    }

    fn near(a: Pt, b: (f32, f32)) -> bool {
        (a.x - b.0).abs() < 0.5 && (a.y - b.1).abs() < 0.5
    }

    fn near_rect(a: RectF, b: (f32, f32, f32, f32)) -> bool {
        near(pt(a.x, a.y), (b.0, b.1)) && near(pt(a.w, a.h), (b.2, b.3))
    }

    #[gpui_kit::test]
    fn a_drag_draws_an_arrow_that_undo_and_redo_follow(cx: &mut TestAppContext) {
        let (view, cx) = editor(cx);
        drag(
            &view,
            cx,
            &[(50.0, 50.0), (150.0, 120.0)],
            Modifiers::none(),
        );
        let drawn = elements(&view, cx);
        let [arrow] = drawn.as_slice() else {
            panic!("expected one element, got {}", drawn.len());
        };
        let Shape::Arrow(a, b) = arrow.shape else {
            panic!("expected an arrow, got {:?}", arrow.shape);
        };
        assert!(
            near(a, (50.0, 50.0)) && near(b, (150.0, 120.0)),
            "{a:?} {b:?}"
        );
        assert_eq!(
            view.read_with(cx, |editor, _| editor.selection.clone()),
            [arrow.id]
        );

        cx.simulate_keystrokes("ctrl-z");
        assert!(elements(&view, cx).is_empty());
        cx.simulate_keystrokes("ctrl-shift-z");
        assert_eq!(elements(&view, cx).len(), 1);
    }

    #[gpui_kit::test]
    fn a_click_without_a_drag_draws_nothing(cx: &mut TestAppContext) {
        let (view, cx) = editor(cx);
        click(&view, cx, (80.0, 80.0));
        assert!(elements(&view, cx).is_empty());
        assert!(!view.read_with(cx, |editor, _| editor.can_undo()));
    }

    #[gpui_kit::test]
    fn shift_constrains_a_rectangle_to_a_square(cx: &mut TestAppContext) {
        let (view, cx) = editor(cx);
        cx.simulate_keystrokes("r");
        drag(
            &view,
            cx,
            &[(20.0, 20.0), (120.0, 60.0)],
            Modifiers::shift(),
        );
        let drawn = elements(&view, cx);
        let Shape::Rect(rect) = drawn[0].shape else {
            panic!("expected a rectangle, got {:?}", drawn[0].shape);
        };
        assert!(near_rect(rect, (20.0, 20.0, 100.0, 100.0)), "{rect:?}");
    }

    #[gpui_kit::test]
    fn the_select_tool_moves_then_deletes(cx: &mut TestAppContext) {
        let (view, cx) = editor(cx);
        cx.simulate_keystrokes("l");
        drag(&view, cx, &[(50.0, 50.0), (150.0, 50.0)], Modifiers::none());
        cx.simulate_keystrokes("v");
        assert_eq!(view.read_with(cx, |editor, _| editor.tool), Tool::Select);

        drag(
            &view,
            cx,
            &[(100.0, 50.0), (100.0, 90.0)],
            Modifiers::none(),
        );
        let moved = elements(&view, cx);
        let Shape::Line(a, b) = moved[0].shape else {
            panic!("expected a line, got {:?}", moved[0].shape);
        };
        assert!(
            near(a, (50.0, 90.0)) && near(b, (150.0, 90.0)),
            "{a:?} {b:?}"
        );

        cx.simulate_keystrokes("delete");
        assert!(elements(&view, cx).is_empty());
        cx.simulate_keystrokes("ctrl-z ctrl-z");
        let restored = elements(&view, cx);
        let Shape::Line(a, _) = restored[0].shape else {
            panic!("expected a line, got {:?}", restored[0].shape);
        };
        assert!(near(a, (50.0, 50.0)), "{a:?}");
    }

    #[gpui_kit::test]
    fn dragging_a_corner_handle_resizes(cx: &mut TestAppContext) {
        let (view, cx) = editor(cx);
        cx.simulate_keystrokes("r");
        drag(&view, cx, &[(20.0, 20.0), (120.0, 80.0)], Modifiers::none());
        drag(
            &view,
            cx,
            &[(120.0, 80.0), (160.0, 100.0)],
            Modifiers::none(),
        );
        let drawn = elements(&view, cx);
        assert_eq!(drawn.len(), 1);
        let Shape::Rect(rect) = drawn[0].shape else {
            panic!("expected a rectangle, got {:?}", drawn[0].shape);
        };
        assert!(near_rect(rect, (20.0, 20.0, 140.0, 80.0)), "{rect:?}");
    }

    #[gpui_kit::test]
    fn the_crop_tool_crops_and_hands_back_to_select(cx: &mut TestAppContext) {
        let (view, cx) = editor(cx);
        cx.simulate_keystrokes("c");
        drag(
            &view,
            cx,
            &[(40.0, 30.0), (240.0, 180.0)],
            Modifiers::none(),
        );
        let (crop, tool) = view.read_with(cx, |editor, _| (editor.doc.crop, editor.tool));
        assert_eq!(crop, RectI::new(40, 30, 200, 150));
        assert_eq!(tool, Tool::Select);
        cx.simulate_keystrokes("ctrl-z");
        assert_eq!(
            view.read_with(cx, |editor, _| editor.doc.crop),
            RectI::new(0, 0, 400, 300)
        );
    }

    #[gpui_kit::test]
    fn step_markers_count_up(cx: &mut TestAppContext) {
        let (view, cx) = editor(cx);
        cx.simulate_keystrokes("n");
        click(&view, cx, (60.0, 60.0));
        click(&view, cx, (200.0, 60.0));
        let numbers: Vec<u32> = elements(&view, cx)
            .iter()
            .filter_map(|el| match el.shape {
                Shape::Step { number, .. } => Some(number),
                _ => None,
            })
            .collect();
        assert_eq!(numbers, [1, 2]);
    }

    #[gpui_kit::test]
    fn a_marquee_selects_only_what_it_touches(cx: &mut TestAppContext) {
        let (view, cx) = editor(cx);
        cx.simulate_keystrokes("l");
        drag(&view, cx, &[(40.0, 40.0), (120.0, 40.0)], Modifiers::none());
        drag(
            &view,
            cx,
            &[(40.0, 200.0), (120.0, 200.0)],
            Modifiers::none(),
        );
        cx.simulate_keystrokes("v");
        drag(&view, cx, &[(20.0, 20.0), (200.0, 80.0)], Modifiers::none());
        let (selection, first) = view.read_with(cx, |editor, _| {
            (editor.selection.clone(), editor.doc.elements[0].id)
        });
        assert_eq!(selection, [first]);
    }

    #[gpui_kit::test]
    fn the_wheel_zooms_about_the_pointer(cx: &mut TestAppContext) {
        let (view, cx) = editor(cx);
        let position = at(&view, cx, (100.0, 100.0));
        let before = view.read_with(cx, |editor, _| editor.zoom);
        cx.simulate_event(ScrollWheelEvent {
            position,
            delta: ScrollDelta::Pixels(point(px(0.), px(120.))),
            modifiers: Modifiers::none(),
            touch_phase: TouchPhase::Moved,
        });
        let (after, fit, under) = view.read_with(cx, |editor, _| {
            (editor.zoom, editor.fit, editor.space().to_image(position))
        });
        assert!(after > before, "{before} -> {after}");
        assert!(!fit);
        assert!(near(under, (100.0, 100.0)), "{under:?}");
    }

    #[gpui_kit::test]
    fn the_pen_collects_points_and_stays_unselected(cx: &mut TestAppContext) {
        let (view, cx) = editor(cx);
        cx.simulate_keystrokes("p");
        let path = [(20.0, 20.0), (60.0, 40.0), (100.0, 20.0), (140.0, 50.0)];
        drag(&view, cx, &path, Modifiers::none());
        let drawn = elements(&view, cx);
        let Shape::Pen(points) = &drawn[0].shape else {
            panic!("expected a pen stroke, got {:?}", drawn[0].shape);
        };
        assert_eq!(points.len(), 4);
        assert!(view.read_with(cx, |editor, _| editor.selection.is_empty()));
    }

    #[gpui_kit::test]
    fn typed_text_becomes_an_element_and_empty_text_does_not(cx: &mut TestAppContext) {
        let (view, cx) = editor(cx);
        cx.simulate_keystrokes("t");
        click(&view, cx, (60.0, 60.0));
        assert!(view.read_with(cx, |editor, _| editor.text_edit.is_some()));
        cx.simulate_input("hi");
        cx.simulate_keystrokes("enter");
        let drawn = elements(&view, cx);
        let Shape::Text { text, .. } = &drawn[0].shape else {
            panic!("expected text, got {:?}", drawn[0].shape);
        };
        assert_eq!(&**text, "hi");
        assert!(view.read_with(cx, |editor, _| editor.text_edit.is_none()));

        click(&view, cx, (60.0, 200.0));
        cx.simulate_keystrokes("enter");
        assert_eq!(elements(&view, cx).len(), 1);
        cx.simulate_keystrokes("ctrl-z");
        assert!(elements(&view, cx).is_empty());
    }

    #[gpui_kit::test]
    fn toolbar_buttons_pick_tools_and_swallow_drags(cx: &mut TestAppContext) {
        let (view, cx) = editor(cx);
        let button = cx
            .debug_bounds("tool-r")
            .expect("the rectangle button is drawn")
            .center();
        cx.simulate_click(button, Modifiers::none());
        assert_eq!(view.read_with(cx, |editor, _| editor.tool), Tool::Rect);

        cx.simulate_mouse_down(button, MouseButton::Left, Modifiers::none());
        let above = point(button.x, button.y - px(200.));
        cx.simulate_mouse_move(above, MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_up(above, MouseButton::Left, Modifiers::none());
        assert!(elements(&view, cx).is_empty());
    }

    #[gpui_kit::test]
    fn a_style_change_recolors_the_selection(cx: &mut TestAppContext) {
        let (view, cx) = editor(cx);
        drag(
            &view,
            cx,
            &[(50.0, 50.0), (150.0, 120.0)],
            Modifiers::none(),
        );
        view.update_in(cx, |editor, window, cx| {
            editor.apply_style(StyleChange::Color(PALETTE[3]), window, cx)
        });
        assert_eq!(elements(&view, cx)[0].color, PALETTE[3]);
        cx.simulate_keystrokes("ctrl-z");
        assert_eq!(elements(&view, cx)[0].color, PALETTE[0]);
    }
}
