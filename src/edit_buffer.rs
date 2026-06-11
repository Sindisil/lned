// EditBuffer presents a list of text lines, the ability to add/remove lines,
// 0 based Indexing, Undo/Redo functionality, and conversion functions.
// It maintains current_index, eol, and content_hash during mutation.
use std::hash::{DefaultHasher, Hash, Hasher};
use std::ops::{
    Index, Range, RangeFrom, RangeFull, RangeInclusive, RangeTo,
    RangeToInclusive,
};

use crate::eol::{Eol, Eols};
use crate::error::Error;
use crate::undo_stack::{Change, ChangeSet, UndoStack};

#[derive(Debug, Default, Clone)]
pub struct EditBuffer {
    current_index: usize,
    eols: Eols,
    undo_stack: UndoStack,
    content_hash: Option<u64>,
    lines: Vec<String>,
}

impl From<Vec<String>> for EditBuffer {
    fn from(lines: Vec<String>) -> Self {
        let line_count = lines.len();
        let mut buf = EditBuffer::with_capacity(line_count);
        buf.insert(0, lines);
        buf
    }
}

impl Index<usize> for EditBuffer {
    type Output = String;

    #[inline]
    fn index(&self, index: usize) -> &Self::Output {
        &self.lines[index]
    }
}

impl Index<Range<usize>> for EditBuffer {
    type Output = [String];

    #[inline]
    fn index(&self, index: Range<usize>) -> &Self::Output {
        &self.lines[index]
    }
}

impl Index<RangeInclusive<usize>> for EditBuffer {
    type Output = [String];

    #[inline]
    fn index(&self, index: RangeInclusive<usize>) -> &Self::Output {
        &self.lines[index]
    }
}

impl Index<RangeFrom<usize>> for EditBuffer {
    type Output = [String];

    #[inline]
    fn index(&self, index: RangeFrom<usize>) -> &Self::Output {
        &self.lines[index]
    }
}

impl Index<RangeTo<usize>> for EditBuffer {
    type Output = [String];

    #[inline]
    fn index(&self, index: RangeTo<usize>) -> &Self::Output {
        &self.lines[index]
    }
}

impl Index<RangeToInclusive<usize>> for EditBuffer {
    type Output = [String];

    #[inline]
    fn index(&self, index: RangeToInclusive<usize>) -> &Self::Output {
        &self.lines[index]
    }
}

impl Index<RangeFull> for EditBuffer {
    type Output = [String];

    #[inline]
    fn index(&self, index: RangeFull) -> &Self::Output {
        &self.lines[index]
    }
}

impl PartialEq for EditBuffer {
    fn eq(&self, other: &Self) -> bool {
        self.lines == other.lines && self.current_index == other.current_index
    }
}

impl EditBuffer {
    /// Creates a new empty `EditBuffer`.
    ///
    /// No space will be allocated for text until lines are appended.
    /// This is very inexpensive, but may require excessive allocation
    /// later as lines are added.
    /// Consider the [`with_capacity`] method instead, to prevent this.
    ///
    /// [`with_capacity`]: EditBuffer::with_capacity
    #[inline]
    #[must_use]
    pub fn new() -> EditBuffer {
        EditBuffer {
            current_index: 0,
            eols: Eols::new(Eol::Lf),
            undo_stack: UndoStack::new(),
            content_hash: None,
            lines: Vec::new(),
        }
    }

    /// Creates a new empty `EditBuffer` with room for at least `capacity`
    /// lines of text. Specifying a capacity is useful to reduce the number
    /// of reallocations necessary as lines are added to the `EditBuffer`.
    ///
    /// If the capacity given is `0`, this will be identical to the [`new`]
    /// method, and no allocation will occur.
    ///
    /// [`new`]: EditBuffer::new
    ///
    #[inline]
    #[must_use]
    pub fn with_capacity(capacity: usize) -> EditBuffer {
        EditBuffer {
            current_index: 0,
            eols: Eols::new(Eol::Lf),
            undo_stack: UndoStack::new(),
            content_hash: None,
            lines: Vec::with_capacity(capacity),
        }
    }

