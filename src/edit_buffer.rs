use std::fmt;
use std::io;
use std::path;

pub struct EditBuffer {
    lines: Vec<String>,
    needs_write: bool,
    cur_line: usize,
    default_filename: Option<path::PathBuf>,
}

#[derive(Debug)]
pub enum Error {
    Read(io::Error),
}

impl std::error::Error for Error {}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Error::Read(e) => write!(f, "error reading lines: {e}"),
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
    /// Given that the `EditBuffer` is empty, this will not allocate any
    /// initial space. This will be very inexpensive, but will require
    /// extra, perhaps excessive, allocation later as lines are added.
    /// Consider the [`with_capacity`] method instead, to prevent this.
    ///
    /// [`with_capacity`]: EditBuffer::with_capacity
    #[inline]
    #[must_use]
    pub fn new() -> EditBuffer {
        EditBuffer {
            lines: Vec::new(),
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
            lines: Vec::with_capacity(capacity),
            ..EditBuffer::default()
        }
    }

    /// Returns this `EditBuffer`'s capacity, in bytes.
    pub fn capacity(&self) -> usize {
        self.lines.capacity()
    }

    /// Returns this `EditBuffer`'s length, in lines.
    pub fn len(&self) -> usize {
        self.lines.len()
    }

    /// Returns true if buffer has been changed since last write.
    pub fn needs_write(&self) -> bool {
        self.needs_write
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
