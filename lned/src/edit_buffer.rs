// EditBuffer keeps track of everything specific to a single buffer in the
// editor. All public interface uses one based indexing, and any such function
// is responsible for translating into the 0 based indexing of the Vec<String>
// containing the lines of text.
mod undo_stack;

use std::cmp::{self, Ordering};
use std::ops::{Index, Range, RangeFrom, RangeFull, RangeInclusive};
use std::path::{Path, PathBuf};

use regex::Regex;

use crate::command::Address;
use crate::edit_buffer::undo_stack::{ChangeSet, Diff, UndoStack};

#[derive(Debug, Clone)]
pub struct EditBuffer {
    pub current_line: usize,
    pub filename: Option<PathBuf>,
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

impl From<Vec<&str>> for EditBuffer {
    fn from(value: Vec<&str>) -> Self {
        let mut buf = EditBuffer::with_capacity(value.len());
        let default_eol = compute_default_eol(value.iter());
        buf.default_eol = Some(default_eol);
        let mut value = value
            .iter()
            .map(|v| {
                let mut line = (*v).to_string();
                if !(line.ends_with("\r\n") || line.ends_with('\n')) {
                    line.push_str(default_eol.as_ref());
                }
                line
            })
            .collect::<Vec<String>>();
        buf.text.append(&mut value);
        buf.current_line = buf.text.len();
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

impl Index<RangeFull> for EditBuffer {
    type Output = [String];

    #[inline]
    fn index(&self, index: RangeFull) -> &Self::Output {
        &self.text[index]
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

    pub fn do_append(&mut self, address: Option<Address>, lines: Vec<String>) {
        let location = address.map_or(self.current_line, |addr| addr.end());
        let mut change = ChangeSet::new();
        change.current_line_before = self.current_line;
        if lines.is_empty() {
            self.current_line = location;
        } else {
            self.append(location, lines.clone());
            self.current_line = location + lines.len();
            change.push_add(location, lines);
        }
        change.current_line_after = self.current_line;
        self.undo_stack.push_undo(change);
    }

    pub fn append(&mut self, location: usize, mut lines: Vec<String>) -> bool {
        let default_eol =
            self.default_eol.get_or_insert_with(|| compute_default_eol(&lines));

        // Add missing EOL if necessary
        let eol_added = match lines.last_mut() {
            Some(last) if !(last.ends_with("\r\n") || last.ends_with('\n')) => {
                last.push_str(default_eol);
                true
            }
            _ => false,
        };

        self.text.splice(location..location, lines);
        eol_added
    }

    pub fn do_change(&mut self, address: Option<Address>, lines: Vec<String>) {
        let mut change = ChangeSet::new();
        change.current_line_before = self.current_line;

        // handle deletion of addressed lines
        let b =
            cmp::max(1, address.map_or(self.current_line, |addr| addr.start()));
        let e = address.map_or(self.current_line, |addr| addr.end());
        let mut removed: Vec<String> = Vec::new();
        if b <= e {
            removed.extend(self.text.splice(b - 1..e, None));
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

        if !removed.is_empty() {
            change.push_remove(b - 1, removed);
        }
        change.current_line_after = self.current_line;
        self.undo_stack.push_undo(change);
    }

    pub fn do_delete(&mut self, address: Option<Address>) {
        let (b, e) = address
            .map_or((self.current_line, self.current_line), |addr| {
                (addr.start(), addr.end())
            });

        let removed: Vec<String> = self.text.splice(b - 1..e, None).collect();

        let mut change = ChangeSet::new();
        change.current_line_before = self.current_line;
        self.current_line = usize::min(self.text.len(), b);
        change.current_line_after = self.current_line;
        change.push_remove(b - 1, removed);
        self.undo_stack.push_undo(change);
    }

    pub fn do_insert(&mut self, address: Option<Address>, lines: Vec<String>) {
        let location = if lines.is_empty() {
            address.map_or(self.current_line, |addr| addr.end())
        } else {
            // insertion point is just before addressed line
            address
                .map_or(self.current_line, |addr| addr.end())
                .saturating_sub(1)
        };
        let mut change = ChangeSet::new();
        change.current_line_before = self.current_line;
        if lines.is_empty() {
            self.current_line = location;
        } else {
            //            // set default_eol if neccessary
            //            self.default_eol
            //                .get_or_insert_with(|| compute_default_eol(&lines));
            //            self.text.splice(location..location, lines.iter().cloned());
            self.append(location, lines.clone());
            self.current_line = location + lines.len();
            change.push_add(location, lines);
        }
        change.current_line_after = self.current_line;
        self.undo_stack.push_undo(change);
    }

    pub fn do_move(&mut self, address: Option<Address>, destination: Address) {
        let address =
            address.unwrap_or_else(|| Address::line(self.current_line));
        let lines: Vec<String> =
            self.text.drain(address.start() - 1..address.end()).collect();
        let destination = if destination.end() > address.end() {
            destination.end() - address.line_count()
        } else {
            destination.end()
        };

        let mut change = ChangeSet::new();
        change.current_line_before = self.current_line;
        change.push_add(destination, lines.clone());
        self.text.splice(destination..destination, lines);
        self.current_line = destination + address.line_count();
        change.current_line_after = self.current_line;
        self.undo_stack.push_undo(change);
    }

    pub fn do_undo(&mut self) {
        if let Some(undo) = self.undo_stack.pop_undo() {
            self.current_line = undo.current_line_before;
            {
                for diff in undo.diffs() {
                    match diff {
                        Diff::Add(p, l) => {
                            drop(self.text.splice(*p..*p + l.len(), None));
                        }
                        Diff::Remove(p, l) => {
                            drop(self.text.splice(*p..*p, l.iter().cloned()));
                        }
                    }
                }
            }
            self.undo_stack.push_redo(undo);
        }
    }

    pub fn do_redo(&mut self) {
        if let Some(redo) = self.undo_stack.pop_redo() {
            self.current_line = redo.current_line_after;
            {
                for diff in redo.diffs().rev() {
                    match diff {
                        Diff::Add(p, l) => {
                            self.text.splice(*p..*p, l.iter().cloned());
                        }
                        Diff::Remove(p, l) => {
                            self.text.splice(*p..*p + l.len(), None);
                        }
                    }
                }
            }
            self.undo_stack.push_undo(redo);
        }
    }

    pub fn do_transfer(
        &mut self,
        address: Option<Address>,
        destination: Address,
    ) {
        let address =
            address.unwrap_or_else(|| Address::line(self.current_line));
        let source = self.text[address.start() - 1..address.end()].to_vec();
        let destination = destination.end();

        let mut change = ChangeSet::new();
        change.current_line_before = self.current_line;
        change.push_add(destination, source.clone());
        self.text.splice(destination..destination, source);
        self.current_line = destination + address.line_count();
        change.current_line_after = self.current_line;
        self.undo_stack.push_undo(change);
    }

    pub fn clear_text(&mut self) {
        self.text.clear();
        self.default_eol = None;
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

    use std::io::{self, Read, Write};

    use similar_asserts::assert_eq;

    struct BadReader {}

    impl Read for BadReader {
        fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::from(io::ErrorKind::Other))
        }
    }

    struct BadWriter {}

    impl Write for BadWriter {
        fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
            Err(io::Error::from(io::ErrorKind::Other))
        }

        fn flush(&mut self) -> io::Result<()> {
            Err(io::Error::from(io::ErrorKind::Other))
        }
    }

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
        let buf_fully_terminated = EditBuffer::from(vec!["1\n", "2\n", "3\n"]);
        let buf_non_terminated = EditBuffer::from(vec!["1", "2", "3"]);
        let buf_partially_terminated = EditBuffer::from(vec!["1\n", "2", "3"]);
        assert_eq!(buf_partially_terminated[..], buf_fully_terminated[..]);
        assert!(buf_non_terminated
            .text
            .iter()
            .all(|l| l.ends_with("\r\n") || l.ends_with('\n')));
    }

    #[test]
    fn buffer_from_vec_is_clean() {
        let buf = EditBuffer::from(vec!["1\n", "2", "3"]);
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
        let buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        assert_eq!("1\n", buffer[1]);
        assert_eq!("6\n", buffer[6]);
    }

    #[test]
    #[should_panic = "index out of bounds"]
    fn zero_index_panics() {
        let buffer = EditBuffer::from(vec!["1"]);
        let _ = &buffer[0];
    }

    #[test]
    #[should_panic = "index out of bounds"]
    fn index_too_large_panics() {
        let buffer = EditBuffer::from(vec!["1", "2", "3"]);
        let _ = &buffer[4];
    }

    #[test]
    fn range_full() {
        let content = vec!["1\n", "2\n", "3\n", "4\n"];
        let buffer = EditBuffer::from(content.clone());
        assert_eq!(content, buffer[..]);
    }

    #[test]
    fn range_index() {
        let buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        assert_eq!(vec!["2\n", "3\n", "4\n"], buffer[2..5]);
        assert_eq!(
            vec!["1\n", "2\n", "3\n", "4\n", "5\n", "6\n"],
            buffer[1..7]
        );
    }

    #[test]
    fn range_inclusive_index() {
        let buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        assert_eq!(vec!["2\n", "3\n", "4\n"], buffer[2..=4]);
        assert_eq!(
            vec!["1\n", "2\n", "3\n", "4\n", "5\n", "6\n"],
            buffer[1..=6]
        );
    }

    #[test]
    #[should_panic = "Invalid range"]
    fn zero_based_range_panics() {
        let buffer = EditBuffer::from(vec!["1", "2"]);
        let _ = &buffer[0..2];
    }

    #[test]
    #[should_panic = "Invalid range"]
    fn zero_based_range_inclusive_panics() {
        let buffer = EditBuffer::from(vec!["1", "2"]);
        let _ = &buffer[0..=1];
    }

    #[test]
    #[should_panic = "Invalid range"]
    #[allow(clippy::reversed_empty_ranges)]
    fn zero_terminated_range_panics() {
        let buffer = EditBuffer::from(vec!["1", "2"]);
        let _ = &buffer[1..0];
    }

    #[test]
    #[should_panic = "Invalid range"]
    #[allow(clippy::reversed_empty_ranges)]
    fn zero_terminated_range_inclusive_panics() {
        let buffer = EditBuffer::from(vec!["1", "2"]);
        let _ = &buffer[1..=0];
    }

    #[test]
    #[should_panic = "range end index 4 out of range for slice of length 3"]
    fn range_too_far_beyond_end_panics() {
        let buffer = EditBuffer::from(vec!["1", "2", "3"]);
        let _ = &buffer[3..5];
    }

    #[test]
    #[should_panic = "range end index 4 out of range for slice of length 3"]
    fn range_inclusive_beyond_end_panics() {
        let buffer = EditBuffer::from(vec!["1", "2", "3"]);
        let _ = &buffer[3..=4];
    }

    #[test]
    fn range_from() {
        let buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        assert_eq!(vec!["4\n", "5\n", "6\n"], buffer[4..]);
        assert_eq!(vec!["6\n"], buffer[6..]);
        assert!(buffer[7..].is_empty());
    }

    #[test]
    #[should_panic = "Invalid range"]
    fn zero_based_range_from_panics() {
        let buffer = EditBuffer::from(vec!["1", "2", "3"]);
        let _ = &buffer[0..];
    }

    #[test]
    fn set_current_line() {
        let mut buffer = EditBuffer::from(vec!["1", "2", "3"]);
        buffer.set_current_line(2);
        assert_eq!(2, buffer.current_line());
    }

    #[test]
    #[should_panic = "0 is an invalid index (1-3)"]
    fn set_current_line_bad_index() {
        let mut buffer = EditBuffer::from(vec!["1", "2", "3"]);
        buffer.set_current_line(0);
    }

    #[test]
    #[should_panic = "99 is an invalid index (1-3)"]
    fn set_current_line_beyond_end() {
        let mut buffer = EditBuffer::from(vec!["1", "2", "3"]);
        buffer.set_current_line(99);
    }

    /////
    // cmd impl tests

    #[test]
    fn do_append_one_to_empty_buffer() {
        let mut buffer = EditBuffer::new();
        let expected = EditBuffer::from(vec!["one\n"]);
        let lines = ["one\n"].map(ToOwned::to_owned).to_vec();
        buffer.do_append(Some(Address::line(0)), lines);
        assert_eq!(1, buffer.current_line);
        assert_eq!(1, buffer.len());
        assert_eq!(&expected[..], &buffer[..]);
    }

    #[test]
    fn do_append_of_zero_lines() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3"]);
        let expected = EditBuffer::from(vec!["1\n", "2", "3"]);
        assert_eq!(3, buffer.current_line());
        buffer.do_append(Some(Address::line(2)), Vec::new());
        assert_eq!(2, buffer.current_line);
        assert_eq!(3, buffer.len());
        assert_eq!(buffer[..], expected[..]);
    }

