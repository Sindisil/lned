use once_cell::unsync::Lazy as LazyCell;
use std::io::{self, prelude::*};

use regex::Regex;

// Read lines of text input until a line with a single . is entered
// Clears previous content of buffer, but doesn't shrink capacity.
// Returns a Vec of all lines entered *except* the terminating line
// containing a single dot.
fn read_lines<R>(mut reader: R, buf: &mut Vec<String>) -> Result<usize, io::Error>
where
    R: BufRead,
{
    let just_a_dot = LazyCell::new(|| Regex::new(r"^\.\r?\n$").unwrap());
    let mut line = String::new(); // single line input buffer
    buf.clear(); // get rid of any old input

    loop {
        reader.read_line(&mut line)?;
        if just_a_dot.is_match(&line) {
            return Ok(buf.len());
        }
        buf.push(line);
        line = String::new();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_input_gives_zero_lines() {
        let input = b".\n";
        let mut lines = Vec::new();
        let line_count = read_lines(&input[..], &mut lines).expect("Error reading lines");
        assert_eq!(0, line_count);
        assert_eq!(0, lines.len());
    }

    #[test]
    fn returns_lines_entered() {
        let three_lines = vec!["line1\n", "line 2\n", "line 3\n", ".\n"];
        let mut input = Vec::new();
        for line in &three_lines {
            input.extend(line.as_bytes());
        }
        let mut lines = Vec::new();
        let line_count = read_lines(&input[..], &mut lines).expect("Error reading lines");

        assert_eq!(3, line_count);
        assert_eq!(3, lines.len());
        assert_eq!(three_lines[..3], lines);
    }
}
