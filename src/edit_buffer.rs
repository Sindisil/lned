// EditBuffer keeps track of everything specific to a single buffer in the
// editor. All public interface uses one based indexing, and any such function
// is responsible for translating into the 0 based indexing of the Vec<String>
// containing the lines of text.
mod undo_stack;

use std::borrow::ToOwned;
use std::cmp::Ordering;
use std::fmt::{self, Display, Formatter};
use std::fs::OpenOptions;
use std::io::{self, BufRead, Write};
use std::ops::{Index, Range, RangeFrom, RangeFull, RangeInclusive};
use std::path::{Path, PathBuf};

use crate::command::{Address, Cmd};
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

#[derive(Debug)]
pub enum Error {
    Read(io::Error),
    ReadBadIndex(usize, usize),
    InvalidAddress,
    WriteOutput(io::Error),
    NoFilename,
    FileOpen(io::Error),
    WriteLines(io::Error),
    ReadLines(io::Error),
}

impl std::error::Error for Error {}

impl Display for Error {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Error::Read(e) => write!(f, "error reading lines: {e}"),
            Error::ReadBadIndex(sz, i) => write!(
                f,
                "error reading lines: location {i} beyond end of buffer {sz}"
            ),
            Error::InvalidAddress => write!(f, "invalid address"),
            Error::WriteOutput(e) => write!(f, "Error writing output: {e}"),
            Error::NoFilename => write!(f, "No filename"),
            Error::FileOpen(e) => write!(f, "Error opening file: {e}"),
            Error::WriteLines(e) => write!(f, "Error writing lines to file: {e}"),
            Error::ReadLines(e) => write!(f, "{e} reading input lines"),
        }
    }
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
#[derive(Debug, PartialEq)]
enum ReadResult {
    AsIs(usize),
    EOLAdded(usize),
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

    /// Reads lines from reader into the buffer at the specified line.
    ///
    /// Default EOL auto-detect:
    ///     If this call to read is on a buffer that has no default EOL, then new lines
    ///     read are examined, and the default is set to the most frequently used EOL
    ///     sequence.
    ///
    /// EOL Correction:
    /// If the final line read has no line terminator, one will be added.
    ///     Added EOLs will be the default EOL for the
    ///    buffer. This is determined either by configuration, or auto-detected
    ///    (e.g., as described above, or similarly when first lines are appended
    ///    or inserted).
    ///
    /// Returns number of bytes read, or an error if read fails
    fn read(&mut self, at_line: usize, mut reader: impl BufRead) -> Result<ReadResult, Error> {
        if at_line > self.text.len() {
            return Err(Error::ReadBadIndex(self.len(), at_line));
        }
        let mut lines = Vec::new();
        let mut line = String::new();
        let mut bytes_read = 0;
        loop {
            let len = reader.read_line(&mut line).map_err(Error::Read)?;
            if len == 0 {
                break;
            }
            bytes_read += len;
            lines.push(line);
            line = String::new();
        }
        let lines_added = lines.len();

        // set default_eol if neccessary
        let default_eol = self
            .default_eol
            .get_or_insert_with(|| compute_default_eol(&lines));

        // Add in missing eol as needed
        let eol_added = match lines.last_mut() {
            Some(last) if !(last.ends_with("\r\n") || last.ends_with('\n')) => {
                last.push_str(default_eol);
                bytes_read += default_eol.len();
                true
            }
            _ => false,
        };

        // actually add new lines to buffer
        self.text.splice(at_line..at_line, lines);
        self.current_line = at_line + lines_added;
        if eol_added {
            Ok(ReadResult::EOLAdded(bytes_read))
        } else {
            Ok(ReadResult::AsIs(bytes_read))
        }
    }

    // fixme - move out into io module or main_loop
    fn write(
        &mut self,
        output: &mut impl Write,
        address: Option<Address>,
        destination: &mut impl Write,
    ) -> Result<(), Error> {
        let line_span = address.map_or_else(|| 1usize..=self.len(), |addr| addr.0..=addr.1);
        let full_buffer_write = line_span == (1usize..=self.len());

        let mut total_bytes_written = 0;

        if !line_span.is_empty() {
            for line in &self[line_span] {
                let bytes_to_write = line.len();
                let mut bytes_written = 0;
                while bytes_written < bytes_to_write {
                    bytes_written = bytes_written
                        + destination
                            .write(line[bytes_written..].as_bytes())
                            .map_err(Error::WriteLines)?;
                }
                total_bytes_written += bytes_written;
            }
        }

        writeln!(output, "{total_bytes_written}").map_err(Error::WriteOutput)?;
        if full_buffer_write {
            self.clean_fingerprint = self.undo_stack.fingerprint();
        }
        Ok(())
    }

