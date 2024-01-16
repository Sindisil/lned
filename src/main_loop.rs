/// Main event loop for lned.
///
use std::fmt;
use std::io::{self, prelude::*};

use crate::cli;
use crate::command::{self, Cmd};
use crate::edit_buffer::{self, EditBuffer};

#[derive(Debug)]
pub enum Error {
    /// I/O Error writing output
    WriteOutput(io::Error),
    ParseCmd(command::Error),
    BufferCmd(edit_buffer::Error),
}

impl std::error::Error for Error {}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Error::WriteOutput(e) => write!(f, "Error writing output: {e}"),
            Error::ParseCmd(e) => write!(f, "Bad command: {e}"),
            Error::BufferCmd(e) => write!(f, "buffer command error: {e}"),
        }
    }
}

/// Main event loop.
///
/// Handles prompting, command input, command dispatch, and error display.
pub fn run(
    mut input: impl BufRead,
    mut output: impl Write,
    args: &cli::CmdArgs,
) -> Result<(), Error> {
    let mut buffer = EditBuffer::new();

    let mut prev_command: Option<Cmd> = None;
    let mut previous_pattern: Option<regex::Regex> = None;

    if let Some(file) = &args.file {
        buffer
            .do_edit(&mut output, Some(file), prev_command.as_ref())
            .or_else(|e| writeln!(output, "{e}").map_err(Error::WriteOutput))?;
    }

    // Accept and process commands until fatal error or exit
    let mut done = false;
    while !done {
        // write prompt
        write_prompt(&mut output)?;

        Cmd::read(&mut input, &mut buffer, &mut previous_pattern)
            .map_err(Error::ParseCmd)
            .and_then(|cmd| {
                let res = match &cmd {
                    // dispatch editor commands
                    Cmd::Append(address) => buffer.do_append(&mut input, &mut output, *address),
                    Cmd::Delete(address) => buffer.do_delete(&mut output, *address),
                    Cmd::Edit(file) => {
                        buffer.do_edit(&mut output, file.as_deref(), prev_command.as_ref())
                    }
                    Cmd::Enumerate(address) => buffer.do_enumerate(&mut output, *address),
                    Cmd::File(filename) => buffer.do_file(&mut output, filename.as_deref()),
                    Cmd::Global(address, pattern, commands) => buffer.do_global(
                        &mut output,
                        *address,
                        pattern,
                        commands,
                        &mut previous_pattern,
                    ),
                    Cmd::Null(address) => buffer.do_null(&mut output, *address),
                    Cmd::Print(address) => buffer.do_print(&mut output, *address),
                    Cmd::Quit => do_quit(&mut output, &buffer, &prev_command).map(|ok_to_exit| {
                        done = ok_to_exit;
                    }),
                    Cmd::Write(address, filename) => {
                        buffer.do_write(&mut output, *address, filename.as_deref())
                    }
                    Cmd::Undo => buffer.do_undo(&mut output),
                    Cmd::Redo => buffer.do_redo(&mut output),
                }
                .map_err(Error::BufferCmd);
                prev_command = Some(cmd);
                res
            })
            .or_else(|e| writeln!(output, "{e}").map_err(Error::WriteOutput))?;
    }
    Ok(())
}

/// Implements quit command.
///
/// Displays warning and doesn't actually exit if unwritten
/// buffer changes are detected.
fn do_quit(
    output: &mut impl Write,
    buffer: &EditBuffer,
    prev_command: &Option<Cmd>,
) -> Result<bool, edit_buffer::Error> {
    match prev_command {
        Some(Cmd::Quit) => Ok(true),
        _ if !buffer.is_dirty() => Ok(true),
        _ => {
            writeln!(
                output,
                "Unwritten changes - repeat quit command to discard changes."
            )
            .map_err(edit_buffer::Error::WriteOutput)?;
            Ok(false)
        }
    }
}

