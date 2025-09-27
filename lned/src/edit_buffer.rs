// EditBuffer keeps track of everything specific to a single buffer in the
// editor. All public interface uses one based indexing, and any such function
// is responsible for translating into the 0 based indexing of the Vec<String>
// containing the lines of text.
mod undo_stack;

use std::cmp::{self, Ordering};
use std::ops::{
    Index, Range, RangeFrom, RangeFull, RangeInclusive, RangeTo,
    RangeToInclusive,
};
use std::path::{Path, PathBuf};

use regex::Regex;

use crate::command::Address;
pub use crate::edit_buffer::undo_stack::Change;
pub use crate::edit_buffer::undo_stack::ChangeSet;
pub use crate::edit_buffer::undo_stack::Diff;
use crate::edit_buffer::undo_stack::UndoStack;
use crate::main_loop::LnedError;

#[derive(Debug, Clone)]
pub struct EditBuffer {
    current_line: usize,
    filename: Option<PathBuf>,
    default_eol: Option<&'static str>,
    undo_stack: UndoStack,
    clean_fingerprint: Option<u64>,
    text: Vec<String>,
}

impl Default for EditBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl From<Vec<String>> for EditBuffer {
    fn from(lines: Vec<String>) -> Self {
        let line_count = lines.len();
        let mut buf = EditBuffer::with_capacity(line_count);
        buf.append(0, lines);
        buf.set_current_line(line_count);
        buf
    }
}

impl Index<usize> for EditBuffer {
    type Output = String;

    #[inline]
    fn index(&self, index: usize) -> &Self::Output {
        assert!(index != 0, "index out of bounds: 0 is an invalid index");

        &self.text[index - 1]
    }
}

impl Index<Range<usize>> for EditBuffer {
    type Output = [String];

    #[inline]
    fn index(&self, index: Range<usize>) -> &Self::Output {
        assert!(index.start > 0 && index.end > 0, "Invalid range");
        &self.text[index.start - 1..index.end - 1]
    }
}

impl Index<RangeInclusive<usize>> for EditBuffer {
    type Output = [String];

    #[inline]
    fn index(&self, index: RangeInclusive<usize>) -> &Self::Output {
        assert!(*index.start() > 0 && *index.end() > 0, "Invalid range");
        &self.text[(*index.start() - 1)..(*index.end())]
    }
}

impl Index<RangeFrom<usize>> for EditBuffer {
    type Output = [String];

    #[inline]
    fn index(&self, index: RangeFrom<usize>) -> &Self::Output {
        assert!(index.start > 0, "Invalid range");
        &self.text[index.start - 1..]
    }
}

impl Index<RangeTo<usize>> for EditBuffer {
    type Output = [String];

    #[inline]
    fn index(&self, index: RangeTo<usize>) -> &Self::Output {
        assert!(index.end > 0, "Invalid range");
        &self.text[..index.end - 1]
    }
}

impl Index<RangeToInclusive<usize>> for EditBuffer {
    type Output = [String];

    #[inline]
    fn index(&self, index: RangeToInclusive<usize>) -> &Self::Output {
        &self.text[..index.end]
    }
}

impl Index<RangeFull> for EditBuffer {
    type Output = [String];

    #[inline]
    fn index(&self, index: RangeFull) -> &Self::Output {
        &self.text[index]
    }
}

impl PartialEq for EditBuffer {
    fn eq(&self, other: &Self) -> bool {
        self.text == other.text && self.current_line == other.current_line
    }
}

impl EditBuffer {
    /// Creates a new empty `EditBuffer`.
    ///
    /// Consider the [`with_capacity`] method instead, to prevent this.
    ///
    /// [`with_capacity`]: EditBuffer::with_capacity
    #[inline]
    #[must_use]
    pub fn new() -> EditBuffer {
        EditBuffer {
            text: Vec::new(),
            current_line: 0,
            filename: None,
            default_eol: None,
            undo_stack: UndoStack::new(),
            clean_fingerprint: None,
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
            text: Vec::with_capacity(capacity),
            ..EditBuffer::default()
        }
    }

    #[cfg(test)]
    pub fn with_text(text: &[&str]) -> EditBuffer {
        let line_count = text.len();
        let text = text.iter().map(ToString::to_string).collect();
        let mut buf = EditBuffer::with_capacity(line_count);
        buf.append(0, text);
        buf.set_current_line(line_count);
        buf
    }