    //    fn execute(&mut self, output: &mut impl Write, op: &mut Op) -> Result<(), Error> {
    //        match op {
    //            Op::Append(data) => {
    //                let b = data.address.map_or(self.current_line, |addr| addr.1);
    //
    //                if data.lines.is_empty() {
    //                    self.current_line = b;
    //                } else {
    //                    // set default_eol if neccessary
    //                    let default_eol = self
    //                        .default_eol
    //                        .get_or_insert_with(|| compute_default_eol(&data.lines));
    //                    self.text.splice(
    //                        b..b,
    //                        data.lines.iter().cloned().map(|mut line| {
    //                            if !(line.ends_with('\n') || line.ends_with("\r\n")) {
    //                                line.push_str(default_eol);
    //                            }
    //                            line
    //                        }),
    //                    );
    //                    self.current_line = b + data.lines.len();
    //                }
    //                Ok(())
    //            }
    //            Op::Delete(data) => {
    //                let (b, e) = data
    //                    .address
    //                    .map_or((self.current_line, self.current_line), |addr| {
    //                        (addr.0, addr.1)
    //                    });
    //
    //                if data.lines_removed.is_empty() {
    //                    data.lines_removed
    //                        .splice(.., self.text.splice(b - 1..e, None));
    //                } else {
    //                    self.text.splice(b - 1..e, None);
    //                }
    //                self.current_line = usize::min(self.text.len(), b);
    //                Ok(())
    //            }
    //            Op::Edit(data) => {
    //                let f = File::open(&data.filename);
    //                let source = match f {
    //                    Ok(f) => Ok(Some(BufReader::new(f))),
    //                    Err(e) => match e.kind() {
    //                        io::ErrorKind::NotFound => {
    //                            writeln!(output, "{e}").map_err(Error::WriteOutput)?;
    //                            Ok(None)
    //                        }
    //                        _ => Err(e),
    //                    },
    //                }
    //                .map_err(Error::FileOpen)?;
    //
    //                self.read_replace(output, source, Some(data))
    //            }
    //            Op::Insert(data) => {
    //                let b = data.address.map_or(self.current_line, |addr| addr.1);
    //
    //                if data.lines.is_empty() {
    //                    self.current_line = b;
    //                } else {
    //                    let b = b.saturating_sub(1); // insert point is before addressed line
    //                                                 // set default_eol if neccessary
    //                    let default_eol = self
    //                        .default_eol
    //                        .get_or_insert_with(|| compute_default_eol(&data.lines));
    //                    self.text.splice(
    //                        b..b,
    //                        data.lines.iter().cloned().map(|mut line| {
    //                            if !(line.ends_with('\n') || line.ends_with("\r\n")) {
    //                                line.push_str(default_eol);
    //                            }
    //                            line
    //                        }),
    //                    );
    //                    self.current_line = b + data.lines.len();
    //                }
    //                Ok(())
    //            }
    //            Op::Inverse(inner) => self.revert(output, inner),
    //        }
    //    }
    //
    //    fn revert(&mut self, output: &mut impl Write, op: &mut Op) -> Result<(), Error> {
    //        match op {
    //            Op::Append(data) => {
    //                let b = data.address.map_or(data.current_line, |addr| addr.1);
    //                self.text.splice(b..b + data.lines.len(), None);
    //                self.current_line = data.current_line;
    //                Ok(())
    //            }
    //            Op::Delete(data) => {
    //                let b = data.address.map_or(data.current_line, |addr| addr.0) - 1;
    //                self.text.splice(b..b, data.lines_removed.iter().cloned());
    //                self.current_line = b + data.lines_removed.len();
    //                Ok(())
    //            }
    //            Op::Edit(data) => {
    //                self.text.splice(.., data.lines_removed.iter().cloned());
    //                self.current_line = data.current_line;
    //                Ok(())
    //            }
    //            Op::Insert(data) => {
    //                let b = data
    //                    .address
    //                    .map_or(data.current_line, |addr| addr.1)
    //                    .saturating_sub(1);
    //                self.text.splice(b..b + data.lines.len(), None);
    //                self.current_line = data.current_line;
    //                Ok(())
    //            }
    //            Op::Inverse(inner) => self.execute(output, inner),
    //        }
    //    }
    //
    //    pub fn read_replace(
    //        &mut self,
    //        output: &mut impl Write,
    //        source: Option<impl BufRead>,
    //        data: Option<&mut EditData>,
    //    ) -> Result<(), Error> {
    //        if let Some(data) = data {
    //            if data.lines_removed.is_empty() {
    //                data.lines_removed.append(&mut self.text);
    //            }
    //        }
    //        self.text.clear();
    //
    //        if let Some(source) = source {
    //            let ret = self.read(0, source)?;
    //            match ret {
    //                ReadResult::EOLAdded(bytes_read) => {
    //                    writeln!(output, "missing line terminator appended\n{bytes_read}")
    //                        .map_err(Error::WriteOutput)?;
    //                }
    //                ReadResult::AsIs(bytes_read) => {
    //                    writeln!(output, "{bytes_read}").map_err(Error::WriteOutput)?;
    //                }
    //            }
    //        }
    //        Ok(())
    //    }

    pub fn prepare_append(
        &mut self,
        input: &mut impl BufRead,
        address: Option<Address>,
    ) -> Result<(), Error> {
        if address.is_some_and(|a| a.1 > self.len()) {
            return Err(Error::InvalidAddress);
        }
        let mut lines = Vec::new();
        Cmd::read_lines(input, &mut lines).map_err(Error::ReadLines)?;
        let location = address.map_or(self.current_line, |addr| addr.1);
        self.do_append(location, lines)
    }

    fn do_append(&mut self, location: usize, lines: Vec<String>) -> Result<(), Error> {
        let mut change = ChangeSet::new();
        change.current_line_before = self.current_line;
        if lines.is_empty() {
            self.current_line = location;
        } else {
            // set default_eol if neccessary
            self.default_eol
                .get_or_insert_with(|| compute_default_eol(&lines));
            self.text.splice(location..location, lines.iter().cloned());
            self.current_line = location + lines.len();
            change.push_add(location, lines);
        }
        change.current_line_after = self.current_line;
        self.undo_stack.push_undo(change);
        Ok(())
    }

    pub fn prepare_delete(&mut self, address: Option<Address>) -> Result<(), Error> {
        match address {
            Some(Address(0, _)) => Err(Error::InvalidAddress),
            None if self.current_line == 0 => Err(Error::InvalidAddress),
            _ => self.do_delete(address),
        }
    }

    pub fn do_delete(&mut self, address: Option<Address>) -> Result<(), Error> {
        let (b, e) = address.map_or((self.current_line, self.current_line), |addr| {
            (addr.0, addr.1)
        });

        let removed: Vec<String> = self.text.splice(b - 1..e, None).collect();

        let mut change = ChangeSet::new();
        change.current_line_before = self.current_line;
        self.current_line = usize::min(self.text.len(), b);
        change.current_line_after = self.current_line;
        change.push_remove(b - 1, removed);
        self.undo_stack.push_undo(change);
        Ok(())
    }

    // fixme - in second phase, implement edit properly
    //    pub fn prepare_edit(
    //        &mut self,
    //        output: &mut impl Write,
    //        filename: Option<&Path>,
    //        prev_command: Option<&Cmd>,
    //    ) -> Result<(), Error> {
    //// This will all move to main_loop in later phase of rewrite
    //        if self.is_dirty() && !matches!(prev_command, Some(Cmd::Edit(_))) {
    //            writeln!(
    //                output,
    //                "Unwritten changes - repeat edit command to discard changes."
    //            )
    //            .map_err(Error::WriteOutput)?;
    //            return Ok(());
    //        }
    //
    //        if let Some(filename) = filename {
    //            self.filename = Some(filename.to_owned());
    //        }
    //        let filename = self.filename.as_ref().ok_or(Error::NoFilename)?;
    //                let f = File::open(&data.filename);
    //                let source = match f {
    //                    Ok(f) => Ok(Some(BufReader::new(f))),
    //                    Err(e) => match e.kind() {
    //                        io::ErrorKind::NotFound => {
    //                            writeln!(output, "{e}").map_err(Error::WriteOutput)?;
    //                            Ok(None)
    //                        }
    //                        _ => Err(e),
    //                    },
    //                }
    //                .map_err(Error::FileOpen)?;
    //
    //        if let Some(source) = source {
    //            let ret = self.read(0, source)?;
    //            match ret {
    //                ReadResult::EOLAdded(bytes_read) => {
    //                    writeln!(output, "missing line terminator appended\n{bytes_read}")
    //                        .map_err(Error::WriteOutput)?;
    //                }
    //                ReadResult::AsIs(bytes_read) => {
    //                    writeln!(output, "{bytes_read}").map_err(Error::WriteOutput)?;
    //                }
    //            }
    //        }
    //self.do_edit(filename.clone())
    //}
    //
    //fn do_edit(&mut self, filename: PathBuf) -> Result<(), Error> {
    //// fixme - create Diff::Change(0, self[..]))
    //        if let Some(data) = data {
    //            if data.lines_removed.is_empty() {
    //                data.lines_removed.append(&mut self.text);
    //            }
    //        }
    //        self.text.clear();
    //
    //        Ok(())
    //    }
    //
    //    pub fn prepare_append(
    //        &mut self,
    //        input: &mut impl BufRead,
    //        address: Option<Address>,
    //    ) -> Result<(), Error> {
    //        if address.is_some_and(|a| a.1 > self.len()) {
    //            return Err(Error::InvalidAddress);
    //        }
    //        let mut lines = Vec::new();
    //        Cmd::read_lines(input, &mut lines).map_err(Error::ReadLines)?;
    //        self.do_append(address, lines)
    //    }