    #[test]
    fn do_transfer_one_line() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        let expected =
            EditBuffer::from(vec!["1\n", "2", "3", "5", "4", "5", "6"]);
        buffer.do_transfer(Some(Address::line(5)), Address::line(3));
        assert_eq!(buffer[..], expected[..]);
        assert_eq!(buffer.current_line(), 4);
    }

    #[test]
    fn do_transfer_span() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        let expected =
            EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "4", "5", "6"]);
        buffer
            .do_transfer(Some(Address::span(4, 5).unwrap()), Address::line(3));
        assert_eq!(buffer[..], expected[..]);
        assert_eq!(buffer.current_line(), 5);
    }

    #[test]
    fn do_transfer_no_addr() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        let expected =
            EditBuffer::from(vec!["1\n", "2", "3", "1", "4", "5", "6"]);
        buffer.set_current_line(1);
        buffer.do_transfer(None, Address::line(3));
        assert_eq!(buffer[..], expected[..]);
        assert_eq!(buffer.current_line(), 4);
    }

    #[test]
    fn do_transfer_to_line_0() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        let expected =
            EditBuffer::from(vec!["4\n", "5", "1", "2", "3", "4", "5", "6"]);
        buffer
            .do_transfer(Some(Address::span(4, 5).unwrap()), Address::line(0));
        assert_eq!(buffer[..], expected[..]);
        assert_eq!(buffer.current_line(), 2);
    }

    #[test]
    fn do_transfer_destination_is_span() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        let expected =
            EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "4", "5", "6"]);
        buffer.do_transfer(
            Some(Address::span(4, 5).unwrap()),
            Address::span(1, 3).unwrap(),
        );
        assert_eq!(buffer[..], expected[..]);
        assert_eq!(buffer.current_line(), 5);
    }

    #[test]
    fn do_undo_redo_change_span() {
        let mut buffer = EditBuffer::new();
        let orig = EditBuffer::new();

        let expected1 = EditBuffer::from(vec!["1\n", "2", "3"]);
        buffer.do_change(Some(Address::line(0)), expected1[..].to_vec());
        assert_eq!(buffer[..], expected1[..]);
        assert_eq!(buffer.current_line(), 3);

        let expected2 =
            EditBuffer::from(vec!["1\n", "2", "4", "5", "6", "7", "8"]);
        buffer.do_change(None, expected2[3..].to_vec());
        assert_eq!(buffer[..], expected2[..]);
        assert_eq!(buffer.current_line(), 7);

        buffer.do_undo();
        assert_eq!(buffer[..], expected1[..]);
        assert_eq!(buffer.current_line(), 3);

        buffer.do_redo();
        assert_eq!(buffer[..], expected2[..]);
        assert_eq!(buffer.current_line(), 7);

        buffer.do_undo();
        assert_eq!(buffer[..], expected1[..]);
        assert_eq!(buffer.current_line(), 3);

        let expected3 = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        buffer.do_change(
            Some(Address::span(2, 3).unwrap()),
            expected3[2..].to_vec(),
        );
        assert_eq!(buffer[..], expected3[..]);
        assert_eq!(buffer.current_line(), 6);

        buffer.do_undo();
        assert_eq!(buffer[..], expected1[..]);
        assert_eq!(buffer.current_line(), 3);

        buffer.do_undo();
        assert_eq!(buffer[..], expected2[..]);
        assert_eq!(buffer.current_line(), 7);

        buffer.do_undo();
        assert_eq!(buffer[..], expected1[..]);
        assert_eq!(buffer.current_line(), 3);
        assert!(buffer.is_dirty());

        buffer.do_undo();
        assert!(buffer.is_empty());
        assert_eq!(buffer[..], orig[..]);
        assert_eq!(buffer.current_line(), 0);
        assert!(!buffer.is_dirty());

        buffer.do_undo();
        assert_eq!(buffer[..], orig[..]);
        assert_eq!(buffer.current_line(), 0);
    }

    #[test]
    fn do_undo_redo_change_line_0() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3"]);
        let orig = buffer.clone();
        let lines = vec!["changed\n".to_owned()];
        assert_eq!(buffer.current_line(), 3);
        assert_eq!(buffer[1], "1\n");

        buffer.do_change(Some(Address::line(0)), lines);
        assert_eq!(buffer.current_line(), 1);
        assert_eq!(buffer[1], "changed\n");
        buffer.do_undo();
        assert_eq!(buffer[..], orig[..]);
        assert_eq!(buffer.current_line(), orig.current_line());
    }

    #[test]
    fn do_undo_redo_change_span_no_input() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        let lines = Vec::new();
        assert_eq!(buffer.current_line(), 6);
        let orig = buffer.clone();

        buffer.do_change(Some(Address::span(3, 5).unwrap()), lines);
        assert_eq!(buffer.current_line(), 3);
        assert_eq!(buffer[3], "6\n");
        buffer.do_undo();
        assert_eq!(buffer[..], orig[..]);
        assert_eq!(buffer.current_line(), orig.current_line());

        let mut buffer = orig.clone();
        assert_eq!(buffer.current_line(), 6);
        buffer.do_change(Some(Address::span(5, 6).unwrap()), Vec::new());
        assert_eq!(buffer.current_line(), 4);
        buffer.do_undo();
        assert_eq!(buffer[..], orig[..]);
        assert_eq!(buffer.current_line(), orig.current_line());

        let mut buffer = orig.clone();
        assert_eq!(buffer.current_line(), 6);
        buffer.do_change(Some(Address::span(0, 2).unwrap()), Vec::new());
        assert_eq!(buffer.current_line(), 1);
        buffer.do_undo();
        assert_eq!(buffer[..], orig[..]);
        assert_eq!(buffer.current_line(), orig.current_line());

        let mut buffer = orig.clone();
        assert_eq!(buffer.current_line(), 6);
        buffer.do_change(Some(Address::span(1, 6).unwrap()), Vec::new());
        assert_eq!(buffer.current_line(), 0);
        assert!(buffer.is_empty());
        buffer.do_undo();
        assert_eq!(buffer[..], orig[..]);
        assert_eq!(buffer.current_line(), orig.current_line());
    }

    #[test]
    fn do_delete_span() {
        let mut buffer =
            EditBuffer::from(vec!["1\r\n", "2", "3", "4", "5", "6"]);
        let expected = EditBuffer::from(vec!["1\r\n", "2", "6"]);
        buffer.do_delete(Some(Address::span(3, 5).unwrap()));
        assert_eq!(3, buffer.len());
        assert_eq!(buffer[..], expected[..]);
    }

    #[test]
    fn do_delete_line() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        let expected = EditBuffer::from(vec!["1\n", "2", "4", "5", "6"]);
        buffer.do_delete(Some(Address::line(3)));
        assert_eq!(5, buffer.len());
        assert_eq!(buffer[..], expected[..]);
    }

    #[test]
    fn do_delete_span_at_start() {
        let mut buffer =
            EditBuffer::from(vec!["1\r\n", "2", "3", "4", "5", "6"]);
        let expected = EditBuffer::from(vec!["4\r\n", "5", "6"]);
        buffer.do_delete(Some(Address::span(1, 3).unwrap()));
        assert_eq!(3, buffer.len());
        assert_eq!(buffer[..], expected[..]);
    }

    #[test]
    fn do_delete_span_at_end() {
        let mut buffer =
            EditBuffer::from(vec!["1\r\n", "2", "3", "4", "5", "6"]);
        let expected = EditBuffer::from(vec!["1\r\n", "2", "3", "4"]);
        buffer.do_delete(Some(Address::span(5, 6).unwrap()));
        assert_eq!(4, buffer.len());
        assert_eq!(buffer[..], expected[..]);
    }

    #[test]
    fn do_delete_no_addr() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        let expected = EditBuffer::from(vec!["1\n", "2", "4", "5", "6"]);
        buffer.set_current_line(3);
        buffer.do_delete(None);
        assert_eq!(5, buffer.len());
        assert_eq!(buffer[..], expected[..]);
    }

    #[test]
    fn buffer_dirty_after_append() {
        let mut buffer = EditBuffer::new();
        let lines = ["1\n", "2\n", "3\n"].map(ToOwned::to_owned).to_vec();
        assert!(!buffer.is_dirty());
        buffer.do_append(Some(Address::line(0)), lines);
        assert!(buffer.is_dirty());
    }

    #[test]
    fn do_undo_append_line() {
        let mut buffer = EditBuffer::new();
        let lines = ["1\n", "2\n", "3\n"].map(ToOwned::to_owned).to_vec();
        buffer.do_append(Some(Address::line(0)), lines);
        assert_eq!(buffer[..], EditBuffer::from(vec!["1\n", "2", "3"])[..]);
        buffer.do_undo();
        assert_eq!(buffer[..], EditBuffer::new()[..]);
    }

    #[test]
    fn do_undo_delete_span() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        let expected = buffer.clone();
        buffer.do_delete(Some(Address::span(1, 4).unwrap()));
        assert_eq!(buffer[..], EditBuffer::from(vec!["5\n", "6"])[..]);
        buffer.do_undo();
        assert_eq!(buffer[..], expected[..]);
    }

    #[test]
    fn do_undo_delete_line() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        let expected = buffer.clone();
        buffer.do_delete(Some(Address::line(3)));
        assert_eq!(
            buffer[..],
            EditBuffer::from(vec!["1\n", "2", "4", "5", "6"])[..]
        );
        buffer.do_undo();
        assert_eq!(buffer[..], expected[..]);
    }

    #[test]
    fn do_undo_delete_current_line() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        buffer.set_current_line(4);
        let expected = buffer.clone();
        buffer.do_delete(None);
        assert_eq!(
            buffer[..],
            EditBuffer::from(vec!["1\n", "2", "3", "5", "6"])[..]
        );
        buffer.do_undo();
        assert_eq!(buffer[..], expected[..]);
    }

    #[test]
    fn do_undo_redo_insert() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        let expected_final =
            EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        let expected_modified = EditBuffer::from(vec![
            "1\n", "2", "a", "b", "c", "3", "4", "5", "6",
        ]);
        let lines = ["a\n", "b\n", "c\n"].map(ToOwned::to_owned).to_vec();
        buffer.do_insert(Some(Address::line(3)), lines);
        assert_eq!(buffer[..], expected_modified[..]);
        buffer.do_undo();
        assert_eq!(expected_final[..], buffer[..]);
        buffer.do_redo();
        assert_eq!(buffer[..], expected_modified[..]);
    }

    #[test]
    fn do_undo_transfer_line() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        let expected_final = buffer.clone();
        let mut expected_tr =
            EditBuffer::from(vec!["1\n", "2", "6", "3", "4", "5", "6"]);
        expected_tr.current_line = 3;
        buffer.do_transfer(Some(Address::line(6)), Address::line(2));
        assert_eq!(buffer[..], expected_tr[..]);
        assert_eq!(buffer.current_line(), expected_tr.current_line());
        buffer.do_undo();
        assert_eq!(buffer[..], expected_final[..]);
        assert_eq!(buffer.current_line(), expected_final.current_line());
    }

    #[test]
    fn do_undo_transfer_span() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        let expected_final = buffer.clone();
        let mut expected_tr =
            EditBuffer::from(vec!["1\n", "2", "5", "6", "3", "4", "5", "6"]);
        expected_tr.current_line = 4;
        buffer
            .do_transfer(Some(Address::span(5, 6).unwrap()), Address::line(2));
        assert_eq!(buffer[..], expected_tr[..]);
        assert_eq!(buffer.current_line(), expected_tr.current_line());
        buffer.do_undo();
        assert_eq!(buffer[..], expected_final[..]);
        assert_eq!(buffer.current_line(), expected_final.current_line());
    }

    #[test]
    fn do_undo_transfer_default() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        buffer.current_line = 6;
        let expected_final = buffer.clone();
        let mut expected_tr =
            EditBuffer::from(vec!["1\n", "2", "6", "3", "4", "5", "6"]);
        expected_tr.current_line = 3;
        buffer.do_transfer(None, Address::line(2));
        assert_eq!(buffer[..], expected_tr[..]);
        assert_eq!(buffer.current_line(), expected_tr.current_line());
        buffer.do_undo();
        assert_eq!(buffer[..], expected_final[..]);
        assert_eq!(buffer.current_line(), expected_final.current_line());
    }

    #[test]
    fn do_undo_multi() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        let lines = ["a\n", "b\n", "c\n"].map(ToOwned::to_owned).to_vec();
        let expected_final = buffer.clone();
        assert_eq!(buffer.current_line(), 6);

        buffer.do_append(Some(Address::line(2)), lines);
        let expected_1 = EditBuffer::from(vec![
            "1\n", "2", "a", "b", "c", "3", "4", "5", "6",
        ]);
        assert_eq!(buffer[..], expected_1[..]);
        assert_eq!(buffer.current_line(), 5);

        buffer.do_delete(Some(Address::span(4, 7).unwrap()));
        let expected_2 = EditBuffer::from(vec!["1\n", "2", "a", "5", "6"]);
        assert_eq!(buffer[..], expected_2[..]);

        buffer.do_undo();
        assert_eq!(buffer[..], expected_1[..]);

        buffer.do_undo();
        assert_eq!(buffer[..], expected_final[..]);
    }

    #[test]
    fn do_undo_redo_multi() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        let lines = ["a\n", "b\n", "c\n"].map(ToOwned::to_owned).to_vec();
        let expected_final = buffer.clone();
        assert_eq!(6, buffer.current_line());

        buffer.do_append(Some(Address::line(2)), lines);
        let expected_1 = EditBuffer::from(vec![
            "1\n", "2", "a", "b", "c", "3", "4", "5", "6",
        ]);
        assert_eq!(&expected_1[..], &buffer[..]);
        assert_eq!(5, buffer.current_line());

        buffer.do_delete(Some(Address::span(4, 7).unwrap()));
        let expected_2 = EditBuffer::from(vec!["1\n", "2", "a", "5", "6"]);
        assert_eq!(buffer[..], expected_2[..]);

        buffer.do_undo();
        assert_eq!(buffer[..], expected_1[..]);

        let lines = vec!["spam!\n".to_owned()];
        buffer.do_append(Some(Address::line(5)), lines);
        let expected_3 = EditBuffer::from(vec![
            "1\n", "2", "a", "b", "c", "spam!", "3", "4", "5", "6",
        ]);
        assert_eq!(buffer[..], expected_3[..]);

        buffer.do_undo();
        assert_eq!(buffer[..], expected_1[..]);

        buffer.do_undo();
        assert_eq!(buffer[..], expected_2[..]);

        buffer.do_undo();
        assert_eq!(buffer[..], expected_1[..]);

        buffer.do_undo();
        assert_eq!(buffer[..], expected_final[..]);

        buffer.do_undo();
        // Undo stack should be empty here, so buffer shouldn't change
        assert_eq!(buffer[..], expected_final[..]);
    }

    #[test]
    fn buffer_clean_after_undo_all() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        let lines = ["a\n", "b\n", "c\n"].map(ToOwned::to_owned).to_vec();

        buffer.do_append(Some(Address::line(2)), lines);

        buffer.do_delete(Some(Address::span(4, 7).unwrap()));

        let lines = ["x\n", "y\n", "z\n"].map(ToOwned::to_owned).to_vec();
        buffer.do_append(Some(Address::line(0)), lines);

        buffer.do_undo();

        buffer.do_undo();

        buffer.do_undo();

        assert!(!buffer.is_dirty());

        buffer.do_undo();
        assert!(!buffer.is_dirty()); // still not dirty
    }

    #[test]
    fn do_redo_multi() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        let buffer_orig = buffer.clone();
        assert_eq!(buffer.current_line(), 6);

        let lines = ["a\n", "b\n", "c\n"].map(ToOwned::to_owned).to_vec();
        buffer.do_append(Some(Address::line(2)), lines);
        let expected_1 = EditBuffer::from(vec![
            "1\n", "2", "a", "b", "c", "3", "4", "5", "6",
        ]);
        assert_eq!(&expected_1[..], &buffer[..]);
        assert_eq!(buffer.current_line(), 5);

        buffer.do_delete(Some(Address::span(4, 7).unwrap()));
        let expected_final = EditBuffer::from(vec!["1\n", "2", "a", "5", "6"]);
        assert_eq!(buffer[..], expected_final[..]);

        buffer.do_undo();
        assert_eq!(buffer[..], expected_1[..]);
        buffer.do_undo();
        assert_eq!(buffer[..], buffer_orig[..]);
        buffer.do_undo();
        assert_eq!(buffer[..], buffer_orig[..]); // buffer unchanged

        buffer.do_redo();
        assert_eq!(&expected_1[..], &buffer[..]);

        buffer.do_redo();
        assert_eq!(buffer[..], expected_final[..]);

        buffer.do_redo();
        assert_eq!(buffer[..], expected_final[..]); // buffer unchanged
    }

    #[test]
    fn do_insert_one_to_empty_buffer() {
        let mut buffer = EditBuffer::new();
        let expected = EditBuffer::from(vec!["one\n"]);
        let lines = vec!["one\n".to_owned()];
        buffer.do_insert(Some(Address::line(0)), lines);
        assert_eq!(1, buffer.current_line);
        assert_eq!(1, buffer.len());
        assert_eq!(buffer[..], expected[..]);
    }

    #[test]
    fn do_insert_empty_buffer() {
        let mut buffer = EditBuffer::new();
        let expected = EditBuffer::from(vec!["a\n", "b", "c"]);
        let lines = ["a\n", "b\n", "c\n"].map(ToOwned::to_owned).to_vec();
        buffer.do_insert(Some(Address::line(0)), lines);
        assert_eq!(3, buffer.current_line);
        assert_eq!(3, buffer.len());
        assert_eq!(buffer[..], expected[..]);
    }

    #[test]
    fn do_insert_non_empty_at_0() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3"]);
        let expected = EditBuffer::from(vec!["a\n", "b", "c", "1", "2", "3"]);
        let lines = ["a\n", "b\n", "c\n"].map(ToOwned::to_owned).to_vec();
        buffer.do_insert(Some(Address::line(0)), lines);
        assert_eq!(3, buffer.current_line);
        assert_eq!(6, buffer.len());
        assert!(&expected[..].eq(&buffer[..]));
    }

    #[test]
    fn do_insert_span_address() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        let lines = ["a\n", "b\n", "c\n"].map(ToOwned::to_owned).to_vec();
        let expected = EditBuffer::from(vec![
            "1\n", "2", "a", "b", "c", "3", "4", "5", "6",
        ]);
        buffer.do_insert(Some(Address::span(2, 3).unwrap()), lines);
        assert_eq!(5, buffer.current_line);
        assert_eq!(9, buffer.len());
        assert_eq!(buffer[..], expected[..]);
    }

    #[test]
    fn do_insert_at_end() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3"]);
        let lines = ["a\n", "b\n", "c\n"].map(ToOwned::to_owned).to_vec();
        let expected = EditBuffer::from(vec!["1\n", "2", "a", "b", "c", "3"]);
        buffer.do_insert(Some(Address::line(3)), lines);
        assert_eq!(5, buffer.current_line);
        assert_eq!(6, buffer.len());
        assert_eq!(buffer[..], expected[..]);
    }

    #[test]
    fn do_insert_of_zero_lines() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3"]);
        let expected = EditBuffer::from(vec!["1\n", "2", "3"]);
        assert_eq!(3, buffer.current_line());
        buffer.do_insert(Some(Address::line(2)), Vec::new());
        assert_eq!(2, buffer.current_line);
        assert_eq!(3, buffer.len());
        assert_eq!(buffer[..], expected[..]);
    }

    #[test]
    fn do_move_one_line() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        let expected = EditBuffer::from(vec!["1\n", "2", "3", "5", "4", "6"]);
        buffer.do_move(Some(Address::line(5)), Address::line(3));
        assert_eq!(buffer[..], expected[..]);
        assert_eq!(buffer.current_line(), 4);
    }

    #[test]
    fn do_move_span() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        let expected = EditBuffer::from(vec!["1\n", "2", "3", "5", "6", "4"]);
        buffer.do_move(Some(Address::span(5, 6).unwrap()), Address::line(3));
        assert_eq!(buffer[..], expected[..]);
        assert_eq!(buffer.current_line(), 5);
    }

    #[test]
    fn do_move_no_addr() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        let expected = EditBuffer::from(vec!["2\n", "3", "1", "4", "5", "6"]);
        buffer.set_current_line(1);
        buffer.do_move(None, Address::line(3));
        assert_eq!(buffer[..], expected[..]);
        assert_eq!(buffer.current_line(), 3);
    }

    #[test]
    fn do_move_to_line_0() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        let expected = EditBuffer::from(vec!["4\n", "5", "1", "2", "3", "6"]);
        buffer.do_move(Some(Address::span(4, 5).unwrap()), Address::line(0));
        assert_eq!(buffer[..], expected[..]);
        assert_eq!(buffer.current_line(), 2);
    }

    #[test]
    fn do_move_destination_is_span() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        let expected = EditBuffer::from(vec!["1\n", "2", "4", "5", "3", "6"]);
        buffer.do_move(
            Some(Address::span(4, 5).unwrap()),
            Address::span(1, 2).unwrap(),
        );
        assert_eq!(buffer[..], expected[..]);
        assert_eq!(buffer.current_line(), 4);
    }
}
