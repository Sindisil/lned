use crate::cli;
use crate::command::{self, Address, Cmd};
use crate::edit_buffer::{EditBuffer, Remove};
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
}

impl std::error::Error for Error {}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Error::ReadCommand(e) => write!(f, "Error reading command: {e}"),
            Error::WriteOutput(e) => write!(f, "Error writing output: {e}"),
            Error::ReadLines(e) => write!(f, "Error reading input lines: {e}"),
            Error::ParseCmd(e) => write!(f, "Bad command: {e}"),
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
            &mut buffers,
            current_buffer,
            &mut previous_pattern,
        )
        .map_err(Error::ParseCmd)
        .and_then(|mut cmd| {
            let res = match cmd {
                // dispatch command
                Cmd::Quit => do_quit(&prev_command, &buffers),
                Cmd::Null(i, ref address) => do_null(&mut output, &mut buffers[i], address),
                Cmd::Print(i, ref address) => do_print(&mut output, &mut buffers[i], address),
                Cmd::Append(i, ref address, ref mut lines) => {
                    do_append(&mut input, &mut buffers[i], address, lines)
                }
                Cmd::Delete(i, ref address) => do_delete(&mut buffers[i], address),
            };
            prev_command = Some(cmd);
            res
        })
        .or_else(|e| {
            eprintln!("{e}");
            Ok(false)
        })?;
    }
    Ok(())
}

fn do_quit(prev_command: &Option<Cmd>, buffers: &[EditBuffer]) -> Result<bool, Error> {
    Ok(ok_to_exit(prev_command, buffers))
}

fn do_null<W>(
    output: &mut W,
    buffer: &mut EditBuffer,
    address: &Option<Address>,
) -> Result<bool, Error>
where
    W: Write,
{
    match address {
        None => {
            if buffer.is_empty() || buffer.current_line() == buffer.len() {
                return Err(Error::ParseCmd(command::Error::InvalidLineNumber));
            }
            do_print(
                output,
                buffer,
                &Some(Address::Line(buffer.current_line() + 1)),
            )
        }
        _ => do_print(output, buffer, address),
    }
}

