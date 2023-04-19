use std::fmt;
use std::path::Path;

pub struct EditBuffer {
    text: String,
}

impl Default for EditBuffer {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug)]
pub enum Error {
    Other(String),
}

impl std::error::Error for Error {}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Error::Other(s) => write!(f, "Error: {s}"),
        }
    }
}

impl EditBuffer {
    /// Creates a new empty `EditBuffer`.
    ///
    /// Given that the `EditBuffer` is empty, this will not allocate any
    /// initial space. This will be very inexpensive, but will require
    /// extra, perhaps excessive, allocation later as text is added.
    /// Consider the [`with_capacity`] method instead, to prevent this.
    ///
    /// [`with_capacity`]: EditBuffer::with_capacity
    #[inline]
    #[must_use]
    pub fn new() -> EditBuffer {
        EditBuffer {
            text: String::new(),
        }
    }

    /// Creates a new empty `EditBuffer` with room for at least `capacity`
    /// bytes of text. Specifying a capacity is useful to reduce the number
    /// of reallocations necessary as text is appended to the `EditBuffer`.
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
            text: String::with_capacity(capacity),
        }
    }

    // Returns a buffer containing the lines in the file referenced
    // by __path__.
    pub fn with_file(path: &Path) -> Result<EditBuffer, Error> {
        Err(Error::Other("not implemented".to_string()))
    }

    /// Returns this `EditBuffer`'s capacity, in bytes.
    pub fn capacity(&self) -> usize {
        self.text.capacity()
    }

    /// Returns this `EditBuffer`'s length, in lines.
    pub fn len(&self) -> usize {
        self.text.len()
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