    //    pub fn prepare_edit(
    //        &mut self,
    //        output: &mut impl Write,
    //        filename: Option<&Path>,
    //        prev_command: Option<&Cmd>,
    //    ) -> Result<(), Error> {
    //        if self.is_dirty() && !matches!(prev_command, Some(Cmd::Edit(_))) {
    //            writeln!(
    //                output,
    //                "Unwritten changes - repeat edit command to discard changes."
    //            )
    //            .map_err(Error::WriteOutput)?;
    //            return Ok(());
    //        }
    //
    //        if let Some(filename) = filename {
    //            self.filename = Some(filename.to_owned());
    //        }
    //        let filename = self.filename.as_ref().ok_or(Error::NoFilename)?;
    //self.do_edit(filename.clone())
    //}
    //
    //fn do_edit(&mut self, filename: PathBuf) -> Result<(), Error> {
    //                let f = File::open(&data.filename);
    //                let source = match f {
    //                    Ok(f) => Ok(Some(BufReader::new(f))),
    //                    Err(e) => match e.kind() {
    //                        io::ErrorKind::NotFound => {
    //                            writeln!(output, "{e}").map_err(Error::WriteOutput)?;
    //                            Ok(None)
    //                        }
    //                        _ => Err(e),
    //                    },
    //}
    //
    //        let mut op = Op::Edit(EditData {
    //            filename: filename.clone(),
    //            current_line: self.current_line,
    //            lines_removed: Vec::new(),
    //            clean_fingerprint: self.clean_fingerprint,
    //        });
    //
    //        let res = self.execute(output, &mut op);
    //        self.undo_stack.push_undo(op);
    //        self.clean_fingerprint = self.undo_stack.fingerprint();
    //        res
    //    }

    //    pub fn do_enumerate(
    //        &mut self,
    //        output: &mut impl Write,
    //        address: Option<Address>,
    //    ) -> Result<(), Error> {
    //        let span = if let Some(Address(b, e)) = address {
    //            b..=e
    //        } else {
    //            if self.current_line == 0 {
    //                return Err(Error::InvalidAddress);
    //            }
    //            self.current_line..=self.current_line
    //        };
    //
    //        if *span.start() < 1
    //            || *span.start() > self.len()
    //            || *span.end() < 1
    //            || *span.end() > self.len()
    //        {
    //            return Err(Error::InvalidAddress);
    //        }
    //
    //        let width = span.end().decimal_digits();
    //        let start = *span.start();
    //        self.current_line = *span.end();
    //
    //        for (i, l) in self[span].iter().enumerate() {
    //            output
    //                .write_all(format!("{:>width$}  {l}", start + i).as_bytes())
    //                .map_err(Error::WriteOutput)?;
    //        }
    //        output.flush().map_err(Error::WriteOutput)?;
    //        Ok(())
    //    }
    //
    pub fn do_file(
        &mut self,
        output: &mut impl Write,
        filename: Option<&Path>,
    ) -> Result<(), Error> {
        if let Some(filename) = filename {
            self.filename = Some(filename.to_owned());
        }

        match self.filename() {
            None => writeln!(output, "No current filename").map_err(Error::WriteOutput),
            Some(f) => writeln!(output, "{}", f.display()).map_err(Error::WriteOutput),
        }
    }


    pub fn prepare_insert(
        &mut self,
        input: &mut impl BufRead,
        address: Option<Address>,
    ) -> Result<(), Error> {
        if address.is_some_and(|a| a.1 > self.len()) {
            return Err(Error::InvalidAddress);
        }
        let mut lines = Vec::new();
        Cmd::read_lines(input, &mut lines).map_err(Error::ReadLines)?;
        let location = if lines.is_empty() {
            address.map_or(self.current_line, |addr| addr.1)
        } else {
            // insertion point is just before addressed line
            address
                .map_or(self.current_line, |addr| addr.1)
                .saturating_sub(1)
        };
        self.do_append(location, lines)
    }

    pub fn do_undo(&mut self) -> Result<(), Error> {
        if let Some(undo) = self.undo_stack.pop_undo() {
            self.current_line = undo.current_line_before;
            {
                let mut diffs = undo.diffs();
                while let Some(diff) = diffs.next() {
                    match diff {
                        Diff::Add(p, l) => drop(self.text.splice(*p..*p + l.len(), None)),
                        Diff::Remove(p, l) => drop(self.text.splice(*p..*p, l.iter().cloned())),
                    }
                }
            }
            self.undo_stack.push_redo(undo);
        }
        Ok(())
    }

