// EditBuffer keeps track of everything specific to a single buffer in the
// editor. All public interface uses one based indexing, and any such function
// is responsible for translating into the 0 based indexing of the Vec<String>
// containing the lines of text.
mod operation;

use core::cmp::Ordering;
use core::fmt::{self, Display, Formatter};
use core::ops::{Index, Range, RangeFrom, RangeFull, RangeInclusive};
use core::slice::Iter;
use std::collections::hash_map::DefaultHasher;
use std::fs::{File, OpenOptions};
use std::hash::{Hash, Hasher};
use std::io::{self, BufRead, BufReader, Write};
use std::path::PathBuf;

use crate::command::{Address, Cmd};
use crate::edit_buffer::operation::{AppendData, DeleteData, EditData, Op};
use crate::num_utils::NumUtils;

#[derive(Debug, Clone, Hash, Default)]
pub struct Revert {
    current_line: usize,
    commands: Vec<Cmd>,
    clean_fingerprint: Option<Option<u64>>,
}

#[derive(Debug, Clone)]
pub struct EditBuffer {
    text: Vec<String>,
    current_line: usize,
    filename: Option<PathBuf>,
    default_eol: Option<&'static str>,
    undo_stack: Vec<Op>,
    redo_stack: Vec<Op>,
    clean_fingerprint: Option<u64>,
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
}

impl std::error::Error for Error {}

impl Display for Error {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
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
                let mut line = v.to_string();
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
    fn index(&self, index: usize) -> &String {
        self.get(index).expect("Out of bounds access")
    }
}

impl Index<Range<usize>> for EditBuffer {
    type Output = [String];

    #[inline]
    fn index(&self, index: Range<usize>) -> &[String] {
        assert!(index.start > 0 && index.end > 0, "Invalid range");
        &self.text[index.start - 1..index.end - 1]
    }
}

impl Index<RangeInclusive<usize>> for EditBuffer {
    type Output = [String];

    #[inline]
    fn index(&self, index: RangeInclusive<usize>) -> &[String] {
        assert!(*index.start() > 0 && *index.end() > 0, "Invalid range");
        &self.text[*index.start() - 1..=*index.end() - 1]
    }
}

impl Index<RangeFrom<usize>> for EditBuffer {
    type Output = [String];

    #[inline]
    fn index(&self, index: RangeFrom<usize>) -> &[String] {
        assert!(index.start > 0, "Invalid range");
        &self.text[index.start - 1..]
    }
}

impl Index<RangeFull> for EditBuffer {
    type Output = [String];