fn do_print<W>(
    output: &mut W,
    buffer: &mut EditBuffer,
    address: &Option<Address>,
) -> Result<bool, Error>
where
    W: Write,
{
    match address {
        Some(Address::Line(n)) => {
            output
                .write_all(buffer[*n].as_bytes())
                .map_err(Error::WriteOutput)?;
            buffer.set_current_line(*n);
        }
        Some(Address::Span(first, last)) => {
            for l in &buffer[*first..=*last] {
                output.write_all(l.as_bytes()).map_err(Error::WriteOutput)?;
            }
            buffer.set_current_line(*last);
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

fn do_append<R>(
    input: R,
    buffer: &mut EditBuffer,
    address: &Option<Address>,
    lines: &mut Vec<String>,
) -> Result<bool, Error>
where
    R: BufRead,
{
    read_lines(input, lines)?;
    let i = match address {
        Some(Address::Line(line)) => *line,
        Some(Address::Span(_, last)) => *last,
        None => buffer.current_line(),
    };
    buffer.insert(i, lines);
    Ok(false)
}

fn do_delete(buffer: &mut EditBuffer, address: &Option<Address>) -> Result<bool, Error> {
    match address {
        Some(Address::Line(0)) => return Err(Error::ParseCmd(command::Error::InvalidLineNumber)),
        Some(Address::Line(n)) => buffer.remove(*n),
        Some(Address::Span(b, e)) => buffer.remove(*b..=*e),
        None if buffer.current_line() == 0 => {
            return Err(Error::ParseCmd(command::Error::InvalidLineNumber))
        }
        None => buffer.remove(buffer.current_line()),
    };
    Ok(false)
}

fn ok_to_exit(prev_command: &Option<Cmd>, buffers: &[EditBuffer]) -> bool {
    let ok = match prev_command {
        Some(Cmd::Quit) => true,
        _ => !buffers.iter().any(|buf| buf.needs_write()),
    };
    if !ok {
        eprintln!("Unwritten changes - a second quit will exit w/o saving.");
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
        todo!("Reading files not yet implemented");
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
    use io::BufReader;

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
        let mut input = BufReader::new(input);
        let mut cmd = String::new();
        let _res = read_command(&mut input, &mut cmd);
        assert!(matches!(Err::<Error, _>(Error::ReadCommand), _res));
    }

    #[test]
    fn read_command_returns_input() {
        let cmd_input = "q\n";
        let mut input = Vec::new();
        input.extend(cmd_input.bytes());
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
        let mut input = BufReader::new(input);
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

    #[test]
    fn read_lines_returns_lines_entered_crlf() {
        let three_lines = vec!["line1\n", "line 2\n", "line 3\n", ".\r\n"];
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
        let prev_cmd = Some(Cmd::Quit);
        let buffers = vec![EditBuffer::new()];
        let safe = ok_to_exit(&prev_cmd, &buffers);
        assert!(safe);
    }

    #[test]
    fn ok_to_exit_if_buffer_unchanged() {
        let prev_cmd = None;
        let buffers = vec![EditBuffer::new()];
        let safe = ok_to_exit(&prev_cmd, &buffers);
        assert!(safe);
    }

    ////
    // Cmd tests

    #[test]
    fn do_quit_twice_is_done() {
        let mut buffers = vec![EditBuffer::new()];
        let input = b"1\n2\n3\n.\n";
        let mut lines = Vec::new();
        let res =
            do_append(&input[..], &mut buffers[0], &None, &mut lines).expect("successful append");
        assert!(!res);
        let mut prev_command = Some(Cmd::Append(0, Some(Address::Line(0)), lines));
        let res = do_quit(&prev_command, &buffers).expect("no error");
        prev_command = Some(Cmd::Quit);
        assert!(!res);
        let res = do_quit(&prev_command, &buffers).expect("no error");
        assert!(res);
    }

    #[test]
    fn do_null_no_addr() {
        let mut output = Vec::new();
        let mut buffer = EditBuffer::from(vec!["1\r\n", "2", "3"]);
        buffer.set_current_line(2);
        let res = do_null(&mut output, &mut buffer, &None).expect("successful print");
        assert!(!res);
        assert_eq!(b"3\r\n\n", &output[..]);
    }

    #[test]
    fn do_null_single_line() {
        let mut output = Vec::new();
        let mut buffer = EditBuffer::from(vec!["1\r\n", "2", "3"]);
        buffer.set_current_line(2);
        let res =
            do_null(&mut output, &mut buffer, &Some(Address::Line(3))).expect("successful print");
        assert!(!res);
        assert_eq!(b"3\r\n\n", &output[..]);
    }

    #[test]
    fn do_null_span() {
        let mut output = Vec::new();
        let mut buffer = EditBuffer::from(vec!["1\r\n", "2", "3", "4", "5", "6"]);
        buffer.set_current_line(5);
        let res = do_null(&mut output, &mut buffer, &Some(Address::Span(2, 4)))
            .expect("successful print");
        assert!(!res);
        assert_eq!(b"2\r\n3\r\n4\r\n\n", &output[..]);
    }

    #[test]
    fn do_null_empty_buffer_gives_error() {
        let mut output = Vec::new();
        let mut buffer = EditBuffer::new();
        let res = do_null(&mut output, &mut buffer, &None);
        assert!(match res {
            Err(Error::ParseCmd(e)) => e == command::Error::InvalidLineNumber,
            _ => false,
        });
    }

    #[test]
    fn do_print_no_addr() {
        let mut output = Vec::new();
        let mut buffer = EditBuffer::from(vec!["1\r\n", "2", "3"]);
        buffer.set_current_line(2);
        let res = do_print(&mut output, &mut buffer, &None).expect("successful print");
        assert!(!res);
        assert_eq!(b"2\r\n\n", &output[..]);
    }

    #[test]
    fn do_print_single_line() {
        let mut output = Vec::new();
        let mut buffer = EditBuffer::from(vec!["1\r\n", "2", "3"]);
        buffer.set_current_line(2);
        let res =
            do_print(&mut output, &mut buffer, &Some(Address::Line(3))).expect("successful print");
        assert!(!res);
        assert_eq!(b"3\r\n\n", &output[..]);
    }

    #[test]
    fn do_print_span() {
        let mut output = Vec::new();
        let mut buffer = EditBuffer::from(vec!["1\r\n", "2", "3", "4", "5", "6"]);
        buffer.set_current_line(5);
        let res = do_print(&mut output, &mut buffer, &Some(Address::Span(2, 4)))
            .expect("successful print");
        assert!(!res);
        assert_eq!(b"2\r\n3\r\n4\r\n\n", &output[..]);
    }

    #[test]
    fn do_print_empty_buffer_gives_error() {
        let mut output = Vec::new();
        let mut buffer = EditBuffer::new();
        let res = do_print(&mut output, &mut buffer, &None);
        assert!(match res {
            Err(Error::ParseCmd(e)) => e == command::Error::InvalidLineNumber,
            _ => false,
        });
    }

    #[test]
    fn do_append_empty_buffer() {
        let mut buffer = EditBuffer::new();
        let mut lines = Vec::new();
        let input = b"some test text\nanother line\none more\n.\n";
        let res = do_append(&input[..], &mut buffer, &Some(Address::Line(0)), &mut lines)
            .expect("successful append");
        assert_eq!(3, buffer.current_line());
        assert_eq!(3, buffer.len());
        assert!(!res);
    }

    #[test]
    fn do_append_non_empty_at_0() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3"]);
        let mut lines = Vec::new();
        let input = b"some test text\nanother line\none more\n.\n";
        let res = do_append(&input[..], &mut buffer, &Some(Address::Line(0)), &mut lines)
            .expect("successful append");
        assert_eq!(3, buffer.current_line());
        assert_eq!(6, buffer.len());
        assert!(!res);
    }

    #[test]
    fn do_append_in_middle() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3"]);
        let mut lines = Vec::new();
        let input = b"some test text\nanother line\none more\n.\n";
        let res = do_append(&input[..], &mut buffer, &Some(Address::Line(2)), &mut lines)
            .expect("successful append");
        assert_eq!(5, buffer.current_line());
        assert_eq!(6, buffer.len());
        assert!(!res);
    }

    #[test]
    fn do_append_span_address() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        let mut lines = Vec::new();
        let input = b"some test text\nanother line\none more\n.\n";
        let res = do_append(
            &input[..],
            &mut buffer,
            &Some(Address::Span(2, 3)),
            &mut lines,
        )
        .expect("successful append");
        assert_eq!(6, buffer.current_line());
        assert_eq!(9, buffer.len());
        assert!(!res);
    }

    #[test]
    fn do_append_at_end() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3"]);
        let mut lines = Vec::new();
        let input = b"some test text\nanother line\none more\n.\n";
        let res = do_append(&input[..], &mut buffer, &Some(Address::Line(3)), &mut lines)
            .expect("successful append");
        assert_eq!(6, buffer.current_line());
        assert_eq!(6, buffer.len());
        assert!(!res);
    }

    #[test]
    fn do_append_of_zero_lines() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3"]);
        let mut lines = Vec::new();
        let input = b".\n";
        assert_eq!(3, buffer.current_line());
        let res = do_append(&input[..], &mut buffer, &Some(Address::Line(2)), &mut lines)
            .expect("successful append");
        assert_eq!(2, buffer.current_line());
        assert_eq!(3, buffer.len());
        assert!(!res);
    }

    #[test]
    fn do_delete_span() {
        let mut buffer = EditBuffer::from(vec!["1\r\n", "2", "3", "4", "5", "6"]);
        let res = do_delete(&mut buffer, &Some(Address::Span(3, 5))).expect("deleted span");
        assert!(!res);
        assert_eq!(3, buffer.len());
        assert_eq!(vec!["1\r\n", "2\r\n", "6\r\n"], buffer[..]);
    }

    #[test]
    fn do_delete_line() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        let res = do_delete(&mut buffer, &Some(Address::Line(3))).expect("deleted line");
        assert!(!res);
        assert_eq!(5, buffer.len());
        assert_eq!(vec!["1\n", "2\n", "4\n", "5\n", "6\n"], buffer[..]);
    }

    #[test]
    fn do_delete_span_at_start() {
        let mut buffer = EditBuffer::from(vec!["1\r\n", "2", "3", "4", "5", "6"]);
        let res = do_delete(&mut buffer, &Some(Address::Span(1, 3))).expect("deleted span");
        assert!(!res);
        assert_eq!(3, buffer.len());
        assert_eq!(vec!["4\r\n", "5\r\n", "6\r\n"], buffer[..]);
    }

    #[test]
    fn do_delete_span_at_end() {
        let mut buffer = EditBuffer::from(vec!["1\r\n", "2", "3", "4", "5", "6"]);
        let res = do_delete(&mut buffer, &Some(Address::Span(5, 6))).expect("deleted span");
        assert!(!res);
        assert_eq!(4, buffer.len());
        assert_eq!(vec!["1\r\n", "2\r\n", "3\r\n", "4\r\n"], buffer[..]);
    }

    #[test]
    fn do_delete_no_addr() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        buffer.set_current_line(3);
        let res = do_delete(&mut buffer, &None).expect("deleted line");
        assert!(!res);
        assert_eq!(5, buffer.len());
        assert_eq!(vec!["1\n", "2\n", "4\n", "5\n", "6\n"], buffer[..]);
    }

    #[test]
    fn do_delete_empty_buffer() {
        let mut buffer = EditBuffer::new();
        let res = do_delete(&mut buffer, &None);
        assert!(match res {
            Err(Error::ParseCmd(e)) => e == command::Error::InvalidLineNumber,
            _ => false,
        });
    }
}
