// EditBuffer keeps track of everything specific to a single buffer in the
// editor. All public interface uses one based indexing, and any such function
// is responsible for translating into the 0 based indexing of the Vec<String>
// containing the lines of text.

use regex::Regex;
use std::cmp::Ordering;
use std::fmt;
use std::io;
use std::ops::Deref;
use std::path;

#[derive(Debug, PartialEq)]
pub struct EditBuffer {
    text: Vec<String>,
    needs_write: bool,
    current_line: usize,
    default_filename: Option<path::PathBuf>,
    default_eol: Option<&'static str>,
}

#[derive(Debug)]
pub enum Error {
    Read(io::Error),
    ReadBadIndex(usize, usize),
    InvalidIndex,
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
            Error::InvalidIndex => write!(f, "invalid index"),
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
        let mut value = value.iter().map(|v| v.to_string()).collect::<Vec<String>>();
        buf.text.append(&mut value);
        buf.current_line = buf.text.len();
        buf
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
            current_line: 0,
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
    pub fn needs_write(&self) -> bool {
        self.needs_write
    }

    pub fn current_line(&self) -> usize {
        self.current_line
    }

    pub fn set_current_line(&mut self, line: usize) -> Result<(), Error> {
        if line == 0 || line > self.text.len() {
            Err(Error::InvalidIndex)
        } else {
            self.current_line = line;
            Ok(())
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
            self.default_eol = Some(compute_default_eol(&lines[..]));
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
        self.needs_write = true;
        self.current_line = at_line + lines_added;
        Ok(self.current_line)
    }
}

fn compute_native_eol() -> &'static str {
    if std::env::consts::FAMILY == "windows" {
        "\r\n"
    } else {
        "\n"
    }
}

