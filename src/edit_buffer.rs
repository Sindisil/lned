// EditBuffer presents a list of text lines, the ability to add/remove lines,
// 0 based Indexing, Undo/Redo functionality, and conversion functions.
// It maintains current_index, eol, and content_hash during mutation.
use std::hash::{DefaultHasher, Hash, Hasher};
use std::iter::Peekable;
use std::ops::{
    Index, Range, RangeFrom, RangeFull, RangeInclusive, RangeTo,
    RangeToInclusive,
};

use regex::Regex;
use unicode_segmentation::{Graphemes, UnicodeSegmentation};

use crate::command;
use crate::eol::{Eol, Eols};
use crate::error::Error;
use crate::iter_utils::Peeking;
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

    #[cfg(test)]
    pub fn with_lines(lines: &[&str]) -> EditBuffer {
        let line_count = lines.len();
        let mut lines: Vec<_> = lines.iter().map(ToString::to_string).collect();
        let eols = Eols::from_lines(&lines);
        for line in &mut lines[..] {
            if Eol::from_line(&mut *line).is_none() {
                line.push_str(eols.prevailing().into());
            }
        }
        let mut buf = EditBuffer::with_capacity(line_count);
        buf.insert(0, lines);
        buf
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

    #[cfg(test)]
    pub fn current_address(&mut self) -> Option<Address> {
        let index = self.current_index.to_string();
        Address::eval(&mut index.graphemes(true).peekable(), self, &mut None)
            .unwrap()
    }

    pub fn current_index(&self) -> usize {
        self.current_index
    }

    pub fn current_index_as_range(&self) -> Range<usize> {
        self.current_index..(self.current_index + 1)
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

    fn find_line(&self, pattern: &Regex) -> Option<usize> {
        let index = if self.current_index == self.len() - 1 {
            (0..self.len()).find(|&i| pattern.is_match(Eol::strip(&self[i])))
        } else {
            (self.current_index + 1..self.len())
                .find(|&i| pattern.is_match(Eol::strip(&self[i])))
                .or_else(|| {
                    (0..=self.current_index)
                        .find(|&i| pattern.is_match(Eol::strip(&self[i])))
                })
        };
        index.map(|i| i + 1)
    }

    fn find_line_rev(&self, pattern: &Regex) -> Option<usize> {
        let index = if self.current_index == 0 {
            (0..self.len())
                .rev()
                .find(|&i| pattern.is_match(Eol::strip(&self[i])))
        } else {
            (0..self.current_index)
                .rev()
                .find(|&i| pattern.is_match(Eol::strip(&self[i])))
                .or_else(|| {
                    (self.current_index..self.len())
                        .rev()
                        .find(|&i| pattern.is_match(Eol::strip(&self[i])))
                })
        };
        index.map(|i| i + 1)
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
        self.lines.splice(index..index, lines.clone());

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
    /// Moves lines indexed by `range` to `destination`.
    ///
    /// It is invalid for `destination` to be within `range`.
    ///
    /// The last line moved becomes the new `current_index`.
    ///
    /// Returns a [`ChangeSet`] rerpresenting the buffer changes.
    ///
    /// If `range` is empty no action is taken and an empty
    /// [`ChangeSet`] is returned.
    ///
    /// # Panics
    ///
    /// Will panic if `range` extends beyond last line of buffer, if
    /// `destination` is within `range`, or if `destination` is
    /// beyond last line of buffer.
    ///
    pub fn relocate(
        &mut self,
        span: Range<usize>,
        mut destination: usize,
    ) -> ChangeSet {
        assert!(
            !span.contains(&destination),
            "span (is {span:?}) must not overlap destination (is {destination})"
        );
        assert!(
            destination <= self.len(),
            "destination (is {destination}) must not be > buffer length (is {})",
            self.len()
        );
        assert!(
            !span.contains(&self.len()),
            "span (is {span:?}) must be within buffer (is 0..{})",
            self.len()
        );

        let mut changes = ChangeSet::new(self.current_index, self.eols);
        changes.push(Change::Relocate { span: span.clone(), destination });
        if destination >= span.end {
            destination -= span.end - span.start;
        }
        let lines: Vec<_> = self.lines.drain(span).collect();
        self.current_index = destination + lines.len();
        self.lines.splice(destination..destination, lines);
        self.content_hash = None;
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
                Change::Relocate { span, destination } => {
                    let n = span.end - span.start;
                    let r = *destination..*destination + n;
                    let lines: Vec<_> = self.lines.drain(r).collect();
                    self.lines.splice(span.start..span.start, lines);
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
                Change::Relocate { span, destination } => {
                    let destination = if *destination > span.end {
                        *destination - span.end - span.start
                    } else {
                        *destination
                    };
                    let lines: Vec<_> =
                        self.lines.drain(span.clone()).collect();
                    self.lines.splice(destination..destination, lines);
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

#[derive(Debug, PartialEq, Copy, Clone)]
pub struct Address {
    first: usize,
    last: usize,
}

impl Address {
    pub fn first(&self) -> usize {
        self.first
    }

    pub fn last(&self) -> usize {
        self.last
    }

    pub fn contains(&self, line: usize) -> bool {
        self.first <= line && line <= self.last
    }

    pub fn line_count(&self) -> usize {
        self.last - self.first + 1
    }

    #[cfg(test)]
    pub fn from_str(
        s: &str,
        buffer: &mut EditBuffer,
        previous_pattern: &mut Option<Regex>,
    ) -> Option<Address> {
        Self::eval(&mut s.graphemes(true).peekable(), buffer, previous_pattern)
            .expect("from_str must be passed a valid address string")
    }

    pub fn eval(
        graphemes: &mut Peekable<Graphemes<'_>>,
        buffer: &mut EditBuffer,
        previous_pattern: &mut Option<Regex>,
    ) -> Result<Option<Address>, Error> {
        let mut left = None;
        let mut right = None;

        loop {
            match graphemes.peek() {
                Some(&",") => {
                    graphemes.next();
                    left = right.or(Some(1));
                    right = right.or_else(|| Some(buffer.len()));
                }
                Some(&";") => {
                    graphemes.next();
                    left = Some(match right {
                        Some(r) if r > buffer.len() => {
                            return Err(Error::InvalidAddress);
                        }
                        Some(r) => {
                            buffer.set_current_index(r - 1);
                            r
                        }
                        None => buffer.current_index() + 1,
                    });
                    right = right.or_else(|| Some(buffer.len()));
                }
                Some(&"+" | &"-") => {
                    right = Some(eval_line_number(
                        graphemes,
                        buffer.current_index() + 1,
                    )?);
                }
                Some(&".") => {
                    graphemes.next();
                    right = Some(eval_line_number(
                        graphemes,
                        buffer.current_index() + 1,
                    )?);
                }
                Some(&"$") => {
                    graphemes.next();
                    right = Some(eval_line_number(graphemes, buffer.len())?);
                }
                Some(&"/") => {
                    graphemes.next();
                    let (pattern, _) =
                        command::parse_pattern(graphemes, Some("/"), false)?;
                    if !pattern.is_empty() {
                        *previous_pattern =
                            Some(Regex::new(&pattern).map_err(|e| {
                                Error::Regex { source: Some(Box::new(e)) }
                            })?);
                    }
                    let re = previous_pattern
                        .as_ref()
                        .ok_or(Error::NoPreviousPattern)?;
                    let line = buffer.find_line(re).ok_or(Error::NoMatch)?;
                    right = Some(eval_line_number(graphemes, line)?);
                }
                Some(&"?") => {
                    graphemes.next();
                    let (pattern, _) =
                        command::parse_pattern(graphemes, Some("?"), false)?;
                    if !pattern.is_empty() {
                        *previous_pattern =
                            Some(Regex::new(&pattern).map_err(|e| {
                                Error::Regex { source: Some(Box::new(e)) }
                            })?);
                    }
                    let re = previous_pattern
                        .as_ref()
                        .ok_or(Error::NoPreviousPattern)?;
                    let line =
                        buffer.find_line_rev(re).ok_or(Error::NoMatch)?;
                    right = Some(eval_line_number(graphemes, line)?);
                }
                Some(&" " | &"\t") => {
                    graphemes.next();
                }
                Some(_) => {
                    if let Some(num) = command::parse_usize(graphemes)? {
                        right = Some(eval_line_number(graphemes, num)?);
                    } else {
                        break;
                    }
                }
                None => break,
            }
            if left.is_none() && right.is_some() {
                left = right;
            }
        }

        if let Some(last) = right {
            if buffer.is_empty() {
                return Err(Error::InvalidAddress);
            }
            let first = left.unwrap_or(last);
            if first > last || last > buffer.len() {
                return Err(Error::InvalidAddress);
            }
            Ok(Some(Address { first, last }))
        } else {
            Ok(None)
        }
    }
}

impl IntoIterator for Address {
    type Item = usize;
    type IntoIter = Range<usize>;

    fn into_iter(self) -> Self::IntoIter {
        self.into()
    }
}

impl From<Address> for Range<usize> {
    fn from(address: Address) -> Self {
        (address.first() - 1)..address.last()
    }
}

fn eval_line_number(
    graphemes: &mut Peekable<Graphemes<'_>>,
    line: usize,
) -> Result<usize, Error> {
    let offset = compute_line_offset(graphemes)?;
    line.checked_add_signed(offset).ok_or(Error::InvalidOffset)
}

fn compute_line_offset(
    graphemes: &mut Peekable<Graphemes<'_>>,
) -> Result<isize, Error> {
    let mut total_offset = 0isize;
    while let Some(n) = parse_offset_element(graphemes)? {
        total_offset =
            total_offset.checked_add(n).ok_or(Error::InvalidOffset)?;
    }
    Ok(total_offset)
}

fn parse_offset_element(
    graphemes: &mut Peekable<Graphemes<'_>>,
) -> Result<Option<isize>, Error> {
    // Skip leading whitespace
    while graphemes.peek().is_some_and(|s| *s == " " || *s == "\t") {
        graphemes.next();
    }

    let sign = graphemes
        .next_if(|c| *c == "+" || *c == "-")
        .map(|c| if c == "-" { -1 } else { 1 });

    let sign_mul = sign.unwrap_or(1);

    let digits = graphemes
        .peeking_take_while(|s| {
            s.len() == 1 && s.chars().next().unwrap().is_ascii_digit()
        })
        .map(|s| {
            isize::try_from(
                s.chars()
                    .next()
                    .and_then(|c| c.to_digit(10))
                    .expect("ascii 0-9"),
            )
            .expect("0-9 always fit isize")
        })
        .try_fold(None, |acc: Option<isize>, d| {
            let v = acc.map_or(Some(sign_mul * d), |a| {
                a.checked_mul(10).and_then(|n| n.checked_add(sign_mul * d))
            });
            v.and(Some(v))
        });

    Ok(digits.ok_or(Error::InvalidOffset)?.or(sign))
}

#[cfg(test)]
mod tests {
    use super::*;

    use similar_asserts::assert_eq;
    use unicode_segmentation::UnicodeSegmentation;

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
            EditBuffer::with_lines(&["1\n", "2\n", "3\n"]);
        let buf_non_terminated = EditBuffer::with_lines(&["1", "2", "3"]);
        let buf_partially_terminated =
            EditBuffer::with_lines(&["1\n", "2", "3"]);
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
        let mut buffer = EditBuffer::with_lines(&["1\n", "2", "3"]);
        buffer.set_current_index(2);
        assert_eq!(2, buffer.current_index());
    }

    #[test]
    #[should_panic = "index (is 99) must be within buffer (is 0..3)"]
    fn set_current_index_beyond_end() {
        let mut buffer = EditBuffer::with_lines(&["1\r\n", "2", "3"]);
        buffer.set_current_index(99);
    }

    #[test]
    fn insert_one_to_empty_buffer() {
        let mut buffer = EditBuffer::new();
        let expected = EditBuffer::with_lines(&["one\n"]);
        let lines = ["one\n"].map(ToOwned::to_owned).to_vec();
        let changes = buffer.insert(0, lines);
        assert_eq!(buffer.current_index, 0);
        assert_eq!(1, buffer.len());
        assert_eq!(&expected[..], &buffer[..]);
        assert!(!changes.is_empty());
    }

    #[test]
    fn insert_of_zero_lines() {
        let mut buffer = EditBuffer::with_lines(&["1\n", "2", "3"]);
        let expected = EditBuffer::with_lines(&["1\n", "2", "3"]);
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
            EditBuffer::with_lines(&["1\r\n", "2", "3", "4", "5", "6"]);
        let expected = EditBuffer::with_lines(&["1\r\n", "2", "6"]);
        let changes = buffer.remove(2..5);
        assert!(!changes.is_empty());
        assert_eq!(buffer[..], expected[..]);
        assert_eq!(buffer.current_index(), 2);
    }

    #[test]
    fn remove_span_at_start() {
        let mut buffer =
            EditBuffer::with_lines(&["1\r\n", "2", "3", "4", "5", "6"]);
        let expected = EditBuffer::with_lines(&["4\r\n", "5", "6"]);
        buffer.remove(0..3);
        assert_eq!(buffer[..], expected[..]);
        assert_eq!(buffer.current_index(), 0);
    }

    #[test]
    fn remove_span_at_end() {
        let mut buffer =
            EditBuffer::with_lines(&["1\r\n", "2", "3", "4", "5", "6"]);
        let expected = EditBuffer::with_lines(&["1\r\n", "2", "3", "4"]);
        buffer.remove(4..6);
        assert_eq!(buffer[..], expected[..]);
        assert_eq!(buffer.current_index(), 3);
    }

    #[test]
    fn relocate_span() {
        let mut buffer =
            EditBuffer::with_lines(&["1\n", "2", "3", "4", "5", "6"]);
        let orig = buffer.clone();
        let mut expected =
            EditBuffer::with_lines(&["1\n", "2", "3", "5", "6", "4"]);
        expected.current_index = 5;
        let changes = buffer.relocate(4..6, 3);
        buffer.push_undo(changes);
        assert_eq!(buffer[..], expected[..]);
        assert_eq!(buffer.current_index(), expected.current_index);

        buffer.undo().expect("something on undo stack");
        assert_eq!(buffer[..], orig[..]);
        assert_eq!(buffer.current_index, orig.current_index);

        buffer.redo().expect("something on redo stack");
        assert_eq!(buffer[..], expected[..]);
        assert_eq!(buffer.current_index, expected.current_index);
    }

    #[test]
    fn relocate_to_line_0() {
        let mut buffer =
            EditBuffer::with_lines(&["1\n", "2", "3", "4", "5", "6"]);
        let orig = buffer.clone();
        let mut expected =
            EditBuffer::with_lines(&["4\n", "5", "1", "2", "3", "6"]);
        expected.set_current_index(2);
        let changes = buffer.relocate(3..5, 0);
        buffer.push_undo(changes);
        assert_eq!(buffer[..], expected[..]);
        assert_eq!(buffer.current_index(), expected.current_index());

        buffer.undo().expect("something on undo stack");
        assert_eq!(buffer[..], orig[..]);
        assert_eq!(buffer.current_index(), orig.current_index());

        buffer.redo().expect("something on redo stack");
        assert_eq!(buffer[..], expected[..]);
        assert_eq!(buffer.current_index(), expected.current_index());
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
            EditBuffer::with_lines(&["1\n", "2", "3", "4", "5", "6"]);
        let expected = buffer.clone();
        let changes = buffer.remove(0..4);
        buffer.push_undo(changes);
        assert_eq!(buffer[..], EditBuffer::with_lines(&["5\n", "6"])[..]);
        buffer.undo().unwrap();
        assert_eq!(buffer[..], expected[..]);
    }
    #[test]
    fn undo_redo_insert() {
        let mut buffer =
            EditBuffer::with_lines(&["1\n", "2", "3", "4", "5", "6"]);
        let expected_final =
            EditBuffer::with_lines(&["1\n", "2", "3", "4", "5", "6"]);
        let expected_modified = EditBuffer::with_lines(&[
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
            EditBuffer::with_lines(&["1\n", "2", "3", "4", "5", "6"]);
        let lines = ["a\n", "b\n", "c\n"].map(ToOwned::to_owned).to_vec();
        let expected_final = buffer.clone();
        assert_eq!(buffer.current_index(), 5);

        let changes = buffer.insert(2, lines);
        buffer.push_undo(changes);
        let expected_1 = EditBuffer::with_lines(&[
            "1\n", "2", "a", "b", "c", "3", "4", "5", "6",
        ]);
        assert_eq!(buffer[..], expected_1[..]);
        assert_eq!(buffer.current_index(), 4);

        let changes = buffer.remove(3..7);
        buffer.push_undo(changes);
        let expected_2 = EditBuffer::with_lines(&["1\n", "2", "a", "5", "6"]);
        assert_eq!(buffer[..], expected_2[..]);

        buffer.undo().unwrap();
        assert_eq!(buffer[..], expected_1[..]);

        buffer.undo().unwrap();
        assert_eq!(buffer[..], expected_final[..]);
    }

    #[test]
    fn undo_redo_multi() {
        let mut buffer =
            EditBuffer::with_lines(&["1\n", "2", "3", "4", "5", "6"]);
        let lines = ["a\n", "b\n", "c\n"].map(ToOwned::to_owned).to_vec();
        let expected_final = buffer.clone();
        assert_eq!(5, buffer.current_index());

        let changes = buffer.insert(2, lines);
        buffer.push_undo(changes);
        let expected_1 = EditBuffer::with_lines(&[
            "1\n", "2", "a", "b", "c", "3", "4", "5", "6",
        ]);
        assert_eq!(&expected_1[..], &buffer[..]);
        assert_eq!(4, buffer.current_index());

        let changes = buffer.remove(3..7);
        buffer.push_undo(changes);
        let expected_2 = EditBuffer::with_lines(&["1\n", "2", "a", "5", "6"]);
        assert_eq!(buffer[..], expected_2[..]);

        buffer.undo().unwrap();
        assert_eq!(buffer[..], expected_1[..]);

        let lines = vec!["spam!\n".to_owned()];
        let changes = buffer.insert(5, lines);
        buffer.push_undo(changes);
        let expected_3 = EditBuffer::with_lines(&[
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
            EditBuffer::with_lines(&["1\n", "2", "3", "4", "5", "6"]);
        let buffer_orig = buffer.clone();
        assert_eq!(buffer.current_index(), 5);

        let lines = ["a\n", "b\n", "c\n"].map(ToOwned::to_owned).to_vec();
        let changes = buffer.insert(2, lines);
        buffer.push_undo(changes);
        let expected_1 = EditBuffer::with_lines(&[
            "1\n", "2", "a", "b", "c", "3", "4", "5", "6",
        ]);
        assert_eq!(&expected_1[..], &buffer[..]);
        assert_eq!(buffer.current_index(), 4);

        let changes = buffer.remove(3..7);
        buffer.push_undo(changes);
        let expected_final =
            EditBuffer::with_lines(&["1\n", "2", "a", "5", "6"]);
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

    #[test]
    fn eval_positive_offset() {
        let mut input = "3p".graphemes(true).peekable();
        let res = compute_line_offset(&mut input).unwrap();
        assert_eq!(res, 3);
        assert!(matches!(input.next(), Some("p")));
        let mut input = "+42p".graphemes(true).peekable();
        let res = compute_line_offset(&mut input).unwrap();
        assert_eq!(res, 42);
        assert!(matches!(input.next(), Some("p")));
    }

    #[test]
    fn eval_negative_offsets() {
        let mut input = "-2p".graphemes(true).peekable();
        let res = compute_line_offset(&mut input).unwrap();
        assert_eq!(res, -2);
        assert!(matches!(input.next(), Some("p")));
    }

    #[test]
    fn eval_mixed_offsets() {
        let mut input = "2-7+6p".graphemes(true).peekable();
        let res = compute_line_offset(&mut input).unwrap();
        assert_eq!(res, 1);
        assert!(matches!(input.next(), Some("p")));
    }

    #[test]
    fn eval_offset_overflow() {
        let mut input =
            "8399999999999999999+839999999999999999+8399999999999999999p"
                .graphemes(true)
                .peekable();
        let res = compute_line_offset(&mut input).expect_err("shouldn't parse");
        assert!(matches!(res, Error::InvalidOffset));

        let mut input =
            "-839999999999999999-83999999999999999-8399999999999999999p"
                .graphemes(true)
                .peekable();
        let res = compute_line_offset(&mut input).expect_err("shouldn't parse");
        assert!(matches!(res, Error::InvalidOffset));
    }

    #[test]
    fn eval_offset_too_large() {
        let mut input = "999999999999999999999p".graphemes(true).peekable();
        let res = compute_line_offset(&mut input).expect_err("shouldn't parse");
        assert!(matches!(res, Error::InvalidOffset));
        let mut input = "+999999999999999999999p".graphemes(true).peekable();
        let res = compute_line_offset(&mut input).expect_err("shouldn't parse");
        assert!(matches!(res, Error::InvalidOffset));
    }

    #[test]
    fn eval_offset_too_small() {
        let mut input = "-999999999999999999999p".graphemes(true).peekable();
        let res = compute_line_offset(&mut input).expect_err("shouldn't parse");
        assert!(matches!(res, Error::InvalidOffset));
    }

    #[test]
    fn eval_mixed_offsets_with_spaces() {
        let mut input = "   2 -7  6 +1p".graphemes(true).peekable();
        let res = compute_line_offset(&mut input).unwrap();
        assert_eq!(res, 2);
        assert!(matches!(input.next(), Some("p")));
    }

    #[test]
    fn eval_addr_no_eol() {
        let mut cmd_line = "".graphemes(true).peekable();
        let address =
            Address::eval(&mut cmd_line, &mut EditBuffer::new(), &mut None)
                .unwrap();
        assert!(address.is_none());
    }

    #[test]
    fn eval_no_addr() {
        let mut cmd_line = "q\n".graphemes(true).peekable();
        let address =
            Address::eval(&mut cmd_line, &mut EditBuffer::new(), &mut None)
                .unwrap();
        assert!(address.is_none());
        assert_eq!(cmd_line.next(), Some("q"));
    }

    #[test]
    fn eval_dot_addr() {
        let mut cmd_line = ".d\r\n".graphemes(true).peekable();
        let mut buffer = EditBuffer::with_lines(&["1\r\n", "2", "3"]);
        buffer.set_current_index(1);
        let address =
            Address::eval(&mut cmd_line, &mut buffer, &mut None).unwrap();
        assert_eq!(address, Some(Address { first: 2, last: 2 }));
        assert_eq!(cmd_line.next(), Some("d"));
    }

    #[test]
    fn eval_dollar_addr() {
        let mut cmd_line = "$d\r\n".graphemes(true).peekable();
        let mut buffer = EditBuffer::with_lines(&["1\r\n", "2", "3"]);
        buffer.set_current_index(2);
        let address =
            Address::eval(&mut cmd_line, &mut buffer, &mut None).unwrap();
        assert_eq!(address, Some(Address { first: 3, last: 3 }));
        assert_eq!(cmd_line.next(), Some("d"));
    }

    #[test]
    fn eval_simple_number_addr() {
        let mut buffer =
            EditBuffer::with_lines(&["1\n", "2", "3", "4", "5", "6"]);
        let mut cmd_line = "5d\n".graphemes(true).peekable();
        let address =
            Address::eval(&mut cmd_line, &mut buffer, &mut None).unwrap();
        assert_eq!(cmd_line.next(), Some("d"));
        assert_eq!(address, Some(Address { first: 5, last: 5 }));
    }

    #[test]
    fn regex_line_addr_regex_syntax() {
        let mut input = "/\\lo.+/n\n".graphemes(true).peekable();
        let mut buffer = EditBuffer::with_lines(&[
            "one\r\n", "two", "three", "four", "five", "six",
        ]);
        buffer.set_current_index(2);
        let mut previous_pattern: Option<Regex> = None;
        let res = Address::eval(&mut input, &mut buffer, &mut previous_pattern)
            .expect_err("bad pattern");
        assert!(matches!(res, Error::Regex { .. }));
    }

    #[test]
    fn rev_regex_line_addr_regex_syntax() {
        let mut input = "?\\lo.+?n\n".graphemes(true).peekable();
        let mut buffer = EditBuffer::with_lines(&[
            "one\r\n", "two", "three", "four", "five", "six",
        ]);
        buffer.set_current_index(2);
        let mut previous_pattern: Option<Regex> = None;
        let res = Address::eval(&mut input, &mut buffer, &mut previous_pattern)
            .expect_err("bad pattern");
        assert!(matches!(res, Error::Regex { .. }));
    }

    #[test]
    fn regex_line_addr_embedded_delim() {
        let mut input = "/o.+\\//n\n".graphemes(true).peekable();
        let mut buffer = EditBuffer::with_lines(&[
            "one/\n", "two", "three", "four", "five", "six",
        ]);
        buffer.set_current_index(2);
        let mut previous_pattern: Option<Regex> = None;
        let res = Address::eval(&mut input, &mut buffer, &mut previous_pattern)
            .unwrap();
        assert_eq!(res, Some(Address { first: 1, last: 1 }));
    }

    #[test]
    fn regex_line_addr_no_final_delimiter() {
        let mut input = "/o.+\n".graphemes(true).peekable();
        let mut buffer = EditBuffer::with_lines(&[
            "one\r\n", "two", "three", "four", "five", "six",
        ]);
        buffer.set_current_index(2);
        let mut previous_pattern: Option<Regex> = None;
        let res = Address::eval(&mut input, &mut buffer, &mut previous_pattern)
            .unwrap();
        assert_eq!(res, Some(Address { first: 4, last: 4 }));
    }

    #[test]
    fn regex_line_addr_needle_in_first_half_of_split_range() {
        let mut input = "/o.+/n\n".graphemes(true).peekable();
        let mut previous_pattern: Option<Regex> = None;
        let mut buffer = EditBuffer::with_lines(&[
            "one\r\n", "two", "three", "four", "five", "six",
        ]);
        buffer.set_current_index(2);
        let res = Address::eval(&mut input, &mut buffer, &mut previous_pattern)
            .unwrap();
        assert_eq!(res, Some(Address { first: 4, last: 4 }));
    }

    #[test]
    fn regex_line_addr_needle_in_second_half_of_split_range() {
        let mut input = "/on.+/n\n".graphemes(true).peekable();
        let mut previous_pattern: Option<Regex> = None;
        let mut buffer = EditBuffer::with_lines(&[
            "one\r\n", "two", "three", "four", "five", "six",
        ]);
        buffer.set_current_index(4);
        let res = Address::eval(&mut input, &mut buffer, &mut previous_pattern)
            .unwrap();
        assert_eq!(res, Some(Address { first: 1, last: 1 }));
    }

    #[test]
    fn regex_line_addr_contiguous_search_range() {
        let mut input = "/o.+/n\n".graphemes(true).peekable();
        let mut previous_pattern: Option<Regex> = None;
        let mut buffer = EditBuffer::with_lines(&[
            "one\r\n", "two", "three", "four", "five", "six",
        ]);
        buffer.set_current_index(5);
        let res = Address::eval(&mut input, &mut buffer, &mut previous_pattern)
            .unwrap();
        assert_eq!(res, Some(Address { first: 1, last: 1 }));
    }

    #[test]
    fn rev_regex_line_addr_needle_in_first_half_of_split_range() {
        let mut input = "?o.+?n\n".graphemes(true).peekable();
        let mut previous_pattern: Option<Regex> = None;
        let mut buffer = EditBuffer::with_lines(&[
            "one\r\n", "two", "three", "four", "five", "six",
        ]);
        buffer.set_current_index(2);
        let res = Address::eval(&mut input, &mut buffer, &mut previous_pattern)
            .unwrap();
        assert_eq!(res, Some(Address { first: 1, last: 1 }));
    }

    #[test]
    fn rev_regex_line_addr_needle_in_second_half_of_split_range() {
        let mut input = "?ou.+?n\n".graphemes(true).peekable();
        let mut previous_pattern: Option<Regex> = None;
        let mut buffer = EditBuffer::with_lines(&[
            "one\r\n", "two", "three", "four", "five", "six",
        ]);
        buffer.set_current_index(4);
        let res = Address::eval(&mut input, &mut buffer, &mut previous_pattern)
            .unwrap();
        assert_eq!(res, Some(Address { first: 4, last: 4 }));
    }

    #[test]
    fn rev_regex_line_addr_contiguous_search_range() {
        let mut input = "?o.+?n\n".graphemes(true).peekable();
        let mut previous_pattern: Option<Regex> = None;
        let mut buffer = EditBuffer::with_lines(&[
            "one\r\n", "two", "three", "four", "five", "six",
        ]);
        buffer.set_current_index(0);
        let res = Address::eval(&mut input, &mut buffer, &mut previous_pattern)
            .unwrap();
        assert_eq!(res, Some(Address { first: 4, last: 4 }));
    }

    #[test]
    fn regex_line_addr_with_offset() {
        let mut input = "/o.+/+2\n".graphemes(true).peekable();
        let mut buffer = EditBuffer::with_lines(&[
            "one\r\n", "two", "three", "four", "five", "six",
        ]);
        buffer.set_current_index(2);
        let mut previous_pattern: Option<Regex> = None;
        let res = Address::eval(&mut input, &mut buffer, &mut previous_pattern)
            .unwrap();
        assert_eq!(res, Some(Address { first: 6, last: 6 }));
    }

    #[test]
    fn rev_regex_line_addr_with_offset() {
        let mut input = "?o.+?+2\n".graphemes(true).peekable();
        let mut buffer = EditBuffer::with_lines(&[
            "one\r\n", "two", "three", "four", "five", "six",
        ]);
        buffer.set_current_index(2);
        let mut previous_pattern: Option<Regex> = None;
        let res = Address::eval(&mut input, &mut buffer, &mut previous_pattern)
            .unwrap();
        assert_eq!(res, Some(Address { first: 3, last: 3 }));
    }

    #[test]
    fn eval_simple_comma_addr() {
        let mut buffer =
            EditBuffer::with_lines(&["1\r\n", "2", "3", "4", "5", "6"]);
        let mut input = "1,2p\n".graphemes(true).peekable();
        let res = Address::eval(&mut input, &mut buffer, &mut None).unwrap();
        assert_eq!(res, Some(Address { first: 1, last: 2 }));
        assert_eq!(input.next(), Some("p"));
    }

    #[test]
    fn eval_leading_comma_addr() {
        let mut buffer =
            EditBuffer::with_lines(&["1\r\n", "2", "3", "4", "5", "6"]);
        let mut input = ",4p\r\n".graphemes(true).peekable();
        let res = Address::eval(&mut input, &mut buffer, &mut None).unwrap();
        assert_eq!(input.next(), Some("p"));
        assert_eq!(res, Some(Address { first: 1, last: 4 }));
    }

    #[test]
    fn eval_trailing_comma_addr() {
        let mut input = "5,p\n".graphemes(true).peekable();
        let mut buffer =
            EditBuffer::with_lines(&["1\n", "2", "3", "4", "5", "6"]);
        let res = Address::eval(&mut input, &mut buffer, &mut None).unwrap();
        assert_eq!(input.next(), Some("p"));
        assert_eq!(res, Some(Address { first: 5, last: 5 }));
    }

    #[test]
    fn eval_comma_only_addr() {
        let mut input = ",p\r\n".graphemes(true).peekable();
        let mut buffer =
            EditBuffer::with_lines(&["1\n", "2", "3", "4", "5", "6"]);
        let res = Address::eval(&mut input, &mut buffer, &mut None).unwrap();
        assert_eq!(input.next(), Some("p"));
        assert_eq!(res, Some(Address { first: 1, last: 6 }));
    }

    #[test]
    fn eval_comma_only_chain_addr() {
        let mut input = ",,p\n".graphemes(true).peekable();
        let mut buffer =
            EditBuffer::with_lines(&["1\n", "2", "3", "4", "5", "6"]);
        let res = Address::eval(&mut input, &mut buffer, &mut None).unwrap();
        assert_eq!(input.next(), Some("p"));
        assert_eq!(res, Some(Address { first: 6, last: 6 }));
    }

    #[test]
    fn eval_comma_chain_addr() {
        let mut input = ",12, 3+1,p\n".graphemes(true).peekable();
        let mut buffer =
            EditBuffer::with_lines(&["1\n", "2", "3", "4", "5", "6"]);
        let res = Address::eval(&mut input, &mut buffer, &mut None).unwrap();
        assert_eq!(input.next(), Some("p"));
        assert_eq!(res, Some(Address { first: 4, last: 4 }));
    }

    #[test]
    fn eval_semicolon_addr_past_end() {
        let mut input = "+;np\n".graphemes(true).peekable();
        let mut buffer =
            EditBuffer::with_lines(&["1\r\n", "2", "3", "4", "5", "6"]);
        assert_eq!(buffer.current_index(), 5);
        let res = Address::eval(&mut input, &mut buffer, &mut None)
            .expect_err("invalid address");
        assert!(matches!(res, Error::InvalidAddress));
    }

    #[test]
    fn eval_simple_semicolon_addr() {
        let mut input = "1;2p\n".graphemes(true).peekable();
        let mut buffer =
            EditBuffer::with_lines(&["1\n", "2", "3", "4", "5", "6"]);
        assert_eq!(buffer.current_index(), 5);
        let res = Address::eval(&mut input, &mut buffer, &mut None).unwrap();
        assert_eq!(res, Some(Address { first: 1, last: 2 }));
        assert_eq!(buffer.current_index(), 0);
        assert_eq!(input.next(), Some("p"));
    }

    #[test]
    fn eval_leading_semicolon_addr() {
        let mut input = ";5p\n".graphemes(true).peekable();
        let mut buffer =
            EditBuffer::with_lines(&["1\n", "2", "3", "4", "5", "6"]);
        buffer.set_current_index(2);
        let res = Address::eval(&mut input, &mut buffer, &mut None).unwrap();
        assert_eq!(input.next(), Some("p"));
        assert_eq!(res, Some(Address { first: 3, last: 5 }));
        assert_eq!(buffer.current_index(), 2);
    }

    #[test]
    fn eval_trailing_semicolon_addr() {
        let mut input = "5;p\n".graphemes(true).peekable();
        let mut buffer =
            EditBuffer::with_lines(&["1\n", "2", "3", "4", "5", "6"]);
        buffer.set_current_index(3);
        let res = Address::eval(&mut input, &mut buffer, &mut None).unwrap();
        assert_eq!(input.next(), Some("p"));
        assert_eq!(res, Some(Address { first: 5, last: 5 }));
        assert_eq!(buffer.current_index(), 4);
    }

    #[test]
    fn eval_semicolon_only_addr() {
        let mut input = ";p\n".graphemes(true).peekable();
        let mut buffer =
            EditBuffer::with_lines(&["1\n", "2", "3", "4", "5", "6"]);
        buffer.set_current_index(2);
        let res = Address::eval(&mut input, &mut buffer, &mut None).unwrap();
        assert_eq!(input.next(), Some("p"));
        assert_eq!(res, Some(Address { first: 3, last: 6 }));
        assert_eq!(buffer.current_index(), 2);
    }

    #[test]
    fn eval_semicolon_only_chain_addr() {
        let mut input = ";;p\n".graphemes(true).peekable();
        let mut buffer =
            EditBuffer::with_lines(&["1\n", "2", "3", "4", "5", "6"]);
        buffer.set_current_index(2);
        let res = Address::eval(&mut input, &mut buffer, &mut None).unwrap();
        assert_eq!(input.next(), Some("p"));
        assert_eq!(res, Some(Address { first: 6, last: 6 }));
    }

    #[test]
    fn eval_big_before_small_semicolon_chain_addr() {
        let mut input = "4;$;2p\n".graphemes(true).peekable();
        let mut buffer =
            EditBuffer::with_lines(&["1\n", "2", "3", "4", "5", "6"]);
        buffer.set_current_index(3);
        let res = Address::eval(&mut input, &mut buffer, &mut None)
            .expect_err("invalid address");
        assert!(matches!(res, Error::InvalidAddress));
    }

    #[test]
    fn eval_simple_offset_only_addrs() {
        let mut input = "+p\n".graphemes(true).peekable();
        let mut buffer =
            EditBuffer::with_lines(&["1\n", "2", "3", "4", "5", "6"]);
        buffer.set_current_index(2);
        let res = Address::eval(&mut input, &mut buffer, &mut None).unwrap();
        assert_eq!(input.next(), Some("p"));
        assert_eq!(res, Some(Address { first: 4, last: 4 }));

        let mut input = "+10p\n".graphemes(true).peekable();
        let mut buffer =
            EditBuffer::with_lines(&["1\n", "2", "3", "4", "5", "6"]);
        buffer.set_current_index(3);
        let res = Address::eval(&mut input, &mut buffer, &mut None)
            .expect_err("InvalidAddress");
        assert_eq!(input.next(), Some("p"));
        assert!(matches!(res, Error::InvalidAddress));

        let mut input = "-p\n".graphemes(true).peekable();
        let mut buffer =
            EditBuffer::with_lines(&["1\n", "2", "3", "4", "5", "6"]);
        buffer.set_current_index(2);
        let res = Address::eval(&mut input, &mut buffer, &mut None).unwrap();
        assert_eq!(input.next(), Some("p"));
        assert_eq!(res, Some(Address { first: 2, last: 2 }));

        let mut input = "-2p\n".graphemes(true).peekable();
        let mut buffer =
            EditBuffer::with_lines(&["1\n", "2", "3", "4", "5", "6"]);
        buffer.set_current_index(2);
        let res = Address::eval(&mut input, &mut buffer, &mut None).unwrap();
        assert_eq!(input.next(), Some("p"));
        assert_eq!(res, Some(Address { first: 1, last: 1 }));
    }

    #[test]
    fn eval_too_big_offset_only_addr_overflows() {
        let mut input = "-10p\n".graphemes(true).peekable();
        let mut buffer =
            EditBuffer::with_lines(&["1\n", "2", "3", "4", "5", "6"]);
        buffer.set_current_index(3);
        let res = Address::eval(&mut input, &mut buffer, &mut None)
            .expect_err("offset overflow");
        assert!(matches!(res, Error::InvalidOffset));
    }
}