    #[inline]
    fn index(&self, index: RangeFull) -> &[String] {
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
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            clean_fingerprint: None,
        }
    }

    /// Creates a new empty `EditBuffer` with room for at least `capacity`
    /// lines of text. Specifying a capacity is useful to reduce the number
    /// of reallocations necessary as lines are added to the `EditBuffer`.
    ///
    /// The capacity can be queried with the [`capacity`] method.
    ///
    /// If the capacity given is `0`, this will be identical to the [`new`]
    /// method, and no allocation will occur.
    ///
    /// [`capacity`]: EditBuffer::capacity
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
    /// Returns this `EditBuffer`'s capacity, in bytes.
    pub fn capacity(&self) -> usize {
        self.text.capacity()
    }

    /// Returns this `EditBuffer`'s length, in lines.
    pub fn len(&self) -> usize {
        self.text.len()
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// Returns true if buffer has been changed since last write.
    pub fn is_dirty(&self) -> bool {
        self.clean_fingerprint.map_or_else(
            || !self.undo_stack.is_empty(),
            |f| f != fingerprint(&self.undo_stack),
        )
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

    pub fn filename(&self) -> &Option<PathBuf> {
        &self.filename
    }

    pub fn get(&self, index: usize) -> Option<&String> {
        match index {
            0 => None,
            _ => self.text.get(index - 1),
        }
    }

    /// Reads lines from reader into the buffer at the specified line.
    ///
    /// Default EOL auto-detect:
    ///     If this call to read is on a buffer that has no default EOL, then new lines
    ///     read are examined, and the default is set to the most frequently used EOL
    ///     sequence.
    ///
    /// EOL Correction:
    ///    If the final line read lacks an EOL, it will not be corrected
    ///    if it is the last line of the buffer. Otherwise missing EOLs
    ///    will be added. Added EOLs will be the default EOL for the
    ///    buffer. This is determined either by configuration, or auto-detected
    ///    (e.g., as described above, or similarly when first lines are appended
    ///    or inserted).
    ///
    /// Returns number of bytes read, or an error if read fails
    fn read<R>(&mut self, at_line: usize, mut reader: R) -> Result<ReadResult, Error>
    where
        R: BufRead,
    {
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

    fn write<W1, W2>(
        &mut self,
        output: &mut W1,
        address: Option<Address>,
        destination: &mut W2,
    ) -> Result<(), Error>
    where
        W1: Write,
        W2: Write,
    {
        let line_span = match address {
            None => 1usize..=self.len(),
            Some(Address::Line(n)) => n..=n,
            Some(Address::Span(b, e)) => b..=e,
        };

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
            self.clean_fingerprint = Some(fingerprint(&self.undo_stack));
        }
        Ok(())
    }

    fn iter(&self) -> Iter<'_, String> {
        self.text.iter()
    }

    fn execute(&mut self, output: &mut impl Write, op: &mut Op) -> Result<(), Error> {
        match op {
            Op::Append(ref mut data) => {
                let b = match data.address {
                    Some(Address::Line(line)) => line,
                    Some(Address::Span(_, last)) => last,
                    None => self.current_line,
                };

                if !data.lines.is_empty() {
                    self.text.splice(b..b, data.lines.iter().cloned());
                }
                self.current_line = b + data.lines.len();
                Ok(())
            }
            Op::Delete(ref mut data) => {
                let (b, e) = match data.address {
                    Some(Address::Line(line)) => (line, line),
                    Some(Address::Span(b, e)) => (b, e),
                    None => (self.current_line, self.current_line),
                };
                if data.lines_removed.is_empty() {
                    data.lines_removed
                        .splice(.., self.text.splice(b - 1..e, None));
                } else {
                    self.text.splice(b - 1..e, None);
                }
                self.current_line = usize::min(self.text.len(), b);
                Ok(())
            }
            Op::Edit(ref mut data) => {
                let f = File::open(&data.filename);
                let source = match f {
                    Ok(f) => Ok(Some(BufReader::new(f))),
                    Err(e) => match e.kind() {
                        io::ErrorKind::NotFound => {
                            writeln!(output, "{e}").map_err(Error::WriteOutput)?;
                            Ok(None)
                        }
                        _ => Err(e),
                    },
                }
                .map_err(Error::FileOpen)?;

                self.read_replace(output, source, data)
            }
            Op::Inverse(_) => self.revert(output, op),
        }
    }

    fn revert(&mut self, output: &mut impl Write, op: &mut Op) -> Result<(), Error> {
        match op {
            Op::Append(ref mut data) => {
                let b = match data.address {
                    Some(Address::Line(line)) => line,
                    Some(Address::Span(_, last)) => last,
                    None => data.current_line,
                };
                self.text.splice(b..b + data.lines.len(), None);
                self.current_line = data.current_line;
                Ok(())
            }
            Op::Delete(ref data) => {
                let b = match data.address {
                    Some(Address::Line(line)) => line,
                    Some(Address::Span(b, _)) => b,
                    None => data.current_line,
                } - 1;
                self.text.splice(b..b, data.lines_removed.iter().cloned());
                self.current_line = b + data.lines_removed.len();
                Ok(())
            }
            Op::Edit(ref data) => {
                self.text.splice(.., data.lines_removed.iter().cloned());
                self.current_line = data.current_line;
                Ok(())
            }
            Op::Inverse(_) => self.execute(output, op),
        }
    }

    fn read_replace(
        &mut self,
        output: &mut impl Write,
        source: Option<impl BufRead>,
        data: &mut EditData,
    ) -> Result<(), Error> {
        if data.lines_removed.is_empty() {
            data.lines_removed.append(&mut self.text);
        } else {
            self.text.clear();
        }

        if let Some(source) = source {
            let ret = self.read(0, source)?;
            match ret {
                ReadResult::EOLAdded(bytes_read) => {
                    writeln!(output, "missing line terminator appended\n{bytes_read}")
                        .map_err(Error::WriteOutput)?
                }
                ReadResult::AsIs(bytes_read) => {
                    writeln!(output, "{bytes_read}").map_err(Error::WriteOutput)?
                }
            }
        }
        Ok(())
    }

    pub fn do_append(
        &mut self,
        output: &mut impl Write,
        address: Option<Address>,
        lines: Vec<String>,
    ) -> Result<(), Error> {
        let mut op = Op::Append(AppendData {
            address,
            lines,
            current_line: self.current_line,
        });
        let res = self.execute(output, &mut op);
        self.undo_stack.push(op);
        res
    }

    pub fn do_delete(
        &mut self,
        output: &mut impl Write,
        address: Option<Address>,
    ) -> Result<(), Error> {
        let address = match address {
            Some(Address::Line(0)) | Some(Address::Span(0, _)) => Err(Error::InvalidAddress),
            None if self.current_line == 0 => Err(Error::InvalidAddress),
            _ => Ok(address),
        }?;

        let mut op = Op::Delete(DeleteData {
            address,
            lines_removed: Vec::new(),
            current_line: self.current_line,
        });
        let res = self.execute(output, &mut op);
        self.undo_stack.push(op);
        res
    }

    pub fn do_edit(
        &mut self,
        output: &mut impl Write,
        filename: &Option<PathBuf>,
        prev_command: &Option<Cmd>,
    ) -> Result<(), Error> {
        if self.is_dirty() && !matches!(prev_command, Some(Cmd::Edit(_))) {
            writeln!(
                output,
                "Unwritten changes - repeat edit command to discard changes."
            )
            .map_err(Error::WriteOutput)?;
            return Ok(());
        }

        if filename.is_some() {
            self.filename = filename.clone();
        }
        let filename = self.filename.as_ref().ok_or(Error::NoFilename)?;

        let mut op = Op::Edit(EditData {
            filename: filename.clone(),
            current_line: self.current_line,
            lines_removed: Vec::new(),
            clean_fingerprint: self.clean_fingerprint,
        });

        let res = self.execute(output, &mut op);
        self.undo_stack.push(op);
        self.clean_fingerprint = Some(fingerprint(&self.undo_stack));
        res
    }

    pub fn do_enumerate<W>(&mut self, output: &mut W, address: Option<Address>) -> Result<(), Error>
    where
        W: Write,
    {
        let span = match address {
            Some(Address::Line(n)) => n..=n,
            Some(Address::Span(first, last)) => first..=last,
            None => {
                if self.current_line == 0 {
                    return Err(Error::InvalidAddress);
                }
                self.current_line..=self.current_line
            }
        };

        if *span.start() < 1
            || *span.start() > self.len()
            || *span.end() < 1
            || *span.end() > self.len()
        {
            return Err(Error::InvalidAddress);
        }

        let width = span.end().decimal_digits();
        let start = *span.start();
        self.current_line = *span.end();

        for (i, l) in self[span].iter().enumerate() {
            output
                .write_all(format!("{:>width$}  {l}", start + i).as_bytes())
                .map_err(Error::WriteOutput)?;
        }
        output.flush().map_err(Error::WriteOutput)?;
        Ok(())
    }

    pub fn do_file<W>(&mut self, output: &mut W, filename: &Option<PathBuf>) -> Result<(), Error>
    where
        W: Write,
    {
        if filename.is_some() {
            self.filename = filename.clone();
        }

        match &self.filename {
            None => output
                .write_all(b"No current filename\n")
                .map_err(Error::WriteOutput),
            Some(f) => output
                .write_all(format!("{}\n", f.display()).as_bytes())
                .map_err(Error::WriteOutput),
        }
    }

    pub fn do_null<W>(&mut self, output: &mut W, address: Option<Address>) -> Result<(), Error>
    where
        W: Write,
    {
        match address {
            None => {
                if self.is_empty() || self.current_line == self.len() {
                    return Err(Error::InvalidAddress);
                }
                self.do_print(output, Some(Address::Line(self.current_line + 1)))
            }
            _ => self.do_print(output, address),
        }
    }

    pub fn do_print<W>(&mut self, output: &mut W, address: Option<Address>) -> Result<(), Error>
    where
        W: Write,
    {
        let span = match address {
            Some(Address::Line(n)) => n..=n,
            Some(Address::Span(first, last)) => first..=last,
            None => {
                if self.current_line == 0 {
                    return Err(Error::InvalidAddress);
                }
                self.current_line..=self.current_line
            }
        };

        if *span.start() < 1
            || *span.start() > self.len()
            || *span.end() < 1
            || *span.end() > self.len()
        {
            return Err(Error::InvalidAddress);
        }

        self.current_line = *span.end();
        for l in &self[span] {
            output.write_all(l.as_bytes()).map_err(Error::WriteOutput)?;
        }
        output.flush().map_err(Error::WriteOutput)?;
        Ok(())
    }

    pub fn do_undo(&mut self, output: &mut impl Write) -> Result<(), Error> {
        match self.undo_stack.pop() {
            Some(mut op) => {
                let res = self.revert(output, &mut op);
                self.redo_stack.push(op);
                res
            }
            None => Ok(()),
        }
    }

    pub fn do_write<W>(
        &mut self,
        output: &mut W,
        address: Option<Address>,
        filename: &Option<PathBuf>,
    ) -> Result<(), Error>
    where
        W: Write,
    {
        if self.filename.is_none() {
            if filename.is_none() {
                return Err(Error::NoFilename);
            } else {
                self.filename = filename.clone();
            }
        }

        let filename = filename.as_ref().unwrap_or(self.filename.as_ref().unwrap());

        let mut dest = OpenOptions::new()
            .write(true)
            .create(true)
            .open(filename)
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

fn compute_default_eol<I, T>(lines: I) -> &'static str
where
    I: IntoIterator<Item = T>,
    T: AsRef<str>,
{
    let native_eol = if std::env::consts::FAMILY == "windows" {
        "\r\n"
    } else {
        "\n"
    };
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
        _ => native_eol,
    }
}

fn fingerprint<T>(t: &T) -> u64
where
    T: Hash,
{
    let mut h = DefaultHasher::new();
    t.hash(&mut h);
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::io::{BufReader, Read};
    use std::ops::Deref;

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
        let _res = buf
            .write(&mut output, Some(Address::Span(1, 2)), &mut dummy_file)
            .expect_err("io error");
        assert!(matches!(_res, Error::WriteLines(_)));
    }

    #[test]
    fn write_one_line() {
        let mut buf = EditBuffer::from(vec!["1\n", "2", "3"]);
        let mut dummy_file = Vec::new();
        let mut output = Vec::new();
        buf.write(&mut output, Some(Address::Line(2)), &mut dummy_file)
            .expect("successful write");
        assert_eq!(b"2\n", &output[..]);
    }

    #[test]
    fn write_many_lines() {
        let mut buf = EditBuffer::from(vec!["1\r\n", "2", "3", "4", "5", "6"]);
        let mut dummy_file = Vec::new();
        let mut output = Vec::new();
        buf.write(&mut output, Some(Address::Span(1, 6)), &mut dummy_file)
            .expect("successful write");
        assert_eq!(b"18\n", &output[..]);
    }

    #[test]
    fn write_empty_buffer() {
        let mut buf = EditBuffer::new();
        let mut dummy_file = Vec::new();
        let mut output = Vec::new();
        buf.write(&mut output, None, &mut dummy_file)
            .expect("successful write");
        assert_eq!(b"0\n", &output[..]);
    }

    #[test]
    fn write_no_addr_leaves_clean_buffer() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3"]);
        assert!(!buffer.is_dirty());
        let mut output = Vec::new();
        buffer
            .do_append(
                &mut output,
                Some(Address::Line(0)),
                vec!["one more line\n".to_owned()],
            )
            .expect("line appended");
        assert!(buffer.is_dirty());
        let mut dummy_file = Vec::new();
        output.clear();
        buffer
            .write(&mut output, None, &mut dummy_file)
            .expect("successful write");
        assert_eq!(b"20\n", &output[..]);
        assert!(!buffer.is_dirty());
    }

    #[test]
    fn write_full_buffer_leaves_clean_buffer() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3"]);
        assert!(!buffer.is_dirty());
        let mut output = Vec::new();
        buffer
            .do_append(
                &mut output,
                Some(Address::Line(0)),
                vec!["one more line\n".to_owned()],
            )
            .expect("line appended");
        assert!(buffer.is_dirty());
        let mut dummy_file = Vec::new();
        output.clear();
        buffer
            .write(
                &mut output,
                Some(Address::Span(1, buffer.len())),
                &mut dummy_file,
            )
            .expect("successful write");
        assert_eq!(b"20\n", &output[..]);
        assert!(!buffer.is_dirty());
    }

    #[test]
    fn write_partial_buffer_leaves_dirty_buffer() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3"]);
        assert!(!buffer.is_dirty());
        let mut output = Vec::new();
        buffer
            .do_append(
                &mut output,
                Some(Address::Line(0)),
                vec!["one more line\n".to_owned()],
            )
            .expect("line appended");
        assert!(buffer.is_dirty());
        let mut dummy_file = Vec::new();
        output.clear();
        buffer
            .write(&mut output, Some(Address::Span(1, 2)), &mut dummy_file)
            .expect("successful write");
        assert_eq!(b"16\n", &output[..]);
        assert!(buffer.is_dirty());
    }

    /////
    // EditBuffer creation tests

    #[test]
    fn new_buffer_has_zero_capacity() {
        let buffer = EditBuffer::new();
        assert_eq!(buffer.capacity(), 0);
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
        assert_eq!(buffer.capacity(), INIT_CAPACITY);
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

    fn new_input_buf<S>(content: &[S]) -> Vec<u8>
    where
        S: Deref<Target = str>,
    {
        let mut input = Vec::new();
        for line in content {
            input.extend(line.bytes());
        }
        input
    }

    macro_rules! read_test {
        { $name:ident,
        initial: $initial:expr,
        added: $added:expr,
        at: $at:expr,
        expect: $expect:expr,
        bytes read: $bytes_read:expr,
        eol added: $eol_added:expr,
        current line after: $current_line:expr$(,)? } => {
            #[test]
            fn $name() {
                let expected_read_result = if $eol_added {
                    ReadResult::EOLAdded($bytes_read)
                } else {
                    ReadResult::AsIs($bytes_read)
                };

                let mut buffer = $initial;
                let added = $added;
                let input = new_input_buf(&added[..]);
                let read_result = buffer
                    .read($at, &input[..])
                    .expect("lines read");

                assert_eq!(read_result, expected_read_result);

                assert_eq!($expect,
                        buffer.text,
                        "expected text: {:?}, got {:?}", $expect, &buffer.text
                );
                assert_eq!($current_line,
                        buffer.current_line(),
                        "expected current_line: {}, got {}", $current_line, buffer.current_line()
                );
            }
        };
    }

    read_test! {
        read_to_empty_buf_all_lf,
        initial: EditBuffer::new(),
        added: ["Line1\n", "Line2\n", "Line3\n",],
        at: 0,
        expect: vec!["Line1\n", "Line2\n", "Line3\n",],
        bytes read: 18,
        eol added: false,
        current line after: 3,
    }

    read_test! {
        read_to_empty_buf_all_lf_no_final,
        initial: EditBuffer::new(),
        added: ["Line1\n", "Line2\n", "Line3",],
        at: 0,
        expect: vec!["Line1\n", "Line2\n", "Line3\n",],
        bytes read: 17,
        eol added: true,
        current line after: 3,
    }

    read_test! {
        read_insert_at_start,
        initial: EditBuffer::from(vec!["1\r\n", "2", "3",]),
        added: ["New1\n", "New2\n", "New3\n"],
        at: 0,
        expect: vec![
            "New1\n", "New2\n", "New3\n", "1\r\n", "2\r\n", "3\r\n",
        ],
        bytes read: 15,
        eol added: false,
        current line after: 3,
    }

    read_test! {
        read_append_all_lf,
        initial: EditBuffer::from(vec!["Line1\n", "Line2\n", "Line3\n",]),
        added: ["New1\n", "New2\n", "New3\n"],
        at: 3,
        expect: vec![
            "Line1\n", "Line2\n", "Line3\n", "New1\n", "New2\n", "New3\n",
        ],
        bytes read: 15,
        eol added: false,
        current line after: 6,
    }

    read_test! {
        read_append_most_lf_no_final,
        initial: EditBuffer::from(vec!["Line1\n", "Line2\r\n", "Line3\n", "Line4",]),
        added: ["New1\n", "New2\n", "New3"],
        at: 4,
        expect: vec![
            "Line1\n", "Line2\r\n", "Line3\n", "Line4\n", "New1\n", "New2\n", "New3\n",
        ],
        bytes read: 14,
        eol added: true,
        current line after: 7,
    }

    read_test! {
        read_append_most_crlf_no_final,
        initial: EditBuffer::from(vec!["Line1\r\n", "Line2\r\n", "Line3\n", "Line4",]),
        added: ["New1\r\n", "New2\n", "New3"],
        at: 4,
        expect: vec![
            "Line1\r\n", "Line2\r\n", "Line3\n", "Line4\r\n", "New1\r\n", "New2\n", "New3\r\n",
        ],
        bytes read: 15,
        eol added: true,
        current line after: 7,
    }

    read_test! {
        read_append_all_lf_no_final,
        initial: EditBuffer::from(vec!["Line1\n", "Line2\n", "Line3",]),
        added: ["New1\n", "New2\n", "New3\n"],
        at: 3,
        expect: vec![
            "Line1\n", "Line2\n", "Line3\n", "New1\n", "New2\n", "New3\n",
        ],
        bytes read: 15,
        eol added: false,
        current line after: 6,
    }

    read_test! {
        read_append_all_crlf_no_final,
        initial: EditBuffer::from(vec!["Line1\r\n", "Line2\r\n", "Line3",]),
        added: ["New1\r\n", "New2\r\n", "New3\r\n"],
        at: 3,
        expect: vec![
            "Line1\r\n", "Line2\r\n", "Line3\r\n", "New1\r\n", "New2\r\n", "New3\r\n",
        ],
        bytes read: 18,
        eol added: false,
        current line after: 6,
    }

    #[test]
    fn read_append_equal_eol_no_final() {
        let initial = vec!["Line1\n", "Line2\r\n", "Line3"];
        let mut buffer = EditBuffer::from(initial);
        assert!(buffer
            .default_eol
            .is_some_and(|eol| eol == compute_native_eol()));
        let def_eol = buffer.default_eol.unwrap();
        assert!(buffer[..].last().expect("last line").ends_with(def_eol));

        let at = 3;
        let added = ["New1\n", "New2\r\n", "New3"];
        let input = new_input_buf(&added[..]);
        let ret = buffer
            .read(at, &input[..])
            .expect("Error reading added lines");

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
        assert!(matches!(ret, ReadResult::EOLAdded(15)));
    }

    read_test! {
        read_insert_all_lf,
        initial: EditBuffer::from(vec!["Line1\n", "Line2\n", "Line3\n",]),
        added: ["New1\n", "New2\n", "New3\n"],
        at: 2,
        expect: vec![
            "Line1\n", "Line2\n", "New1\n", "New2\n", "New3\n", "Line3\n",
        ],
        bytes read: 15,
        eol added: false,
        current line after: 5,
    }

    read_test! {
        read_insert_most_lf_no_final,
        initial: EditBuffer::from(vec!["Line1\n", "Line2\r\n", "Line3\n", "Line4\n",]),
        added: ["New1\n", "New2\n", "New3"],
        at: 2,
        expect: vec![
            "Line1\n",
            "Line2\r\n",
            "New1\n",
            "New2\n",
            "New3\n",
            "Line3\n",
            "Line4\n",
        ],
        bytes read: 14,
        eol added: true,
        current line after: 5,
    }

    read_test! {
        read_insert_most_crlf_no_final,
        initial: EditBuffer::from(vec!["Line1\r\n", "Line2\r\n", "Line3\n", "Line4\r\n",]),
        added: ["New1\r\n", "New2\n", "New3"],
        at: 2,
        expect: vec![
            "Line1\r\n",
            "Line2\r\n",
            "New1\r\n",
            "New2\n",
            "New3\r\n",
            "Line3\n",
            "Line4\r\n",
        ],
        bytes read: 15,
        eol added: true,
        current line after: 5,
    }

    read_test! {
        read_insert_all_lf_no_final,
        initial: EditBuffer::from(vec!["Line1\n", "Line2\n", "Line3\n", "Line4\n",]),
        added: ["New1\n", "New2\n", "New3"],
        at: 2,
        expect: vec![
            "Line1\n",
            "Line2\n",
            "New1\n",
            "New2\n",
            "New3\n",
            "Line3\n",
            "Line4\n",
        ],
        bytes read: 14,
        eol added: true,
        current line after: 5,
    }

    read_test! {
        read_insert_all_crlf_no_final,
        initial: EditBuffer::from(vec!["Line1\r\n", "Line2\r\n", "Line3\r\n", "Line4\r\n",]),
        added: ["New1\r\n", "New2\r\n", "New3"],
        at: 2,
        expect: vec![
            "Line1\r\n",
            "Line2\r\n",
            "New1\r\n",
            "New2\r\n",
            "New3\r\n",
            "Line3\r\n",
            "Line4\r\n",
        ],
        bytes read: 16,
        eol added: true,
        current line after: 5,
    }

    #[test]
    fn read_insert_equal_eol_no_final() {
        let initial = vec!["Line1\r\n", "Line2\n", "Line3\n", "Line4\r\n"];
        let mut buffer = EditBuffer::from(initial);

        let at = 2;
        let added = ["New1\r\n", "New2\r\n", "New3"];
        let input = new_input_buf(&added[..]);
        let ret = buffer
            .read(at, &input[..])
            .expect("Error reading added lines");

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
        assert!(matches!(ret, ReadResult::EOLAdded(16)));
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

    /////
    // Indexing tests

    #[test]
    fn usize_index() {
        let buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        assert_eq!("1\n", buffer[1]);
        assert_eq!("6\n", buffer[6]);
    }

    #[test]
    #[should_panic]
    fn zero_index_panics() {
        let buffer = EditBuffer::from(vec!["1"]);
        let _ = &buffer[0];
    }

    #[test]
    #[should_panic]
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
    #[should_panic]
    fn zero_based_range_panics() {
        let buffer = EditBuffer::from(vec!["1", "2"]);
        let _ = &buffer[0..2];
    }

    #[test]
    #[should_panic]
    fn zero_based_range_inclusive_panics() {
        let buffer = EditBuffer::from(vec!["1", "2"]);
        let _ = &buffer[0..=1];
    }

    #[test]
    #[should_panic]
    #[allow(clippy::reversed_empty_ranges)]
    fn zero_terminated_range_panics() {
        let buffer = EditBuffer::from(vec!["1", "2"]);
        let _ = &buffer[1..0];
    }

    #[test]
    #[should_panic]
    #[allow(clippy::reversed_empty_ranges)]
    fn zero_terminated_range_inclusive_panics() {
        let buffer = EditBuffer::from(vec!["1", "2"]);
        let _ = &buffer[1..=0];
    }

    #[test]
    #[should_panic]
    fn range_too_far_beyond_end_panics() {
        let buffer = EditBuffer::from(vec!["1", "2", "3"]);
        let _ = &buffer[3..5];
    }

    #[test]
    #[should_panic]
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
    #[should_panic]
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
    #[should_panic]
    fn set_current_line_bad_index() {
        let mut buffer = EditBuffer::from(vec!["1", "2", "3"]);
        buffer.set_current_line(0);
    }

    #[test]
    #[should_panic]
    fn set_current_line_beyond_end() {
        let mut buffer = EditBuffer::from(vec!["1", "2", "3"]);
        buffer.set_current_line(99);
    }

    /////
    // cmd impl tests

    #[test]
    fn do_null_no_addr() {
        let mut output = Vec::new();
        let mut buffer = EditBuffer::from(vec!["1\r\n", "2", "3"]);
        buffer.set_current_line(2);
        buffer.do_null(&mut output, None).expect("successful print");
        assert_eq!(&output[..], b"3\r\n");
    }

    #[test]
    fn do_null_single_line() {
        let mut output = Vec::new();
        let mut buffer = EditBuffer::from(vec!["1\r\n", "2", "3"]);
        buffer.set_current_line(2);
        buffer
            .do_null(&mut output, Some(Address::Line(3)))
            .expect("successful print");
        assert_eq!(&output[..], b"3\r\n");
    }

    #[test]
    fn do_null_span() {
        let mut output = Vec::new();
        let mut buffer = EditBuffer::from(vec!["1\r\n", "2", "3", "4", "5", "6"]);
        buffer.set_current_line(5);
        buffer
            .do_null(&mut output, Some(Address::Span(2, 4)))
            .expect("successful print");
        assert_eq!(&output[..], b"2\r\n3\r\n4\r\n");
    }

    #[test]
    fn do_null_sets_current_line() {
        let mut output = Vec::new();
        let mut buffer = EditBuffer::from(vec!["1\r\n", "2", "3", "4", "5", "6"]);
        buffer.set_current_line(5);
        buffer
            .do_null(&mut output, Some(Address::Span(2, 4)))
            .expect("successful print");
        assert_eq!(4, buffer.current_line());
    }

    #[test]
    fn do_null_empty_buffer_gives_error() {
        let mut output = Vec::new();
        let mut buffer = EditBuffer::new();
        let res = buffer
            .do_null(&mut output, None)
            .expect_err("invalid address");
        assert!(matches!(res, Error::InvalidAddress));
        let res = buffer
            .do_null(&mut output, Some(Address::Line(0)))
            .expect_err("invalid address");
        assert!(matches!(res, Error::InvalidAddress));
    }

    #[test]
    fn enumerate_empty_buffer_error() {
        let mut output = Vec::new();
        let mut buffer = EditBuffer::new();
        let res = buffer
            .do_enumerate(&mut output, None)
            .expect_err("invalid address");
        assert!(matches!(res, Error::InvalidAddress));
        let res = buffer
            .do_enumerate(&mut output, Some(Address::Line(1)))
            .expect_err("invalid address");
        assert!(matches!(res, Error::InvalidAddress));
    }

    #[test]
    fn enumerate_sm_buffer() {
        let mut output = Vec::new();
        let mut buffer =
            EditBuffer::from(vec!["1\r\n", "2", "3", "4", "5", "6", "7", "8", "9", "10"]);
        buffer.set_current_line(2);
        buffer
            .do_enumerate(&mut output, None)
            .expect("lines enumerated");
        assert_eq!(&output[..], b"2  2\r\n", "output line 2");
    }

    #[test]
    fn enumerate_sets_current_line() {
        let mut output = Vec::new();
        let mut buffer =
            EditBuffer::from(vec!["1\r\n", "2", "3", "4", "5", "6", "7", "8", "9", "10"]);
        buffer.set_current_line(2);
        buffer
            .do_enumerate(&mut output, Some(Address::Span(6, 9)))
            .expect("lines enumerated");
        assert_eq!(9usize, buffer.current_line(), "current line");
    }

    #[test]
    fn enumerate_lg_buffer() {
        let mut output = Vec::new();
        let mut buffer =
            EditBuffer::from(vec!["1\r\n", "2", "3", "4", "5", "6", "7", "8", "9", "10"]);
        let mut line = String::new();
        for i in 11..=1024 {
            line.clear();
            line.push_str(&format!("{i}\r\n.\n"));
            buffer
                .do_append(
                    &mut output,
                    Some(Address::Line(buffer.len())),
                    vec![line.clone()],
                )
                .expect("line appended");
        }
        buffer.set_current_line(2);
        assert_eq!(1024, buffer.len());
        output.clear();
        buffer
            .do_enumerate(&mut output, Some(Address::Span(4, 900)))
            .expect("lines enumerated");
        let expected = b"  4  4\r\n";
        assert_eq!(&expected[..], &output[0..expected.len()]);
        output.clear();
        buffer
            .do_enumerate(&mut output, Some(Address::Line(999)))
            .expect("Line enumerated");
        let expected = b"999  999\r\n";
        assert_eq!(&expected[..], &output[0..expected.len()]);
    }

    #[test]
    fn do_print_no_addr() {
        let mut output = Vec::new();
        let mut buffer = EditBuffer::from(vec!["1\r\n", "2", "3"]);
        buffer.set_current_line(2);
        buffer
            .do_print(&mut output, None)
            .expect("successful print");
        assert_eq!(&output[..], b"2\r\n");
    }

    #[test]
    fn do_print_single_line() {
        let mut output = Vec::new();
        let mut buffer = EditBuffer::from(vec!["1\r\n", "2", "3"]);
        buffer.set_current_line(2);
        buffer
            .do_print(&mut output, Some(Address::Line(3)))
            .expect("successful print");
        assert_eq!(&output[..], b"3\r\n");
    }

    #[test]
    fn do_print_span() {
        let mut output = Vec::new();
        let mut buffer = EditBuffer::from(vec!["1\r\n", "2", "3", "4", "5", "6"]);
        buffer.set_current_line(5);
        buffer
            .do_print(&mut output, Some(Address::Span(2, 4)))
            .expect("successful print");
        assert_eq!(&output[..], b"2\r\n3\r\n4\r\n");
    }

    #[test]
    fn do_print_sets_current_line() {
        let mut output = Vec::new();
        let mut buffer = EditBuffer::from(vec!["1\r\n", "2", "3", "4", "5", "6"]);
        buffer.set_current_line(5);
        buffer
            .do_print(&mut output, Some(Address::Span(2, 4)))
            .expect("successful print");
        assert_eq!(4, buffer.current_line());
    }

    #[test]
    fn do_print_empty_buffer_gives_error() {
        let mut output = Vec::new();
        let mut buffer = EditBuffer::new();
        let res = buffer
            .do_print(&mut output, None)
            .expect_err("invalid address");
        assert!(matches!(res, Error::InvalidAddress));
        let res = buffer
            .do_print(&mut output, Some(Address::Line(0)))
            .expect_err("invalid address");
        assert!(matches!(res, Error::InvalidAddress));
    }

    #[test]
    fn do_append_one_to_empty_buffer() {
        let mut buffer = EditBuffer::new();
        let expected = EditBuffer::from(vec!["one\n"]);
        buffer
            .do_append(
                &mut Vec::new(),
                Some(Address::Line(0)),
                vec!["one\n".to_owned()],
            )
            .expect("successful append");
        assert_eq!(1, buffer.current_line);
        assert_eq!(1, buffer.len());
        assert_eq!(&expected[..], &buffer[..]);
    }

    #[test]
    fn do_append_empty_buffer() {
        let mut buffer = EditBuffer::new();
        let expected = EditBuffer::from(vec!["a\n", "b", "c"]);
        buffer
            .do_append(
                &mut Vec::new(),
                Some(Address::Line(0)),
                vec!["a\n".to_owned(), "b\n".to_owned(), "c\n".to_owned()],
            )
            .expect("successful append");
        assert_eq!(3, buffer.current_line);
        assert_eq!(3, buffer.len());
        assert!(&expected[..].eq(&buffer[..]));
    }

    #[test]
    fn do_append_non_empty_at_0() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3"]);
        let expected = EditBuffer::from(vec!["a\n", "b", "c", "1", "2", "3"]);
        buffer
            .do_append(
                &mut Vec::new(),
                Some(Address::Line(0)),
                vec!["a\n".to_owned(), "b\n".to_owned(), "c\n".to_owned()],
            )
            .expect("successful append");
        assert_eq!(3, buffer.current_line);
        assert_eq!(6, buffer.len());
        assert!(&expected[..].eq(&buffer[..]));
    }

    #[test]
    fn do_append_in_middle() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3"]);
        let expected = EditBuffer::from(vec!["1\n", "2", "a", "b", "c", "3"]);
        buffer
            .do_append(
                &mut Vec::new(),
                Some(Address::Line(2)),
                vec!["a\n".to_owned(), "b\n".to_owned(), "c\n".to_owned()],
            )
            .expect("successful append");
        assert_eq!(5, buffer.current_line());
        assert_eq!(6, buffer.len());
        assert!(&expected[..].eq(&buffer[..]));
    }

    #[test]
    fn do_append_span_address() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        let expected = EditBuffer::from(vec!["1\n", "2", "3", "a", "b", "c", "4", "5", "6"]);
        buffer
            .do_append(
                &mut Vec::new(),
                Some(Address::Span(2, 3)),
                vec!["a\n".to_owned(), "b\n".to_owned(), "c\n".to_owned()],
            )
            .expect("successful append");
        assert_eq!(6, buffer.current_line);
        assert_eq!(9, buffer.len());
        assert!(&expected[..].eq(&buffer[..]));
    }

    #[test]
    fn do_append_at_end() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3"]);
        let expected = EditBuffer::from(vec!["1\n", "2", "3", "a", "b", "c"]);
        buffer
            .do_append(
                &mut Vec::new(),
                Some(Address::Line(3)),
                vec!["a\n".to_owned(), "b\n".to_owned(), "c\n".to_owned()],
            )
            .expect("successful append");
        assert_eq!(6, buffer.current_line);
        assert_eq!(6, buffer.len());
        assert!(&expected[..].eq(&buffer[..]));
    }

    #[test]
    fn do_append_of_zero_lines() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3"]);
        let expected = EditBuffer::from(vec!["1\n", "2", "3"]);
        assert_eq!(3, buffer.current_line());
        buffer
            .do_append(&mut Vec::new(), Some(Address::Line(2)), Vec::new())
            .expect("successful append");
        assert_eq!(2, buffer.current_line);
        assert_eq!(3, buffer.len());
        assert!(&expected[..].eq(&buffer[..]));
    }

    #[test]
    fn do_delete_span() {
        let mut buffer = EditBuffer::from(vec!["1\r\n", "2", "3", "4", "5", "6"]);
        let expected = EditBuffer::from(vec!["1\r\n", "2", "6"]);
        buffer
            .do_delete(&mut Vec::new(), Some(Address::Span(3, 5)))
            .expect("deleted span");
        assert_eq!(3, buffer.len());
        assert!(&expected[..].eq(&buffer[..]));
    }

    #[test]
    fn do_delete_line() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        let expected = EditBuffer::from(vec!["1\n", "2", "4", "5", "6"]);
        buffer
            .do_delete(&mut Vec::new(), Some(Address::Line(3)))
            .expect("deleted line");
        assert_eq!(5, buffer.len());
        assert!(&expected[..].eq(&buffer[..]));
    }

    #[test]
    fn do_delete_span_at_start() {
        let mut buffer = EditBuffer::from(vec!["1\r\n", "2", "3", "4", "5", "6"]);
        let expected = EditBuffer::from(vec!["4\r\n", "5", "6"]);
        buffer
            .do_delete(&mut Vec::new(), Some(Address::Span(1, 3)))
            .expect("delete span");
        assert_eq!(3, buffer.len());
        assert_eq!(expected[..], buffer[..]);
    }

    #[test]
    fn do_delete_span_at_end() {
        let mut buffer = EditBuffer::from(vec!["1\r\n", "2", "3", "4", "5", "6"]);
        let expected = EditBuffer::from(vec!["1\r\n", "2", "3", "4"]);
        buffer
            .do_delete(&mut Vec::new(), Some(Address::Span(5, 6)))
            .expect("deleted span");
        assert_eq!(4, buffer.len());
        assert_eq!(expected[..], buffer[..]);
    }

    #[test]
    fn do_delete_no_addr() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        let expected = EditBuffer::from(vec!["1\n", "2", "4", "5", "6"]);
        buffer.set_current_line(3);
        buffer
            .do_delete(&mut Vec::new(), None)
            .expect("deleted line");
        assert_eq!(5, buffer.len());
        assert_eq!(expected[..], buffer[..]);
    }

    #[test]
    fn do_delete_empty_buffer() {
        let mut buffer = EditBuffer::new();
        let _res = buffer
            .do_delete(&mut Vec::new(), None)
            .expect_err("invalid address");
        assert!(matches!(Error::InvalidAddress, _res));
    }

    #[test]
    fn do_delete_line_zero() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3"]);
        let res = buffer
            .do_delete(&mut Vec::new(), Some(Address::Line(0)))
            .expect_err("invalid address");
        assert!(matches!(res, Error::InvalidAddress));
    }

    #[test]
    fn do_delete_span_starting_at_zero() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5"]);
        let res = buffer
            .do_delete(&mut Vec::new(), Some(Address::Span(0, 3)))
            .expect_err("invalid address");
        assert!(matches!(res, Error::InvalidAddress));
    }

    #[test]
    fn buffer_dirty_after_append() {
        let mut buffer = EditBuffer::new();
        assert!(!buffer.is_dirty());
        buffer
            .do_append(
                &mut Vec::new(),
                Some(Address::Line(0)),
                vec!["1\n".to_owned(), "2\n".to_owned(), "3\n".to_owned()],
            )
            .expect("lines appended");
        assert!(buffer.is_dirty());
    }

    #[test]
    fn do_undo_append() {
        let mut buffer = EditBuffer::new();
        buffer
            .do_append(
                &mut Vec::new(),
                Some(Address::Line(0)),
                vec!["1\n".to_owned(), "2\n".to_owned(), "3\n".to_owned()],
            )
            .expect("lines appended");
        assert_eq!(&EditBuffer::from(vec!["1\n", "2", "3"])[..], &buffer[..]);
        buffer.do_undo(&mut Vec::new()).expect("append undone");
        assert_eq!(EditBuffer::new()[..], buffer[..]);
    }

    #[test]
    fn do_undo_delete() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        let expected = buffer.clone();
        buffer
            .do_delete(&mut Vec::new(), Some(Address::Span(1, 4)))
            .expect("lines deleted");
        assert_eq!(&EditBuffer::from(vec!["5\n", "6"])[..], &buffer[..]);
        buffer.do_undo(&mut Vec::new()).expect("undone Delete");
        assert_eq!(&expected[..], &buffer[..]);
    }

    #[test]
    fn do_undo_multi() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        let expected_final = buffer.clone();
        assert_eq!(6, buffer.current_line());

        buffer
            .do_append(
                &mut Vec::new(),
                Some(Address::Line(2)),
                vec!["a\n".to_owned(), "b\n".to_owned(), "c\n".to_owned()],
            )
            .expect("3 lines appended");
        let expected_1 = EditBuffer::from(vec!["1\n", "2", "a", "b", "c", "3", "4", "5", "6"]);
        assert_eq!(&expected_1[..], &buffer[..]);
        assert_eq!(5, buffer.current_line());

        buffer
            .do_delete(&mut Vec::new(), Some(Address::Span(4, 7)))
            .expect("lines deleted");
        let expected_2 = EditBuffer::from(vec!["1\n", "2", "a", "5", "6"]);
        assert_eq!(&expected_2[..], &buffer[..]);

        buffer.do_undo(&mut Vec::new()).expect("undone Delete");
        assert_eq!(&expected_1[..], &buffer[..]);

        buffer.do_undo(&mut Vec::new()).expect("undone Append");
        assert_eq!(&expected_final[..], &buffer[..]);
    }

    #[test]
    fn buffer_clean_after_undo_all() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);

        buffer
            .do_append(
                &mut Vec::new(),
                Some(Address::Line(2)),
                vec!["a\n".to_owned(), "b\n".to_owned(), "c\n".to_owned()],
            )
            .expect("3 lines appended");

        buffer
            .do_delete(&mut Vec::new(), Some(Address::Span(4, 7)))
            .expect("lines deleted");

        buffer
            .do_append(
                &mut Vec::new(),
                Some(Address::Line(0)),
                vec!["x\n".to_owned(), "y\n".to_owned(), "z\n".to_owned()],
            )
            .expect("3 lines appended");

        buffer.do_undo(&mut Vec::new()).expect("undone Append");

        buffer.do_undo(&mut Vec::new()).expect("undone Delete");

        buffer.do_undo(&mut Vec::new()).expect("undone Append");

        assert!(!buffer.is_dirty());
    }

    #[test]
    fn print_filename_none_set() {
        let mut buffer = EditBuffer::from(vec!["1\r\n", "2", "3"]);
        let mut output = Vec::new();
        buffer
            .do_file(&mut output, &None)
            .expect("notice of no current filename");
        assert_eq!(b"No current filename\n", &output[..]);
        assert_eq!(None, *buffer.filename());
    }

    #[test]
    fn set_filename() {
        let new_filename = "a_new_filename.txt";
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3"]);
        let mut output = Vec::new();
        assert_eq!(None, *buffer.filename());
        buffer
            .do_file(&mut output, &Some(PathBuf::from(new_filename)))
            .expect("successful setting of filename");
        assert_eq!(format!("{}\n", new_filename).as_bytes(), &output[..]);
        assert_eq!(Some(PathBuf::from(new_filename)), *buffer.filename());
    }

    #[test]
    fn print_filename() {
        let new_filename = "a_new_filename.txt";
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3"]);
        let mut output = Vec::new();
        assert_eq!(None, *buffer.filename());
        buffer
            .do_file(&mut output, &Some(PathBuf::from(new_filename)))
            .expect("successful setting of filename");
        assert_eq!(Some(PathBuf::from(new_filename)), *buffer.filename());
        output.clear();
        buffer
            .do_file(&mut output, &None)
            .expect("displayed filename");
        assert_eq!(format!("{}\n", new_filename).as_bytes(), &output[..]);
    }

    #[test]
    fn change_filename() {
        let orig_filename = "a_filename.md";
        let new_filename = "a_new_filename.txt";
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3"]);
        let mut output = Vec::new();
        buffer
            .do_file(&mut output, &Some(PathBuf::from(orig_filename)))
            .expect("successful setting of filename");
        output.clear();
        buffer
            .do_file(&mut output, &Some(PathBuf::from(new_filename)))
            .expect("displayed filename");
        assert_eq!(format!("{}\n", new_filename).as_bytes(), &output[..]);
        assert_eq!(Some(PathBuf::from(new_filename)), *buffer.filename());
    }

    #[test]
    fn do_edit_no_file() {
        let mut buffer = EditBuffer::new();
        let mut output = Vec::new();
        let res = buffer
            .do_edit(&mut output, &None, &None)
            .expect_err("no filename");
        assert!(matches!(res, Error::NoFilename));
    }

    #[test]
    fn do_edit_file_not_found() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3"]);
        let file_to_edit = "a_file_that_is_not_there.ext";
        let mut output = Vec::new();
        buffer
            .do_edit(&mut output, &Some(PathBuf::from(file_to_edit)), &None)
            .expect("edit with message");
        assert!(buffer.is_empty());
        assert!(!buffer.is_dirty());
        assert_eq!(buffer.filename(), &Some(PathBuf::from(file_to_edit)));
    }

    #[test]
    fn read_replace_io_error() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3"]);
        let reader = BadReader {};
        let source = Some(BufReader::new(reader));
        let mut output = Vec::new();
        let mut data = EditData {
            ..Default::default()
        };
        let res = buffer.read_replace(&mut output, source, &mut data);
        assert!(matches!(res, Err(Error::Read(_))));
    }

    #[test]
    fn read_replace_zero_length() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3"]);
        let reader = &b""[..];
        let source = Some(BufReader::new(reader));
        let mut output = Vec::new();
        let mut data = EditData {
            ..Default::default()
        };
        buffer
            .read_replace(&mut output, source, &mut data)
            .expect("no error");
        assert_eq!(buffer[..], Vec::<String>::new());
    }

    #[test]
    fn read_replace_empty_buffer() {
        let mut buffer = EditBuffer::new();
        let reader = &b"one\ntwo\nthree\n"[..];
        let source = Some(BufReader::new(reader));
        let mut output = Vec::new();
        assert_eq!(buffer.current_line(), 0);

        let mut data = EditData {
            ..Default::default()
        };
        buffer
            .read_replace(&mut output, source, &mut data)
            .expect("no error");
        assert_eq!(buffer[..], vec!["one\n", "two\n", "three\n"]);
        assert_eq!(buffer.current_line(), 3usize);
    }

    #[test]
    fn read_replace_non_empty_buffer() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4"]);
        let reader = &b"one\ntwo\nthree\n"[..];
        let source = Some(BufReader::new(reader));
        let mut output = Vec::new();
        assert_eq!(buffer.current_line(), 4);

        let mut data = EditData {
            ..Default::default()
        };
        buffer
            .read_replace(&mut output, source, &mut data)
            .expect("no error");
        assert_eq!(buffer[..], vec!["one\n", "two\n", "three\n"]);
        assert_eq!(buffer.current_line(), 3usize);
    }

    #[test]
    fn read_replace_prints_chars_read() {
        let mut buffer = EditBuffer::new();
        let reader = &b"one\ntwo\nthree\n"[..];
        let source = Some(BufReader::new(reader));
        let mut output = Vec::new();
        assert_eq!(buffer.current_line(), 0);

        let mut data = EditData {
            ..Default::default()
        };
        buffer
            .read_replace(&mut output, source, &mut data)
            .expect("no error");
        assert_eq!(&output[..], &b"14\n"[..]);
    }
}
