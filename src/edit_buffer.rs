// EditBuffer keeps track of everything specific to a single buffer in the
// editor. All public interface uses one based indexing, and any such function
// is responsible for translating into the 0 based indexing of the Vec<String>
// containing the lines of text.

use std::cmp::{self, Ordering};
use std::fmt::{self, Display, Formatter};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::iter::Peekable;
use std::ops::{
    Index, Range, RangeFrom, RangeFull, RangeInclusive, RangeTo,
    RangeToInclusive,
};
use std::str::FromStr;

use regex::Regex;
use unicode_segmentation::Graphemes;

use crate::command;
use crate::undo_stack::{Change, ChangeSet, UndoStack};
use crate::eol::Eol;
use crate::error::{Error, ParsePrevailingEolError};
use crate::iter_utils::Peeking;

#[derive(Debug, Default, Clone)]
pub struct EditBuffer {
    current_line: usize,
    prevailing_eol: Option<PrevailingEol>,
    undo_stack: UndoStack,
    content_hash: Option<u64>,
    text: Vec<String>,
}

impl From<Vec<String>> for EditBuffer {
    fn from(lines: Vec<String>) -> Self {
        let line_count = lines.len();
        let mut buf = EditBuffer::with_capacity(line_count);
        buf.append(0, lines);
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
            current_line: 0,
            prevailing_eol: None,
            undo_stack: UndoStack::new(),
            content_hash: None,
            text: Vec::new(),
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
            current_line: 0,
            prevailing_eol: None,
            undo_stack: UndoStack::new(),
            content_hash: None,
            text: Vec::with_capacity(capacity),
        }
    }

    #[cfg(test)]
    pub fn with_text(text: &[&str]) -> EditBuffer {
        let line_count = text.len();
        let text: Vec<_> = text.iter().map(ToString::to_string).collect();
        let mut buf = EditBuffer::with_capacity(line_count);
        buf.prevailing_eol = PrevailingEol::compute_prevailing_eol(&text);
        buf.append(0, text);
        buf
    }

    #[must_use]
    /// Returns this `EditBuffer`'s length, in lines.
    pub fn len(&self) -> usize {
        self.text.len()
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    pub fn content_hash(&mut self) -> u64 {
        *self.content_hash.get_or_insert_with(|| {
            let mut h = DefaultHasher::new();
            self.text.hash(&mut h);
            h.finish()
        })
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

    pub fn push_undo(&mut self, changes: ChangeSet) {
        self.undo_stack.push_undo(
            changes,
            self.current_line,
            self.prevailing_eol,
        );
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

    pub fn try_line(&self, n: usize) -> Result<Address, Error> {
        if n > self.len() {
            return Err(Error::InvalidAddress);
        }
        Ok(Address { first: n, last: n })
    }

    pub fn try_span(
        &self,
        first: usize,
        last: usize,
    ) -> Result<Address, Error> {
        if first > last || last > self.len() {
            return Err(Error::InvalidAddress);
        }
        Ok(Address { first, last })
    }

    pub fn do_append(
        &mut self,
        address: Option<Address>,
        lines: Vec<String>,
    ) -> Option<ChangeSet> {
        let mut changes =
            ChangeSet::new(self.current_line, self.prevailing_eol);

        let location = address.map_or(self.current_line, |addr| addr.last());
        if lines.is_empty() {
            self.current_line = location;
            return None;
        }

        self.append(location, lines.clone());
        changes.push(Change::Add(location, lines));
        Some(changes)
    }

    pub fn append(&mut self, location: usize, mut lines: Vec<String>) -> bool {
        let Some(new_eol) = PrevailingEol::compute_prevailing_eol(&lines)
        else {
            // Nothing to do
            return false;
        };

        let prevailing_eol = self.prevailing_eol.get_or_insert(new_eol);
        if new_eol.mixed || (new_eol.eol != prevailing_eol.eol) {
            prevailing_eol.mixed = true;
        }

        // Normalize EOLs of lines to add
        let mut eol_added = false;
        for l in &mut lines {
            let line_eol = Eol::get_eol(&mut *l);
            if let Some(line_eol) = line_eol {
                if line_eol != prevailing_eol.eol {
                    // Wrong EOL -- replace with prevailing
                    l.truncate(l.len() - line_eol.as_str().len());
                    l.push_str(prevailing_eol.eol.as_str());
                }
            } else {
                l.push_str(prevailing_eol.eol.as_str());
                eol_added = true;
            }
        }

        self.current_line = location + lines.len();
        self.text.splice(location..location, lines);
        self.content_hash = None;
        eol_added
    }

    pub fn do_change(
        &mut self,
        address: Option<Address>,
        lines: Vec<String>,
    ) -> ChangeSet {
        let mut changes =
            ChangeSet::new(self.current_line, self.prevailing_eol);

        // handle deletion of addressed lines
        let b =
            cmp::max(1, address.map_or(self.current_line, |addr| addr.first()));
        let e = address.map_or(self.current_line, |addr| addr.last());
        if b <= e {
            let removed = self.text.splice(b - 1..e, None).collect();
            changes.push(Change::Remove(b - 1, removed));
        }

        // handle insertion of new lines, if any
        if lines.is_empty() {
            // remove only
            self.current_line = usize::min(self.text.len(), b);
            self.content_hash = None;
        } else {
            let b = b.saturating_sub(1);
            self.append(b, lines.clone());
            changes.push(Change::Add(b, lines));
        }

        changes
    }

    pub fn do_delete(&mut self, address: Option<Address>) -> ChangeSet {
        let (b, e) = address
            .map_or((self.current_line, self.current_line), |addr| {
                (addr.first(), addr.last())
            });

        let removed: Vec<String> = self.text.splice(b - 1..e, None).collect();

        let mut changes =
            ChangeSet::new(self.current_line, self.prevailing_eol);
        self.current_line = usize::min(self.text.len(), b);
        self.content_hash = None;
        changes.push(Change::Remove(b - 1, removed));
        changes
    }

    pub fn do_insert(
        &mut self,
        address: Option<Address>,
        lines: Vec<String>,
    ) -> Option<ChangeSet> {
        let location = if lines.is_empty() {
            address.map_or(self.current_line, |addr| addr.last())
        } else {
            // insertion point is just before addressed line
            address
                .map_or(self.current_line, |addr| addr.last())
                .saturating_sub(1)
        };
        let mut changes =
            ChangeSet::new(self.current_line, self.prevailing_eol);
        if lines.is_empty() {
            self.current_line = location;
            return None;
        }

        self.append(location, lines.clone());
        changes.push(Change::Add(location, lines));
        Some(changes)
    }

    pub fn do_join(
        &mut self,
        address: Option<Address>,
        separator: Option<&str>,
    ) -> ChangeSet {
        let address = address.map_or_else(
            || self.try_span(self.current_line, self.current_line + 1).unwrap(),
            |addr| {
                if addr.line_count() == 1 {
                    self.try_span(addr.last(), addr.last() + 1).unwrap()
                } else {
                    addr
                }
            },
        );
        let mut changes =
            ChangeSet::new(self.current_line, self.prevailing_eol);

        let mut joined =
            self[address.first()].lines().next().unwrap().to_owned();
        if let Some(separator) = separator {
            joined.push_str(separator);
            for l in &self[address.first() + 1..address.last()] {
                joined.push_str(l.trim_start().lines().next().unwrap());
                joined.push_str(separator);
            }
            joined.push_str(self[address.last()].trim_start());
        } else {
            joined.extend(
                self[address.first() + 1..address.last()]
                    .iter()
                    .map(|l| l.lines().next().unwrap()),
            );
            joined.push_str(&self[address.last()]);
        }

        let replaced: Vec<_> = self
            .text
            .splice(address.first() - 1..address.last(), vec![joined.clone()])
            .collect();
        self.current_line = address.first();
        self.content_hash = None;
        changes.push(Change::Add(address.first() - 1, vec![joined]));
        changes.push(Change::Remove(address.first(), replaced));
        changes
    }

    pub fn do_move(
        &mut self,
        address: Option<Address>,
        destination: Address,
    ) -> ChangeSet {
        let address =
            address.or_else(|| self.try_line(self.current_line).ok()).unwrap();
        let lines: Vec<String> =
            self.text.drain(address.first() - 1..address.last()).collect();
        let destination = if destination.last() >= address.last() {
            destination.last() - address.line_count()
        } else {
            destination.last()
        };

        let mut changes =
            ChangeSet::new(self.current_line, self.prevailing_eol);
        changes.push(Change::Remove(address.first() - 1, lines.clone()));
        changes.push(Change::Add(destination, lines.clone()));
        self.text.splice(destination..destination, lines);
        self.current_line = destination + address.line_count();
        self.content_hash = None;
        changes
    }

    pub fn do_undo(&mut self) -> Result<(), Error> {
        let Some(undo) = self.undo_stack.pop_undo() else {
            return Err(Error::NothingToUndo);
        };
        for change in undo.changes().rev() {
            match change {
                Change::Add(p, l) => {
                    drop(self.text.splice(*p..*p + l.len(), None));
                }
                Change::Remove(p, l) => {
                    drop(self.text.splice(*p..*p, l.iter().cloned()));
                }
                Change::SetEol(span, old_eol, new_eol) => {
                    for line in &mut self.text[span.clone()] {
                        line.replace_range(
                            line.len() - new_eol.as_str().len()..,
                            old_eol.as_str(),
                        );
                    }
                }
            }
        }
        self.current_line = undo.current_line_before;
        self.content_hash = None;
        self.prevailing_eol = undo.prevailing_eol_before;
        self.undo_stack.push_redo(undo);
        Ok(())
    }

    pub fn do_redo(&mut self) -> Result<(), Error> {
        let Some(redo) = self.undo_stack.pop_redo() else {
            return Err(Error::NothingToRedo);
        };
        for change in redo.changes() {
            match change {
                Change::Add(p, l) => {
                    self.text.splice(*p..*p, l.iter().cloned());
                }
                Change::Remove(p, l) => {
                    self.text.splice(*p..*p + l.len(), None);
                }
                Change::SetEol(span, old_eol, new_eol) => {
                    for line in &mut self.text[span.clone()] {
                        line.replace_range(
                            line.len() - old_eol.as_str().len()..,
                            new_eol.as_str(),
                        );
                    }
                }
            }
        }
        self.current_line = redo.current_line_after;
        self.content_hash = None;
        self.prevailing_eol = redo.prevailing_eol_after;
        self.undo_stack.push_undo(redo, self.current_line, self.prevailing_eol);
        Ok(())
    }

    pub fn do_transfer(
        &mut self,
        address: Option<Address>,
        destination: Address,
    ) -> ChangeSet {
        let address =
            address.or_else(|| self.try_line(self.current_line).ok()).unwrap();
        let source = self.text[address.first() - 1..address.last()].to_vec();
        let destination = destination.last();

        let mut changes =
            ChangeSet::new(self.current_line, self.prevailing_eol);
        changes.push(Change::Add(destination, source.clone()));
        self.text.splice(destination..destination, source);
        self.current_line = destination + address.line_count();
        self.content_hash = None;
        changes
    }

    pub fn clear_text(&mut self) {
        self.text.clear();
        self.current_line = 0;
        self.content_hash = None;
        self.prevailing_eol = None;
    }

    #[must_use]
    pub fn prevailing_eol(&self) -> Option<PrevailingEol> {
        self.prevailing_eol
    }

    pub fn set_prevailing_eol(&mut self, eol: Eol) -> Option<ChangeSet> {
        if self.prevailing_eol.is_some_and(|v| v.eol == eol && !v.mixed) {
            // Same prevailing eol && not mixed, so nothing to do
            return None;
        }

        // Prepare change set for undo/redo
        let mut changes =
            ChangeSet::new(self.current_line, self.prevailing_eol);

        // Set new previaling eol & normalize buffer lines
        self.prevailing_eol = Some(PrevailingEol { eol, mixed: false });
        let mut corrections: Option<(Range<usize>, Eol)> = None;

        for (i, line) in self.text.iter_mut().enumerate() {
            let line_eol =
                Eol::get_eol(&mut *line).expect("all buffer lines terminated");
            if line_eol != eol {
                line.replace_range(
                    line.len() - line_eol.as_str().len()..,
                    eol.as_str(),
                );
                let corrections = corrections.get_or_insert((i..i, line_eol));
                corrections.0.end += 1;
            } else if let Some((span, line_eol)) = corrections.take() {
                changes.push(Change::SetEol(span, line_eol, eol));
            }
        }

        if let Some((span, line_eol)) = corrections {
            changes.push(Change::SetEol(span, line_eol, eol));
        }

        if !changes.is_empty() {
            self.content_hash = None;
        }
        Some(changes)
    }
}

impl PrevailingEol {
    #[must_use]
    pub fn lf(mixed: bool) -> PrevailingEol {
        PrevailingEol { eol: Eol::Lf, mixed }
    }

    #[must_use]
    pub fn crlf(mixed: bool) -> PrevailingEol {
        PrevailingEol { eol: Eol::Crlf, mixed }
    }

    #[must_use]
    pub fn display_str(self) -> &'static str {
        match self.eol {
            Eol::Lf if self.mixed => "LF/mixed",
            Eol::Lf => "LF",
            Eol::Crlf if self.mixed => "CRLF/mixed",
            Eol::Crlf => "CRLF",
        }
    }

    #[must_use]
    fn compute_prevailing_eol(lines: &Vec<String>) -> Option<PrevailingEol> {
        if lines.is_empty() {
            // lines empty, nothing to compute
            return None;
        }

        let mut crlf = 0;
        let mut lf = 0;

        for line in lines {
            if line.ends_with("\r\n") {
                crlf += 1;
            } else if line.ends_with('\n') {
                lf += 1;
            }
        }

        let mixed = crlf > 0 && lf > 0;
        let eol = match crlf.cmp(&lf) {
            Ordering::Greater => Eol::Crlf,
            Ordering::Less => Eol::Lf,
            Ordering::Equal => Eol::native(),
        };
        Some(PrevailingEol { eol, mixed })
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PrevailingEol {
    pub eol: Eol,
    pub mixed: bool,
}

impl Display for PrevailingEol {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.display_str())
    }
}

impl FromStr for PrevailingEol {
    type Err = ParsePrevailingEolError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.to_lowercase();
        if s == "crlf" {
            Ok(PrevailingEol::crlf(false))
        } else if s == "lf" {
            Ok(PrevailingEol::lf(false))
        } else {
            Err(ParsePrevailingEolError)
        }
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

    pub fn is_valid(&self, buffer: &EditBuffer) -> bool {
        0 < self.first && self.first <= self.last && self.last <= buffer.len()
    }

    pub fn is_valid_0_ok(&self, buffer: &EditBuffer) -> bool {
        self.first <= self.last && self.last <= buffer.len()
    }

    pub fn contains(&self, line: usize) -> bool {
        self.first <= line && line <= self.last
    }

    pub fn line_count(&self) -> usize {
        self.last - self.first + 1
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
                            buffer.set_current_line(r);
                            r
                        }
                        None => buffer.current_line(),
                    });
                    right = right.or_else(|| Some(buffer.len()));
                }
                Some(&"+" | &"-") => {
                    right = Some(eval_line_number(
                        graphemes,
                        buffer.current_line(),
                    )?);
                }
                Some(&".") => {
                    graphemes.next();
                    right = Some(eval_line_number(
                        graphemes,
                        buffer.current_line(),
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

        if let Some(right) = right {
            Ok(Some(buffer.try_span(left.unwrap_or(right), right)?))
        } else {
            Ok(None)
        }
    }
}

impl IntoIterator for Address {
    type Item = usize;
    type IntoIter = RangeInclusive<usize>;

    fn into_iter(self) -> Self::IntoIter {
        self.into()
    }
}

impl From<Address> for RangeInclusive<usize> {
    fn from(address: Address) -> Self {
        address.first()..=address.last()
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
        assert_eq!(buffer.text.capacity(), 0);
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

    /////
    // PrevailingEol::compute_prevailing_eol() tests

    #[test]
    fn prevailing_eol_when_all_crlf() {
        let lines =
            vec!["L1\r\n".to_owned(), "L2\r\n".to_owned(), "L3\r\n".to_owned()];
        let expected = Some(PrevailingEol { eol: Eol::Crlf, mixed: false });
        assert_eq!(PrevailingEol::compute_prevailing_eol(&lines), expected);
    }

    #[test]
    fn prevailing_eol_when_all_lf() {
        let lines =
            vec!["L1\n".to_owned(), "L2\n".to_owned(), "L3\n".to_owned()];
        let expected = Some(PrevailingEol { eol: Eol::Lf, mixed: false });
        assert_eq!(PrevailingEol::compute_prevailing_eol(&lines), expected);
    }

    #[test]
    fn prevailing_eol_when_most_crlf() {
        let lines =
            vec!["L1\r\n".to_owned(), "L2\n".to_owned(), "L3\r\n".to_owned()];
        let expected = Some(PrevailingEol { eol: Eol::Crlf, mixed: true });
        assert_eq!(PrevailingEol::compute_prevailing_eol(&lines), expected);
    }

    #[test]
    fn prevailing_eol_when_most_lf() {
        let lines =
            vec!["L1\n".to_owned(), "L2\n".to_owned(), "L3\r\n".to_owned()];
        let expected = Some(PrevailingEol { eol: Eol::Lf, mixed: true });
        assert_eq!(PrevailingEol::compute_prevailing_eol(&lines), expected);
    }

    #[test]
    fn prevailing_eol_when_equal_lf_crlf() {
        let lines = vec![
            "L1\n".to_owned(),
            "L2\r\n".to_owned(),
            "L3\r\n".to_owned(),
            "L4\n".to_owned(),
        ];
        let expected = Some(PrevailingEol { eol: Eol::native(), mixed: true });
        assert_eq!(PrevailingEol::compute_prevailing_eol(&lines), expected);
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
        assert_eq!(buffer[..], content);
    }

    #[test]
    fn range_index() {
        let buffer = EditBuffer::with_text(&["1\n", "2", "3", "4", "5", "6"]);
        assert_eq!(vec!["2\n", "3\n", "4\n"], buffer[2..5]);
        assert_eq!(
            buffer[1..7],
            vec!["1\n", "2\n", "3\n", "4\n", "5\n", "6\n"],
        );
    }

    #[test]
    fn range_inclusive_index() {
        let buffer = EditBuffer::with_text(&["1\n", "2", "3", "4", "5", "6"]);
        assert_eq!(buffer[2..=4], vec!["2\n", "3\n", "4\n"],);
        assert_eq!(
            buffer[1..=6],
            vec!["1\n", "2\n", "3\n", "4\n", "5\n", "6\n"],
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
        buffer.do_append(buffer.try_line(0).ok(), lines);
        assert_eq!(1, buffer.current_line);
        assert_eq!(1, buffer.len());
        assert_eq!(&expected[..], &buffer[..]);
    }

    #[test]
    fn do_append_of_zero_lines() {
        let mut buffer = EditBuffer::with_text(&["1\n", "2", "3"]);
        let expected = EditBuffer::with_text(&["1\n", "2", "3"]);
        assert_eq!(3, buffer.current_line());
        buffer.do_append(buffer.try_line(2).ok(), Vec::new());
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
        buffer
            .do_transfer(buffer.try_line(5).ok(), buffer.try_line(3).unwrap());
        assert_eq!(buffer[..], expected[..]);
        assert_eq!(buffer.current_line(), 4);
    }

    #[test]
    fn do_transfer_span() {
        let mut buffer =
            EditBuffer::with_text(&["1\n", "2", "3", "4", "5", "6"]);
        let expected =
            EditBuffer::with_text(&["1\n", "2", "3", "4", "5", "4", "5", "6"]);
        buffer.do_transfer(
            buffer.try_span(4, 5).ok(),
            buffer.try_line(3).unwrap(),
        );
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
        buffer.do_transfer(None, buffer.try_line(3).unwrap());
        assert_eq!(buffer[..], expected[..]);
        assert_eq!(buffer.current_line(), 4);
    }

    #[test]
    fn do_transfer_to_line_0() {
        let mut buffer =
            EditBuffer::with_text(&["1\n", "2", "3", "4", "5", "6"]);
        let expected =
            EditBuffer::with_text(&["4\n", "5", "1", "2", "3", "4", "5", "6"]);
        buffer.do_transfer(
            buffer.try_span(4, 5).ok(),
            buffer.try_line(0).unwrap(),
        );
        assert_eq!(buffer[..], expected[..]);
        assert_eq!(buffer.current_line(), 2);
    }

    #[test]
    fn do_transfer_destination_is_span() {
        let mut buffer =
            EditBuffer::with_text(&["1\n", "2", "3", "4", "5", "6"]);
        let expected =
            EditBuffer::with_text(&["1\n", "2", "3", "4", "5", "4", "5", "6"]);
        buffer.do_transfer(
            buffer.try_span(4, 5).ok(),
            buffer.try_span(1, 3).unwrap(),
        );
        assert_eq!(buffer[..], expected[..]);
        assert_eq!(buffer.current_line(), 5);
    }

    #[test]
    fn do_delete_span() {
        let mut buffer =
            EditBuffer::with_text(&["1\r\n", "2", "3", "4", "5", "6"]);
        let expected = EditBuffer::with_text(&["1\r\n", "2", "6"]);
        buffer.do_delete(buffer.try_span(3, 5).ok());
        assert_eq!(3, buffer.len());
        assert_eq!(buffer[..], expected[..]);
    }

    #[test]
    fn do_delete_line() {
        let mut buffer =
            EditBuffer::with_text(&["1\n", "2", "3", "4", "5", "6"]);
        let expected = EditBuffer::with_text(&["1\n", "2", "4", "5", "6"]);
        buffer.do_delete(buffer.try_line(3).ok());
        assert_eq!(5, buffer.len());
        assert_eq!(buffer[..], expected[..]);
    }

    #[test]
    fn do_delete_span_at_start() {
        let mut buffer =
            EditBuffer::with_text(&["1\r\n", "2", "3", "4", "5", "6"]);
        let expected = EditBuffer::with_text(&["4\r\n", "5", "6"]);
        buffer.do_delete(buffer.try_span(1, 3).ok());
        assert_eq!(3, buffer.len());
        assert_eq!(buffer[..], expected[..]);
    }

    #[test]
    fn do_delete_span_at_end() {
        let mut buffer =
            EditBuffer::with_text(&["1\r\n", "2", "3", "4", "5", "6"]);
        let expected = EditBuffer::with_text(&["1\r\n", "2", "3", "4"]);
        buffer.do_delete(buffer.try_span(5, 6).ok());
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
        buffer.do_insert(buffer.try_line(0).ok(), lines);
        assert_eq!(1, buffer.current_line);
        assert_eq!(1, buffer.len());
        assert_eq!(buffer[..], expected[..]);
    }

    #[test]
    fn do_insert_empty_buffer() {
        let mut buffer = EditBuffer::new();
        let expected = EditBuffer::with_text(&["a\n", "b", "c"]);
        let lines = ["a\n", "b\n", "c\n"].map(ToOwned::to_owned).to_vec();
        buffer.do_insert(buffer.try_line(0).ok(), lines);
        assert_eq!(3, buffer.current_line);
        assert_eq!(3, buffer.len());
        assert_eq!(buffer[..], expected[..]);
    }

    #[test]
    fn do_insert_non_empty_at_0() {
        let mut buffer = EditBuffer::with_text(&["1\n", "2", "3"]);
        let expected = EditBuffer::with_text(&["a\n", "b", "c", "1", "2", "3"]);
        let lines = ["a\n", "b\n", "c\n"].map(ToOwned::to_owned).to_vec();
        buffer.do_insert(buffer.try_line(0).ok(), lines);
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
        buffer.do_insert(buffer.try_span(2, 3).ok(), lines);
        assert_eq!(5, buffer.current_line);
        assert_eq!(9, buffer.len());
        assert_eq!(buffer[..], expected[..]);
    }

    #[test]
    fn do_insert_at_end() {
        let mut buffer = EditBuffer::with_text(&["1\n", "2", "3"]);
        let lines = ["a\n", "b\n", "c\n"].map(ToOwned::to_owned).to_vec();
        let expected = EditBuffer::with_text(&["1\n", "2", "a", "b", "c", "3"]);
        buffer.do_insert(buffer.try_line(3).ok(), lines);
        assert_eq!(5, buffer.current_line);
        assert_eq!(6, buffer.len());
        assert_eq!(buffer[..], expected[..]);
    }

    #[test]
    fn do_insert_of_zero_lines() {
        let mut buffer = EditBuffer::with_text(&["1\n", "2", "3"]);
        let expected = EditBuffer::with_text(&["1\n", "2", "3"]);
        assert_eq!(3, buffer.current_line());
        buffer.do_insert(buffer.try_line(2).ok(), Vec::new());
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
        let mut expected =
            EditBuffer::with_text(&["1\n", "2", "3 4", "5", "6"]);
        expected.set_current_line(3);
        buffer.do_join(buffer.try_span(3, 4).ok(), Some(" "));
        assert_eq!(buffer, expected);
    }

    #[test]
    fn do_join_several_lines() {
        let mut buffer =
            EditBuffer::with_text(&["1\n", "2", "3", "4", "5", "6"]);
        buffer.current_line = 2;
        let mut expected = EditBuffer::with_text(&["1\n", "2", "345", "6"]);
        expected.set_current_line(3);
        buffer.do_join(buffer.try_span(3, 5).ok(), None);
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
        let changes = buffer
            .do_move(buffer.try_line(5).ok(), buffer.try_line(3).unwrap());
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
        let changes = buffer
            .do_move(buffer.try_span(5, 6).ok(), buffer.try_line(3).unwrap());
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
        let changes = buffer.do_move(None, buffer.try_line(3).unwrap());
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
        let changes = buffer
            .do_move(buffer.try_span(4, 5).ok(), buffer.try_line(0).unwrap());
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
        let changes = buffer.do_move(
            buffer.try_span(4, 5).ok(),
            buffer.try_span(1, 2).unwrap(),
        );
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
        let changes = buffer
            .do_append(buffer.try_line(0).ok(), lines)
            .expect("Some(ChangeSet)");
        buffer.push_undo(changes);
        assert!(buffer.content_hash.is_none());
    }

    #[test]
    fn do_undo_append_line() {
        let mut buffer = EditBuffer::new();
        let lines = ["1\n", "2\n", "3\n"].map(ToOwned::to_owned).to_vec();
        let changes = buffer
            .do_append(buffer.try_line(0).ok(), lines)
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
        let changes = buffer.do_delete(buffer.try_span(1, 4).ok());
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
        let changes = buffer.do_delete(buffer.try_line(3).ok());
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
            .do_insert(buffer.try_line(3).ok(), lines)
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
        let changes = buffer
            .do_transfer(buffer.try_line(6).ok(), buffer.try_line(2).unwrap());
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
        let changes = buffer.do_transfer(
            buffer.try_span(5, 6).ok(),
            buffer.try_line(2).unwrap(),
        );
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
        let changes = buffer.do_transfer(None, buffer.try_line(2).unwrap());
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
            .do_append(buffer.try_line(2).ok(), lines)
            .expect("Some(ChangeSet)");
        buffer.push_undo(changes);
        let expected_1 = EditBuffer::with_text(&[
            "1\n", "2", "a", "b", "c", "3", "4", "5", "6",
        ]);
        assert_eq!(buffer[..], expected_1[..]);
        assert_eq!(buffer.current_line(), 5);

        let changes = buffer.do_delete(buffer.try_span(4, 7).ok());
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
            .do_append(buffer.try_line(2).ok(), lines)
            .expect("Some(ChangeSet)");
        buffer.push_undo(changes);
        let expected_1 = EditBuffer::with_text(&[
            "1\n", "2", "a", "b", "c", "3", "4", "5", "6",
        ]);
        assert_eq!(&expected_1[..], &buffer[..]);
        assert_eq!(5, buffer.current_line());

        let changes = buffer.do_delete(buffer.try_span(4, 7).ok());
        buffer.push_undo(changes);
        let expected_2 = EditBuffer::with_text(&["1\n", "2", "a", "5", "6"]);
        assert_eq!(buffer[..], expected_2[..]);

        buffer.do_undo().unwrap();
        assert_eq!(buffer[..], expected_1[..]);

        let lines = vec!["spam!\n".to_owned()];
        let changes = buffer
            .do_append(buffer.try_line(5).ok(), lines)
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
        assert!(matches!(Error::NothingToUndo, _ret));
        // Undo stack should be empty here, so buffer shouldn't change
        assert_eq!(buffer[..], expected_final[..]);
    }

    #[test]
    fn do_redo_multi() {
        let mut buffer =
            EditBuffer::with_text(&["1\n", "2", "3", "4", "5", "6"]);
        let buffer_orig = buffer.clone();
        assert_eq!(buffer.current_line(), 6);

        let lines = ["a\n", "b\n", "c\n"].map(ToOwned::to_owned).to_vec();
        let changes = buffer
            .do_append(buffer.try_line(2).ok(), lines)
            .expect("Some(ChangeSet)");
        buffer.push_undo(changes);
        let expected_1 = EditBuffer::with_text(&[
            "1\n", "2", "a", "b", "c", "3", "4", "5", "6",
        ]);
        assert_eq!(&expected_1[..], &buffer[..]);
        assert_eq!(buffer.current_line(), 5);

        let changes = buffer.do_delete(buffer.try_span(4, 7).ok());
        buffer.push_undo(changes);
        let expected_final =
            EditBuffer::with_text(&["1\n", "2", "a", "5", "6"]);
        assert_eq!(buffer[..], expected_final[..]);

        buffer.do_undo().unwrap();
        assert_eq!(buffer[..], expected_1[..]);
        buffer.do_undo().unwrap();
        assert_eq!(buffer[..], buffer_orig[..]);
        let _ret = buffer.do_undo().expect_err("nothing to undo");
        assert!(matches!(Error::NothingToUndo, _ret));
        assert_eq!(buffer[..], buffer_orig[..]); // buffer unchanged

        buffer.do_redo().unwrap();
        assert_eq!(&expected_1[..], &buffer[..]);

        buffer.do_redo().unwrap();
        assert_eq!(buffer[..], expected_final[..]);

        let _ret = buffer.do_redo().expect_err("nothing to redo");
        assert!(matches!(Error::NothingToRedo, _ret));
        assert_eq!(buffer[..], expected_final[..]); // buffer unchanged
    }
    #[test]
    fn do_undo_redo_change_span() {
        let mut buffer = EditBuffer::new();
        let orig = EditBuffer::new();

        let expected1 = EditBuffer::with_text(&["1\n", "2", "3"]);
        let changes =
            buffer.do_change(buffer.try_line(0).ok(), expected1[..].to_vec());
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
            .do_change(buffer.try_span(2, 3).ok(), expected3[2..].to_vec());
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
        assert!(buffer.content_hash.is_none());

        buffer.do_undo().unwrap();
        assert!(buffer.is_empty());
        assert_eq!(buffer[..], orig[..]);
        assert_eq!(buffer.current_line(), 0);
        assert!(buffer.content_hash.is_none());

        let _ret = buffer.do_undo().expect_err("nothing to undo");
        assert!(matches!(Error::NothingToUndo, _ret));
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

        let changes = buffer.do_change(buffer.try_line(0).ok(), lines);
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

        let changes = buffer.do_change(buffer.try_span(3, 5).ok(), lines);
        buffer.push_undo(changes);
        assert_eq!(buffer.current_line(), 3);
        assert_eq!(buffer[3], "6\n");
        buffer.do_undo().unwrap();
        assert_eq!(buffer[..], orig[..]);
        assert_eq!(buffer.current_line(), orig.current_line());

        let mut buffer = orig.clone();
        assert_eq!(buffer.current_line(), 6);
        let changes = buffer.do_change(buffer.try_span(5, 6).ok(), Vec::new());
        buffer.push_undo(changes);
        assert_eq!(buffer.current_line(), 4);
        buffer.do_undo().unwrap();
        assert_eq!(buffer[..], orig[..]);
        assert_eq!(buffer.current_line(), orig.current_line());

        let mut buffer = orig.clone();
        assert_eq!(buffer.current_line(), 6);
        let changes = buffer.do_change(buffer.try_span(0, 2).ok(), Vec::new());
        buffer.push_undo(changes);
        assert_eq!(buffer.current_line(), 1);
        buffer.do_undo().unwrap();
        assert_eq!(buffer[..], orig[..]);
        assert_eq!(buffer.current_line(), orig.current_line());

        let mut buffer = orig.clone();
        assert_eq!(buffer.current_line(), 6);
        let changes = buffer.do_change(buffer.try_span(1, 6).ok(), Vec::new());
        buffer.push_undo(changes);
        assert_eq!(buffer.current_line(), 0);
        assert!(buffer.is_empty());
        buffer.do_undo().unwrap();
        assert_eq!(buffer[..], orig[..]);
        assert_eq!(buffer.current_line(), orig.current_line());
    }

    #[test]
    fn append_zero_lines_does_nothing() {
        let mut buffer = EditBuffer::new();
        let res = buffer.append(0, Vec::new());
        assert_eq!(res, false);
        assert!(buffer.is_empty());
    }

    #[test]
    fn append_normalizes_eols() {
        let mut buf = EditBuffer::with_text(&["1\n", "2", "3"]);
        let expected = ["1\n", "2\n", "a\n", "b\n", "c\n", "3\n"];
        let added = buf.append(
            2,
            vec!["a\r\n".to_owned(), "b\r\n".to_owned(), "c\r\n".to_owned()],
        );

        assert!(!added);
        assert_eq!(buf[..], expected);
    }
    #[test]
    fn prevailing_eol_from_str() {
        assert_eq!(
            "CRLF".parse::<PrevailingEol>().unwrap(),
            PrevailingEol::crlf(false)
        );
        assert_eq!(
            "LF".parse::<PrevailingEol>().unwrap(),
            PrevailingEol::lf(false)
        );
    }

    #[test]
    fn prevailing_eol_display_str() {
        let mut eol = PrevailingEol::lf(false);
        assert_eq!(eol.display_str(), "LF");
        eol.mixed = true;
        assert_eq!(eol.display_str(), "LF/mixed");
        eol.eol = Eol::Crlf;
        assert_eq!(eol.display_str(), "CRLF/mixed");
        eol.mixed = false;
        assert_eq!(eol.display_str(), "CRLF");
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
        let mut buffer = EditBuffer::with_text(&["1\r\n", "2", "3"]);
        buffer.set_current_line(2);
        let address =
            Address::eval(&mut cmd_line, &mut buffer, &mut None).unwrap();
        assert_eq!(address, buffer.try_line(2).ok());
        assert_eq!(cmd_line.next(), Some("d"));
    }

    #[test]
    fn eval_dollar_addr() {
        let mut cmd_line = "$d\r\n".graphemes(true).peekable();
        let mut buffer = EditBuffer::with_text(&["1\r\n", "2", "3"]);
        buffer.set_current_line(2);
        let address =
            Address::eval(&mut cmd_line, &mut buffer, &mut None).unwrap();
        assert_eq!(address, buffer.try_line(3).ok());
        assert_eq!(cmd_line.next(), Some("d"));
    }

    #[test]
    fn eval_simple_number_addr() {
        let mut buffer =
            EditBuffer::with_text(&["1\n", "2", "3", "4", "5", "6"]);
        let mut cmd_line = "5d\n".graphemes(true).peekable();
        let address =
            Address::eval(&mut cmd_line, &mut buffer, &mut None).unwrap();
        assert_eq!(cmd_line.next(), Some("d"));
        assert_eq!(address, buffer.try_line(5).ok());
    }

    #[test]
    fn regex_line_addr_regex_syntax() {
        let mut input = "/\\lo.+/n\n".graphemes(true).peekable();
        let mut buffer = EditBuffer::with_text(&[
            "one\r\n", "two", "three", "four", "five", "six",
        ]);
        buffer.set_current_line(2);
        let mut previous_pattern: Option<Regex> = None;
        let res = Address::eval(&mut input, &mut buffer, &mut previous_pattern)
            .expect_err("bad pattern");
        assert!(matches!(res, Error::Regex { .. }));
    }

    #[test]
    fn rev_regex_line_addr_regex_syntax() {
        let mut input = "?\\lo.+?n\n".graphemes(true).peekable();
        let mut buffer = EditBuffer::with_text(&[
            "one\r\n", "two", "three", "four", "five", "six",
        ]);
        buffer.set_current_line(2);
        let mut previous_pattern: Option<Regex> = None;
        let res = Address::eval(&mut input, &mut buffer, &mut previous_pattern)
            .expect_err("bad pattern");
        assert!(matches!(res, Error::Regex { .. }));
    }

    #[test]
    fn regex_line_addr_embedded_delim() {
        let mut input = "/o.+\\//n\n".graphemes(true).peekable();
        let mut buffer = EditBuffer::with_text(&[
            "one/\n", "two", "three", "four", "five", "six",
        ]);
        buffer.set_current_line(2);
        let mut previous_pattern: Option<Regex> = None;
        let res = Address::eval(&mut input, &mut buffer, &mut previous_pattern)
            .unwrap();
        assert_eq!(res, buffer.try_line(1).ok());
    }

    #[test]
    fn regex_line_addr_no_final_delimiter() {
        let mut input = "/o.+\n".graphemes(true).peekable();
        let mut buffer = EditBuffer::with_text(&[
            "one\r\n", "two", "three", "four", "five", "six",
        ]);
        buffer.set_current_line(2);
        let mut previous_pattern: Option<Regex> = None;
        let res = Address::eval(&mut input, &mut buffer, &mut previous_pattern)
            .unwrap();
        assert_eq!(res, buffer.try_line(4).ok());
    }

    #[test]
    fn regex_line_addr_needle_in_first_half_of_split_range() {
        let mut input = "/o.+/n\n".graphemes(true).peekable();
        let mut previous_pattern: Option<Regex> = None;
        let mut buffer = EditBuffer::with_text(&[
            "one\r\n", "two", "three", "four", "five", "six",
        ]);
        buffer.set_current_line(2);
        let res = Address::eval(&mut input, &mut buffer, &mut previous_pattern)
            .unwrap();
        assert_eq!(res, buffer.try_line(4).ok());
    }

    #[test]
    fn regex_line_addr_needle_in_second_half_of_split_range() {
        let mut input = "/on.+/n\n".graphemes(true).peekable();
        let mut previous_pattern: Option<Regex> = None;
        let mut buffer = EditBuffer::with_text(&[
            "one\r\n", "two", "three", "four", "five", "six",
        ]);
        buffer.set_current_line(4);
        let res = Address::eval(&mut input, &mut buffer, &mut previous_pattern)
            .unwrap();
        assert_eq!(res, buffer.try_line(1).ok());
    }

    #[test]
    fn regex_line_addr_contiguous_search_range() {
        let mut input = "/o.+/n\n".graphemes(true).peekable();
        let mut previous_pattern: Option<Regex> = None;
        let mut buffer = EditBuffer::with_text(&[
            "one\r\n", "two", "three", "four", "five", "six",
        ]);
        buffer.set_current_line(6);
        let res = Address::eval(&mut input, &mut buffer, &mut previous_pattern)
            .unwrap();
        assert_eq!(res, buffer.try_line(1).ok());
    }

    #[test]
    fn rev_regex_line_addr_needle_in_first_half_of_split_range() {
        let mut input = "?o.+?n\n".graphemes(true).peekable();
        let mut previous_pattern: Option<Regex> = None;
        let mut buffer = EditBuffer::with_text(&[
            "one\r\n", "two", "three", "four", "five", "six",
        ]);
        buffer.set_current_line(2);
        let res = Address::eval(&mut input, &mut buffer, &mut previous_pattern)
            .unwrap();
        assert_eq!(res, buffer.try_line(1).ok());
    }

    #[test]
    fn rev_regex_line_addr_needle_in_second_half_of_split_range() {
        let mut input = "?ou.+?n\n".graphemes(true).peekable();
        let mut previous_pattern: Option<Regex> = None;
        let mut buffer = EditBuffer::with_text(&[
            "one\r\n", "two", "three", "four", "five", "six",
        ]);
        buffer.set_current_line(4);
        let res = Address::eval(&mut input, &mut buffer, &mut previous_pattern)
            .unwrap();
        assert_eq!(res, buffer.try_line(4).ok());
    }

    #[test]
    fn rev_regex_line_addr_contiguous_search_range() {
        let mut input = "?o.+?n\n".graphemes(true).peekable();
        let mut previous_pattern: Option<Regex> = None;
        let mut buffer = EditBuffer::with_text(&[
            "one\r\n", "two", "three", "four", "five", "six",
        ]);
        buffer.set_current_line(1);
        let res = Address::eval(&mut input, &mut buffer, &mut previous_pattern)
            .unwrap();
        assert_eq!(res, buffer.try_line(4).ok());
    }

    #[test]
    fn regex_line_addr_with_offset() {
        let mut input = "/o.+/+2\n".graphemes(true).peekable();
        let mut buffer = EditBuffer::with_text(&[
            "one\r\n", "two", "three", "four", "five", "six",
        ]);
        buffer.set_current_line(2);
        let mut previous_pattern: Option<Regex> = None;
        let res = Address::eval(&mut input, &mut buffer, &mut previous_pattern)
            .unwrap();
        assert_eq!(res, buffer.try_line(6).ok());
    }

    #[test]
    fn rev_regex_line_addr_with_offset() {
        let mut input = "?o.+?+2\n".graphemes(true).peekable();
        let mut buffer = EditBuffer::with_text(&[
            "one\r\n", "two", "three", "four", "five", "six",
        ]);
        buffer.set_current_line(2);
        let mut previous_pattern: Option<Regex> = None;
        let res = Address::eval(&mut input, &mut buffer, &mut previous_pattern)
            .unwrap();
        assert_eq!(res, buffer.try_line(3).ok());
    }

    #[test]
    fn eval_simple_comma_addr() {
        let mut buffer =
            EditBuffer::with_text(&["1\r\n", "2", "3", "4", "5", "6"]);
        let mut input = "1,2p\n".graphemes(true).peekable();
        let res = Address::eval(&mut input, &mut buffer, &mut None).unwrap();
        assert_eq!(res, buffer.try_span(1, 2).ok());
        assert_eq!(input.next(), Some("p"));
    }

    #[test]
    fn eval_leading_comma_addr() {
        let mut buffer =
            EditBuffer::with_text(&["1\r\n", "2", "3", "4", "5", "6"]);
        let mut input = ",4p\r\n".graphemes(true).peekable();
        let res = Address::eval(&mut input, &mut buffer, &mut None).unwrap();
        assert_eq!(input.next(), Some("p"));
        assert_eq!(res, buffer.try_span(1, 4).ok());
    }

    #[test]
    fn eval_trailing_comma_addr() {
        let mut input = "5,p\n".graphemes(true).peekable();
        let mut buffer =
            EditBuffer::with_text(&["1\n", "2", "3", "4", "5", "6"]);
        let res = Address::eval(&mut input, &mut buffer, &mut None).unwrap();
        assert_eq!(input.next(), Some("p"));
        assert_eq!(res, buffer.try_line(5).ok());
    }

    #[test]
    fn eval_comma_only_addr() {
        let mut input = ",p\r\n".graphemes(true).peekable();
        let mut buffer =
            EditBuffer::with_text(&["1\n", "2", "3", "4", "5", "6"]);
        let res = Address::eval(&mut input, &mut buffer, &mut None).unwrap();
        assert_eq!(input.next(), Some("p"));
        assert_eq!(res, buffer.try_span(1, 6).ok());
    }

    #[test]
    fn eval_comma_only_chain_addr() {
        let mut input = ",,p\n".graphemes(true).peekable();
        let mut buffer =
            EditBuffer::with_text(&["1\n", "2", "3", "4", "5", "6"]);
        let res = Address::eval(&mut input, &mut buffer, &mut None).unwrap();
        assert_eq!(input.next(), Some("p"));
        assert_eq!(res, buffer.try_line(6).ok());
    }

    #[test]
    fn eval_comma_chain_addr() {
        let mut input = ",12, 3+1,p\n".graphemes(true).peekable();
        let mut buffer =
            EditBuffer::with_text(&["1\n", "2", "3", "4", "5", "6"]);
        let res = Address::eval(&mut input, &mut buffer, &mut None).unwrap();
        assert_eq!(input.next(), Some("p"));
        assert_eq!(res, buffer.try_line(4).ok());
    }

    #[test]
    fn eval_semicolon_addr_past_end() {
        let mut input = "+;np\n".graphemes(true).peekable();
        let mut buffer =
            EditBuffer::with_text(&["1\r\n", "2", "3", "4", "5", "6"]);
        assert_eq!(buffer.current_line(), 6);
        let res = Address::eval(&mut input, &mut buffer, &mut None)
            .expect_err("invalid address");
        assert!(matches!(res, Error::InvalidAddress));
    }

    #[test]
    fn eval_simple_semicolon_addr() {
        let mut input = "1;2p\n".graphemes(true).peekable();
        let mut buffer =
            EditBuffer::with_text(&["1\n", "2", "3", "4", "5", "6"]);
        assert_eq!(buffer.current_line(), 6);
        let res = Address::eval(&mut input, &mut buffer, &mut None).unwrap();
        assert_eq!(res, buffer.try_span(1, 2).ok());
        assert_eq!(buffer.current_line(), 1);
        assert_eq!(input.next(), Some("p"));
    }

    #[test]
    fn eval_leading_semicolon_addr() {
        let mut input = ";5p\n".graphemes(true).peekable();
        let mut buffer =
            EditBuffer::with_text(&["1\n", "2", "3", "4", "5", "6"]);
        buffer.set_current_line(3);
        let res = Address::eval(&mut input, &mut buffer, &mut None).unwrap();
        assert_eq!(input.next(), Some("p"));
        assert_eq!(res, buffer.try_span(3, 5).ok());
        assert_eq!(buffer.current_line(), 3);
    }

    #[test]
    fn eval_trailing_semicolon_addr() {
        let mut input = "5;p\n".graphemes(true).peekable();
        let mut buffer =
            EditBuffer::with_text(&["1\n", "2", "3", "4", "5", "6"]);
        buffer.set_current_line(3);
        let res = Address::eval(&mut input, &mut buffer, &mut None).unwrap();
        assert_eq!(input.next(), Some("p"));
        assert_eq!(res, buffer.try_line(5).ok());
        assert_eq!(buffer.current_line(), 5);
    }

    #[test]
    fn eval_semicolon_only_addr() {
        let mut input = ";p\n".graphemes(true).peekable();
        let mut buffer =
            EditBuffer::with_text(&["1\n", "2", "3", "4", "5", "6"]);
        buffer.set_current_line(3);
        let res = Address::eval(&mut input, &mut buffer, &mut None).unwrap();
        assert_eq!(input.next(), Some("p"));
        assert_eq!(res, buffer.try_span(3, 6).ok());
        assert_eq!(buffer.current_line(), 3);
    }

    #[test]
    fn eval_semicolon_only_chain_addr() {
        let mut input = ";;p\n".graphemes(true).peekable();
        let mut buffer =
            EditBuffer::with_text(&["1\n", "2", "3", "4", "5", "6"]);
        buffer.set_current_line(3);
        let res = Address::eval(&mut input, &mut buffer, &mut None).unwrap();
        assert_eq!(input.next(), Some("p"));
        assert_eq!(res, buffer.try_line(6).ok());
    }

    #[test]
    fn eval_big_before_small_semicolon_chain_addr() {
        let mut input = "4;$;2p\n".graphemes(true).peekable();
        let mut buffer =
            EditBuffer::with_text(&["1\n", "2", "3", "4", "5", "6"]);
        buffer.set_current_line(3);
        let res = Address::eval(&mut input, &mut buffer, &mut None)
            .expect_err("invalid address");
        assert!(matches!(res, Error::InvalidAddress));
    }

    #[test]
    fn eval_simple_offset_only_addrs() {
        let mut input = "+p\n".graphemes(true).peekable();
        let mut buffer =
            EditBuffer::with_text(&["1\n", "2", "3", "4", "5", "6"]);
        buffer.set_current_line(3);
        let res = Address::eval(&mut input, &mut buffer, &mut None).unwrap();
        assert_eq!(input.next(), Some("p"));
        assert_eq!(res, buffer.try_line(4).ok());

        let mut input = "+10p\n".graphemes(true).peekable();
        let mut buffer =
            EditBuffer::with_text(&["1\n", "2", "3", "4", "5", "6"]);
        buffer.set_current_line(3);
        let res = Address::eval(&mut input, &mut buffer, &mut None)
            .expect_err("InvalidAddress");
        assert_eq!(input.next(), Some("p"));
        assert!(matches!(res, Error::InvalidAddress));

        let mut input = "-p\n".graphemes(true).peekable();
        let mut buffer =
            EditBuffer::with_text(&["1\n", "2", "3", "4", "5", "6"]);
        buffer.set_current_line(3);
        let res = Address::eval(&mut input, &mut buffer, &mut None).unwrap();
        assert_eq!(input.next(), Some("p"));
        assert_eq!(res, buffer.try_line(2).ok());

        let mut input = "-2p\n".graphemes(true).peekable();
        let mut buffer =
            EditBuffer::with_text(&["1\n", "2", "3", "4", "5", "6"]);
        buffer.set_current_line(3);
        let res = Address::eval(&mut input, &mut buffer, &mut None).unwrap();
        assert_eq!(input.next(), Some("p"));
        assert_eq!(res, buffer.try_line(1).ok());
    }

    #[test]
    fn eval_too_big_offset_only_addr_overflows() {
        let mut input = "-10p\n".graphemes(true).peekable();
        let mut buffer =
            EditBuffer::with_text(&["1\n", "2", "3", "4", "5", "6"]);
        buffer.set_current_line(3);
        let res = Address::eval(&mut input, &mut buffer, &mut None)
            .expect_err("offset overflow");
        assert!(matches!(res, Error::InvalidOffset));
    }
}
