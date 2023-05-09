use std::fmt;
use std::io::{self, prelude::*};
use std::path;

pub struct EditBuffer {
    text: Vec<String>,
    needs_write: bool,
    cur_line: usize,
    default_filename: Option<path::PathBuf>,
}

#[derive(Debug)]
pub enum Error {
    Read(io::Error),
    ReadBadIndex(usize, usize),
}

impl std::error::Error for Error {}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Error::Read(e) => write!(f, "error reading lines: {e}"),
            Error::ReadBadIndex(sz, i) => write!(
                f,
                "error reading lines: location {i} beyond end of buffer {sz}"
            ),
        }
    }
}

impl Default for EditBuffer {
    fn default() -> Self {
        Self::new()
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
            needs_write: false,
            cur_line: 0,
            default_filename: None,
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

    /// Returns this `EditBuffer`'s capacity, in bytes.
    pub fn capacity(&self) -> usize {
        self.text.capacity()
    }

    /// Returns this `EditBuffer`'s length, in lines.
    pub fn len(&self) -> usize {
        self.text.len()
    }

    /// Returns true if buffer has been changed since last write.
    pub fn needs_write(&self) -> bool {
        self.needs_write
    }

    /// Reads lines from reader and appends them after the specified line.
    /// Returns index of last line read, or an error if read fails
    pub fn read<R>(&mut self, after_line: usize, mut reader: R) -> Result<usize, Error>
    where
        R: io::BufRead,
    {
        if after_line > self.text.len() {
            return Err(Error::ReadBadIndex(self.len(), after_line));
        }
        let mut lines = Vec::new();
        let mut line = String::new();
        loop {
            let len = reader.read_line(&mut line).map_err(Error::Read)?;
            if len == 0 {
                break;
            }
            lines.push(line);
            line = String::new();
        }
        // Insert or append read lines reasonably efficiently
        let lines_added = lines.len();
        self.text.splice(after_line..after_line, lines.into_iter());
        Ok(after_line + lines_added - 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct BadReader {}

    impl Read for BadReader {
        fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::from(io::ErrorKind::Other))
        }
    }

    #[test]
    fn create_empty_buffer() {
        let buffer = EditBuffer::new();
        assert_eq!(buffer.capacity(), 0);
    }

    #[test]
    fn create_buffer_with_capacity() {
        const INIT_CAPACITY: usize = 1024;
        let buffer = EditBuffer::with_capacity(INIT_CAPACITY);
        assert_eq!(buffer.capacity(), INIT_CAPACITY);
    }

    #[test]
    fn empty_buffer_returns_zero_len() {
        let buffer = EditBuffer::new();
        assert_eq!(0, buffer.len());
        let buffer = EditBuffer::with_capacity(1024);
        assert_eq!(0, buffer.len());
    }

    ////
    // read() tests

    #[test]
    fn read_to_empty_buffer() {
        let mut buffer = EditBuffer::new();
        let content = vec!["Line1\n", "Line2\n", "Line3\n", "Line4\n"];
        let mut input = Vec::new();
        for line in &content {
            input.extend(line.bytes());
        }
        let last_line_read = buffer
            .read(buffer.len(), &input[..])
            .expect("Error reading content");
        assert_eq!(content, buffer.text);
        assert_eq!(3, last_line_read);
        assert_eq!(content.len(), buffer.len());
    }

    #[test]
    fn read_to_empty_buffer_no_trailing_eol() {
        let mut buffer = EditBuffer::new();
        let content = vec!["Line1\n", "Line2\n", "Line3\n", "Line4"];
        let mut input = Vec::new();
        for line in &content {
            input.extend(line.bytes());
        }
        let last_line_read = buffer
            .read(buffer.len(), &input[..])
            .expect("Error reading content");
        assert_eq!(content, buffer.text);
        assert_eq!(3, last_line_read);
        assert_eq!(content.len(), buffer.len());
    }

    #[test]
    fn read_append() {
        assert!(false, "TODO");
    }

    #[test]
    fn read_insert() {
        assert!(false, "TODO");
    }

    #[test]
    fn read_with_bad_index() {
        let mut buffer = EditBuffer::new();
        let content = vec!["Line1]n"];
        let mut input = Vec::new();
        for line in &content {
            input.extend(line.bytes());
        }
        let _res = buffer.read(999, &input[..]);
        assert!(matches!(
            Err::<Error, _>(Error::ReadBadIndex),
            _res
        ));
    }

    #[test]
    fn read_with_io_error() {
        let reader = BadReader {};
        let mut input = io::BufReader::new(reader);
        let mut buffer = EditBuffer::new();
        let _res = buffer.read(0, &mut input);
        assert!(matches!(Err::<Error, _>(Error::Read), _res));
    }
}
