use std::fmt;
use std::mem;
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
use std::vec::Drain;

#[derive(Debug, Clone, PartialEq)]
pub struct UndoStack {
    undo: Vec<ChangeSet>,
    redo: Vec<ChangeSet>,
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct ChangeSet {
    id: Option<u64>,
    pub current_line_before: usize,
    pub current_line_after: usize,
    changes: Vec<Change>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Change {
    pub current_line_before: usize,
    pub current_line_after: usize,
    diffs: Vec<Diff>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Diff {
    Add(usize, Vec<String>),    // Add/Insert of lines
    Remove(usize, Vec<String>), // Removal of lines
}

#[derive(Debug)]
pub struct TryFromChangeSetError;

impl fmt::Display for TryFromChangeSetError {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        "ChangeSet doesn't contain exactly one Change".fmt(fmt)
    }
}

impl std::error::Error for TryFromChangeSetError {}

static INST_COUNTER: AtomicU64 = AtomicU64::new(0);

fn next_id() -> u64 {
    INST_COUNTER.fetch_add(1, Ordering::Relaxed)
}

impl TryFrom<ChangeSet> for Change {
    type Error = TryFromChangeSetError;

    fn try_from(mut v: ChangeSet) -> Result<Self, Self::Error> {
        match v.changes.len() {
            1 => Ok(v.changes.remove(0)),
            _ => Err(TryFromChangeSetError),
        }
    }
}
impl ChangeSet {
    pub fn new(current_line: usize) -> Self {
        ChangeSet {
            current_line_before: current_line,
            current_line_after: current_line,
            ..Default::default()
        }
    }

    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }

    pub fn push(&mut self, mut change: Change, current_line: usize) {
        change.current_line_after = current_line;
        self.changes.push(change);
    }

    pub fn changes(&self) -> impl DoubleEndedIterator<Item = &Change> {
        self.changes.iter()
    }

    pub fn drain(&mut self) -> Drain<'_, Change> {
        self.current_line_after = self.current_line_before;
        self.changes.drain(..)
    }

    fn invert(mut change_set: ChangeSet) -> ChangeSet {
        mem::swap(
            &mut change_set.current_line_before,
            &mut change_set.current_line_after,
        );
        for change in &mut change_set.changes {
            mem::swap(
                &mut change.current_line_before,
                &mut change.current_line_after,
            );
            for diff in &mut change.diffs {
                let new_diff = match diff {
                    Diff::Add(p, l) => Diff::Remove(*p, mem::take(l)),
                    Diff::Remove(p, l) => Diff::Add(*p, mem::take(l)),
                };
                *diff = new_diff;
            }
            change.diffs.reverse();
        }
        change_set.changes.reverse();
        change_set
    }
}

impl Change {
    pub fn new(current_line: usize) -> Self {
        Change {
            current_line_before: current_line,
            current_line_after: current_line,
            diffs: Vec::new(),
        }
    }

    pub fn push_add(&mut self, position: usize, lines_added: Vec<String>) {
        self.diffs.push(Diff::Add(position, lines_added));
    }

    pub fn push_remove(&mut self, position: usize, lines_removed: Vec<String>) {
        self.diffs.push(Diff::Remove(position, lines_removed));
    }

    pub fn diffs(&self) -> impl DoubleEndedIterator<Item = &Diff> {
        self.diffs.iter()
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
    pub fn push_undo(&mut self, mut change: ChangeSet, current_line: usize) {
        if change.is_empty() {
            return;
        }
        if change.id.is_none() {
            change.id = Some(next_id());
            if !self.redo.is_empty() {
                // replay redo stack in reverse onto undo stack
                self.undo.extend(self.redo.iter().rev().cloned());
                // replay redo stack from bottom, with inverted operations
                self.undo.extend(self.redo.drain(..).map(ChangeSet::invert));
            }
        }
        change.current_line_after = current_line;
        self.undo.push(change);
    }

    pub fn push_redo(&mut self, change: ChangeSet) {
        self.redo.push(change);
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
    fn undo_stack_empty_fingerprint() {
        let s = UndoStack::new();
        assert!(s.undo.is_empty());
        assert!(s.fingerprint().is_none());
    }

    #[test]
    fn undo_stack_non_empty_fingerprint() {
        let mut s = UndoStack::new();
        let mut cs = ChangeSet::new(0);
        let ch = Change::new(0);
        cs.push(ch, 0);
        s.push_undo(cs, 0);
        let fp1 = s.fingerprint();
        assert!(fp1.is_some());
        let mut cs = ChangeSet::new(0);
        let ch = Change::new(0);
        cs.push(ch, 0);
        s.push_undo(cs, 0);
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
    fn invert_swaps_add_and_remove() {
        let mut orig = ChangeSet::new(13);
        let mut orig_change = Change::new(13);
        orig_change.current_line_after = 42;
        orig_change.push_add(2, vec!["added\n".to_owned()]);
        orig_change.push_remove(1, vec!["removed\n".to_owned()]);
        orig.push(orig_change.clone(), orig_change.current_line_after);
        let inverted = ChangeSet::invert(orig.clone());
        let inverted_change = &inverted.changes[0];
        assert_eq!(
            inverted_change.current_line_before,
            orig_change.current_line_after
        );
        assert_eq!(
            inverted_change.current_line_after,
            orig_change.current_line_before
        );
        for diff in inverted_change.diffs() {
            match diff {
                Diff::Add(p, l) => {
                    assert_eq!(*p, 1);
                    assert_eq!(*l, vec!["removed\n".to_owned()]);
                }
                Diff::Remove(p, l) => {
                    assert_eq!(*p, 2);
                    assert_eq!(*l, vec!["added\n".to_owned()]);
                }
            }
        }
    }

    #[test]
    fn try_from_changeset() {
        let good =
            ChangeSet { changes: vec![Change::new(1)], ..Default::default() };
        let bad = ChangeSet::new(0);
        let bad2 = ChangeSet {
            changes: vec![Change::new(1), Change::new(1)],
            ..Default::default()
        };
        Change::try_from(good).expect("successful conversion");
        Change::try_from(bad).expect_err("should fail because no Changes");
        Change::try_from(bad2).expect_err("should fail because > 1 Change");
    }
}