    #[must_use]
    /// Returns this `EditBuffer`'s length, in lines.
    pub fn len(&self) -> usize {
        self.text.len()
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// Returns true if buffer has been changed since last write.
    pub fn is_dirty(&self) -> bool {
        self.clean_fingerprint != self.undo_stack.fingerprint()
    }

    pub fn current_line(&self) -> usize {
        self.current_line
    }

    pub fn set_current_line(&mut self, line: usize) {
        if (line == 0 && !self.text.is_empty()) || line > self.text.len() {
            panic!("{line} is an invalid index (1-{})", self.len());
        } else {
            self.current_line = line;
        }
    }

    pub fn filename(&self) -> Option<&Path> {
        self.filename.as_deref()
    }

    pub fn set_filename(&mut self, filename: Option<PathBuf>) {
        self.filename = filename;
    }

    pub fn reset_clean_fingerprint(&mut self) -> Option<u64> {
        self.clean_fingerprint = self.undo_stack.fingerprint();
        self.clean_fingerprint
    }

    pub fn push_undo(&mut self, changes: ChangeSet) {
        self.undo_stack.push_undo(changes, self.current_line);
    }

    pub fn find_line(&self, pattern: &Regex) -> Option<usize> {
        if self.current_line == self.len() {
            (1..=self.len()).find(|&i| pattern.is_match(&self[i]))
        } else {
            (self.current_line + 1..=self.len())
                .find(|&i| pattern.is_match(&self[i]))
                .or_else(|| {
                    (1..=self.current_line)
                        .find(|&i| pattern.is_match(&self[i]))
                })
        }
    }

    pub fn find_line_rev(&self, pattern: &Regex) -> Option<usize> {
        if self.current_line == 1 {
            (1..=self.len()).rev().find(|&i| pattern.is_match(&self[i]))
        } else {
            (1..self.current_line)
                .rev()
                .find(|&i| pattern.is_match(&self[i]))
                .or_else(|| {
                    (self.current_line..=self.len())
                        .rev()
                        .find(|&i| pattern.is_match(&self[i]))
                })
        }
    }

    pub fn do_append(
        &mut self,
        address: Option<Address>,
        lines: Vec<String>,
    ) -> Option<ChangeSet> {
        let mut changes = ChangeSet::new(self.current_line);

        let mut change = Change::new(self.current_line);
        let location = address.map_or(self.current_line, |addr| addr.end());
        if lines.is_empty() {
            self.current_line = location;
            return None;
        }

        self.append(location, lines.clone());
        self.current_line = location + lines.len();
        change.push_add(location, lines);
        changes.push(change, self.current_line);
        Some(changes)
    }

    pub fn append(&mut self, location: usize, mut lines: Vec<String>) -> bool {
        let default_eol =
            self.default_eol.get_or_insert_with(|| compute_default_eol(&lines));

        // Add missing EOL if necessary
        let mut eol_added = false;
        for l in &mut lines {
            if !(l.ends_with("\r\n") || l.ends_with('\n')) {
                l.push_str(default_eol);
                eol_added = true;
            }
        }

        self.text.splice(location..location, lines);
        eol_added
    }

    pub fn do_change(
        &mut self,
        address: Option<Address>,
        lines: Vec<String>,
    ) -> ChangeSet {
        let mut changes = ChangeSet::new(self.current_line);
        let mut change = Change::new(self.current_line);

        // handle deletion of addressed lines
        let b =
            cmp::max(1, address.map_or(self.current_line, |addr| addr.start()));
        let e = address.map_or(self.current_line, |addr| addr.end());
        if b <= e {
            let removed = self.text.splice(b - 1..e, None).collect();
            change.push_remove(b - 1, removed);
        }

        // handle insertion of new lines, if any
        if lines.is_empty() {
            // remove only
            self.current_line = usize::min(self.text.len(), b);
        } else {
            let b = b.saturating_sub(1);
            self.append(b, lines.clone());
            self.current_line = b + lines.len();
            change.push_add(b, lines);
        }

        changes.push(change, self.current_line);
        changes
    }

    pub fn do_delete(&mut self, address: Option<Address>) -> ChangeSet {
        let (b, e) = address
            .map_or((self.current_line, self.current_line), |addr| {
                (addr.start(), addr.end())
            });

        let removed: Vec<String> = self.text.splice(b - 1..e, None).collect();

        let mut changes = ChangeSet::new(self.current_line);
        let mut change = Change::new(self.current_line);
        self.current_line = usize::min(self.text.len(), b);
        change.push_remove(b - 1, removed);
        changes.push(change, self.current_line);
        changes
    }

    pub fn do_insert(
        &mut self,
        address: Option<Address>,
        lines: Vec<String>,
    ) -> Option<ChangeSet> {
        let location = if lines.is_empty() {
            address.map_or(self.current_line, |addr| addr.end())
        } else {
            // insertion point is just before addressed line
            address
                .map_or(self.current_line, |addr| addr.end())
                .saturating_sub(1)
        };
        let mut changes = ChangeSet::new(self.current_line);
        let mut change = Change::new(self.current_line);
        if lines.is_empty() {
            self.current_line = location;
            return None;
        }

        self.append(location, lines.clone());
        self.current_line = location + lines.len();
        change.push_add(location, lines);
        changes.push(change, self.current_line);
        Some(changes)
    }

    pub fn do_join(
        &mut self,
        address: Option<Address>,
        separator: Option<&str>,
    ) -> ChangeSet {
        let address = address.unwrap_or_else(|| {
            Address::span(self.current_line, self.current_line + 1)
        });
        let mut changes = ChangeSet::new(self.current_line);
        let mut change = Change::new(self.current_line);

        let mut joined =
            self[address.start()].lines().next().unwrap().to_owned();
        if let Some(separator) = separator {
            joined.push_str(separator);
            for l in &self[address.start() + 1..address.end()] {
                joined.push_str(l.trim_start().lines().next().unwrap());
                joined.push_str(separator);
            }
            joined.push_str(self[address.end()].trim_start());
        } else {
            joined.extend(
                self[address.start() + 1..address.end()]
                    .iter()
                    .map(|l| l.lines().next().unwrap()),
            );
            joined.push_str(&self[address.end()]);
        }

        let replaced: Vec<_> = self
            .text
            .splice(address.start() - 1..address.end(), vec![joined.clone()])
            .collect();
        self.current_line = address.start();
        change.push_add(address.start() - 1, vec![joined]);
        change.push_remove(address.start(), replaced);
        changes.push(change, self.current_line);
        changes
    }

    pub fn do_move(
        &mut self,
        address: Option<Address>,
        destination: Address,
    ) -> ChangeSet {
        let address =
            address.unwrap_or_else(|| Address::line(self.current_line));
        let lines: Vec<String> =
            self.text.drain(address.start() - 1..address.end()).collect();
        let destination = if destination.end() >= address.end() {
            destination.end() - address.line_count()
        } else {
            destination.end()
        };

        let mut changes = ChangeSet::new(self.current_line);
        let mut change = Change::new(self.current_line);
        change.push_remove(address.start() - 1, lines.clone());
        change.push_add(destination, lines.clone());
        self.text.splice(destination..destination, lines);
        self.current_line = destination + address.line_count();
        changes.push(change, self.current_line);
        changes
    }

    pub fn do_undo(&mut self) -> Result<(), LnedError> {
        let Some(undo) = self.undo_stack.pop_undo() else {
            return Err(LnedError::NothingToUndo);
        };
        for change in undo.changes().rev() {
            self.current_line = change.current_line_after;
            for diff in change.diffs().rev() {
                match diff {
                    Diff::Add(p, l) => {
                        drop(self.text.splice(*p..*p + l.len(), None));
                    }
                    Diff::Remove(p, l) => {
                        drop(self.text.splice(*p..*p, l.iter().cloned()));
                    }
                }
            }
            self.current_line = change.current_line_before;
        }
        self.current_line = undo.current_line_before;
        self.undo_stack.push_redo(undo);
        Ok(())
    }

    pub fn do_redo(&mut self) -> Result<(), LnedError> {
        let Some(redo) = self.undo_stack.pop_redo() else {
            return Err(LnedError::NothingToRedo);
        };
        for change in redo.changes() {
            self.current_line = change.current_line_before;
            for diff in change.diffs() {
                match diff {
                    Diff::Add(p, l) => {
                        self.text.splice(*p..*p, l.iter().cloned());
                    }
                    Diff::Remove(p, l) => {
                        self.text.splice(*p..*p + l.len(), None);
                    }
                }
            }
            self.current_line = change.current_line_after;
        }
        self.current_line = redo.current_line_after;
        self.undo_stack.push_undo(redo, self.current_line);
        Ok(())
    }

    pub fn do_transfer(
        &mut self,
        address: Option<Address>,
        destination: Address,
    ) -> ChangeSet {
        let address =
            address.unwrap_or_else(|| Address::line(self.current_line));
        let source = self.text[address.start() - 1..address.end()].to_vec();
        let destination = destination.end();

        let mut changes = ChangeSet::new(self.current_line);
        let mut change = Change::new(self.current_line);
        change.push_add(destination, source.clone());
        self.text.splice(destination..destination, source);
        self.current_line = destination + address.line_count();
        changes.push(change, self.current_line);
        changes
    }

    pub fn clear_text(&mut self) {
        self.text.clear();
        self.current_line = 0;
        self.default_eol = None;
    }

    pub fn default_eol(&mut self) -> &'static str {
        self.default_eol.get_or_insert_with(line_reader::native_eol)
    }
}

