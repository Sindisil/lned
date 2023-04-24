use crate::cli;
use crate::command::{self, Cmd};
use crate::edit_buffer::EditBuffer;
use std::fmt;
use std::io::{self, prelude::*};

#[derive(Debug)]
pub enum Error {
    /// I/O Error writing out prompt
    WritePrompt(io::Error),
    /// I/O Error reading command input
    ReadCommand(io::Error),
    /// I/O Error reading input lines
    ReadLines(io::Error),
    ParseCmd(command::ParseError),
    Other(String),
}

impl std::error::Error for Error {}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Error::ReadCommand(e) => write!(f, "Error reading command: {e}"),
            Error::WritePrompt(e) => write!(f, "Error writing prompt: {e}"),
            Error::ReadLines(e) => write!(f, "Error reading input lines: {e}"),
            Error::ParseCmd(e) => write!(f, "Error parsing command input: {e}"),
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
    let _buffers = initialize_buffers(args)?;

    // Initialize context (e.g., current buffer)
    let mut _current_buffer = 0;
    let mut cmd_buf = String::new();

    // Accept and process commands until fatal error or exit
    loop {
        // write prompt
        write_prompt(&mut output)?;

        // read command
        cmd_buf.clear();
        read_command(&mut input, &mut cmd_buf)?;

        // parse command
        let cmd = cmd_buf.parse::<Cmd>().map_err(Error::ParseCmd);

        // execute command
        match cmd {
            Err(e) => {
                eprintln!("{e}");
                continue;
            }
            Ok(Cmd::Quit) => return Ok(()),
        }
    }
}

fn write_prompt<W>(output: &mut W) -> Result<(), Error>
where
    W: Write,
{
    write!(output, ":").map_err(Error::WritePrompt)?;
    output.flush().map_err(Error::WritePrompt)?;
    Ok(())
}

fn read_command<R>(mut input: R, cmd_buf: &mut String) -> Result<usize, Error>
where
    R: BufRead,
{
    input.read_line(cmd_buf).map_err(Error::ReadCommand)
}

fn initialize_buffers(args: &cli::CmdArgs) -> Result<Vec<EditBuffer>, Error> {
    let mut buffers = Vec::with_capacity(args.files.len());

    if !buffers.is_empty() {
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
fn read_lines<R>(mut reader: R, buf: &mut Vec<String>) -> Result<usize, Error>
where
    R: BufRead,
{
    let mut line = String::new(); // single line input buffer
    buf.clear(); // get rid of any old input

    loop {
        reader.read_line(&mut line).map_err(Error::ReadLines)?;
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

    struct BadWriter {}

    impl Write for BadWriter {
        fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
            Err(io::Error::from(io::ErrorKind::Other))
        }

        fn flush(&mut self) -> io::Result<()> {
            Err(io::Error::from(io::ErrorKind::Other))
        }
    }
    struct BadReader {}

    impl Read for BadReader {
        fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::from(io::ErrorKind::Other))
        }
    }

    ////
    // write_prompt() tests

    #[test]
    fn write_prompt_io_error_gives_correct_error() {
        let mut output = BadWriter {};
        let _res = write_prompt(&mut output);
        assert!(matches!(Err::<Error, _>(Error::WritePrompt), _res));
    }

    #[test]
    fn write_prompt_clean_buffer() {
        let mut output = Vec::new();
        write_prompt(&mut output).expect("write should never fail");
        assert_eq!(b":", &output[..]);
    }

    ////
    // read_command() tests

    #[test]
    fn read_command_io_error_gives_correct_error() {
        let input = BadReader {};
        let mut input = io::BufReader::new(input);
        let mut cmd = String::new();
        let _res = read_command(&mut input, &mut cmd);
        assert!(matches!(Err::<Error, _>(Error::ReadCommand), _res));
    }

    #[test]
    fn read_command_returns_input() {
        let cmd_input = "q\n";
        let mut input = Vec::new();
        input.extend(cmd_input.as_bytes());
        let mut cmd_ret = String::new();
        let len = read_command(&input[..], &mut cmd_ret).expect("Error reading command string");
        assert_eq!(2, len);
        assert_eq!(cmd_input.trim(), cmd_ret.trim());
    }

    ////
    // read_lines() tests

    #[test]
    fn read_line_io_error_gives_correct_error() {
        let input = BadReader {};
        let mut input = io::BufReader::new(input);
        let mut lines = Vec::new();
        let _line_count = read_lines(&mut input, &mut lines);
        assert!(matches!(Err::<Error, _>(Error::ReadLines), _line_count));
    }

    #[test]
    fn read_lines_with_no_input_gives_zero_lines() {
        let input = b".\n";
        let mut lines = Vec::new();
        let line_count = read_lines(&input[..], &mut lines).expect("Error reading lines");
        assert_eq!(0, line_count);
        assert_eq!(0, lines.len());
    }

    #[test]
    fn read_lines_returns_lines_entered() {
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

    ////
    // initialize_buffers() tests

    #[test]
    fn initialize_buffers_no_files_gives_single_empty_buffer() {
        let args = cli::CmdArgs {
            files: Vec::new(),
            ..cli::CmdArgs::default()
        };
        let buffers = initialize_buffers(&args).unwrap();
        assert_eq!(1, buffers.len());
        assert_eq!(0, buffers[0].len());
    }
}
