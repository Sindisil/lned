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

use crate::eol::{Eol, Eols};

#[derive(Debug, Default, Clone, PartialEq)]
pub struct UndoStack {
    undo: Vec<ChangeSet>,
    redo: Vec<ChangeSet>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChangeSet {
    id: Option<u64>,
    pub current_index_before: usize,
    pub current_index_after: usize,
    pub eols_before: Eols,
    pub eols_after: Eols,
    changes: Vec<Change>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    /// Lines inserted
    Insert { index: usize, lines: Vec<String> },
    /// Lines Removed
    Remove { index: usize, lines: Vec<String> },
    /// EOLs changed for span of lines
    SetEols { span: Range<usize>, old: Eol, new: Eol },
}

static INST_COUNTER: AtomicU64 = AtomicU64::new(0);

fn next_id() -> u64 {
    INST_COUNTER.fetch_add(1, Ordering::Relaxed)
}

impl ChangeSet {
    pub fn new(current_index: usize, eol: Eols) -> Self {
        ChangeSet {
            id: None,
            current_index_before: current_index,
            current_index_after: current_index,
            eols_before: eol,
            eols_after: eol,
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
        self.current_index_after = self.current_index_before;
        self.eols_after = self.eols_before;
        self.changes.drain(..)
    }

    fn invert(mut change_set: ChangeSet) -> ChangeSet {
        mem::swap(
            &mut change_set.current_index_before,
            &mut change_set.current_index_after,
        );
        mem::swap(&mut change_set.eols_before, &mut change_set.eols_after);
        for change in &mut change_set.changes {
            let new_change = match change {
                Change::Insert { index, lines } => {
                    Change::Remove { index: *index, lines: mem::take(lines) }
                }
                Change::Remove { index, lines } => {
                    Change::Insert { index: *index, lines: mem::take(lines) }
                }
                Change::SetEols { span, old, new } => Change::SetEols {
                    span: mem::take(span),
                    old: *new,
                    new: *old,
                },
            };
            *change = new_change;
        }
        change_set.changes.reverse();
        change_set
    }

    pub fn extend(&mut self, other: ChangeSet) {
        self.current_index_after = other.current_index_after;
        self.eols_after = other.eols_after;
        self.changes.extend(other.changes);
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
        current_index: usize,
        eol: Eols,
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
        cset.current_index_after = current_index;
        cset.eols_after = eol;
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
        let eols_before = Eols { default_eol: Eol::Lf, lfs: 10, crlfs: 0 };
        let eols_after = Eols { default_eol: Eol::Crlf, lfs: 0, crlfs: 10 };
        let ci_before = 3;
        let ci_after = 8;
        let mut orig = ChangeSet::new(ci_before, eols_before);
        orig.push(Change::Insert {
            index: 2,
            lines: vec!["added\n".to_owned()],
        });
        orig.push(Change::Remove {
            index: 1,
            lines: vec!["removed\n".to_owned()],
        });
        orig.push(Change::SetEols { span: 1..5, old: Eol::Lf, new: Eol::Crlf });
        orig.current_index_after = ci_after;
        orig.eols_after = eols_after;

        let inverted = ChangeSet::invert(orig.clone());
        assert_eq!(inverted.current_index_before, orig.current_index_after);
        assert_eq!(inverted.current_index_after, orig.current_index_before);
        assert_eq!(inverted.eols_before, orig.eols_after);
        assert_eq!(inverted.eols_after, orig.eols_before);
        for change in inverted.changes() {
            match change {
                Change::Insert { index, lines } => {
                    assert_eq!(*index, 1);
                    assert_eq!(*lines, vec!["removed\n".to_owned()]);
                }
                Change::Remove { index, lines } => {
                    assert_eq!(*index, 2);
                    assert_eq!(*lines, vec!["added\n".to_owned()]);
                }
                Change::SetEols { span, old, new } => {
                    assert_eq!(*span, 1..5);
                    assert_eq!(*old, Eol::Crlf);
                    assert_eq!(*new, Eol::Lf);
                }
            }
        }
    }
}
