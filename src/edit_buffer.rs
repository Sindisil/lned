use regex::Regex;
use std::fmt;
use std::io::{self, prelude::*};
use std::path;

pub struct EditBuffer {
    text: Vec<String>,
    needs_write: bool,
    cur_line: usize,
    default_filename: Option<path::PathBuf>,
    default_eol: Option<String>,
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
            default_eol: None,
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

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// Returns true if buffer has been changed since last write.
    pub fn needs_write(&self) -> bool {
        self.needs_write
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
    /// Returns index of last line read, or an error if read fails
    pub fn read<R>(&mut self, at_line: usize, mut reader: R) -> Result<usize, Error>
    where
        R: io::BufRead,
    {
        if at_line > self.text.len() {
            return Err(Error::ReadBadIndex(self.len(), at_line));
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
        let lines_added = lines.len();

        // set default_eol if neccessary
        if self.default_eol.is_none() {
            self.default_eol = Some(find_default_eol(&lines[..]));
        }

        // Add in missing eol as needed
        let default_eol = self
            .default_eol
            .as_ref()
            .expect("default_eol should have been set");
        if !self.is_empty() {
            let last_line = if at_line == self.len() {
                let i = self.len() - 1;
                &mut self.text[i]
            } else {
                let i = lines.len() - 1;
                &mut lines[i]
            };
            let eol_pat = Regex::new(r"^.*(\r\n|\n|\r)$").unwrap();
            if !eol_pat.is_match(last_line) {
                last_line.push_str(default_eol.as_ref());
            }
        }

        // actually add new lines to buffer
        self.text.splice(at_line..at_line, lines.into_iter());
        Ok(at_line + lines_added - 1)
    }
}

fn find_default_eol(lines: &[String]) -> String {
    let mut eols = vec![("\r\n", 0), ("\n", 0), ("\r", 0)];

    for line in lines {
        for eol in &mut eols {
            if line.ends_with(eol.0) {
                eol.1 += 1;
                break;
            }
        }
    }

    eols.sort_by(|(_, a), (_, b)| b.cmp(a));
    if eols[0].1 > 0 {
        eols[0].0.to_string()
    } else {
        if std::env::consts::FAMILY == "windows" {
            "\r\n".to_string()
        } else {
            "\n".to_string()
        }
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

    fn new_input_buf(content: &[&str]) -> Vec<u8> {
        let mut input = Vec::new();
        for line in content {
            input.extend(line.bytes());
        }
        input
    }

    #[test]
    fn read_to_empty_buffer() {
        let mut buffer = EditBuffer::new();
        let content = vec!["Line1\n", "Line2\n", "Line3\n", "Line4\n"];
        let input = new_input_buf(&content);
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
        let input = new_input_buf(&content);
        let last_line_read = buffer
            .read(buffer.len(), &input[..])
            .expect("Error reading content");
        assert_eq!(content, buffer.text);
        assert_eq!(3, last_line_read);
        assert_eq!(content.len(), buffer.len());
        assert_eq!(buffer.default_eol, Some("\n".to_string()));
    }

    #[test]
    fn read_append() {
        let mut buffer = EditBuffer::new();

        let initial_content = vec!["Line1\n", "Line2\n", "Line3\n"];
        let input = new_input_buf(&initial_content[..]);
        let _last_read = buffer
            .read(0, &input[..])
            .expect("Error reading initial_content");

        let new_content = vec!["New1\n", "New2\n", "New3\n"];
        let input = new_input_buf(&new_content[..]);
        let index = buffer.len();
        let last_read = buffer
            .read(index, &input[..])
            .expect("Error reading new_content");

        let final_content = vec![
            "Line1\n", "Line2\n", "Line3\n", "New1\n", "New2\n", "New3\n",
        ];
        assert_eq!(final_content, buffer.text);
        assert_eq!(final_content.len(), buffer.len());
        assert_eq!(index + new_content.len() - 1, last_read);
        assert_eq!(buffer.default_eol, Some("\n".to_string()));
    }

    #[test]
    fn read_append_no_trailing_eol() {
        let mut buffer = EditBuffer::new();

        let initial_content = vec!["Line1\n", "Line2\n", "Line3"];
        let input = new_input_buf(&initial_content[..]);
        let _last_read = buffer
            .read(0, &input[..])
            .expect("Error reading initial_content");

        let new_content = vec!["New1\n", "New2\n", "New3"];
        let input = new_input_buf(&new_content[..]);
        let index = buffer.len();
        let last_read = buffer
            .read(index, &input[..])
            .expect("Error reading new_content");

        let final_content = vec!["Line1\n", "Line2\n", "Line3\n", "New1\n", "New2\n", "New3"];
        assert_eq!(final_content, buffer.text);
        assert_eq!(final_content.len(), buffer.len());
        assert_eq!(index + new_content.len() - 1, last_read);
        assert_eq!(buffer.default_eol, Some("\n".to_string()));
    }

    #[test]
    fn read_insert() {
        let mut buffer = EditBuffer::new();

        let initial_content = vec!["Line1\n", "Line2\n", "Line3\n"];
        let input = new_input_buf(&initial_content[..]);
        let _last_read = buffer
            .read(0, &input[..])
            .expect("Error reading initial_content");

        let new_content = vec!["New1\n", "New2\n", "New3\n"];
        let input = new_input_buf(&new_content[..]);
        let index = 2;
        let last_read = buffer
            .read(index, &input[..])
            .expect("Error reading new_content");

        let final_content = vec![
            "Line1\n", "Line2\n", "New1\n", "New2\n", "New3\n", "Line3\n",
        ];
        assert_eq!(final_content, buffer.text);
        assert_eq!(final_content.len(), buffer.len());
        assert_eq!(index + new_content.len() - 1, last_read);
        assert_eq!(buffer.default_eol, Some("\n".to_string()));
    }

    #[test]
    fn read_with_bad_index() {
        let mut buffer = EditBuffer::new();
        let content = vec!["Line1]n"];
        let input = new_input_buf(&content);
        let _res = buffer.read(999, &input[..]);
        assert!(matches!(Err::<Error, _>(Error::ReadBadIndex), _res));
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