fn compute_default_eol<S>(lines: &[S]) -> &'static str
where
    S: Deref<Target = str>,
{
    let native_eol = if std::env::consts::FAMILY == "windows" {
        "\r\n"
    } else {
        "\n"
    };
    let mut crlf = 0;
    let mut lf = 0;

    for line in lines {
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

#[cfg(test)]
mod tests {
    use super::*;

    struct BadReader {}

    impl io::Read for BadReader {
        fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::from(io::ErrorKind::Other))
        }
    }

    ////
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

    ////
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

    ////
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
        last line read: $last_read:expr$(,)? } => {
            #[test]
            fn $name() {
                let initial = $initial;
                let mut buffer = EditBuffer::from(initial);
                let added = $added;
                let input = new_input_buf(&added[..]);
                let last_read = buffer
                    .read($at, &input[..])
                    .expect("Error reading added lines");

                assert_eq!($expect,
                        buffer.text,
                        "expected text: {:?}, got {:?}", $expect, &buffer.text
                );
                assert_eq!($last_read,
                        last_read,
                        "expected last_read {}, got {}", $last_read, last_read
                );
                assert_eq!(true,
                        buffer.needs_write(),
                        "expected buffer needs write, got {}", buffer.needs_write()
                );
                assert_eq!($last_read,
                        buffer.current_line(),
                        "expected current_line: {}, got {}", $last_read, buffer.current_line()
                );
            }
        };
    }

    read_test! {
        read_to_empty_buf_all_lf,
        initial: Vec::<&str>::new(),
        added: vec!["Line1\n", "Line2\n", "Line3\n",],
        at: 0,
        expect: vec!["Line1\n", "Line2\n", "Line3\n",],
        last line read: 3,
    }

    read_test! {
        read_to_empty_buf_all_lf_no_final,
        initial: Vec::<&str>::new(),
        added: vec!["Line1\n", "Line2\n", "Line3",],
        at: 0,
        expect: vec!["Line1\n", "Line2\n", "Line3",],
        last line read: 3,
    }

    read_test! {
        read_append_all_lf,
        initial: vec!["Line1\n", "Line2\n", "Line3\n",],
        added: vec!["New1\n", "New2\n", "New3\n"],
        at: 3,
        expect: vec![
            "Line1\n", "Line2\n", "Line3\n", "New1\n", "New2\n", "New3\n",
        ],
        last line read: 6,
    }

    read_test! {
        read_append_most_lf_no_final,
        initial: vec!["Line1\n", "Line2\r\n", "Line3\n", "Line4",],
        added: vec!["New1\n", "New2\n", "New3"],
        at: 4,
        expect: vec![
            "Line1\n", "Line2\r\n", "Line3\n", "Line4\n", "New1\n", "New2\n", "New3",
        ],
        last line read: 7,
    }

    read_test! {
        read_append_most_crlf_no_final,
        initial: vec!["Line1\r\n", "Line2\r\n", "Line3\n", "Line4",],
        added: vec!["New1\r\n", "New2\n", "New3"],
        at: 4,
        expect: vec![
            "Line1\r\n", "Line2\r\n", "Line3\n", "Line4\r\n", "New1\r\n", "New2\n", "New3",
        ],
        last line read: 7,
    }

    read_test! {
        read_append_all_lf_no_final,
        initial: vec!["Line1\n", "Line2\n", "Line3",],
        added: vec!["New1\n", "New2\n", "New3\n"],
        at: 3,
        expect: vec![
            "Line1\n", "Line2\n", "Line3\n", "New1\n", "New2\n", "New3\n",
        ],
        last line read: 6,
    }

    read_test! {
        read_append_all_crlf_no_final,
        initial: vec!["Line1\r\n", "Line2\r\n", "Line3",],
        added: vec!["New1\r\n", "New2\r\n", "New3\r\n"],
        at: 3,
        expect: vec![
            "Line1\r\n", "Line2\r\n", "Line3\r\n", "New1\r\n", "New2\r\n", "New3\r\n",
        ],
        last line read: 6,
    }

    #[test]
    fn read_append_equal_eol_no_final() {
        let initial = vec!["Line1\n", "Line2\r\n", "Line3"];
        let mut buffer = EditBuffer::from(initial);

        let at = 3;
        let added = vec!["New1\n", "New2\r\n", "New3"];
        let input = new_input_buf(&added[..]);
        let last_read = buffer
            .read(at, &input[..])
            .expect("Error reading added lines");

        let mut line3 = "Line3".to_string();
        line3.push_str(compute_native_eol());
        let expect = vec![
            "Line1\n",
            "Line2\r\n",
            &line3[..],
            "New1\n",
            "New2\r\n",
            "New3",
        ];
        assert_eq!(expect, buffer.text);
        assert_eq!(6, last_read);
        assert_eq!(true, buffer.needs_write());
    }

    read_test! {
        read_insert_all_lf,
        initial: vec!["Line1\n", "Line2\n", "Line3\n",],
        added: vec!["New1\n", "New2\n", "New3\n"],
        at: 2,
        expect: vec![
            "Line1\n", "Line2\n", "New1\n", "New2\n", "New3\n", "Line3\n",
        ],
        last line read: 5,
    }

    read_test! {
        read_insert_most_lf_no_final,
        initial: vec!["Line1\n", "Line2\r\n", "Line3\n", "Line4\n",],
        added: vec!["New1\n", "New2\n", "New3"],
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
        last line read: 5,
    }

    read_test! {
        read_insert_most_crlf_no_final,
        initial: vec!["Line1\r\n", "Line2\r\n", "Line3\n", "Line4\r\n",],
        added: vec!["New1\r\n", "New2\n", "New3"],
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
        last line read: 5,
    }

    read_test! {
        read_insert_all_lf_no_final,
        initial: vec!["Line1\n", "Line2\n", "Line3\n", "Line4\n",],
        added: vec!["New1\n", "New2\n", "New3"],
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
        last line read: 5,
    }

    read_test! {
        read_insert_all_crlf_no_final,
        initial: vec!["Line1\r\n", "Line2\r\n", "Line3\r\n", "Line4\r\n",],
        added: vec!["New1\r\n", "New2\r\n", "New3"],
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
        last line read: 5,
    }

    #[test]
    fn read_insert_equal_eol_no_final() {
        let initial = vec!["Line1\r\n", "Line2\n", "Line3\n", "Line4\r\n"];
        let mut buffer = EditBuffer::from(initial);

        let at = 2;
        let added = vec!["New1\r\n", "New2\r\n", "New3"];
        let input = new_input_buf(&added[..]);
        let last_read = buffer
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
        assert_eq!(5, last_read);
        assert_eq!(true, buffer.needs_write());
    }

    #[test]
    fn read_with_bad_index() {
        let mut buffer = EditBuffer::new();
        let content = vec!["Line1\n"];
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
