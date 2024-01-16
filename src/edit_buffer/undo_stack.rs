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
use std::sync::atomic::{AtomicU64, Ordering};

use std::ops::{Deref, DerefMut};

use crate::edit_buffer::operation::Op;

#[derive(Debug, Clone)]
pub struct Undoable {
    id: Option<u64>,
    op: Op,
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

static INST_COUNTER: AtomicU64 = AtomicU64::new(0);

impl UndoStack {
    pub fn new() -> Self {
        UndoStack {
            undo: Vec::new(),
            redo: Vec::new(),
        }
    }

    /// Push the supplied Undoable onto the undo stack.
    ///
    /// If the pushed item doesn't yet have an id value, it
    /// must be a new operation, rather than a redone operation.
    ///
    /// In that case, if the redo stack is not
    /// empty, `push_undo()` will walk the items on the undo stack
    /// in reverse, pushing a clone of each onto the undo stack,
    /// then drain the redo stack, pushing Undoables of
    /// their Inverse action onto the undo stack.
    ///
    /// This will preserve full history, including the undo
    /// commands issued before the current change.
    pub fn push_undo(&mut self, item: impl Into<Undoable>) {
        let mut item = item.into();
        if item.id.is_none() {
            item.id = Some(INST_COUNTER.fetch_add(1, Ordering::SeqCst));
            if !self.redo.is_empty() {
                let mut inv: Vec<Undoable> = self
                    .redo
                    .iter()
                    .map(Redoable::to_inverse_undoable)
                    .collect();

                for item in self.redo.drain(..).rev() {
                    self.undo.push(item.undo);
                }

                self.undo.append(&mut inv);
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

    /// Return the id of the top item in the undo stack,
    /// or None if the stack is empty.
    ///
    /// Used to determine if undo stack has changed,
    /// as a proxy for an `EditBuffer` with changes
    /// that have not been written.
    pub fn fingerprint(&self) -> Option<u64> {
        self.undo.last().and_then(|i| i.id)
    }
}

impl Redoable {
    /// Returns an Undoable that is the inverse of the
    /// Redoable upon which this method is called.
    ///
    /// This is to support flushing the redo stack onto the
    /// undo stack when a new change is made, allowing
    /// previous undo actions to be undone and thus
    /// ensuring that there are no unreachable past
    /// states.
    fn to_inverse_undoable(&self) -> Undoable {
        Undoable {
            id: Some(INST_COUNTER.fetch_add(1, Ordering::SeqCst)),
            op: self.undo.op.inverse(),
        }
    }
}

impl From<Op> for Undoable {
    fn from(value: Op) -> Self {
        Undoable {
            id: None,
            op: value,
        }
    }
}

impl DerefMut for Undoable {
    #[cfg(not(tarpaulin_include))]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.op
    }
}

impl Deref for Undoable {
    type Target = Op;

    #[cfg(not(tarpaulin_include))]
    fn deref(&self) -> &Self::Target {
        &self.op
    }
}

impl DerefMut for Redoable {
    #[cfg(not(tarpaulin_include))]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.undo.op
    }
}

impl Deref for Redoable {
    type Target = Op;

    #[cfg(not(tarpaulin_include))]
    fn deref(&self) -> &Self::Target {
        &self.undo.op
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edit_buffer::{AppendData, DeleteData};

    #[test]
    fn create_new_undo_stack() {
        let s = UndoStack::new();
        assert!(s.undo.is_empty());
    }

    #[test]
    fn undo_stack_empty_fingerprint() {
        let s = UndoStack::new();
        assert!(s.undo.is_empty());
        assert!(s.fingerprint().is_none());
    }

    #[test]
    fn undo_stack_non_empty_fingerprint() {
        let mut s = UndoStack::new();
        s.push_undo(Op::Append(AppendData {
            ..AppendData::default()
        }));
        let fp1 = s.fingerprint();
        assert!(fp1.is_some());
        s.push_undo(Op::Append(AppendData {
            ..AppendData::default()
        }));
        let fp2 = s.fingerprint();
        assert!(fp2.is_some() && fp1 != fp2);
        assert!(!s.undo.is_empty());
        s.pop_undo();
        assert!(s.fingerprint() == fp1);
        s.pop_undo();
        assert!(s.fingerprint().is_none());
        assert!(s.undo.is_empty());
    }

    #[test]
    fn undo_stack_push_and_pop() {
        use crate::command::Address;

        let mut s = UndoStack::new();
        let o_app = Op::Append(AppendData {
            address: Some(Address(1, 1)),
            lines: vec!["spam".to_owned()],
            current_line: 0,
        });
        let o_del = Op::Delete(DeleteData {
            address: Some(Address(1, 1)),
            lines_removed: Vec::new(),
            current_line: 1,
        });

        assert!(s.undo.is_empty());
        assert!(s.pop_undo().is_none());
        assert!(s.pop_redo().is_none());
        s.push_undo(o_app.clone());
        assert!(!s.undo.is_empty());
        s.push_undo(o_del.clone());
        assert!(!s.undo.is_empty());
        let ret1 = s.pop_undo();
        assert!(!s.undo.is_empty());
        let u1 = ret1.unwrap();
        assert!(matches!(*u1, Op::Delete(_)));
        s.push_redo(u1);

        s.push_undo(o_del.clone());
        // redo_stack now empty
        assert!(s.pop_redo().is_none());

        let ret3 = s.pop_undo();
        s.push_redo(ret3.unwrap());

        let ret2 = s.pop_redo();
        let u2 = ret2.unwrap();
        assert!(matches!(*u2, Op::Delete(_)));
        assert!(s.pop_redo().is_none());
        assert!(s.pop_undo().is_some());
        assert!(s.pop_undo().is_some());
        assert!(s.pop_undo().is_some());
        assert!(s.pop_undo().is_none());
        assert!(s.undo.is_empty());
    }
}