    pub fn with_lines<'a>(
        lines: impl IntoIterator<Item = &'a str>,
    ) -> EditBuffer {
        let mut lines: Vec<_> =
            lines.into_iter().map(ToOwned::to_owned).collect();
        let eols = Eols::from_lines(&lines);
        for line in &mut lines[..] {
            if Eol::from_line(&mut *line).is_none() {
                line.push_str(eols.prevailing().into());
            }
        }
        EditBuffer {
            current_index: lines.len().saturating_sub(1),
            eols: Eols::from_lines(&lines),
            undo_stack: UndoStack::new(),
            content_hash: None,
            lines,
        }
    }

    #[must_use]
    /// Returns this `EditBuffer`'s length, in lines.
    pub fn len(&self) -> usize {
        self.lines.len()
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    pub fn content_hash(&mut self) -> u64 {
        *self.content_hash.get_or_insert_with(|| {
            let mut h = DefaultHasher::new();
            self.lines.hash(&mut h);
            h.finish()
        })
    }

    pub fn current_index(&self) -> usize {
        self.current_index
    }

    pub fn current_index_as_range(&self) -> Range<usize> {
        if self.is_empty() {
            return 0..0;
        }

        self.current_index..self.current_index + 1
    }

    pub fn set_current_index(&mut self, index: usize) {
        if !self.is_empty() && index > self.len() - 1 {
            panic!(
                "new index (is {index}) must be within buffer (is 0..{})",
                self.len()
            );
        } else {
            self.current_index = index;
        }
    }

    pub fn push_undo(&mut self, changes: ChangeSet) {
        self.undo_stack.push_undo(changes, self.current_index, self.eols);
    }

    /// Insert lines of text into buffer at index, shifting existing
    /// lines down.
    ///
    /// All lines must end with a newline (i.e., '\r' or "\r\n").
    ///
    /// The last line added becomes the `current_index`.
    ///
    /// Returns [`ChangeSet`] representing the buffer changes.
    ///
    /// If `lines` is empty, no action is taken and an emtpy
    /// [`ChangeSet`] is returned.
    ///
    /// # Panics
    ///
    /// Will panic if index is > buffer length, or if any inserted lines
    /// lack EOL sequences.
    ///
    pub fn insert(&mut self, index: usize, lines: Vec<String>) -> ChangeSet {
        let mut changes = ChangeSet::new(self.current_index, self.eols);
        if lines.is_empty() {
            return changes;
        }

        assert!(
            index <= self.len(),
            "insertion index (is {index}) should be <= len (is {})",
            self.len()
        );

        let new_eols = Eols::from_lines(&lines);

        if self.is_empty() {
            self.eols = new_eols;
        } else {
            self.eols += new_eols;
        }
        self.current_index = index + lines.len() - 1;
        self.content_hash = None;
        self.lines.splice(index..index, lines.iter().cloned());

        changes.push(Change::Insert { index, lines });
        changes
    }

    /// Remove a span of lines.
    ///
    /// The first line after those removed becomes the new
    /// `current_index`. If the removed lines were at the end of
    /// the buffer, the new last line becomes the `current_index`.
    /// If the buffer is empty after the lines are removed,
    /// `current_index` becomes unset.
    ///
    /// Returns a [`ChangeSet`] rerpresenting the buffer changes.
    ///
    /// If `range` is empty, no action is taken and an empty
    /// [`ChangeSet`] is returned.
    ///
    /// # Panics
    ///
    /// Will panic if `range` extends beyon the buffer's end.
    ///
    pub fn remove(&mut self, range: Range<usize>) -> ChangeSet {
        assert!(
            !range.contains(&self.len()),
            "range (is {range:?}) must be within buffer (is 0..{})",
            self.len()
        );

        let mut changes = ChangeSet::new(self.current_index, self.eols);

        if range.is_empty() {
            return changes;
        }

        let first_removed = range.start;
        let removed: Vec<_> = self.lines.splice(range, None).collect();
        self.eols -= Eols::from_lines(&removed);
        self.current_index =
            usize::min(first_removed, self.len().saturating_sub(1));
        self.content_hash = None;
        changes.push(Change::Remove { index: first_removed, lines: removed });
        changes
    }

    pub fn undo(&mut self) -> Result<(), Error> {
        let Some(undo) = self.undo_stack.pop_undo() else {
            return Err(Error::NothingToUndo);
        };
        for change in undo.changes().rev() {
            match change {
                Change::Insert { index, lines } => {
                    drop(self.lines.splice(*index..*index + lines.len(), None));
                }
                Change::Remove { index, lines } => {
                    drop(
                        self.lines
                            .splice(*index..*index, lines.iter().cloned()),
                    );
                }
                Change::SetEols { span, old, new } => {
                    for line in &mut self.lines[span.clone()] {
                        line.replace_range(
                            line.len() - new.str_value().len()..,
                            old.into(),
                        );
                    }
                }
            }
        }
        self.current_index = undo.current_index_before;
        self.content_hash = None;
        self.eols = undo.eols_before;
        self.undo_stack.push_redo(undo);
        Ok(())
    }

    pub fn redo(&mut self) -> Result<(), Error> {
        let Some(redo) = self.undo_stack.pop_redo() else {
            return Err(Error::NothingToRedo);
        };
        for change in redo.changes() {
            match change {
                Change::Insert { index, lines } => {
                    self.lines.splice(index..index, lines.iter().cloned());
                }
                Change::Remove { index, lines } => {
                    self.lines.splice(*index..*index + lines.len(), None);
                }
                Change::SetEols { span, old, new } => {
                    for line in &mut self.lines[span.clone()] {
                        line.replace_range(
                            line.len() - old.str_value().len()..,
                            new.into(),
                        );
                    }
                }
            }
        }
        self.current_index = redo.current_index_after;
        self.content_hash = None;
        self.eols = redo.eols_after;
        self.undo_stack.push_undo(redo, self.current_index, self.eols);
        Ok(())
    }

    pub fn clear(&mut self) {
        self.lines.clear();
        self.current_index = 0;
        self.content_hash = None;
        self.eols = Eols::new(Eol::Lf);
    }

    #[must_use]
    pub fn eols(&self) -> Eols {
        self.eols
    }

    pub fn set_eols(&mut self, eol: Eol) -> Option<ChangeSet> {
        if self.is_empty()
            || (self.eols.prevailing() == eol && !self.eols.is_mixed())
        {
            // Empty buffer or same eol && not mixed, so nothing to do
            return None;
        }

        // Prepare change set for undo/redo
        let mut changes = ChangeSet::new(self.current_index, self.eols);

        // normalize buffer lines
        let mut to_change = match eol {
            Eol::Lf => self.eols.crlfs,
            Eol::Crlf => self.eols.lfs,
        };
        let mut corrections: Option<(Range<usize>, Eol)> = None;

        for (i, line) in self.lines.iter_mut().enumerate() {
            if let Some(line_eol) = Eol::from_line(&mut *line)
                && line_eol != eol
            {
                line.replace_range(
                    line.len() - line_eol.str_value().len()..,
                    eol.into(),
                );
                let corrections = corrections.get_or_insert((i..i, line_eol));
                corrections.0.end += 1;
                to_change -= 1;
            } else if let Some((span, old)) = corrections.take() {
                changes.push(Change::SetEols { span, old, new: eol });
            }
            if to_change == 0 {
                break;
            }
        }

        if let Some((span, old)) = corrections {
            // Push last correction, if any
            changes.push(Change::SetEols { span, old, new: eol });
        }

        if !changes.is_empty() {
            self.eols = match eol {
                Eol::Lf => Eols { default_eol: eol, crlfs: 0, lfs: self.len() },
                Eol::Crlf => {
                    Eols { default_eol: eol, crlfs: self.len(), lfs: 0 }
                }
            };
            self.content_hash = None;
        }
        Some(changes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use similar_asserts::assert_eq;

    /////
    // EditBuffer creation tests

    #[test]
    fn new_buffer_has_zero_capacity() {
        let buffer = EditBuffer::new();
        assert_eq!(buffer.lines.capacity(), 0);
    }

    #[test]
    fn new_buffer_has_0_len() {
        let buffer = EditBuffer::new();
        assert_eq!(buffer.len(), 0);
    }

    #[test]
    fn buffer_with_capacity_has_correct_capacity() {
        const INIT_CAPACITY: usize = 1024;
        let buffer = EditBuffer::with_capacity(INIT_CAPACITY);
        assert_eq!(buffer.lines.capacity(), INIT_CAPACITY);
    }

    #[test]
    fn buffer_with_capacity_has_zero_len() {
        let buffer = EditBuffer::with_capacity(1024);
        assert_eq!(0, buffer.len());
    }

    #[test]
    fn buffer_from_vec_ensures_eols() {
        let buf_fully_terminated =
            EditBuffer::with_lines(["1\n", "2\n", "3\n"]);
        let buf_non_terminated = EditBuffer::with_lines(["1", "2", "3"]);
        let buf_partially_terminated =
            EditBuffer::with_lines(["1\n", "2", "3"]);
        assert_eq!(buf_partially_terminated[..], buf_fully_terminated[..]);
        assert!(
            buf_non_terminated
                .lines
                .iter()
                .all(|l| l.ends_with("\r\n") || l.ends_with('\n'))
        );
    }
    #[test]
    fn set_current_index() {
        let mut buffer = EditBuffer::with_lines(["1\n", "2", "3"]);
        buffer.set_current_index(2);
        assert_eq!(2, buffer.current_index());
    }

    #[test]
    #[should_panic = "index (is 99) must be within buffer (is 0..3)"]
    fn set_current_index_beyond_end() {
        let mut buffer = EditBuffer::with_lines(["1\r\n", "2", "3"]);
        buffer.set_current_index(99);
    }

    #[test]
    fn insert_one_to_empty_buffer() {
        let mut buffer = EditBuffer::new();
        let expected = EditBuffer::with_lines(["one\n"]);
        let lines = ["one\n"].map(ToOwned::to_owned).to_vec();
        let changes = buffer.insert(0, lines);
        assert_eq!(buffer.current_index, 0);
        assert_eq!(1, buffer.len());
        assert_eq!(&expected[..], &buffer[..]);
        assert!(!changes.is_empty());
    }

    #[test]
    fn insert_of_zero_lines() {
        let mut buffer = EditBuffer::with_lines(["1\n", "2", "3"]);
        let expected = EditBuffer::with_lines(["1\n", "2", "3"]);
        assert_eq!(buffer.current_index, 2);
        let changes = buffer.insert(2, Vec::new());
        assert!(changes.is_empty());
        assert_eq!(buffer.current_index, 2);
        assert_eq!(3, buffer.len());
        assert_eq!(buffer[..], expected[..]);
    }

    #[test]
    fn remove_span() {
        let mut buffer =
            EditBuffer::with_lines(["1\r\n", "2", "3", "4", "5", "6"]);
        let expected = EditBuffer::with_lines(["1\r\n", "2", "6"]);
        let changes = buffer.remove(2..5);
        assert!(!changes.is_empty());
        assert_eq!(buffer[..], expected[..]);
        assert_eq!(buffer.current_index(), 2);
    }

    #[test]
    fn remove_span_at_start() {
        let mut buffer =
            EditBuffer::with_lines(["1\r\n", "2", "3", "4", "5", "6"]);
        let expected = EditBuffer::with_lines(["4\r\n", "5", "6"]);
        dbg!(&buffer);
        dbg!(&expected);
        buffer.remove(0..3);
        assert_eq!(buffer[..], expected[..]);
        assert_eq!(buffer.current_index(), 0);
    }

    #[test]
    fn remove_span_at_end() {
        let mut buffer =
            EditBuffer::with_lines(["1\r\n", "2", "3", "4", "5", "6"]);
        let expected = EditBuffer::with_lines(["1\r\n", "2", "3", "4"]);
        buffer.remove(4..6);
        assert_eq!(buffer[..], expected[..]);
        assert_eq!(buffer.current_index(), 3);
    }

    #[test]
    fn buffer_dirty_after_insert() {
        let mut buffer = EditBuffer::new();
        let lines = ["1\n", "2\n", "3\n"].map(ToOwned::to_owned).to_vec();
        buffer.insert(0, lines);
        assert!(buffer.content_hash.is_none());
    }

    #[test]
    fn undo_remove_span() {
        let mut buffer =
            EditBuffer::with_lines(["1\n", "2", "3", "4", "5", "6"]);
        let expected = buffer.clone();
        let changes = buffer.remove(0..4);
        buffer.push_undo(changes);
        assert_eq!(buffer[..], EditBuffer::with_lines(["5\n", "6"])[..]);
        buffer.undo().unwrap();
        assert_eq!(buffer[..], expected[..]);
    }
    #[test]
    fn undo_redo_insert() {
        let mut buffer =
            EditBuffer::with_lines(["1\n", "2", "3", "4", "5", "6"]);
        let expected_final =
            EditBuffer::with_lines(["1\n", "2", "3", "4", "5", "6"]);
        let expected_modified = EditBuffer::with_lines([
            "1\n", "2", "a", "b", "c", "3", "4", "5", "6",
        ]);
        let lines = ["a\n", "b\n", "c\n"].map(ToOwned::to_owned).to_vec();
        let changes = buffer.insert(2, lines);
        buffer.push_undo(changes);
        assert_eq!(buffer[..], expected_modified[..]);
        buffer.undo().unwrap();
        assert_eq!(expected_final[..], buffer[..]);
        buffer.redo().unwrap();
        assert_eq!(buffer[..], expected_modified[..]);
    }

    #[test]
    fn undo_multi() {
        let mut buffer =
            EditBuffer::with_lines(["1\n", "2", "3", "4", "5", "6"]);
        let lines = ["a\n", "b\n", "c\n"].map(ToOwned::to_owned).to_vec();
        let expected_final = buffer.clone();
        assert_eq!(buffer.current_index(), 5);

        let changes = buffer.insert(2, lines);
        buffer.push_undo(changes);
        let expected_1 = EditBuffer::with_lines([
            "1\n", "2", "a", "b", "c", "3", "4", "5", "6",
        ]);
        assert_eq!(buffer[..], expected_1[..]);
        assert_eq!(buffer.current_index(), 4);

        let changes = buffer.remove(3..7);
        buffer.push_undo(changes);
        let expected_2 = EditBuffer::with_lines(["1\n", "2", "a", "5", "6"]);
        assert_eq!(buffer[..], expected_2[..]);

        buffer.undo().unwrap();
        assert_eq!(buffer[..], expected_1[..]);

        buffer.undo().unwrap();
        assert_eq!(buffer[..], expected_final[..]);
    }

    #[test]
    fn undo_redo_multi() {
        let mut buffer =
            EditBuffer::with_lines(["1\n", "2", "3", "4", "5", "6"]);
        let lines = ["a\n", "b\n", "c\n"].map(ToOwned::to_owned).to_vec();
        let expected_final = buffer.clone();
        assert_eq!(5, buffer.current_index());

        let changes = buffer.insert(2, lines);
        buffer.push_undo(changes);
        let expected_1 = EditBuffer::with_lines([
            "1\n", "2", "a", "b", "c", "3", "4", "5", "6",
        ]);
        assert_eq!(&expected_1[..], &buffer[..]);
        assert_eq!(4, buffer.current_index());

        let changes = buffer.remove(3..7);
        buffer.push_undo(changes);
        let expected_2 = EditBuffer::with_lines(["1\n", "2", "a", "5", "6"]);
        assert_eq!(buffer[..], expected_2[..]);

        buffer.undo().unwrap();
        assert_eq!(buffer[..], expected_1[..]);

        let lines = vec!["spam!\n".to_owned()];
        let changes = buffer.insert(5, lines);
        buffer.push_undo(changes);
        let expected_3 = EditBuffer::with_lines([
            "1\n", "2", "a", "b", "c", "spam!", "3", "4", "5", "6",
        ]);
        assert_eq!(buffer[..], expected_3[..]);

        buffer.undo().unwrap();
        assert_eq!(buffer[..], expected_1[..]);

        buffer.undo().unwrap();
        assert_eq!(buffer[..], expected_2[..]);

        buffer.undo().unwrap();
        assert_eq!(buffer[..], expected_1[..]);

        buffer.undo().unwrap();
        assert_eq!(buffer[..], expected_final[..]);

        let _ret = buffer.undo().expect_err("nothing to undo");
        assert!(matches!(Error::NothingToUndo, _ret));
        // Undo stack should be empty here, so buffer shouldn't change
        assert_eq!(buffer[..], expected_final[..]);
    }

    #[test]
    fn do_redo_multi() {
        let mut buffer =
            EditBuffer::with_lines(["1\n", "2", "3", "4", "5", "6"]);
        let buffer_orig = buffer.clone();
        assert_eq!(buffer.current_index(), 5);

        let lines = ["a\n", "b\n", "c\n"].map(ToOwned::to_owned).to_vec();
        let changes = buffer.insert(2, lines);
        buffer.push_undo(changes);
        let expected_1 = EditBuffer::with_lines([
            "1\n", "2", "a", "b", "c", "3", "4", "5", "6",
        ]);
        assert_eq!(&expected_1[..], &buffer[..]);
        assert_eq!(buffer.current_index(), 4);

        let changes = buffer.remove(3..7);
        buffer.push_undo(changes);
        let expected_final =
            EditBuffer::with_lines(["1\n", "2", "a", "5", "6"]);
        assert_eq!(buffer[..], expected_final[..]);

        buffer.undo().unwrap();
        assert_eq!(buffer[..], expected_1[..]);
        buffer.undo().unwrap();
        assert_eq!(buffer[..], buffer_orig[..]);
        let _ret = buffer.undo().expect_err("nothing to undo");
        assert!(matches!(Error::NothingToUndo, _ret));
        assert_eq!(buffer[..], buffer_orig[..]); // buffer unchanged

        buffer.redo().unwrap();
        assert_eq!(&expected_1[..], &buffer[..]);

        buffer.redo().unwrap();
        assert_eq!(buffer[..], expected_final[..]);

        let _ret = buffer.redo().expect_err("nothing to redo");
        assert!(matches!(Error::NothingToRedo, _ret));
        assert_eq!(buffer[..], expected_final[..]); // buffer unchanged
    }
}
