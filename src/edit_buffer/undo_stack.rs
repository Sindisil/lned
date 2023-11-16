use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

/// `UndoStack` ecapsulates the undo and redo stacks, with methods that
/// maintain the correct invarients.
///
/// Namely, that we must be able to distinguish a new Undoable getting
/// pushed from a do_*() method in `EditBuffer` (driven by a user command)
/// from one getting pushed from `EditBuffer::do_redo()`. If the
/// redo stack is non-empty, the former will cause a flush of the
/// redo stack onto the undo stack (both verbatum and inversed) in
/// order to allow "undoing the undos" (i.e., not losing any edit
/// history).
use std::ops::{Deref, DerefMut};

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

    /// Push the supplied Undoable onto the undo stack.
    ///
    /// If the pushed Undoable is new (i.e., hadn't previously
    /// been pushed to the undo stack), it must be coming from
    /// a user command. In that case, if the redo stack is not
    /// empty, `push_undo()` will walk the items on the undo stack
    /// in reverse, pushing a clone of each onto the undo stack,
    /// then drain the redo stack, pushing Undoables of
    /// their Inverse action onto the undo stack.
    ///
    /// This will preserve full history, including the undo
    /// commands issued before the current change.
    pub fn push_undo(&mut self, mut item: Undoable) {
        if item.is_new {
            item.is_new = false;
            if !self.redo.is_empty() {
                for item in self.redo.iter().rev() {
                    self.undo.push(item.undo.clone());
                }

                for item in self.redo.drain(..) {
                    self.undo.push(Undoable {
                        op: item.undo.op.inverse(),
                        is_new: false,
                    });
                }
            }
        }
        self.undo.push(item);
    }

    pub fn push_redo(&mut self, item: Redoable) {
        self.redo.push(item);
    }

    /// Pops the top item from the undo stack and returns it
    /// as an (optional) Redoable. None is returned if
    /// the undo stack was empty.
    ///
    /// A Redoable is returned so that it is ready to be
    /// pushed on the redo stack (the typical next destination
    /// for an item popped from the undo stack).
    ///
    /// The returned value implements Deref<Op>, so it can
    /// be used anywhere that an Op reference could be.
    pub fn pop_undo(&mut self) -> Option<Redoable> {
        self.undo.pop().map(|undo| Redoable { undo })
    }

    /// Pops the top item from the redo stack and returns it
    /// as an (optional) Undoable. None is returned if
    /// the undo stack was empty.
    ///
    /// An Undoable is returned so that it is ready to be
    /// pushed on the undo stack (the typical next destination
    /// for an item popped from the redo stack).
    ///
    /// The returned value implements Deref<Op>, so it can
    /// be used anywhere that an Op reference could be.
    pub fn pop_redo(&mut self) -> Option<Undoable> {
        self.redo.pop().map(|redo| redo.undo)
    }

    /// Return hash of entire undo stack.
    ///
    /// Used to determine if undo stack has changed,
    /// as a proxy for an `EditBuffer` with changes
    /// that have not been written.
    pub fn fingerprint(&self) -> u64 {
        let mut h = DefaultHasher::new();
        self.undo.hash(&mut h);
        h.finish()
    }
}

impl DerefMut for Undoable {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.op
    }
}

impl Deref for Undoable {
    type Target = Op;

    fn deref(&self) -> &Self::Target {
        &self.op
    }
}

impl DerefMut for Redoable {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.undo.op
    }
}

impl Deref for Redoable {
    type Target = Op;

    fn deref(&self) -> &Self::Target {
        &self.undo.op
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_new_undo_stack() {
        let s = UndoStack::new();
        assert!(s.is_empty());
    }

    #[test]
    fn push_and_pop() {
        use crate::command::Address;
        use crate::edit_buffer::{AppendData, DeleteData};

        let mut s = UndoStack::new();
        let o_app = Op::Append(AppendData {
            address: Some(Address::Line(1)),
            lines: vec!["spam".to_owned()],
            current_line: 0,
        });
        let o_del = Op::Delete(DeleteData {
            address: Some(Address::Line(1)),
            lines_removed: Vec::new(),
            current_line: 1,
        });

        assert!(s.is_empty());
        assert!(s.pop_undo().is_none());
        assert!(s.pop_redo().is_none());
        s.push_undo(Undoable::new(o_app));
        assert!(!s.is_empty());
        s.push_undo(Undoable::new(o_del));
        assert!(!s.is_empty());
        let ret1 = s.pop_undo();
        assert!(!s.is_empty());
        let u1 = ret1.unwrap();
        assert!(matches!(*u1, Op::Delete(_)));
        s.push_redo(u1);
        let ret2 = s.pop_redo();
        let u2 = ret2.unwrap();
        assert!(matches!(*u2, Op::Delete(_)));
        assert!(s.pop_redo().is_none());
        assert!(s.pop_undo().is_some());
        assert!(s.pop_undo().is_none());
        assert!(s.is_empty());
    }
}