    pub fn do_redo(&mut self) -> Result<(), Error> {
        if let Some(redo) = self.undo_stack.pop_redo() {
            self.current_line = redo.current_line_after;
            {
                let mut diffs = redo.diffs();
                while let Some(diff) = diffs.next() {
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
        Ok(())
    }

    pub fn do_write(
        &mut self,
        output: &mut impl Write,
        address: Option<Address>,
        filename: Option<&Path>,
    ) -> Result<(), Error> {
        if self.filename.is_none() {
            if filename.is_none() {
                return Err(Error::NoFilename);
            }
            self.filename = filename.map(ToOwned::to_owned);
        }

        let mut dest = OpenOptions::new()
            .write(true)
            .create(true)
            .open(self.filename.as_ref().unwrap())
            .map_err(Error::FileOpen)?;

        self.write(output, address, &mut dest)?;
        Ok(())
    }
}

fn compute_native_eol() -> &'static str {
    if std::env::consts::FAMILY == "windows" {
        "\r\n"
    } else {
        "\n"
    }
}

fn compute_default_eol(lines: impl IntoIterator<Item = impl AsRef<str>>) -> &'static str {
    let native_eol = compute_native_eol();
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

// Read lines of text input until a line with a single . is entered
// Clears previous content of buffer, but doesn't shrink capacity.
// Returns a Vec of all lines entered *except* the terminating line
// containing a single dot.

#[cfg(test)]
mod tests {
    use super::*;

    use std::io::{BufReader, Read};
    use std::ops::Deref;
    use std::str;

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

    // write() tests
    #[test]
    fn write_propegates_errors() {
        let mut buf = EditBuffer::from(vec!["1\r\n", "2", "3"]);
        let mut dummy_file = BadWriter {};
        let mut output = Vec::new();
        let res = buf
            .write(&mut output, Some(Address(1, 2)), &mut dummy_file)
            .expect_err("io error");
        assert!(matches!(res, Error::WriteLines(_)));
    }

    #[test]
    fn write_one_line() {
        let mut buf = EditBuffer::from(vec!["1\n", "2", "3"]);
        let mut dummy_file = Vec::new();
        let mut output = Vec::new();
        buf.write(&mut output, Some(Address(2, 2)), &mut dummy_file)
            .unwrap();
        assert_eq!(b"2\n", &output[..]);
    }

    #[test]
    fn write_many_lines() {
        let mut buf = EditBuffer::from(vec!["1\r\n", "2", "3", "4", "5", "6"]);
        let mut dummy_file = Vec::new();
        let mut output = Vec::new();
        buf.write(&mut output, Some(Address(1, 6)), &mut dummy_file)
            .unwrap();
        assert_eq!(b"18\n", &output[..]);
    }

    #[test]
    fn write_empty_buffer() {
        let mut buf = EditBuffer::new();
        let mut dummy_file = Vec::new();
        let mut output = Vec::new();
        buf.write(&mut output, None, &mut dummy_file).unwrap();
        assert_eq!(b"0\n", &output[..]);
    }

    //    #[test]
    //    fn write_no_addr_leaves_clean_buffer() {
    //        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3"]);
    //        assert!(!buffer.is_dirty());
    //        let mut output = Vec::new();
    //        let mut input = "one more line\n.\n".as_bytes();
    //        buffer
    //            .do_append(&mut input, &mut output, Some(Address(0, 0)))
    //            .unwrap();
    //        assert!(buffer.is_dirty());
    //        let mut dummy_file = Vec::new();
    //        output.clear();
    //        buffer.write(&mut output, None, &mut dummy_file).unwrap();
    //        assert_eq!(b"20\n", &output[..]);
    //        assert!(!buffer.is_dirty());
    //    }
    //
    //    #[test]
    //    fn write_full_buffer_leaves_clean_buffer() {
    //        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3"]);
    //        assert!(!buffer.is_dirty());
    //        let mut output = Vec::new();
    //        let mut input = "one more line\n.\n".as_bytes();
    //        buffer
    //            .do_append(&mut input, &mut output, Some(Address(0, 0)))
    //            .unwrap();
    //        assert!(buffer.is_dirty());
    //        let mut dummy_file = Vec::new();
    //        output.clear();
    //        buffer
    //            .write(&mut output, Some(Address(1, buffer.len())), &mut dummy_file)
    //            .unwrap();
    //        assert_eq!(b"20\n", &output[..]);
    //        assert!(!buffer.is_dirty());
    //    }
    //
    //    #[test]
    //    fn write_partial_buffer_leaves_dirty_buffer() {
    //        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3"]);
    //        assert!(!buffer.is_dirty());
    //        let mut output = Vec::new();
    //        let mut input = "one more line\n.\n".as_bytes();
    //        buffer
    //            .do_append(&mut input, &mut output, Some(Address(0, 0)))
    //            .unwrap();
    //        assert!(buffer.is_dirty());
    //        let mut dummy_file = Vec::new();
    //        output.clear();
    //        buffer
    //            .write(&mut output, Some(Address(1, 2)), &mut dummy_file)
    //            .unwrap();
    //        assert_eq!(b"16\n", &output[..]);
    //        assert!(buffer.is_dirty());
    //    }

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
        assert_eq!(compute_native_eol(), compute_default_eol(&lines));
    }

    /////
    /////
    // read() tests

    fn new_input_buf(content: &[impl Deref<Target = str>]) -> Vec<u8> {
        let mut input = Vec::new();
        for line in content {
            input.extend(line.bytes());
        }
        input
    }

    #[test]
    fn read_append_equal_eol_no_final() {
        let initial = vec!["Line1\n", "Line2\r\n", "Line3"];
        let mut buffer = EditBuffer::from(initial);
        assert!(buffer
            .default_eol
            .is_some_and(|eol| eol == compute_native_eol()));
        let def_eol = buffer.default_eol.unwrap();
        assert!(buffer[..].last().unwrap().ends_with(def_eol));

        let at = 3;
        let added = ["New1\n", "New2\r\n", "New3"];
        let input = new_input_buf(&added[..]);
        let ret = buffer.read(at, &input[..]).unwrap();

        let mut line3 = "Line3".to_string();
        line3.push_str(def_eol);
        let mut new3 = added[2].to_owned();
        new3.push_str(def_eol);
        let expect = vec![
            "Line1\n",
            "Line2\r\n",
            &line3[..],
            "New1\n",
            "New2\r\n",
            &new3[..],
        ];
        assert_eq!(buffer.text, expect);
        assert_eq!(buffer.current_line(), 6);
        assert!(if let ReadResult::EOLAdded(bytes) = ret {
            bytes == 15 + def_eol.len()
        } else {
            false
        });
    }

    #[test]
    fn read_insert_equal_eol_no_final() {
        let initial = vec!["Line1\r\n", "Line2\n", "Line3\n", "Line4\r\n"];
        let mut buffer = EditBuffer::from(initial);

        let at = 2;
        let added = ["New1\r\n", "New2\r\n", "New3"];
        let input = new_input_buf(&added[..]);
        let ret = buffer.read(at, &input[..]).unwrap();

        let mut new3 = "New3".to_string();
        new3.push_str(compute_native_eol());
        let expect = vec![
            "Line1\r\n",
            "Line2\n",
            "New1\r\n",
            "New2\r\n",
            &new3[..],
            "Line3\n",
            "Line4\r\n",
        ];
        assert_eq!(expect, buffer.text);
        assert!(if let ReadResult::EOLAdded(bytes) = ret {
            bytes == 16 + buffer.default_eol.unwrap().len()
        } else {
            false
        });
        assert_eq!(buffer.current_line(), 5);
    }

    #[test]
    fn read_with_bad_index() {
        let mut buffer = EditBuffer::new();
        let content = vec!["Line1\n"];
        let input = new_input_buf(&content);
        let res = buffer.read(999, &input[..]).expect_err("bad index");
        assert!(matches!(res, Error::ReadBadIndex(0, 999)));
    }

    #[test]
    fn read_with_io_error() {
        let reader = BadReader {};
        let mut input = BufReader::new(reader);
        let mut buffer = EditBuffer::new();
        let res = buffer.read(0, &mut input).expect_err("IO error");
        assert!(matches!(res, Error::Read(_)));
    }

    #[test]
    fn read_with_io_error_preserves_text() {
        let reader = BadReader {};
        let mut input = BufReader::new(reader);
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3"]);
        let expected = buffer.clone();
        let res = buffer.read(0, &mut input).expect_err("IO error");
        assert!(matches!(res, Error::Read(_)));
        assert_eq!(buffer[..], expected[..]);
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
        assert_eq!(vec!["1\n", "2\n", "3\n", "4\n", "5\n", "6\n"], buffer[1..7]);
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
    fn do_append_past_end_gives_error_before_input() {
        let mut buffer = EditBuffer::new();
        let mut input = "one\n.\n".as_bytes();
        let expected = "one\n.\n".as_bytes();
        let res = buffer
            .prepare_append(&mut input, Some(Address(2, 2)))
            .expect_err("invalid addr");
        assert_eq!(0, buffer.len());
        assert_eq!(input, expected);
        assert!(matches!(res, Error::InvalidAddress));
    }

    #[test]
    fn do_append_one_to_empty_buffer() {
        let mut buffer = EditBuffer::new();
        let expected = EditBuffer::from(vec!["one\n"]);
        let mut input = "one\n.\n".as_bytes();
        buffer
            .prepare_append(&mut input, Some(Address(0, 0)))
            .unwrap();
        assert_eq!(1, buffer.current_line);
        assert_eq!(1, buffer.len());
        assert_eq!(&expected[..], &buffer[..]);
    }

    #[test]
    fn do_append_empty_buffer() {
        let mut buffer = EditBuffer::new();
        let expected = EditBuffer::from(vec!["a\n", "b", "c"]);
        let mut input = "a\nb\nc\n.\n".as_bytes();
        buffer
            .prepare_append(&mut input, Some(Address(0, 0)))
            .unwrap();
        assert_eq!(3, buffer.current_line);
        assert_eq!(3, buffer.len());
        assert!(&expected[..].eq(&buffer[..]));
    }

    #[test]
    fn do_append_non_empty_at_0() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3"]);
        let expected = EditBuffer::from(vec!["a\n", "b", "c", "1", "2", "3"]);
        let mut input = "a\nb\nc\n.\n".as_bytes();
        buffer
            .prepare_append(&mut input, Some(Address(0, 0)))
            .unwrap();
        assert_eq!(3, buffer.current_line);
        assert_eq!(6, buffer.len());
        assert!(&expected[..].eq(&buffer[..]));
    }

    #[test]
    fn do_append_in_middle() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3"]);
        let mut input = "a\nb\nc\n.\n".as_bytes();
        let expected = EditBuffer::from(vec!["1\n", "2", "a", "b", "c", "3"]);
        buffer
            .prepare_append(&mut input, Some(Address(2, 2)))
            .unwrap();
        assert_eq!(5, buffer.current_line());
        assert_eq!(6, buffer.len());
        assert!(&expected[..].eq(&buffer[..]));
    }

