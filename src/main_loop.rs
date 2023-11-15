use crate::cli;
use crate::command::{self, Cmd};
use crate::edit_buffer::{self, EditBuffer};
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
    BufferCmd(edit_buffer::Error),
}

impl std::error::Error for Error {}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Error::WriteOutput(e) => write!(f, "Error writing output: {e}"),
            Error::ReadCommand(e) => write!(f, "Error reading command: {e}"),
            Error::ReadLines(e) => write!(f, "{e} reading input lines"),
            Error::ParseCmd(e) => write!(f, "Bad command: {e}"),
            Error::BufferCmd(e) => write!(f, "buffer command error: {e}"),
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

        Cmd::parse(
            &mut cmd_chars,
            &mut buffers,
            current_buffer,
            &mut previous_pattern,
        )
        .map_err(Error::ParseCmd)
        .and_then(|cmd| {
            let res = match cmd {
                // dispatch editor commands
                Cmd::Append(address) => {
                    let mut lines = Vec::new();
                    let _line_count = read_lines(&mut input, &mut lines)?;
                    buffers[current_buffer]
                        .do_append(&mut output, address, lines)
                        .map_err(Error::BufferCmd)
                }
                Cmd::Delete(address) => buffers[current_buffer]
                    .do_delete(&mut output, address)
                    .map_err(Error::BufferCmd),
                Cmd::Edit(ref file) => buffers[current_buffer]
                    .do_edit(&mut output, file.as_deref(), prev_command.as_ref())
                    .map_err(Error::BufferCmd),
                Cmd::Enumerate(address) => buffers[current_buffer]
                    .do_enumerate(&mut output, address)
                    .map_err(Error::BufferCmd),
                Cmd::File(ref filename) => buffers[current_buffer]
                    .do_file(&mut output, filename.as_deref())
                    .map_err(Error::BufferCmd),
                Cmd::Null(address) => buffers[current_buffer]
                    .do_null(&mut output, address)
                    .map_err(Error::BufferCmd),
                Cmd::Print(address) => buffers[current_buffer]
                    .do_print(&mut output, address)
                    .map_err(Error::BufferCmd),
                Cmd::Quit => do_quit(&mut output, &buffers, &prev_command).map(|ok_to_exit| {
                    done = ok_to_exit;
                }),
                Cmd::Write(address, ref filename) => buffers[current_buffer]
                    .do_write(&mut output, address, filename.as_ref())
                    .map_err(Error::BufferCmd),
                Cmd::Undo => buffers[current_buffer]
                    .do_undo(&mut output)
                    .map_err(Error::BufferCmd),
                Cmd::Redo => buffers[current_buffer]
                    .do_redo(&mut output)
                    .map_err(Error::BufferCmd),
            };
            prev_command = Some(cmd);
            res
        })
        .or_else(|e| writeln!(output, "{e}").map_err(Error::WriteOutput))?;
    }
    Ok(())
}

fn do_quit<W>(
    output: &mut W,
    buffers: &[EditBuffer],
    prev_command: &Option<Cmd>,
) -> Result<bool, Error>
where
    W: Write,
{
    let ok = ok_to_exit(prev_command, buffers);
    if !ok {
        writeln!(
            output,
            "Unwritten changes - repeat quit command to discard changes."
        )
        .map_err(Error::WriteOutput)?;
    }
    Ok(ok)
}

fn ok_to_exit(prev_command: &Option<Cmd>, buffers: &[EditBuffer]) -> bool {
    match prev_command {
        Some(Cmd::Quit) => true,
        _ => !buffers.iter().any(|buf| buf.is_dirty()),
    }
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
fn read_lines<R>(reader: &mut R, buf: &mut Vec<String>) -> Result<usize, Error>
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

    /////
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

    /////
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
        let line_count = read_lines(&mut &input[..], &mut lines).expect("Error reading lines");
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
        let line_count = read_lines(&mut &input[..], &mut lines).expect("Error reading lines");

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
        let line_count = read_lines(&mut &input[..], &mut lines).expect("Error reading lines");

        assert_eq!(3, line_count);
        assert_eq!(3, lines.len());
        assert_eq!(three_lines[..3], lines);
    }

    /////
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

    /////
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

    /////
    // Cmd tests

    #[test]
    fn do_quit_twice_exits() {
        let input = b"a\n1\n2\n3\n.\nq\nq\n";
        let mut output = Vec::new();

        run(&mut &input[..], &mut output, &Default::default()).expect("no error");
        assert_eq!(
            &b"::Unwritten changes - repeat quit command to discard changes.\n:"[..],
            &output[..]
        );
    }

    #[test]
    fn do_edit_twice_overrides_warning() {
        let input =
            b"a\n1\n2\n3\n.\ne a_file_that_is_not_there.ext\ne a_file_that_is_not_there.ext\nq\n";
        let mut output = Vec::new();

        run(&mut &input[..], &mut output, &Default::default()).expect("no error");
        assert!(&output[..]
            .starts_with(b"::Unwritten changes - repeat edit command to discard changes.\n:"));
    }

    #[test]
    fn new_prompt_on_line_after_error_message() {
        let input = b"1p\nq\n";
        let mut output = Vec::new();

        run(&mut &input[..], &mut output, &Default::default()).expect("no error");
        assert_eq!(
            &output[..],
            &b":buffer command error: invalid address\n:"[..],
        );
    }
}
