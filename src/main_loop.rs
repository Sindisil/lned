use crate::cli;
use crate::command;
use crate::edit_buffer::EditBuffer;
use std::fmt;
use std::io::{self, prelude::*};

#[derive(Debug)]
pub enum Error {
    Other(String),
    CommandInput,
}

impl std::error::Error for Error {}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Error::CommandInput => write!(f, "Error reading command"),
            Error::Other(s) => write!(f, "Error: {s}"),
        }
    }
}

pub(crate) fn run<R, W>(mut input: R, mut output: W, args: &cli::CmdArgs) -> Result<(), Error>
where
    R: BufRead,
    W: Write,
{
    // Initialize Buffers
    let buffers = initialize_buffers(&args)?;

    // Initialize context (e.g., current buffer)
    let mut current_buffer = 0;

    // Accept and process commands until fatal error or exit
    loop {
        // compute prompt
        let prompt = ":";

        // accept command
        let cmd = read_command(prompt, &mut input, &mut output)?;

        // parse command
        let cmd = parse_command(&cmd)?;

        // execute command
        return Err(Error::Other("Nothing implemented yet".to_string()));
    }
}

fn read_command<R, W>(prompt: &str, mut input: R, mut output: W) -> Result<String, Error>
where
    R: BufRead,
    W: Write,
{
    Err(Error::Other("read_command not implemented yet".to_string()))
}

fn parse_command(cmd: &str) -> Result<command::Cmd, Error> {
    Err(Error::Other("command parsing not implemented".to_string()))
}

fn initialize_buffers(args: &cli::CmdArgs) -> Result<Vec<EditBuffer>, Error> {
    let mut buffers = Vec::with_capacity(args.files.len());

    if buffers.len() > 0 {
        return Err(Error::Other(
            "Reading files not yet implemented".to_string(),
        ));
    }

    // No files passed in, or none read successfully, so
    // we must allocate an empty buffer to use instead
    buffers.push(EditBuffer::new());

    Ok(buffers)
}

// Read lines of text input until a line with a single . is entered
// Clears previous content of buffer, but doesn't shrink capacity.
// Returns a Vec of all lines entered *except* the terminating line
// containing a single dot.
fn read_lines<R>(mut reader: R, buf: &mut Vec<String>) -> Result<usize, io::Error>
where
    R: BufRead,
{
    let mut line = String::new(); // single line input buffer
    buf.clear(); // get rid of any old input

    loop {
        reader.read_line(&mut line)?;
        if line == ".\n" || line == ".\r\n" {
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
    fn read_line_with_no_input_gives_zero_lines() {
        let input = b".\n";
        let mut lines = Vec::new();
        let line_count = read_lines(&input[..], &mut lines).expect("Error reading lines");
        assert_eq!(0, line_count);
        assert_eq!(0, lines.len());
    }

    #[test]
    fn read_line_returns_lines_entered() {
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

    #[test]
    fn initialize_buffers_no_files_gives_single_empty_buffer() {
        let args = cli::CmdArgs {
            line_numbers: true,
            files: Vec::new(),
        };
        let buffers = initialize_buffers(&args).unwrap();
        assert_eq!(1, buffers.len());
        assert_eq!(0, buffers[0].len());
    }
}