    #[test]
    fn do_append_span_address() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        let mut input = "a\nb\nc\n.\n".as_bytes();
        let expected = EditBuffer::from(vec!["1\n", "2", "3", "a", "b", "c", "4", "5", "6"]);
        buffer
            .prepare_append(&mut input, Some(Address(2, 3)))
            .unwrap();
        assert_eq!(6, buffer.current_line);
        assert_eq!(9, buffer.len());
        assert!(&expected[..].eq(&buffer[..]));
    }

    #[test]
    fn do_append_at_end() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3"]);
        let mut input = "a\nb\nc\n.\n".as_bytes();
        let expected = EditBuffer::from(vec!["1\n", "2", "3", "a", "b", "c"]);
        buffer
            .prepare_append(&mut input, Some(Address(3, 3)))
            .unwrap();
        assert_eq!(6, buffer.current_line);
        assert_eq!(6, buffer.len());
        assert!(&expected[..].eq(&buffer[..]));
    }

    #[test]
    fn do_append_of_zero_lines() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3"]);
        let mut input = ".\n".as_bytes();
        let expected = EditBuffer::from(vec!["1\n", "2", "3"]);
        assert_eq!(3, buffer.current_line());
        buffer
            .prepare_append(&mut input, Some(Address(2, 2)))
            .unwrap();
        assert_eq!(2, buffer.current_line);
        assert_eq!(3, buffer.len());
        assert!(&expected[..].eq(&buffer[..]));
    }

    #[test]
    fn do_delete_span() {
        let mut buffer = EditBuffer::from(vec!["1\r\n", "2", "3", "4", "5", "6"]);
        let expected = EditBuffer::from(vec!["1\r\n", "2", "6"]);
        buffer.prepare_delete(Some(Address(3, 5))).unwrap();
        assert_eq!(3, buffer.len());
        assert!(&expected[..].eq(&buffer[..]));
    }

    #[test]
    fn do_delete_line() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        let expected = EditBuffer::from(vec!["1\n", "2", "4", "5", "6"]);
        buffer.prepare_delete(Some(Address(3, 3))).unwrap();
        assert_eq!(5, buffer.len());
        assert!(&expected[..].eq(&buffer[..]));
    }

    #[test]
    fn do_delete_span_at_start() {
        let mut buffer = EditBuffer::from(vec!["1\r\n", "2", "3", "4", "5", "6"]);
        let expected = EditBuffer::from(vec!["4\r\n", "5", "6"]);
        buffer.prepare_delete(Some(Address(1, 3))).unwrap();
        assert_eq!(3, buffer.len());
        assert_eq!(expected[..], buffer[..]);
    }

    #[test]
    fn do_delete_span_at_end() {
        let mut buffer = EditBuffer::from(vec!["1\r\n", "2", "3", "4", "5", "6"]);
        let expected = EditBuffer::from(vec!["1\r\n", "2", "3", "4"]);
        buffer.prepare_delete(Some(Address(5, 6))).unwrap();
        assert_eq!(4, buffer.len());
        assert_eq!(expected[..], buffer[..]);
    }

    #[test]
    fn do_delete_no_addr() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        let expected = EditBuffer::from(vec!["1\n", "2", "4", "5", "6"]);
        buffer.set_current_line(3);
        buffer.prepare_delete(None).unwrap();
        assert_eq!(5, buffer.len());
        assert_eq!(expected[..], buffer[..]);
    }

    #[test]
    fn do_delete_empty_buffer() {
        let mut buffer = EditBuffer::new();
        let _res = buffer.prepare_delete(None).expect_err("invalid address");
        assert!(matches!(Error::InvalidAddress, _res));
    }

    #[test]
    fn do_delete_line_zero() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3"]);
        let res = buffer
            .prepare_delete(Some(Address(0, 0)))
            .expect_err("invalid address");
        assert!(matches!(res, Error::InvalidAddress));
    }

    #[test]
    fn do_delete_span_starting_at_zero() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5"]);
        let res = buffer
            .prepare_delete(Some(Address(0, 3)))
            .expect_err("invalid address");
        assert!(matches!(res, Error::InvalidAddress));
    }

    #[test]
    fn buffer_dirty_after_append() {
        let mut buffer = EditBuffer::new();
        let mut input = "1\n2\n3\n.\n".as_bytes();
        assert!(!buffer.is_dirty());
        buffer
            .prepare_append(&mut input, Some(Address(0, 0)))
            .unwrap();
        assert!(buffer.is_dirty());
    }

    #[test]
    fn do_undo_append_line() {
        let mut buffer = EditBuffer::new();
        let mut input = "1\n2\n3\n.\n".as_bytes();
        buffer
            .prepare_append(&mut input, Some(Address(0, 0)))
            .unwrap();
        assert_eq!(&EditBuffer::from(vec!["1\n", "2", "3"])[..], &buffer[..]);
        buffer.do_undo().unwrap();
        assert_eq!(EditBuffer::new()[..], buffer[..]);
    }

    #[test]
    fn do_undo_append_span() {
        let mut buffer = EditBuffer::from(vec!["one\n", "two", "three"]);
        let mut input = "1\n2\n3\n.\n".as_bytes();
        let expected_final = buffer.clone();
        buffer
            .prepare_append(&mut input, Some(Address(2, 3)))
            .unwrap();
        assert_eq!(
            &EditBuffer::from(vec!["one\n", "two", "three", "1\n", "2", "3"])[..],
            &buffer[..]
        );
        buffer.do_undo().unwrap();
        assert_eq!(&expected_final[..], &buffer[..]);
    }

    #[test]
    fn do_undo_append_current_line() {
        let mut buffer = EditBuffer::from(vec!["one\n", "two", "three"]);
        let mut input = "1\n2\n3\n.\n".as_bytes();
        buffer.set_current_line(2);
        let expected_final = buffer.clone();
        buffer.prepare_append(&mut input, None).unwrap();
        assert_eq!(
            &EditBuffer::from(vec!["one\n", "two", "1\n", "2", "3", "three"])[..],
            &buffer[..]
        );
        buffer.do_undo().unwrap();
        assert_eq!(&expected_final[..], &buffer[..]);
    }

    #[test]
    fn do_undo_delete_span() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        let expected = buffer.clone();
        buffer.prepare_delete(Some(Address(1, 4))).unwrap();
        assert_eq!(&EditBuffer::from(vec!["5\n", "6"])[..], &buffer[..]);
        buffer.do_undo().unwrap();
        assert_eq!(&expected[..], &buffer[..]);
    }

    #[test]
    fn do_undo_delete_line() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        let expected = buffer.clone();
        buffer.prepare_delete(Some(Address(3, 3))).unwrap();
        assert_eq!(
            &EditBuffer::from(vec!["1\n", "2", "4", "5", "6"])[..],
            &buffer[..]
        );
        buffer.do_undo().unwrap();
        assert_eq!(&expected[..], &buffer[..]);
    }

    #[test]
    fn do_undo_delete_current_line() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        buffer.set_current_line(4);
        let expected = buffer.clone();
        buffer.prepare_delete(None).unwrap();
        assert_eq!(
            &EditBuffer::from(vec!["1\n", "2", "3", "5", "6"])[..],
            &buffer[..]
        );
        buffer.do_undo().unwrap();
        assert_eq!(&expected[..], &buffer[..]);
    }

    #[test]
    fn do_undo_redo_insert() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        let expected_final = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        let expected_modified =
            EditBuffer::from(vec!["1\n", "2", "a", "b", "c", "3", "4", "5", "6"]);
        let mut input = "a\nb\nc\n.\n".as_bytes();
        buffer
            .prepare_insert(&mut input, Some(Address(3, 3)))
            .unwrap();
        assert_eq!(buffer[..], expected_modified[..]);
        buffer.do_undo().unwrap();
        assert_eq!(expected_final[..], buffer[..]);
        buffer.do_redo().unwrap();
        assert_eq!(buffer[..], expected_modified[..]);
    }

    #[test]
    fn do_undo_multi() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        let mut input = "a\nb\nc\n.\n".as_bytes();
        let expected_final = buffer.clone();
        assert_eq!(6, buffer.current_line());

        buffer
            .prepare_append(&mut input, Some(Address(2, 2)))
            .unwrap();
        let expected_1 = EditBuffer::from(vec!["1\n", "2", "a", "b", "c", "3", "4", "5", "6"]);
        assert_eq!(&expected_1[..], &buffer[..]);
        assert_eq!(5, buffer.current_line());

        buffer.prepare_delete(Some(Address(4, 7))).unwrap();
        let expected_2 = EditBuffer::from(vec!["1\n", "2", "a", "5", "6"]);
        assert_eq!(&expected_2[..], &buffer[..]);

        buffer.do_undo().unwrap();
        assert_eq!(&expected_1[..], &buffer[..]);

        buffer.do_undo().unwrap();
        assert_eq!(&expected_final[..], &buffer[..]);
    }

    #[test]
    fn do_undo_redo_multi() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        let mut input = "a\nb\nc\n.\n".as_bytes();
        let expected_final = buffer.clone();
        assert_eq!(6, buffer.current_line());

        buffer
            .prepare_append(&mut input, Some(Address(2, 2)))
            .unwrap();
        let expected_1 = EditBuffer::from(vec!["1\n", "2", "a", "b", "c", "3", "4", "5", "6"]);
        assert_eq!(&expected_1[..], &buffer[..]);
        assert_eq!(5, buffer.current_line());

        buffer.prepare_delete(Some(Address(4, 7))).unwrap();
        let expected_2 = EditBuffer::from(vec!["1\n", "2", "a", "5", "6"]);
        assert_eq!(&expected_2[..], &buffer[..]);

        buffer.do_undo().unwrap();
        assert_eq!(&expected_1[..], &buffer[..]);

        input = "spam!\n.\n".as_bytes();
        buffer
            .prepare_append(&mut input, Some(Address(4, 5)))
            .unwrap();
        let expected_3 =
            EditBuffer::from(vec!["1\n", "2", "a", "b", "c", "spam!", "3", "4", "5", "6"]);
        assert_eq!(&buffer[..], &expected_3[..]);

        buffer.do_undo().unwrap();
        assert_eq!(&expected_1[..], &buffer[..]);

        buffer.do_undo().unwrap();
        assert_eq!(&expected_2[..], &buffer[..]);

        buffer.do_undo().unwrap();
        assert_eq!(&expected_1[..], &buffer[..]);

        buffer.do_undo().unwrap();
        assert_eq!(&expected_final[..], &buffer[..]);

        buffer.do_undo().unwrap();
        // Undo stack should be empty here, so buffer shouldn't change
        assert_eq!(&expected_final[..], &buffer[..]);
    }

    #[test]
    fn buffer_clean_after_undo_all() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        let mut input = "a\nb\nc\n.\n".as_bytes();

        buffer
            .prepare_append(&mut input, Some(Address(2, 2)))
            .unwrap();

        buffer.prepare_delete(Some(Address(4, 7))).unwrap();

        input = "x\ny\nz\n.\n".as_bytes();
        buffer
            .prepare_append(&mut input, Some(Address(0, 0)))
            .unwrap();

        buffer.do_undo().unwrap();

        buffer.do_undo().unwrap();

        buffer.do_undo().unwrap();

        assert!(!buffer.is_dirty());

        buffer.do_undo().unwrap();
        assert!(!buffer.is_dirty()); // still not dirty
    }

    #[test]
    fn do_redo_multi() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        let buffer_orig = buffer.clone();
        assert_eq!(6, buffer.current_line());

        let mut input = "a\nb\nc\n.\n".as_bytes();
        buffer
            .prepare_append(&mut input, Some(Address(2, 2)))
            .unwrap();
        let expected_1 = EditBuffer::from(vec!["1\n", "2", "a", "b", "c", "3", "4", "5", "6"]);
        assert_eq!(&expected_1[..], &buffer[..]);
        assert_eq!(5, buffer.current_line());

        buffer.prepare_delete(Some(Address(4, 7))).unwrap();
        let expected_final = EditBuffer::from(vec!["1\n", "2", "a", "5", "6"]);
        assert_eq!(&expected_final[..], &buffer[..]);

        buffer.do_undo().unwrap();
        assert_eq!(buffer[..], expected_1[..]);
        buffer.do_undo().unwrap();
        assert_eq!(buffer[..], buffer_orig[..]);
        buffer.do_undo().unwrap();
        assert_eq!(buffer[..], buffer_orig[..]); // buffer unchanged

        buffer.do_redo().unwrap();
        assert_eq!(&expected_1[..], &buffer[..]);

        buffer.do_redo().unwrap();
        assert_eq!(buffer[..], expected_final[..]);

        buffer.do_redo().unwrap();
        assert_eq!(buffer[..], expected_final[..]); // buffer unchanged
    }

    #[test]
    fn print_filename_none_set() {
        let mut buffer = EditBuffer::from(vec!["1\r\n", "2", "3"]);
        let mut output = Vec::new();
        buffer.do_file(&mut output, None).unwrap();
        assert_eq!(
            str::from_utf8(&output[..]).unwrap(),
            "No current filename\n"
        );
        assert_eq!(None, buffer.filename());
    }

    #[test]
    fn set_filename() {
        let new_filename = "a_new_filename.txt\n";
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3"]);
        let mut output = Vec::new();
        assert_eq!(None, buffer.filename());
        buffer
            .do_file(&mut output, Some(Path::new(new_filename.trim())))
            .unwrap();
        assert_eq!(str::from_utf8(&output[..]).unwrap(), new_filename);
        assert_eq!(Some(Path::new(new_filename.trim())), buffer.filename());
    }

    #[test]
    fn print_filename() {
        let new_filename = "a_new_filename.txt\n";
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3"]);
        let mut output = Vec::new();
        assert_eq!(None, buffer.filename());
        buffer
            .do_file(&mut output, Some(Path::new(new_filename.trim())))
            .unwrap();
        assert_eq!(Some(Path::new(new_filename.trim())), buffer.filename());
        output.clear();
        buffer.do_file(&mut output, None).unwrap();
        assert_eq!(str::from_utf8(&output[..]).unwrap(), new_filename);
    }

    #[test]
    fn change_filename() {
        let orig_filename = "a_filename.md";
        let new_filename = "a_new_filename.txt\n";
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3"]);
        let mut output = Vec::new();
        buffer
            .do_file(&mut output, Some(Path::new(orig_filename)))
            .unwrap();
        output.clear();
        buffer
            .do_file(&mut output, Some(Path::new(new_filename.trim())))
            .unwrap();
        assert_eq!(str::from_utf8(&output[..]).unwrap(), new_filename);
        assert_eq!(Some(Path::new(new_filename.trim())), buffer.filename());
    }

    #[test]
    fn do_insert_past_end_gives_error_before_input() {
        let mut buffer = EditBuffer::new();
        let mut input = "one\n.\n".as_bytes();
        let expected = "one\n.\n".as_bytes();
        let res = buffer
            .prepare_insert(&mut input, Some(Address(2, 2)))
            .expect_err("invalid addr");
        assert_eq!(0, buffer.len());
        assert_eq!(input, expected);
        assert!(matches!(res, Error::InvalidAddress));
    }

    #[test]
    fn do_insert_one_to_empty_buffer() {
        let mut buffer = EditBuffer::new();
        let expected = EditBuffer::from(vec!["one\n"]);
        let mut input = "one\n.\n".as_bytes();
        buffer
            .prepare_insert(&mut input, Some(Address(0, 0)))
            .unwrap();
        assert_eq!(1, buffer.current_line);
        assert_eq!(1, buffer.len());
        assert_eq!(&expected[..], &buffer[..]);
    }

    #[test]
    fn do_insert_empty_buffer() {
        let mut buffer = EditBuffer::new();
        let expected = EditBuffer::from(vec!["a\n", "b", "c"]);
        let mut input = "a\nb\nc\n.\n".as_bytes();
        buffer
            .prepare_insert(&mut input, Some(Address(0, 0)))
            .unwrap();
        assert_eq!(3, buffer.current_line);
        assert_eq!(3, buffer.len());
        assert!(&expected[..].eq(&buffer[..]));
    }

    #[test]
    fn do_insert_non_empty_at_0() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3"]);
        let expected = EditBuffer::from(vec!["a\n", "b", "c", "1", "2", "3"]);
        let mut input = "a\nb\nc\n.\n".as_bytes();
        buffer
            .prepare_insert(&mut input, Some(Address(0, 0)))
            .unwrap();
        assert_eq!(3, buffer.current_line);
        assert_eq!(6, buffer.len());
        assert!(&expected[..].eq(&buffer[..]));
    }

    #[test]
    fn do_insert_span_address() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        let mut input = "a\nb\nc\n.\n".as_bytes();
        let expected = EditBuffer::from(vec!["1\n", "2", "a", "b", "c", "3", "4", "5", "6"]);
        buffer
            .prepare_insert(&mut input, Some(Address(2, 3)))
            .unwrap();
        assert_eq!(5, buffer.current_line);
        assert_eq!(9, buffer.len());
        assert!(&expected[..].eq(&buffer[..]));
    }

    #[test]
    fn do_insert_at_end() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3"]);
        let mut input = "a\nb\nc\n.\n".as_bytes();
        let expected = EditBuffer::from(vec!["1\n", "2", "a", "b", "c", "3"]);
        buffer
            .prepare_insert(&mut input, Some(Address(3, 3)))
            .unwrap();
        assert_eq!(5, buffer.current_line);
        assert_eq!(6, buffer.len());
        assert!(&expected[..].eq(&buffer[..]));
    }

    #[test]
    fn do_insert_of_zero_lines() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3"]);
        let mut input = ".\n".as_bytes();
        let expected = EditBuffer::from(vec!["1\n", "2", "3"]);
        assert_eq!(3, buffer.current_line());
        buffer
            .prepare_insert(&mut input, Some(Address(2, 2)))
            .unwrap();
        assert_eq!(2, buffer.current_line);
        assert_eq!(3, buffer.len());
        assert!(&expected[..].eq(&buffer[..]));
    }

    //    #[test]
    //    fn do_edit_no_file() {
    //        let mut buffer = EditBuffer::new();
    //        let mut output = Vec::new();
    //        let res = buffer
    //            .do_edit(&mut output, None, None)
    //            .expect_err("no filename");
    //        assert!(matches!(res, Error::NoFilename));
    //    }
    //
    //    #[test]
    //    fn do_edit_file_not_found() {
    //        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3"]);
    //        let file_to_edit = "a_file_that_is_not_there.ext";
    //        let mut output = Vec::new();
    //        buffer
    //            .do_edit(&mut output, Some(Path::new(file_to_edit)), None)
    //            .unwrap();
    //        assert!(buffer.is_empty());
    //        assert!(!buffer.is_dirty());
    //        assert_eq!(buffer.filename(), Some(Path::new(file_to_edit)));
    //    }
    //
    //    #[test]
    //    fn do_edit_default_filename() {
    //        let filename = Path::new(r"test/assets/text_with_final_eol.txt");
    //        let mut buffer = EditBuffer::new();
    //        let mut output = Vec::new();
    //        buffer.do_file(&mut output, Some(filename)).unwrap();
    //        assert_eq!(buffer.filename(), Some(filename));
    //        output.clear();
    //        buffer.do_edit(&mut output, None, None).unwrap();
    //        assert_eq!(&b"312\n"[..], &output[..]);
    //    }
    //
    //    #[test]
    //    fn do_edit() {
    //        let filename = Path::new(r"test/assets/text_with_final_eol.txt");
    //        let mut buffer = EditBuffer::new();
    //        let mut output = Vec::new();
    //        buffer.do_edit(&mut output, Some(filename), None).unwrap();
    //        assert_eq!(&b"312\n"[..], &output[..]);
    //    }
    //
    //    #[test]
    //    fn do_edit_no_final_eol() {
    //        let filename = Path::new(r"test/assets/text_with_no_final_eol.txt");
    //        let mut buffer = EditBuffer::new();
    //        let mut output = Vec::new();
    //        buffer.do_edit(&mut output, Some(filename), None).unwrap();
    //        let expected = b"missing line terminator appended\n319\n";
    //        assert_eq!(&output[..], &expected[..]);
    //    }
    //
    //    #[test]
    //    fn read_replace_io_error() {
    //        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3"]);
    //        let reader = BadReader {};
    //        let source = Some(BufReader::new(reader));
    //        let mut output = Vec::new();
    //        let mut data = EditData {
    //            ..Default::default()
    //        };
    //        let res = buffer.read_replace(&mut output, source, Some(&mut data));
    //        assert!(matches!(res, Err(Error::Read(_))));
    //    }
    //
    //    #[test]
    //    fn read_replace_zero_length() {
    //        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3"]);
    //        let reader = &b""[..];
    //        let source = Some(BufReader::new(reader));
    //        let mut output = Vec::new();
    //        let mut data = EditData {
    //            ..Default::default()
    //        };
    //        buffer
    //            .read_replace(&mut output, source, Some(&mut data))
    //            .unwrap();
    //        assert_eq!(buffer[..], Vec::<String>::new());
    //    }
    //
    //    #[test]
    //    fn read_replace_empty_buffer() {
    //        let mut buffer = EditBuffer::new();
    //        let reader = &b"one\ntwo\nthree\n"[..];
    //        let source = Some(BufReader::new(reader));
    //        let mut output = Vec::new();
    //        assert_eq!(buffer.current_line(), 0);
    //
    //        let mut data = EditData {
    //            ..Default::default()
    //        };
    //        buffer
    //            .read_replace(&mut output, source, Some(&mut data))
    //            .unwrap();
    //        assert_eq!(buffer[..], vec!["one\n", "two\n", "three\n"]);
    //        assert_eq!(buffer.current_line(), 3usize);
    //    }
    //
    //    #[test]
    //    fn read_replace_non_empty_buffer() {
    //        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4"]);
    //        let reader = &b"one\ntwo\nthree\n"[..];
    //        let source = Some(BufReader::new(reader));
    //        let mut output = Vec::new();
    //        assert_eq!(buffer.current_line(), 4);
    //
    //        let mut data = EditData {
    //            ..Default::default()
    //        };
    //        buffer
    //            .read_replace(&mut output, source, Some(&mut data))
    //            .unwrap();
    //        assert_eq!(buffer[..], vec!["one\n", "two\n", "three\n"]);
    //        assert_eq!(buffer.current_line(), 3usize);
    //    }
    //
    //    #[test]
    //    fn read_replace_prints_chars_read() {
    //        let mut buffer = EditBuffer::new();
    //        let reader = &b"one\ntwo\nthree\n"[..];
    //        let source = Some(BufReader::new(reader));
    //        let mut output = Vec::new();
    //        assert_eq!(buffer.current_line(), 0);
    //
    //        let mut data = EditData {
    //            ..Default::default()
    //        };
    //        buffer
    //            .read_replace(&mut output, source, Some(&mut data))
    //            .unwrap();
    //        assert_eq!(&output[..], &b"14\n"[..]);
    //    }
}