fn compute_default_eol(
    lines: impl IntoIterator<Item = impl AsRef<str>>,
) -> &'static str {
    let native_eol = line_reader::native_eol();
    let mut crlf = 0;
    let mut lf = 0;

    for line in lines {
        let line = line.as_ref();
        if line.ends_with("\r\n") {
            crlf += 1;
        } else if line.ends_with('\n') {
            lf += 1;
        }
    }

    match crlf.cmp(&lf) {
        Ordering::Greater => "\r\n",
        Ordering::Less => "\n",
        Ordering::Equal => native_eol,
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
        assert_eq!(buffer.text.capacity(), 0);
    }

    #[test]
    fn new_buffer_has_0_len() {
        let buffer = EditBuffer::new();
        assert_eq!(buffer.len(), 0);
    }

    #[test]
    fn new_empty_buffer_is_clean() {
        let buffer = EditBuffer::new();
        assert!(!buffer.is_dirty());
    }

    #[test]
    fn buffer_with_capacity_has_correct_capacity() {
        const INIT_CAPACITY: usize = 1024;
        let buffer = EditBuffer::with_capacity(INIT_CAPACITY);
        assert_eq!(buffer.text.capacity(), INIT_CAPACITY);
    }

    #[test]
    fn buffer_with_capacity_has_zero_len() {
        let buffer = EditBuffer::with_capacity(1024);
        assert_eq!(0, buffer.len());
    }

    #[test]
    fn buffer_from_vec_ensures_eols() {
        let buf_fully_terminated =
            EditBuffer::with_text(&["1\n", "2\n", "3\n"]);
        let buf_non_terminated = EditBuffer::with_text(&["1", "2", "3"]);
        let buf_partially_terminated =
            EditBuffer::with_text(&["1\n", "2", "3"]);
        assert_eq!(buf_partially_terminated[..], buf_fully_terminated[..]);
        assert!(
            buf_non_terminated
                .text
                .iter()
                .all(|l| l.ends_with("\r\n") || l.ends_with('\n'))
        );
    }

    #[test]
    fn buffer_from_vec_is_clean() {
        let buf = EditBuffer::with_text(&["1\n", "2", "3"]);
        assert!(!buf.is_dirty());
    }

    /////
    // compute_default_eol() tests

    #[test]
    fn default_eol_when_all_crlf() {
        let lines = vec!["L1\r\n", "L2\r\n", "L3\r\n"];
        assert_eq!("\r\n", compute_default_eol(&lines));
    }

    #[test]
    fn default_eol_when_all_lf() {
        let lines = vec!["L1\n", "L2\n", "L3\n"];
        assert_eq!("\n", compute_default_eol(&lines));
    }

    #[test]
    fn default_eol_when_most_crlf() {
        let lines = vec!["L1\r\n", "L2\n", "L3\r\n"];
        assert_eq!("\r\n", compute_default_eol(&lines));
    }

    #[test]
    fn default_eol_when_most_lf() {
        let lines = vec!["L1\n", "L2\n", "L3\r\n"];
        assert_eq!("\n", compute_default_eol(&lines));
    }

    #[test]
    fn default_eol_when_equal_lf_crlf() {
        let lines = vec!["L1\n", "L2\r\n", "L3\r\n", "L4\n"];
        assert_eq!(compute_default_eol(&lines), line_reader::native_eol());
    }

    /////
    // Indexing tests

    #[test]
    fn usize_index() {
        let buffer = EditBuffer::with_text(&["1\n", "2", "3", "4", "5", "6"]);
        assert_eq!("1\n", buffer[1]);
        assert_eq!("6\n", buffer[6]);
    }

    #[test]
    #[should_panic = "index out of bounds"]
    fn zero_index_panics() {
        let buffer = EditBuffer::with_text(&["1"]);
        let _ = &buffer[0];
    }

    #[test]
    #[should_panic = "index out of bounds"]
    fn index_too_large_panics() {
        let buffer = EditBuffer::with_text(&["1", "2", "3"]);
        let _ = &buffer[4];
    }

    #[test]
    fn range_full() {
        let content = ["1\n", "2\n", "3\n", "4\n"];
        let buffer = EditBuffer::with_text(&content);
        assert_eq!(content, buffer[..]);
    }

    #[test]
    fn range_index() {
        let buffer = EditBuffer::with_text(&["1\n", "2", "3", "4", "5", "6"]);
        assert_eq!(vec!["2\n", "3\n", "4\n"], buffer[2..5]);
        assert_eq!(
            vec!["1\n", "2\n", "3\n", "4\n", "5\n", "6\n"],
            buffer[1..7]
        );
    }

    #[test]
    fn range_inclusive_index() {
        let buffer = EditBuffer::with_text(&["1\n", "2", "3", "4", "5", "6"]);
        assert_eq!(vec!["2\n", "3\n", "4\n"], buffer[2..=4]);
        assert_eq!(
            vec!["1\n", "2\n", "3\n", "4\n", "5\n", "6\n"],
            buffer[1..=6]
        );
    }

    #[test]
    #[should_panic = "Invalid range"]
    fn zero_based_range_panics() {
        let buffer = EditBuffer::with_text(&["1", "2"]);
        let _ = &buffer[0..2];
    }

    #[test]
    #[should_panic = "Invalid range"]
    fn zero_based_range_inclusive_panics() {
        let buffer = EditBuffer::with_text(&["1", "2"]);
        let _ = &buffer[0..=1];
    }

    #[test]
    #[should_panic = "Invalid range"]
    #[allow(clippy::reversed_empty_ranges)]
    fn zero_terminated_range_panics() {
        let buffer = EditBuffer::with_text(&["1", "2"]);
        let _ = &buffer[1..0];
    }

    #[test]
    #[should_panic = "Invalid range"]
    #[allow(clippy::reversed_empty_ranges)]
    fn zero_terminated_range_inclusive_panics() {
        let buffer = EditBuffer::with_text(&["1", "2"]);
        let _ = &buffer[1..=0];
    }

    #[test]
    #[should_panic = "range end index 4 out of range for slice of length 3"]
    fn range_too_far_beyond_end_panics() {
        let buffer = EditBuffer::with_text(&["1", "2", "3"]);
        let _ = &buffer[3..5];
    }

    #[test]
    #[should_panic = "range end index 4 out of range for slice of length 3"]
    fn range_inclusive_beyond_end_panics() {
        let buffer = EditBuffer::with_text(&["1", "2", "3"]);
        let _ = &buffer[3..=4];
    }

    #[test]
    fn range_from() {
        let buffer = EditBuffer::with_text(&["1\n", "2", "3", "4", "5", "6"]);
        assert_eq!(vec!["4\n", "5\n", "6\n"], buffer[4..]);
        assert_eq!(vec!["6\n"], buffer[6..]);
        assert!(buffer[7..].is_empty());
    }

    #[test]
    #[should_panic = "Invalid range"]
    fn zero_based_range_from_panics() {
        let buffer = EditBuffer::with_text(&["1", "2", "3"]);
        let _ = &buffer[0..];
    }

    #[test]
    fn set_current_line() {
        let mut buffer = EditBuffer::with_text(&["1", "2", "3"]);
        buffer.set_current_line(2);
        assert_eq!(2, buffer.current_line());
    }

    #[test]
    #[should_panic = "0 is an invalid index (1-3)"]
    fn set_current_line_bad_index() {
        let mut buffer = EditBuffer::with_text(&["1", "2", "3"]);
        buffer.set_current_line(0);
    }

    #[test]
    #[should_panic = "99 is an invalid index (1-3)"]
    fn set_current_line_beyond_end() {
        let mut buffer = EditBuffer::with_text(&["1", "2", "3"]);
        buffer.set_current_line(99);
    }

    /////
    // cmd impl tests

    #[test]
    fn do_append_one_to_empty_buffer() {
        let mut buffer = EditBuffer::new();
        let expected = EditBuffer::with_text(&["one\n"]);
        let lines = ["one\n"].map(ToOwned::to_owned).to_vec();
        buffer.do_append(Some(Address::line(0)), lines);
        assert_eq!(1, buffer.current_line);
        assert_eq!(1, buffer.len());
        assert_eq!(&expected[..], &buffer[..]);
    }

    #[test]
    fn do_append_of_zero_lines() {
        let mut buffer = EditBuffer::with_text(&["1\n", "2", "3"]);
        let expected = EditBuffer::with_text(&["1\n", "2", "3"]);
        assert_eq!(3, buffer.current_line());
        buffer.do_append(Some(Address::line(2)), Vec::new());
        assert_eq!(2, buffer.current_line);
        assert_eq!(3, buffer.len());
        assert_eq!(buffer[..], expected[..]);
    }

    #[test]
    fn do_transfer_one_line() {
        let mut buffer =
            EditBuffer::with_text(&["1\n", "2", "3", "4", "5", "6"]);
        let expected =
            EditBuffer::with_text(&["1\n", "2", "3", "5", "4", "5", "6"]);
        buffer.do_transfer(Some(Address::line(5)), Address::line(3));
        assert_eq!(buffer[..], expected[..]);
        assert_eq!(buffer.current_line(), 4);
    }

    #[test]
    fn do_transfer_span() {
        let mut buffer =
            EditBuffer::with_text(&["1\n", "2", "3", "4", "5", "6"]);
        let expected =
            EditBuffer::with_text(&["1\n", "2", "3", "4", "5", "4", "5", "6"]);
        buffer.do_transfer(Some(Address::span(4, 5)), Address::line(3));
        assert_eq!(buffer[..], expected[..]);
        assert_eq!(buffer.current_line(), 5);
    }

    #[test]
    fn do_transfer_no_addr() {
        let mut buffer =
            EditBuffer::with_text(&["1\n", "2", "3", "4", "5", "6"]);
        let expected =
            EditBuffer::with_text(&["1\n", "2", "3", "1", "4", "5", "6"]);
        buffer.set_current_line(1);
        buffer.do_transfer(None, Address::line(3));
        assert_eq!(buffer[..], expected[..]);
        assert_eq!(buffer.current_line(), 4);
    }

    #[test]
    fn do_transfer_to_line_0() {
        let mut buffer =
            EditBuffer::with_text(&["1\n", "2", "3", "4", "5", "6"]);
        let expected =
            EditBuffer::with_text(&["4\n", "5", "1", "2", "3", "4", "5", "6"]);
        buffer.do_transfer(Some(Address::span(4, 5)), Address::line(0));
        assert_eq!(buffer[..], expected[..]);
        assert_eq!(buffer.current_line(), 2);
    }

    #[test]
    fn do_transfer_destination_is_span() {
        let mut buffer =
            EditBuffer::with_text(&["1\n", "2", "3", "4", "5", "6"]);
        let expected =
            EditBuffer::with_text(&["1\n", "2", "3", "4", "5", "4", "5", "6"]);
        buffer.do_transfer(Some(Address::span(4, 5)), Address::span(1, 3));
        assert_eq!(buffer[..], expected[..]);
        assert_eq!(buffer.current_line(), 5);
    }

    #[test]
    fn do_delete_span() {
        let mut buffer =
            EditBuffer::with_text(&["1\r\n", "2", "3", "4", "5", "6"]);
        let expected = EditBuffer::with_text(&["1\r\n", "2", "6"]);
        buffer.do_delete(Some(Address::span(3, 5)));
        assert_eq!(3, buffer.len());
        assert_eq!(buffer[..], expected[..]);
    }

    #[test]
    fn do_delete_line() {
        let mut buffer =
            EditBuffer::with_text(&["1\n", "2", "3", "4", "5", "6"]);
        let expected = EditBuffer::with_text(&["1\n", "2", "4", "5", "6"]);
        buffer.do_delete(Some(Address::line(3)));
        assert_eq!(5, buffer.len());
        assert_eq!(buffer[..], expected[..]);
    }

    #[test]
    fn do_delete_span_at_start() {
        let mut buffer =
            EditBuffer::with_text(&["1\r\n", "2", "3", "4", "5", "6"]);
        let expected = EditBuffer::with_text(&["4\r\n", "5", "6"]);
        buffer.do_delete(Some(Address::span(1, 3)));
        assert_eq!(3, buffer.len());
        assert_eq!(buffer[..], expected[..]);
    }

    #[test]
    fn do_delete_span_at_end() {
        let mut buffer =
            EditBuffer::with_text(&["1\r\n", "2", "3", "4", "5", "6"]);
        let expected = EditBuffer::with_text(&["1\r\n", "2", "3", "4"]);
        buffer.do_delete(Some(Address::span(5, 6)));
        assert_eq!(4, buffer.len());
        assert_eq!(buffer[..], expected[..]);
    }

    #[test]
    fn do_delete_no_addr() {
        let mut buffer =
            EditBuffer::with_text(&["1\n", "2", "3", "4", "5", "6"]);
        let expected = EditBuffer::with_text(&["1\n", "2", "4", "5", "6"]);
        buffer.set_current_line(3);
        buffer.do_delete(None);
        assert_eq!(5, buffer.len());
        assert_eq!(buffer[..], expected[..]);
    }

    #[test]
    fn do_insert_one_to_empty_buffer() {
        let mut buffer = EditBuffer::new();
        let expected = EditBuffer::with_text(&["one\n"]);
        let lines = vec!["one\n".to_owned()];
        buffer.do_insert(Some(Address::line(0)), lines);
        assert_eq!(1, buffer.current_line);
        assert_eq!(1, buffer.len());
        assert_eq!(buffer[..], expected[..]);
    }

    #[test]
    fn do_insert_empty_buffer() {
        let mut buffer = EditBuffer::new();
        let expected = EditBuffer::with_text(&["a\n", "b", "c"]);
        let lines = ["a\n", "b\n", "c\n"].map(ToOwned::to_owned).to_vec();
        buffer.do_insert(Some(Address::line(0)), lines);
        assert_eq!(3, buffer.current_line);
        assert_eq!(3, buffer.len());
        assert_eq!(buffer[..], expected[..]);
    }

    #[test]
    fn do_insert_non_empty_at_0() {
        let mut buffer = EditBuffer::with_text(&["1\n", "2", "3"]);
        let expected = EditBuffer::with_text(&["a\n", "b", "c", "1", "2", "3"]);
        let lines = ["a\n", "b\n", "c\n"].map(ToOwned::to_owned).to_vec();
        buffer.do_insert(Some(Address::line(0)), lines);
        assert_eq!(3, buffer.current_line);
        assert_eq!(6, buffer.len());
        assert!(&expected[..].eq(&buffer[..]));
    }

    #[test]
    fn do_insert_span_address() {
        let mut buffer =
            EditBuffer::with_text(&["1\n", "2", "3", "4", "5", "6"]);
        let lines = ["a\n", "b\n", "c\n"].map(ToOwned::to_owned).to_vec();
        let expected = EditBuffer::with_text(&[
            "1\n", "2", "a", "b", "c", "3", "4", "5", "6",
        ]);
        buffer.do_insert(Some(Address::span(2, 3)), lines);
        assert_eq!(5, buffer.current_line);
        assert_eq!(9, buffer.len());
        assert_eq!(buffer[..], expected[..]);
    }

    #[test]
    fn do_insert_at_end() {
        let mut buffer = EditBuffer::with_text(&["1\n", "2", "3"]);
        let lines = ["a\n", "b\n", "c\n"].map(ToOwned::to_owned).to_vec();
        let expected = EditBuffer::with_text(&["1\n", "2", "a", "b", "c", "3"]);
        buffer.do_insert(Some(Address::line(3)), lines);
        assert_eq!(5, buffer.current_line);
        assert_eq!(6, buffer.len());
        assert_eq!(buffer[..], expected[..]);
    }

    #[test]
    fn do_insert_of_zero_lines() {
        let mut buffer = EditBuffer::with_text(&["1\n", "2", "3"]);
        let expected = EditBuffer::with_text(&["1\n", "2", "3"]);
        assert_eq!(3, buffer.current_line());
        buffer.do_insert(Some(Address::line(2)), Vec::new());
        assert_eq!(2, buffer.current_line);
        assert_eq!(3, buffer.len());
        assert_eq!(buffer[..], expected[..]);
    }

    #[test]
    fn do_join_default_addr() {
        let mut buffer =
            EditBuffer::with_text(&["1\n", "2", "3", "4", "5", "6"]);
        buffer.current_line = 2;
        let mut expected = EditBuffer::with_text(&["1\n", "23", "4", "5", "6"]);
        expected.current_line = 2;
        buffer.do_join(None, None);
        assert_eq!(buffer, expected);
    }

    #[test]
    fn do_join_two_lines() {
        let mut buffer =
            EditBuffer::with_text(&["1\n", "2", "3", "4", "5", "6"]);
        buffer.current_line = 2;
        let mut expected = EditBuffer::with_text(&["1\n", "2", "3 4", "5", "6"]);
        expected.set_current_line(3);
        buffer.do_join(Some(Address::span(3, 4)), Some(" "));
        assert_eq!(buffer, expected);
    }

    #[test]
    fn do_join_several_lines() {
        let mut buffer =
            EditBuffer::with_text(&["1\n", "2", "3", "4", "5", "6"]);
        buffer.current_line = 2;
        let mut expected = EditBuffer::with_text(&["1\n", "2", "345", "6"]);
        expected.set_current_line(3);
        buffer.do_join(Some(Address::span(3, 5)), None);
        assert_eq!(buffer, expected);
    }

    #[test]
    fn do_move_one_line() {
        let mut buffer =
            EditBuffer::with_text(&["1\n", "2", "3", "4", "5", "6"]);
        let orig = buffer.clone();
        let mut expected =
            EditBuffer::with_text(&["1\n", "2", "3", "5", "4", "6"]);
        expected.current_line = 4;
        let changes = buffer.do_move(Some(Address::line(5)), Address::line(3));
        buffer.push_undo(changes);
        assert_eq!(buffer[..], expected[..]);
        assert_eq!(buffer.current_line(), expected.current_line);

        buffer.do_undo().expect("something to undo");
        assert_eq!(buffer[..], orig[..]);
        assert_eq!(buffer.current_line, orig.current_line);

        buffer.do_redo().expect("something on redo stack");
        assert_eq!(buffer[..], expected[..]);
        assert_eq!(buffer.current_line, expected.current_line);
    }

    #[test]
    fn do_move_span() {
        let mut buffer =
            EditBuffer::with_text(&["1\n", "2", "3", "4", "5", "6"]);
        let orig = buffer.clone();
        let mut expected =
            EditBuffer::with_text(&["1\n", "2", "3", "5", "6", "4"]);
        expected.current_line = 5;
        let changes =
            buffer.do_move(Some(Address::span(5, 6)), Address::line(3));
        buffer.push_undo(changes);
        assert_eq!(buffer[..], expected[..]);
        assert_eq!(buffer.current_line(), expected.current_line);

        buffer.do_undo().expect("something on undo stack");
        assert_eq!(buffer[..], orig[..]);
        assert_eq!(buffer.current_line, orig.current_line);

        buffer.do_redo().expect("something on redo stack");
        assert_eq!(buffer[..], expected[..]);
        assert_eq!(buffer.current_line, expected.current_line);
    }

    #[test]
    fn do_move_no_addr() {
        let mut buffer =
            EditBuffer::with_text(&["1\n", "2", "3", "4", "5", "6"]);
        buffer.set_current_line(1);
        let orig = buffer.clone();
        let mut expected =
            EditBuffer::with_text(&["2\n", "3", "1", "4", "5", "6"]);
        expected.set_current_line(3);
        let changes = buffer.do_move(None, Address::line(3));
        buffer.push_undo(changes);
        assert_eq!(buffer[..], expected[..]);
        assert_eq!(buffer.current_line(), expected.current_line());

        buffer.do_undo().expect("something on undo stack");
        assert_eq!(buffer[..], orig[..]);
        assert_eq!(buffer.current_line(), orig.current_line());

        buffer.do_redo().expect("something on redo stack");
        assert_eq!(buffer[..], expected[..]);
        assert_eq!(buffer.current_line(), expected.current_line());
    }

    #[test]
    fn do_move_to_line_0() {
        let mut buffer =
            EditBuffer::with_text(&["1\n", "2", "3", "4", "5", "6"]);
        let orig = buffer.clone();
        let mut expected =
            EditBuffer::with_text(&["4\n", "5", "1", "2", "3", "6"]);
        expected.set_current_line(2);
        let changes =
            buffer.do_move(Some(Address::span(4, 5)), Address::line(0));
        buffer.push_undo(changes);
        assert_eq!(buffer[..], expected[..]);
        assert_eq!(buffer.current_line(), expected.current_line());

        buffer.do_undo().expect("something on undo stack");
        assert_eq!(buffer[..], orig[..]);
        assert_eq!(buffer.current_line(), orig.current_line());

        buffer.do_redo().expect("something on redo stack");
        assert_eq!(buffer[..], expected[..]);
        assert_eq!(buffer.current_line(), expected.current_line());
    }

    #[test]
    fn do_move_destination_is_span() {
        let mut buffer =
            EditBuffer::with_text(&["1\n", "2", "3", "4", "5", "6"]);
        let orig = buffer.clone();
        let mut expected =
            EditBuffer::with_text(&["1\n", "2", "4", "5", "3", "6"]);
        expected.set_current_line(4);
        let changes =
            buffer.do_move(Some(Address::span(4, 5)), Address::span(1, 2));
        buffer.push_undo(changes);
        assert_eq!(buffer[..], expected[..]);
        assert_eq!(buffer.current_line(), expected.current_line());

        buffer.do_undo().expect("something on undo stack");
        assert_eq!(buffer[..], orig[..]);
        assert_eq!(buffer.current_line(), orig.current_line());

        buffer.do_redo().expect("something on redo stack");
        assert_eq!(buffer[..], expected[..]);
        assert_eq!(buffer.current_line(), expected.current_line());
    }

    #[test]
    fn buffer_dirty_after_append() {
        let mut buffer = EditBuffer::new();
        let lines = ["1\n", "2\n", "3\n"].map(ToOwned::to_owned).to_vec();
        assert!(!buffer.is_dirty());
        let changes = buffer
            .do_append(Some(Address::line(0)), lines)
            .expect("Some(ChangeSet)");
        buffer.push_undo(changes);
        assert!(buffer.is_dirty());
    }

    #[test]
    fn buffer_clean_after_0_line_append() {
        let mut buffer = EditBuffer::new();
        let lines = Vec::new();
        assert!(!buffer.is_dirty());
        let changes = buffer.do_append(Some(Address::line(0)), lines);
        assert!(changes.is_none());
        assert!(!buffer.is_dirty());
    }

    #[test]
    fn do_undo_append_line() {
        let mut buffer = EditBuffer::new();
        let lines = ["1\n", "2\n", "3\n"].map(ToOwned::to_owned).to_vec();
        let changes = buffer
            .do_append(Some(Address::line(0)), lines)
            .expect("Some(ChangeSet)");
        buffer.push_undo(changes);
        assert_eq!(buffer[..], EditBuffer::with_text(&["1\n", "2", "3"])[..]);
        buffer.do_undo().unwrap();
        assert_eq!(buffer[..], EditBuffer::new()[..]);
    }

    #[test]
    fn do_undo_delete_span() {
        let mut buffer =
            EditBuffer::with_text(&["1\n", "2", "3", "4", "5", "6"]);
        let expected = buffer.clone();
        let changes = buffer.do_delete(Some(Address::span(1, 4)));
        buffer.push_undo(changes);
        assert_eq!(buffer[..], EditBuffer::with_text(&["5\n", "6"])[..]);
        buffer.do_undo().unwrap();
        assert_eq!(buffer[..], expected[..]);
    }

    #[test]
    fn do_undo_delete_line() {
        let mut buffer =
            EditBuffer::with_text(&["1\n", "2", "3", "4", "5", "6"]);
        let expected = buffer.clone();
        let changes = buffer.do_delete(Some(Address::line(3)));
        buffer.push_undo(changes);
        assert_eq!(
            buffer[..],
            EditBuffer::with_text(&["1\n", "2", "4", "5", "6"])[..]
        );
        buffer.do_undo().unwrap();
        assert_eq!(buffer[..], expected[..]);
    }

    #[test]
    fn do_undo_delete_current_line() {
        let mut buffer =
            EditBuffer::with_text(&["1\n", "2", "3", "4", "5", "6"]);
        buffer.set_current_line(4);
        let expected = buffer.clone();
        let changes = buffer.do_delete(None);
        buffer.push_undo(changes);
        assert_eq!(
            buffer[..],
            EditBuffer::with_text(&["1\n", "2", "3", "5", "6"])[..]
        );
        buffer.do_undo().unwrap();
        assert_eq!(buffer[..], expected[..]);
    }

    #[test]
    fn do_undo_redo_insert() {
        let mut buffer =
            EditBuffer::with_text(&["1\n", "2", "3", "4", "5", "6"]);
        let expected_final =
            EditBuffer::with_text(&["1\n", "2", "3", "4", "5", "6"]);
        let expected_modified = EditBuffer::with_text(&[
            "1\n", "2", "a", "b", "c", "3", "4", "5", "6",
        ]);
        let lines = ["a\n", "b\n", "c\n"].map(ToOwned::to_owned).to_vec();
        let changes = buffer
            .do_insert(Some(Address::line(3)), lines)
            .expect("Some(ChangeSet)");
        buffer.push_undo(changes);
        assert_eq!(buffer[..], expected_modified[..]);
        buffer.do_undo().unwrap();
        assert_eq!(expected_final[..], buffer[..]);
        buffer.do_redo().unwrap();
        assert_eq!(buffer[..], expected_modified[..]);
    }

    #[test]
    fn do_undo_transfer_line() {
        let mut buffer =
            EditBuffer::with_text(&["1\n", "2", "3", "4", "5", "6"]);
        let expected_final = buffer.clone();
        let mut expected_tr =
            EditBuffer::with_text(&["1\n", "2", "6", "3", "4", "5", "6"]);
        expected_tr.current_line = 3;
        let changes =
            buffer.do_transfer(Some(Address::line(6)), Address::line(2));
        buffer.push_undo(changes);
        assert_eq!(buffer[..], expected_tr[..]);
        assert_eq!(buffer.current_line(), expected_tr.current_line());
        buffer.do_undo().unwrap();
        assert_eq!(buffer[..], expected_final[..]);
        assert_eq!(buffer.current_line(), expected_final.current_line());
    }

    #[test]
    fn do_undo_transfer_span() {
        let mut buffer =
            EditBuffer::with_text(&["1\n", "2", "3", "4", "5", "6"]);
        let expected_final = buffer.clone();
        let mut expected_tr =
            EditBuffer::with_text(&["1\n", "2", "5", "6", "3", "4", "5", "6"]);
        expected_tr.current_line = 4;
        let changes =
            buffer.do_transfer(Some(Address::span(5, 6)), Address::line(2));
        buffer.push_undo(changes);
        assert_eq!(buffer[..], expected_tr[..]);
        assert_eq!(buffer.current_line(), expected_tr.current_line());
        buffer.do_undo().unwrap();
        assert_eq!(buffer[..], expected_final[..]);
        assert_eq!(buffer.current_line(), expected_final.current_line());
    }

    #[test]
    fn do_undo_transfer_default() {
        let mut buffer =
            EditBuffer::with_text(&["1\n", "2", "3", "4", "5", "6"]);
        buffer.current_line = 6;
        let expected_final = buffer.clone();
        let mut expected_tr =
            EditBuffer::with_text(&["1\n", "2", "6", "3", "4", "5", "6"]);
        expected_tr.current_line = 3;
        let changes = buffer.do_transfer(None, Address::line(2));
        buffer.push_undo(changes);
        assert_eq!(buffer[..], expected_tr[..]);
        assert_eq!(buffer.current_line(), expected_tr.current_line());
        buffer.do_undo().unwrap();
        assert_eq!(buffer[..], expected_final[..]);
        assert_eq!(buffer.current_line(), expected_final.current_line());
    }

    #[test]
    fn do_undo_multi() {
        let mut buffer =
            EditBuffer::with_text(&["1\n", "2", "3", "4", "5", "6"]);
        let lines = ["a\n", "b\n", "c\n"].map(ToOwned::to_owned).to_vec();
        let expected_final = buffer.clone();
        assert_eq!(buffer.current_line(), 6);

        let changes = buffer
            .do_append(Some(Address::line(2)), lines)
            .expect("Some(ChangeSet)");
        buffer.push_undo(changes);
        let expected_1 = EditBuffer::with_text(&[
            "1\n", "2", "a", "b", "c", "3", "4", "5", "6",
        ]);
        assert_eq!(buffer[..], expected_1[..]);
        assert_eq!(buffer.current_line(), 5);

        let changes = buffer.do_delete(Some(Address::span(4, 7)));
        buffer.push_undo(changes);
        let expected_2 = EditBuffer::with_text(&["1\n", "2", "a", "5", "6"]);
        assert_eq!(buffer[..], expected_2[..]);

        buffer.do_undo().unwrap();
        assert_eq!(buffer[..], expected_1[..]);

        buffer.do_undo().unwrap();
        assert_eq!(buffer[..], expected_final[..]);
    }

    #[test]
    fn do_undo_redo_multi() {
        let mut buffer =
            EditBuffer::with_text(&["1\n", "2", "3", "4", "5", "6"]);
        let lines = ["a\n", "b\n", "c\n"].map(ToOwned::to_owned).to_vec();
        let expected_final = buffer.clone();
        assert_eq!(6, buffer.current_line());

        let changes = buffer
            .do_append(Some(Address::line(2)), lines)
            .expect("Some(ChangeSet)");
        buffer.push_undo(changes);
        let expected_1 = EditBuffer::with_text(&[
            "1\n", "2", "a", "b", "c", "3", "4", "5", "6",
        ]);
        assert_eq!(&expected_1[..], &buffer[..]);
        assert_eq!(5, buffer.current_line());

        let changes = buffer.do_delete(Some(Address::span(4, 7)));
        buffer.push_undo(changes);
        let expected_2 = EditBuffer::with_text(&["1\n", "2", "a", "5", "6"]);
        assert_eq!(buffer[..], expected_2[..]);

        buffer.do_undo().unwrap();
        assert_eq!(buffer[..], expected_1[..]);

        let lines = vec!["spam!\n".to_owned()];
        let changes = buffer
            .do_append(Some(Address::line(5)), lines)
            .expect("Some(ChangeSet)");
        buffer.push_undo(changes);
        let expected_3 = EditBuffer::with_text(&[
            "1\n", "2", "a", "b", "c", "spam!", "3", "4", "5", "6",
        ]);
        assert_eq!(buffer[..], expected_3[..]);

        buffer.do_undo().unwrap();
        assert_eq!(buffer[..], expected_1[..]);

        buffer.do_undo().unwrap();
        assert_eq!(buffer[..], expected_2[..]);

        buffer.do_undo().unwrap();
        assert_eq!(buffer[..], expected_1[..]);

        buffer.do_undo().unwrap();
        assert_eq!(buffer[..], expected_final[..]);

        let _ret = buffer.do_undo().expect_err("nothing to undo");
        assert!(matches!(LnedError::NothingToUndo, _ret));
        // Undo stack should be empty here, so buffer shouldn't change
        assert_eq!(buffer[..], expected_final[..]);
    }

    #[test]
    fn buffer_clean_after_undo_all() {
        let mut buffer =
            EditBuffer::with_text(&["1\n", "2", "3", "4", "5", "6"]);
        let lines = ["a\n", "b\n", "c\n"].map(ToOwned::to_owned).to_vec();

        let changes = buffer
            .do_append(Some(Address::line(2)), lines)
            .expect("Some(ChangeSet)");
        buffer.push_undo(changes);

        let changes = buffer.do_delete(Some(Address::span(4, 7)));
        buffer.push_undo(changes);

        let lines = ["x\n", "y\n", "z\n"].map(ToOwned::to_owned).to_vec();
        let changes = buffer
            .do_append(Some(Address::line(0)), lines)
            .expect("Some(ChangeSet)");
        buffer.push_undo(changes);

        buffer.do_undo().unwrap();

        buffer.do_undo().unwrap();

        buffer.do_undo().unwrap();

        assert!(!buffer.is_dirty());

        let _ret = buffer.do_undo().expect_err("nothing to undo");
        assert!(matches!(LnedError::NothingToUndo, _ret));
        assert!(!buffer.is_dirty()); // still not dirty
    }

    #[test]
    fn do_redo_multi() {
        let mut buffer =
            EditBuffer::with_text(&["1\n", "2", "3", "4", "5", "6"]);
        let buffer_orig = buffer.clone();
        assert_eq!(buffer.current_line(), 6);

        let lines = ["a\n", "b\n", "c\n"].map(ToOwned::to_owned).to_vec();
        let changes = buffer
            .do_append(Some(Address::line(2)), lines)
            .expect("Some(ChangeSet)");
        buffer.push_undo(changes);
        let expected_1 = EditBuffer::with_text(&[
            "1\n", "2", "a", "b", "c", "3", "4", "5", "6",
        ]);
        assert_eq!(&expected_1[..], &buffer[..]);
        assert_eq!(buffer.current_line(), 5);

        let changes = buffer.do_delete(Some(Address::span(4, 7)));
        buffer.push_undo(changes);
        let expected_final =
            EditBuffer::with_text(&["1\n", "2", "a", "5", "6"]);
        assert_eq!(buffer[..], expected_final[..]);

        buffer.do_undo().unwrap();
        assert_eq!(buffer[..], expected_1[..]);
        buffer.do_undo().unwrap();
        assert_eq!(buffer[..], buffer_orig[..]);
        let _ret = buffer.do_undo().expect_err("nothing to undo");
        assert!(matches!(LnedError::NothingToUndo, _ret));
        assert_eq!(buffer[..], buffer_orig[..]); // buffer unchanged

        buffer.do_redo().unwrap();
        assert_eq!(&expected_1[..], &buffer[..]);

        buffer.do_redo().unwrap();
        assert_eq!(buffer[..], expected_final[..]);

        let _ret = buffer.do_redo().expect_err("nothing to redo");
        assert!(matches!(LnedError::NothingToRedo, _ret));
        assert_eq!(buffer[..], expected_final[..]); // buffer unchanged
    }
    #[test]
    fn do_undo_redo_change_span() {
        let mut buffer = EditBuffer::new();
        let orig = EditBuffer::new();

        let expected1 = EditBuffer::with_text(&["1\n", "2", "3"]);
        let changes =
            buffer.do_change(Some(Address::line(0)), expected1[..].to_vec());
        buffer.push_undo(changes);
        assert_eq!(buffer[..], expected1[..]);
        assert_eq!(buffer.current_line(), 3);

        let expected2 =
            EditBuffer::with_text(&["1\n", "2", "4", "5", "6", "7", "8"]);
        let changes = buffer.do_change(None, expected2[3..].to_vec());
        buffer.push_undo(changes);
        assert_eq!(buffer[..], expected2[..]);
        assert_eq!(buffer.current_line(), 7);

        buffer.do_undo().unwrap();
        assert_eq!(buffer[..], expected1[..]);
        assert_eq!(buffer.current_line(), 3);

        buffer.do_redo().unwrap();
        assert_eq!(buffer[..], expected2[..]);
        assert_eq!(buffer.current_line(), 7);

        buffer.do_undo().unwrap();
        assert_eq!(buffer[..], expected1[..]);
        assert_eq!(buffer.current_line(), 3);

        let expected3 =
            EditBuffer::with_text(&["1\n", "2", "3", "4", "5", "6"]);
        let changes = buffer
            .do_change(Some(Address::span(2, 3)), expected3[2..].to_vec());
        buffer.push_undo(changes);
        assert_eq!(buffer[..], expected3[..]);
        assert_eq!(buffer.current_line(), 6);

        buffer.do_undo().unwrap();
        assert_eq!(buffer[..], expected1[..]);
        assert_eq!(buffer.current_line(), 3);

        buffer.do_undo().unwrap();
        assert_eq!(buffer[..], expected2[..]);
        assert_eq!(buffer.current_line(), 7);

        buffer.do_undo().unwrap();
        assert_eq!(buffer[..], expected1[..]);
        assert_eq!(buffer.current_line(), 3);
        assert!(buffer.is_dirty());

        buffer.do_undo().unwrap();
        assert!(buffer.is_empty());
        assert_eq!(buffer[..], orig[..]);
        assert_eq!(buffer.current_line(), 0);
        assert!(!buffer.is_dirty());

        let _ret = buffer.do_undo().expect_err("nothing to undo");
        assert!(matches!(LnedError::NothingToUndo, _ret));
        assert_eq!(buffer[..], orig[..]);
        assert_eq!(buffer.current_line(), 0);
    }

    #[test]
    fn do_undo_redo_change_line_0() {
        let mut buffer = EditBuffer::with_text(&["1\n", "2", "3"]);
        let orig = buffer.clone();
        let lines = vec!["changed\n".to_owned()];
        assert_eq!(buffer.current_line(), 3);
        assert_eq!(buffer[1], "1\n");

        let changes = buffer.do_change(Some(Address::line(0)), lines);
        buffer.push_undo(changes);
        assert_eq!(buffer.current_line(), 1);
        assert_eq!(buffer[1], "changed\n");
        buffer.do_undo().unwrap();
        assert_eq!(buffer[..], orig[..]);
        assert_eq!(buffer.current_line(), orig.current_line());
    }

    #[test]
    fn do_undo_redo_change_span_no_input() {
        let mut buffer =
            EditBuffer::with_text(&["1\n", "2", "3", "4", "5", "6"]);
        let lines = Vec::new();
        assert_eq!(buffer.current_line(), 6);
        let orig = buffer.clone();

        let changes = buffer.do_change(Some(Address::span(3, 5)), lines);
        buffer.push_undo(changes);
        assert_eq!(buffer.current_line(), 3);
        assert_eq!(buffer[3], "6\n");
        buffer.do_undo().unwrap();
        assert_eq!(buffer[..], orig[..]);
        assert_eq!(buffer.current_line(), orig.current_line());

        let mut buffer = orig.clone();
        assert_eq!(buffer.current_line(), 6);
        let changes = buffer.do_change(Some(Address::span(5, 6)), Vec::new());
        buffer.push_undo(changes);
        assert_eq!(buffer.current_line(), 4);
        buffer.do_undo().unwrap();
        assert_eq!(buffer[..], orig[..]);
        assert_eq!(buffer.current_line(), orig.current_line());

        let mut buffer = orig.clone();
        assert_eq!(buffer.current_line(), 6);
        let changes = buffer.do_change(Some(Address::span(0, 2)), Vec::new());
        buffer.push_undo(changes);
        assert_eq!(buffer.current_line(), 1);
        buffer.do_undo().unwrap();
        assert_eq!(buffer[..], orig[..]);
        assert_eq!(buffer.current_line(), orig.current_line());

        let mut buffer = orig.clone();
        assert_eq!(buffer.current_line(), 6);
        let changes = buffer.do_change(Some(Address::span(1, 6)), Vec::new());
        buffer.push_undo(changes);
        assert_eq!(buffer.current_line(), 0);
        assert!(buffer.is_empty());
        buffer.do_undo().unwrap();
        assert_eq!(buffer[..], orig[..]);
        assert_eq!(buffer.current_line(), orig.current_line());
    }
}
