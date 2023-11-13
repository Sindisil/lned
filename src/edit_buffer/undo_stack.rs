use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use crate::edit_buffer::operation::Op;

#[derive(Debug, Clone, Hash)]
pub struct Undoable {
    op: Op,
    is_new: bool,
}

#[derive(Debug, Clone)]
pub struct Redoable {
    undo: Undoable,
}

#[derive(Debug, Clone)]
pub struct UndoStack {
    undo: Vec<Undoable>,
    redo: Vec<Redoable>,
}

impl Undoable {
    pub fn new(op: Op) -> Self {
        Undoable { op, is_new: true }
    }

    pub fn op(&mut self) -> &mut Op {
        &mut self.op
    }
}

impl Redoable {
    pub fn op(&mut self) -> &mut Op {
        &mut self.undo.op
    }
}

impl UndoStack {
    pub fn new() -> Self {
        UndoStack {
            undo: Vec::new(),
            redo: Vec::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.undo.is_empty()
    }

    pub fn push_undo(&mut self, mut item: Undoable) {
        if item.is_new {
            item.is_new = false;
        }
        self.undo.push(item);
    }

    pub fn push_redo(&mut self, item: Redoable) {
        self.redo.push(item);
    }

    pub fn pop_undo(&mut self) -> Option<Redoable> {
        self.undo.pop().map(|undo| Redoable { undo })
    }

    pub fn pop_redo(&mut self) -> Option<Undoable> {
        self.redo.pop().map(|redo| redo.undo)
    }

    pub fn fingerprint(&self) -> u64 {
        let mut h = DefaultHasher::new();
        self.undo.hash(&mut h);
        h.finish()
    }
}
