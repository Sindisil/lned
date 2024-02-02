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

#[derive(Debug, Clone)]
pub struct UndoStack {
    undo: Vec<ChangeSet>,
    redo: Vec<ChangeSet>,
}

#[derive(Debug, Clone)]
pub struct ChangeSet {
    id: Option<u64>,
    pub current_line_before: usize,
    pub current_line_after: usize,
    diffs: Vec<Diff>,
}

//#[derive(Debug, Clone)]
#[derive(Debug, Clone)]
pub enum Diff {
    Add(usize, Vec<String>),    // Add/Insert of lines
    Remove(usize, Vec<String>), // Removal of lines
}

static INST_COUNTER: AtomicU64 = AtomicU64::new(0);

fn next_id() -> u64 {
    INST_COUNTER.fetch_add(1, Ordering::SeqCst)
}

impl ChangeSet {
    pub fn new() -> Self {
        ChangeSet {
            id: None,
            current_line_before: 0,
            current_line_after: 0,
            diffs: Vec::new(),
        }
    }

    pub fn push_add(&mut self, position: usize, lines_added: Vec<String>) {
        self.diffs.push(Diff::Add(position, lines_added));
    }

    pub fn push_remove(&mut self, position: usize, lines_removed: Vec<String>) {
        self.diffs.push(Diff::Remove(position, lines_removed));
    }

    pub fn diffs(&self) -> impl Iterator<Item = &Diff> {
        self.diffs.iter()
    }
    fn invert(mut change: ChangeSet) -> ChangeSet {
        mem::swap(
            &mut change.current_line_before,
            &mut change.current_line_after,
        );
        let inv = change
            .diffs
            .into_iter()
            .rev()
            .map(|d| match d {
                Diff::Add(p, l) => Diff::Remove(p, l),
                Diff::Remove(p, l) => Diff::Add(p, l),
            })
            .collect::<Vec<Diff>>();
        change.diffs = inv;
        change
    }
}

impl UndoStack {
    pub fn new() -> Self {
        UndoStack {
            undo: Vec::new(),
            redo: Vec::new(),
        }
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
    pub fn push_undo(&mut self, mut change: ChangeSet) {
        if change.id.is_none() {
            change.id = Some(next_id());
            if !self.redo.is_empty() {
                // replay redo stack in reverse onto undo stack
                self.undo.extend(self.redo.iter().rev().cloned());
                // replay redo stack from bottom, with inverted operations
                self.undo.extend(self.redo.drain(..).map(ChangeSet::invert));
            }
        }
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
        s.push_undo(ChangeSet::new());
        let fp1 = s.fingerprint();
        assert!(fp1.is_some());
        s.push_undo(ChangeSet::new());
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
        let mut orig = ChangeSet::new();
        orig.current_line_before = 13;
        orig.current_line_after = 42;
        orig.push_add(2, vec!["added\n".to_owned()]);
        orig.push_remove(1, vec!["removed\n".to_owned()]);
        let inverted = ChangeSet::invert(orig.clone());
        assert_eq!(inverted.current_line_before, orig.current_line_after);
        assert_eq!(inverted.current_line_after, orig.current_line_before);
        for diff in inverted.diffs() {
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
}
