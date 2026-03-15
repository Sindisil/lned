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
use std::mem;
use std::ops::Range;

use std::sync::atomic::{AtomicU64, Ordering};
use std::vec::Drain;

use crate::edit_buffer::{Eol, PrevailingEol};

#[derive(Debug, Default, Clone, PartialEq)]
pub struct UndoStack {
    undo: Vec<ChangeSet>,
    redo: Vec<ChangeSet>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChangeSet {
    id: Option<u64>,
    pub current_line_before: usize,
    pub current_line_after: usize,
    pub prevailing_eol_before: Option<PrevailingEol>,
    pub prevailing_eol_after: Option<PrevailingEol>,
    changes: Vec<Change>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    Add(usize, Vec<String>),        // Add/Insert of lines
    Remove(usize, Vec<String>),     // Removal of lines
    SetEol(Range<usize>, Eol, Eol), // Eol change
}

static INST_COUNTER: AtomicU64 = AtomicU64::new(0);

fn next_id() -> u64 {
    INST_COUNTER.fetch_add(1, Ordering::Relaxed)
}

impl ChangeSet {
    pub fn new(
        current_line: usize,
        prevailing_eol: Option<PrevailingEol>,
    ) -> Self {
        ChangeSet {
            id: None,
            current_line_before: current_line,
            current_line_after: current_line,
            prevailing_eol_before: prevailing_eol,
            prevailing_eol_after: prevailing_eol,
            changes: Vec::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }

    pub fn push(&mut self, change: Change) {
        self.changes.push(change);
    }

    pub fn changes(&self) -> impl DoubleEndedIterator<Item = &Change> {
        self.changes.iter()
    }

    pub fn drain(&mut self) -> Drain<'_, Change> {
        self.current_line_after = self.current_line_before;
        self.prevailing_eol_after = self.prevailing_eol_before;
        self.changes.drain(..)
    }

    fn invert(mut change_set: ChangeSet) -> ChangeSet {
        mem::swap(
            &mut change_set.current_line_before,
            &mut change_set.current_line_after,
        );
        mem::swap(
            &mut change_set.prevailing_eol_before,
            &mut change_set.prevailing_eol_after,
        );
        for change in &mut change_set.changes {
            let new_change = match change {
                Change::Add(p, l) => Change::Remove(*p, mem::take(l)),
                Change::Remove(p, l) => Change::Add(*p, mem::take(l)),
                Change::SetEol(r, o, n) => Change::SetEol(mem::take(r), *n, *o),
            };
            *change = new_change;
        }
        change_set.changes.reverse();
        change_set
    }
}

impl UndoStack {
    pub fn new() -> Self {
        UndoStack { undo: Vec::new(), redo: Vec::new() }
    }

    /// Push the supplied Undoable onto the undo stack.
    ///
    /// If the pushed `ChangeSet` doesn't yet have an id value, it
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
    pub fn push_undo(
        &mut self,
        mut cset: ChangeSet,
        current_line: usize,
        eol: Option<PrevailingEol>,
    ) {
        if cset.id.is_none() {
            cset.id = Some(next_id());
            if !self.redo.is_empty() {
                // replay redo stack in reverse onto undo stack
                self.undo.extend(self.redo.iter().rev().cloned());
                // replay redo stack from bottom, with inverted operations
                self.undo.extend(self.redo.drain(..).map(ChangeSet::invert));
            }
        }
        cset.current_line_after = current_line;
        cset.prevailing_eol_after = eol;
        self.undo.push(cset);
    }

    pub fn push_redo(&mut self, cset: ChangeSet) {
        self.redo.push(cset);
    }

    /// Pops the top item from the undo stack and returns it as an Option,
    /// returning None if the undo stack was empty.
    pub fn pop_undo(&mut self) -> Option<ChangeSet> {
        self.undo.pop()
    }

    /// Pops the top item from the redo stack and returns it
    /// as an Option, returning None if the redo stack was empty.
    pub fn pop_redo(&mut self) -> Option<ChangeSet> {
        self.redo.pop()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use similar_asserts::assert_eq;

    #[test]
    fn create_new_undo_stack() {
        let s = UndoStack::new();
        assert!(s.undo.is_empty());
    }

    #[test]
    fn invert_swaps_sense_of_changes() {
        let eol_before = Some(PrevailingEol::lf(false));
        let eol_after = Some(PrevailingEol::crlf(false));
        let cl_before = 13;
        let cl_after = 42;
        let mut orig = ChangeSet::new(cl_before, eol_before);
        orig.push(Change::Add(2, vec!["added\n".to_owned()]));
        orig.push(Change::Remove(1, vec!["removed\n".to_owned()]));
        orig.push(Change::SetEol(1..4, Eol::Lf, Eol::Crlf));
        orig.current_line_after = cl_after;
        orig.prevailing_eol_after = eol_after;

        let inverted = ChangeSet::invert(orig.clone());
        assert_eq!(inverted.current_line_before, orig.current_line_after);
        assert_eq!(inverted.current_line_after, orig.current_line_before);
        assert_eq!(inverted.prevailing_eol_before, orig.prevailing_eol_after);
        assert_eq!(inverted.prevailing_eol_after, orig.prevailing_eol_before);
        for change in inverted.changes() {
            match change {
                Change::Add(p, l) => {
                    assert_eq!(*p, 1);
                    assert_eq!(*l, vec!["removed\n".to_owned()]);
                }
                Change::Remove(p, l) => {
                    assert_eq!(*p, 2);
                    assert_eq!(*l, vec!["added\n".to_owned()]);
                }
                Change::SetEol(span, old, new) => {
                    assert_eq!(*span, 1..4);
                    assert_eq!(*old, Eol::Crlf);
                    assert_eq!(*new, Eol::Lf);
                }
            }
        }
    }
}