fn write_prompt(output: &mut impl Write) -> Result<(), Error> {
    write!(output, ":").map_err(Error::WriteOutput)?;
    output.flush().map_err(Error::WriteOutput)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::cli::CmdArgs;
    use std::path::PathBuf;
    use std::str;

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
        write_prompt(&mut output).unwrap();
        assert_eq!(b":", &output[..]);
    }

    #[test]
    fn do_quit_unchanged() {
        let input = &b"q\n"[..];
        let mut output = Vec::new();
        run(&mut &input[..], &mut output, &CmdArgs::default()).unwrap();
        assert_eq!(&output[..], &b":"[..]);
    }

    #[test]
    fn do_quit_twice_exits() {
        let input = b"a\n1\n2\n3\n.\nq\nq\n";
        let mut output = Vec::new();

        run(&mut &input[..], &mut output, &CmdArgs::default()).unwrap();
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

        run(&mut &input[..], &mut output, &CmdArgs::default()).unwrap();
        assert!(&output[..]
            .starts_with(b"::Unwritten changes - repeat edit command to discard changes.\n:"));
    }

    #[test]
    fn new_prompt_on_line_after_error_message() {
        let input = b"1p\nq\n";
        let mut output = Vec::new();

        run(&mut &input[..], &mut output, &CmdArgs::default()).unwrap();
        assert_eq!(
            &output[..],
            &b":buffer command error: invalid address\n:"[..],
        );
    }

    #[test]
    fn file_on_cmd_line() {
        let args = cli::CmdArgs {
            file: Some(
                ["test", "assets", "text_with_final_eol.txt"]
                    .iter()
                    .collect::<PathBuf>(),
            ),
        };
        let input = b"q\n";
        let mut output = Vec::new();
        run(&mut &input[..], &mut output, &args).unwrap();
        assert!(str::from_utf8(&output[..]).unwrap().contains("312\n"));
    }

    #[test]
    fn file_on_cmd_line_not_found() {
        let args = cli::CmdArgs {
            file: Some(PathBuf::from("not_a_file")),
        };
        let input = b"q\n";
        let mut output = Vec::new();
        run(&mut &input[..], &mut output, &args).unwrap();
        assert!(str::from_utf8(&output[..]).unwrap().contains("cannot find"));
    }

    #[test]
    fn append_cmd_dispatch() {
        let input = b"a\none\ntwo\nthree\n.\n2p\nq\nq\n";
        let mut output = Vec::new();
        run(&mut &input[..], &mut output, &CmdArgs::default()).unwrap();
        let output = str::from_utf8(&output[..]).unwrap();
        assert!(output.contains("two\n"));
        assert!(output.contains("Unwritten changes"));
        assert!(!output.contains("one"));
    }

    #[test]
    fn delete_cmd_dispatch() {
        let input = b"a\none\ntwo\nthree\n.\n1,2d\np\nd\np\nq\nq\n";
        let mut output = Vec::new();
        run(&mut &input[..], &mut output, &CmdArgs::default()).unwrap();
        let output = str::from_utf8(&output[..]).unwrap();
        assert!(output.contains("three"));
        assert!(output.contains("invalid address"));
    }

    #[test]
    fn edit_cmd_dispatch() {
        let input = b"e test/assets/text_with_final_eol.txt\nq\n";
        let mut output = Vec::new();
        run(&mut &input[..], &mut output, &CmdArgs::default()).unwrap();
        let output = str::from_utf8(&output[..]).unwrap();
        assert!(output.contains("312"));
    }

    #[test]
    fn enumerate_cmd_dispatch() {
        let input = b"a\none\ntwo\nthree\n.\n2,3n\nq\nq\n";
        let mut output = Vec::new();
        run(&mut &input[..], &mut output, &CmdArgs::default()).unwrap();
        let output = str::from_utf8(&output[..]).unwrap();
        assert!(output.contains("2  two\n3  three\n"));
    }

    #[test]
    fn file_cmd_dispatch() {
        let input = b"f\nf new_file_name.txt\nq\n";
        let mut output = Vec::new();
        let args = CmdArgs {
            file: Some(PathBuf::from("test/assets/text_with_final_eol.txt")),
        };
        run(&mut &input[..], &mut output, &args).unwrap();
        let output = str::from_utf8(&output[..]).unwrap();
        assert!(output.contains("test/assets/text_with_final_eol.txt"));
        assert!(output.contains("new_file_name.txt"));
    }

    #[test]
    fn global_cmd_dispatch() {
        let input = b"a\none\ntwo\nthree\nfour\nfive\n.\ng/e$/n\nq\nq\n";
        let mut output = Vec::new();
        run(&mut &input[..], &mut output, &CmdArgs::default()).unwrap();
        let output = str::from_utf8(&output[..]).unwrap();
        assert!(output.contains("1  one\n3  three\n5  five\n"));
    }

    #[test]
    fn null_cmd_dispatch() {
        let input = b"a\r\none\r\ntwo\r\nthree\r\n.\r\n1\r\n\r\nq\r\nq\r\n";
        let mut output = Vec::new();
        run(&mut &input[..], &mut output, &CmdArgs::default()).unwrap();
        let output = str::from_utf8(&output[..]).unwrap();
        assert!(output.contains("two"));
    }

    #[test]
    fn print_cmd_dispatch() {
        let input = b"a\none\ntwo\nthree\n.\n1,2p\nq\nq\n";
        let mut output = Vec::new();
        run(&mut &input[..], &mut output, &CmdArgs::default()).unwrap();
        let output = str::from_utf8(&output[..]).unwrap();
        assert!(output.contains("one\ntwo\n"));
    }

    #[test]
    fn quit_cmd_dispatch() {
        let input = b"q\r\n";
        let mut output = Vec::new();
        run(&mut &input[..], &mut output, &CmdArgs::default()).unwrap();
        assert!(str::from_utf8(&output[..]).unwrap() == ":");
    }

    #[test]
    fn write_cmd_dispatch() {
        let input = b"a\none\n.\nw\nq\nq\n";
        let mut output = Vec::new();
        run(&mut &input[..], &mut output, &CmdArgs::default()).unwrap();
        let output = str::from_utf8(&output[..]).unwrap();
        assert!(output.contains("No filename"));
    }

    #[test]
    fn undo_cmd_dispatch() {
        let input = b"a\n1\n2\n3\n.\n2,3d\np\nu\np\nu\nq\n";
        let mut output = Vec::new();
        run(&mut &input[..], &mut output, &CmdArgs::default()).unwrap();
        let output = str::from_utf8(&output[..]).unwrap();
        assert!(output.contains("1\n"));
        assert!(output.contains("3\n"));
    }

    #[test]
    fn redo_cmd_dispatch() {
        let input = b"a\n1\n2\n3\n.\n2,3d\nu\nU\n3p\nq\nq\n";
        let mut output = Vec::new();
        run(&mut &input[..], &mut output, &CmdArgs::default()).unwrap();
        let output = str::from_utf8(&output[..]).unwrap();
        assert!(output.contains("invalid address"));
        assert!(output.contains("Unwritten changes"));
    }
}
