use std::sync::Arc;

use image::RgbaImage;

use super::element::{Element, Shape};
use crate::geom::RectI;

const HISTORY_LIMIT: usize = 200;

#[derive(Clone, PartialEq)]
struct Snapshot {
    elements: Vec<Element>,
    crop: RectI,
}

/// A capture and the markup on it. Effects stay below every other element, as they are drawn.
pub struct Doc {
    pub base: Arc<RgbaImage>,
    pub elements: Vec<Element>,
    pub crop: RectI,
    next_id: u64,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
}

impl Doc {
    pub fn new(base: Arc<RgbaImage>) -> Self {
        let crop = RectI::new(0, 0, base.width() as i32, base.height() as i32);
        Self {
            base,
            elements: Vec::new(),
            crop,
            next_id: 1,
            undo: Vec::new(),
            redo: Vec::new(),
        }
    }

    pub fn full(&self) -> RectI {
        RectI::new(0, 0, self.base.width() as i32, self.base.height() as i32)
    }

    pub fn next_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            elements: self.elements.clone(),
            crop: self.crop,
        }
    }

    /// Call before a change, so undo returns to the state as it is now.
    pub fn checkpoint(&mut self) {
        let snapshot = self.snapshot();
        if self.undo.last() == Some(&snapshot) {
            return;
        }
        self.undo.push(snapshot);
        if self.undo.len() > HISTORY_LIMIT {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    /// Drops the last checkpoint when the gesture it guarded changed nothing.
    pub fn discard_unchanged(&mut self) {
        if self.undo.last() == Some(&self.snapshot()) {
            self.undo.pop();
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo(&mut self) -> bool {
        let Some(previous) = self.undo.pop() else {
            return false;
        };
        self.redo.push(self.snapshot());
        self.restore(previous);
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(next) = self.redo.pop() else {
            return false;
        };
        self.undo.push(self.snapshot());
        self.restore(next);
        true
    }

    fn restore(&mut self, snapshot: Snapshot) {
        self.elements = snapshot.elements;
        self.crop = snapshot.crop;
    }

    fn effect_count(&self) -> usize {
        self.elements.iter().filter(|el| el.is_effect()).count()
    }

    pub fn add(&mut self, element: Element) {
        if element.is_effect() {
            let index = self.effect_count();
            self.elements.insert(index, element);
        } else {
            self.elements.push(element);
        }
    }

    pub fn get(&self, id: u64) -> Option<&Element> {
        self.elements.iter().find(|el| el.id == id)
    }

    pub fn get_mut(&mut self, id: u64) -> Option<&mut Element> {
        self.elements.iter_mut().find(|el| el.id == id)
    }

    pub fn remove(&mut self, ids: &[u64]) {
        self.elements.retain(|el| !ids.contains(&el.id));
    }

    /// Moves the elements to the top or bottom of their own layer.
    pub fn reorder(&mut self, ids: &[u64], to_front: bool) {
        let (mut picked, mut rest): (Vec<Element>, Vec<Element>) =
            std::mem::take(&mut self.elements)
                .into_iter()
                .partition(|el| ids.contains(&el.id));
        let (mut effects, mut marks): (Vec<Element>, Vec<Element>) =
            rest.drain(..).partition(|el| el.is_effect());
        let (picked_effects, picked_marks): (Vec<Element>, Vec<Element>) =
            picked.drain(..).partition(|el| el.is_effect());
        if to_front {
            effects.extend(picked_effects);
            marks.extend(picked_marks);
        } else {
            effects.splice(0..0, picked_effects);
            marks.splice(0..0, picked_marks);
        }
        effects.extend(marks);
        self.elements = effects;
    }

    /// The number the next step marker gets: one more than the highest in use.
    pub fn next_step(&self) -> u32 {
        self.elements
            .iter()
            .filter_map(|el| match el.shape {
                Shape::Step { number, .. } => Some(number),
                _ => None,
            })
            .max()
            .unwrap_or(0)
            + 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::element::PALETTE;
    use crate::geom::{RectF, pt};

    fn doc() -> Doc {
        Doc::new(Arc::new(RgbaImage::new(40, 30)))
    }

    fn element(doc: &mut Doc, shape: Shape) -> Element {
        Element {
            id: doc.next_id(),
            shape,
            color: PALETTE[0],
            fill: false,
            width: 4.0,
            size: 12.0,
        }
    }

    #[test]
    fn undo_and_redo_walk_the_history() {
        let mut doc = doc();
        doc.checkpoint();
        let rect = element(&mut doc, Shape::Rect(RectF::new(1.0, 1.0, 5.0, 5.0)));
        doc.add(rect);
        assert!(doc.undo());
        assert!(doc.elements.is_empty());
        assert!(doc.redo());
        assert_eq!(doc.elements.len(), 1);
        assert!(!doc.redo());
    }

    #[test]
    fn effects_stay_below_marks() {
        let mut doc = doc();
        let line = element(&mut doc, Shape::Line(pt(0.0, 0.0), pt(5.0, 5.0)));
        let blur = element(&mut doc, Shape::Blur(RectF::new(0.0, 0.0, 9.0, 9.0)));
        let (line_id, blur_id) = (line.id, blur.id);
        doc.add(line);
        doc.add(blur);
        assert_eq!(doc.elements[0].id, blur_id);
        doc.reorder(&[blur_id], true);
        assert_eq!(doc.elements[0].id, blur_id);
        assert_eq!(doc.elements[1].id, line_id);
    }

    #[test]
    fn steps_count_up_from_the_highest() {
        let mut doc = doc();
        assert_eq!(doc.next_step(), 1);
        let step = element(
            &mut doc,
            Shape::Step {
                center: pt(3.0, 3.0),
                number: 4,
            },
        );
        doc.add(step);
        assert_eq!(doc.next_step(), 5);
    }

    #[test]
    fn an_unchanged_gesture_leaves_no_history() {
        let mut doc = doc();
        doc.checkpoint();
        doc.discard_unchanged();
        assert!(!doc.can_undo());
    }
}
