use crate::cli;
use crate::command::{self, Address, Cmd};
use crate::edit_buffer::EditBuffer;
use std::fmt;
use std::io::{self, prelude::*};

#[derive(Debug)]
pub enum Error {
    /// I/O Error writing output
    WriteOutput(io::Error),
    /// I/O Error reading command input
    ReadCommand(io::Error),
    /// I/O Error reading input lines
    ReadLines(io::Error),
    ParseCmd(command::Error),
    Other(String),
}

impl std::error::Error for Error {}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Error::ReadCommand(e) => write!(f, "Error reading command: {e}"),
            Error::WriteOutput(e) => write!(f, "Error writing output: {e}"),
            Error::ReadLines(e) => write!(f, "Error reading input lines: {e}"),
            Error::ParseCmd(e) => write!(f, "Bad command: {e}"),
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
    let mut buffers = initialize_buffers(args)?;

    // Initialize context (e.g., current buffer)
    let current_buffer = 0;
    let mut cmd_buf = String::new();
    let mut prev_command: Option<Cmd> = None;
    let mut previous_pattern: Option<regex::Regex> = None;

    // Accept and process commands until fatal error or exit
    let mut done = false;
    while !done {
        // write prompt
        write_prompt(&mut output)?;

        // read command
        cmd_buf.clear();
        read_command(&mut input, &mut cmd_buf)?;

        let mut cmd_chars = cmd_buf.chars().peekable();

        done = Cmd::parse(
            &mut cmd_chars,
            &mut buffers[current_buffer],
            &mut previous_pattern,
        )
        .map_err(Error::ParseCmd)
        .and_then(|cmd| match cmd {
            // dispatch command
            Cmd::Quit => do_quit(&mut prev_command, &buffers),
            Cmd::Print(address) => do_print(&mut output, address, &mut buffers[current_buffer]),
            Cmd::Null(_address) => todo!(),
        })
        .or_else(|e| {
            eprintln!("{e}");
            Ok(false)
        })?;
    }
    Ok(())
}

fn do_quit(prev_command: &mut Option<Cmd>, buffers: &[EditBuffer]) -> Result<bool, Error> {
    Ok(ok_to_exit(prev_command, buffers))
}

fn do_print<W>(
    output: &mut W,
    address: Option<Address>,
    buffer: &mut EditBuffer,
) -> Result<bool, Error>
where
    W: Write,
{
    match address {
        Some(Address::Line(n)) => {
            output
                .write_all(buffer[n].as_bytes())
                .map_err(Error::WriteOutput)?;
            buffer.set_current_line(n);
        }
        Some(Address::Span(first, last)) => {
            for l in &buffer[first..=last] {
                output.write_all(l.as_bytes()).map_err(Error::WriteOutput)?;
            }
            buffer.set_current_line(last);
        }
        None => {
            if buffer.current_line() == 0 {
                return Err(Error::ParseCmd(command::Error::InvalidLineNumber));
            }
            output
                .write_all(buffer[buffer.current_line()].as_bytes())
                .map_err(Error::WriteOutput)?;
        }
    }
    output.write(b"\n").map_err(Error::WriteOutput)?;
    output.flush().map_err(Error::WriteOutput)?;
    Ok(false)
}

fn ok_to_exit(prev_command: &mut Option<Cmd>, buffers: &[EditBuffer]) -> bool {
    let ok = match prev_command {
        Some(Cmd::Quit) => true,
        _ => !buffers.iter().any(|buf| buf.needs_write()),
    };
    if !ok {
        eprintln!("Unwritten changes - a second quit will exit w/o saving.");
        *prev_command = Some(Cmd::Quit);
    }
    ok
}

fn write_prompt<W>(output: &mut W) -> Result<(), Error>
where
    W: Write,
{
    write!(output, ":").map_err(Error::WriteOutput)?;
    output.flush().map_err(Error::WriteOutput)?;
    Ok(())
}

fn read_command<R>(mut input: R, cmd_buf: &mut String) -> Result<usize, Error>
where
    R: BufRead,
{
    input.read_line(cmd_buf).map_err(Error::ReadCommand)
}

fn initialize_buffers(args: &cli::CmdArgs) -> Result<Vec<EditBuffer>, Error> {
    let mut buffers = Vec::new();

    // TODO
    // loop through file names
    // for each name, try to create an EditBuffer from that file
    //   on sucess, push to buffer list
    //   on error, print error message
    // if buffer list is empty, push new empty buffer onto buffer list

    if !args.files.is_empty() {
        return Err(Error::Other(
            "Reading files not yet implemented".to_string(),
        ));
    }

    // No files passed in, or none read successfully, so
    // we must allocate an empty buffer to use instead
    if buffers.is_empty() {
        buffers.push(EditBuffer::new());
    }

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
        assert!(matches!(Err::<Error, _>(Error::WriteOutput), _res));
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

    ////
    // ok_to_exit() tests

    #[test]
    fn ok_to_exit_if_prev_cmd_was_quit() {
        let mut prev_cmd = Some(Cmd::Quit);
        let buffers = vec![EditBuffer::new()];
        let safe = ok_to_exit(&mut prev_cmd, &buffers);
        assert!(safe);
    }

    #[test]
    fn ok_to_exit_if_buffer_unchanged() {
        let mut prev_cmd = None;
        let buffers = vec![EditBuffer::new()];
        let safe = ok_to_exit(&mut prev_cmd, &buffers);
        assert!(safe);
    }

    #[test]
    fn do_print_no_addr() {
        let mut output = Vec::new();
        let mut buffer = EditBuffer::from(vec!["1\r\n", "2", "3"]);
        buffer.set_current_line(2);
        let res = do_print(&mut output, None, &mut buffer).expect("successful print");
        assert_eq!(false, res);
        assert_eq!(b"2\r\n\n", &output[..]);
    }

    #[test]
    fn do_print_single_line() {
        let mut output = Vec::new();
        let mut buffer = EditBuffer::from(vec!["1\r\n", "2", "3"]);
        buffer.set_current_line(2);
        let res =
            do_print(&mut output, Some(Address::Line(3)), &mut buffer).expect("successful print");
        assert_eq!(false, res);
        assert_eq!(b"3\r\n\n", &output[..]);
    }

    #[test]
    fn do_print_span() {
        let mut output = Vec::new();
        let mut buffer = EditBuffer::from(vec!["1\r\n", "2", "3", "4", "5", "6"]);
        buffer.set_current_line(5);
        let res = do_print(&mut output, Some(Address::Span(2, 4)), &mut buffer)
            .expect("successful print");
        assert_eq!(false, res);
        assert_eq!(b"2\r\n3\r\n4\r\n\n", &output[..]);
    }

    #[test]
    fn do_print_empty_buffer_gives_error() {
        let mut output = Vec::new();
        let mut buffer = EditBuffer::new();
        let res = do_print(&mut output, None, &mut buffer);
        assert!(match res {
            Err(Error::ParseCmd(e)) => e == command::Error::InvalidLineNumber,
            _ => false,
        });
    }
}
