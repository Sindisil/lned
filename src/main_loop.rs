use std::collections::VecDeque;
use std::fmt;
use std::io::{self, prelude::*};

use regex::Regex;

use crate::cli;
use crate::command::{self, Address, Cmd};
use crate::edit_buffer::{self, EditBuffer};
use crate::num_utils::NumUtils;

#[derive(Debug)]
pub enum Error {
    WriteOutput(io::Error),
    ParseCmd(command::Error),
    BufferCmd(edit_buffer::Error),
    InvalidAddress,
    NestedGlobalCmd,
    UnsupportedGlobalCmd,
    ReadGlobalCmd,
}

impl std::error::Error for Error {}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::WriteOutput(e) => write!(f, "error writing output: {e}"),
            Error::ParseCmd(e) => write!(f, "Bad command: {e}"),
            Error::BufferCmd(e) => write!(f, "{e}"),
            Error::InvalidAddress => write!(f, "invalid address"),
            Error::NestedGlobalCmd => write!(f, "invalid nested global command"),
            Error::UnsupportedGlobalCmd => write!(f, "unsupported global command"),
            Error::ReadGlobalCmd => write!(f, "error reading global command"),
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

    if let Some(_file) = &args.file {
        todo!("attempt to edit specified file");
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
                    Cmd::Append(address) => buffer
                        .prepare_append(&mut input, *address)
                        .map_err(Error::BufferCmd),
                    Cmd::Delete(address) => {
                        buffer.prepare_delete(*address).map_err(Error::BufferCmd)
                    }
                    Cmd::Edit(_file) => {
                        todo!()
                    }
                    Cmd::Enumerate(address) => do_enumerate(&mut buffer, &mut output, *address),
                    Cmd::File(filename) => buffer
                        .do_file(&mut output, filename.as_deref())
                        .map_err(Error::BufferCmd),
                    Cmd::Global(address, pattern, commands) => do_global(
                        &mut buffer,
                        &mut output,
                        *address,
                        pattern,
                        commands,
                        &mut previous_pattern,
                    ),
                    Cmd::Insert(address) => buffer
                        .prepare_insert(&mut input, *address)
                        .map_err(Error::BufferCmd),
                    Cmd::Null(address) => do_null(&mut buffer, &mut output, *address),
                    Cmd::Print(address) => do_print(&mut buffer, &mut output, *address),
                    Cmd::Quit => do_quit(&mut output, &buffer, &prev_command).map(|ok_to_exit| {
                        done = ok_to_exit;
                    }),
                    Cmd::Write(address, filename) => buffer
                        .do_write(&mut output, *address, filename.as_deref())
                        .map_err(Error::BufferCmd),
                    Cmd::Undo => {
                        buffer.do_undo();
                        Ok(())
                    }
                    Cmd::Redo => {
                        buffer.do_redo();
                        Ok(())
                    }
                };
                prev_command = Some(cmd);
                res
            })
            .or_else(|e| writeln!(output, "{e}").map_err(Error::WriteOutput))?;
    }
    Ok(())
}

pub fn do_enumerate(
    buffer: &mut EditBuffer,
    output: &mut impl Write,
    address: Option<Address>,
) -> Result<(), Error> {
    let span = if let Some(Address(b, e)) = address {
        b..=e
    } else {
        if buffer.current_line() == 0 {
            return Err(Error::InvalidAddress);
        }
        buffer.current_line()..=buffer.current_line()
    };

    if *span.start() < 1
        || *span.start() > buffer.len()
        || *span.end() < 1
        || *span.end() > buffer.len()
    {
        return Err(Error::InvalidAddress);
    }

    let width = span.end().decimal_digits();
    let start = *span.start();
    buffer.set_current_line(*span.end());

    for (i, l) in buffer[span].iter().enumerate() {
        output
            .write_all(format!("{:>width$}  {l}", start + i).as_bytes())
            .map_err(Error::WriteOutput)?;
    }
    output.flush().map_err(Error::WriteOutput)?;
    Ok(())
}

pub fn do_global(
    buffer: &mut EditBuffer,
    output: &mut impl Write,
    address: Option<Address>,
    pattern: &Regex,
    commands: &str,
    previous_pattern: &mut Option<Regex>,
) -> Result<(), Error> {
    // make a list of matching lines
    let search_range = address.map_or_else(|| 1..=buffer.len(), |a| a.0..=a.1);
    let mut matched_lines = (search_range)
        .filter(|&n| {
            buffer[n]
                .lines()
                .next()
                .map_or(false, |l| pattern.is_match(l))
        })
        .collect::<VecDeque<usize>>();

    // iterate over list
    while let Some(line_num) = matched_lines.pop_front() {
        buffer.set_current_line(line_num);
        let mut input = commands.as_bytes();

        // parse and execute command list for line
        let cmd =
            Cmd::read(&mut input, buffer, previous_pattern).map_err(|_| Error::ReadGlobalCmd)?;
        match cmd {
            Cmd::Enumerate(address) => do_enumerate(buffer, output, address)?,
            Cmd::Global(..) => return Err(Error::NestedGlobalCmd),
            Cmd::Null(address) | Cmd::Print(address) => do_print(buffer, output, address)?,
            _ => return Err(Error::UnsupportedGlobalCmd),
        }
    }

    Ok(())
}

pub fn do_null(
    buffer: &mut EditBuffer,
    output: &mut impl Write,
    address: Option<Address>,
) -> Result<(), Error> {
    match address {
        None => {
            if buffer.is_empty() || buffer.current_line() == buffer.len() {
                return Err(Error::InvalidAddress);
            }
            do_print(
                buffer,
                output,
                Some(Address(
                    buffer.current_line() + 1,
                    buffer.current_line() + 1,
                )),
            )
        }
        _ => do_print(buffer, output, address),
    }
}

pub fn do_print(
    buffer: &mut EditBuffer,
    output: &mut impl Write,
    address: Option<Address>,
) -> Result<(), Error> {
    let span = if let Some(Address(b, e)) = address {
        b..=e
    } else {
        if buffer.current_line() == 0 {
            return Err(Error::InvalidAddress);
        }
        buffer.current_line()..=buffer.current_line()
    };

    if *span.start() < 1
        || *span.start() > buffer.len()
        || *span.end() < 1
        || *span.end() > buffer.len()
    {
        return Err(Error::InvalidAddress);
    }

    buffer.set_current_line(*span.end());
    for l in &buffer[span] {
        output.write_all(l.as_bytes()).map_err(Error::WriteOutput)?;
    }
    output.flush().map_err(Error::WriteOutput)?;
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
) -> Result<bool, Error> {
    match prev_command {
        Some(Cmd::Quit) => Ok(true),
        _ if !buffer.is_dirty() => Ok(true),
        _ => {
            writeln!(
                output,
                "Unwritten changes - repeat quit command to discard changes."
            )
            .map_err(Error::WriteOutput)?;
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
    fn do_null_no_addr() {
        let mut output = Vec::new();
        let mut buffer = EditBuffer::from(vec!["1\r\n", "2", "3"]);
        buffer.set_current_line(2);
        do_null(&mut buffer, &mut output, None).unwrap();
        assert_eq!(&output[..], b"3\r\n");
    }

    #[test]
    fn do_null_single_line() {
        let mut output = Vec::new();
        let mut buffer = EditBuffer::from(vec!["1\r\n", "2", "3"]);
        buffer.set_current_line(2);
        do_null(&mut buffer, &mut output, Some(Address(3, 3))).unwrap();
        assert_eq!(&output[..], b"3\r\n");
    }

    #[test]
    fn do_null_span() {
        let mut output = Vec::new();
        let mut buffer = EditBuffer::from(vec!["1\r\n", "2", "3", "4", "5", "6"]);
        buffer.set_current_line(5);
        do_null(&mut buffer, &mut output, Some(Address(2, 4))).unwrap();
        assert_eq!(&output[..], b"2\r\n3\r\n4\r\n");
    }

    #[test]
    fn do_null_sets_current_line() {
        let mut output = Vec::new();
        let mut buffer = EditBuffer::from(vec!["1\r\n", "2", "3", "4", "5", "6"]);
        buffer.set_current_line(5);
        do_null(&mut buffer, &mut output, Some(Address(2, 4))).unwrap();
        assert_eq!(4, buffer.current_line());
    }

    #[test]
    fn do_null_empty_buffer_gives_error() {
        let mut output = Vec::new();
        let mut buffer = EditBuffer::new();
        let res = do_null(&mut buffer, &mut output, None).expect_err("invalid address");
        assert!(matches!(res, Error::InvalidAddress));
        let res =
            do_null(&mut buffer, &mut output, Some(Address(0, 0))).expect_err("invalid address");
        assert!(matches!(res, Error::InvalidAddress));
    }

    #[test]
    fn enumerate_empty_buffer_error() {
        let mut output = Vec::new();
        let mut buffer = EditBuffer::new();
        let res = do_enumerate(&mut buffer, &mut output, None).expect_err("invalid address");
        assert!(matches!(res, Error::InvalidAddress));
        let res = do_enumerate(&mut buffer, &mut output, Some(Address(1, 1)))
            .expect_err("invalid address");
        assert!(matches!(res, Error::InvalidAddress));
    }

    #[test]
    fn enumerate_sm_buffer() {
        let mut output = Vec::new();
        let mut buffer =
            EditBuffer::from(vec!["1\r\n", "2", "3", "4", "5", "6", "7", "8", "9", "10"]);
        buffer.set_current_line(2);
        do_enumerate(&mut buffer, &mut output, None).unwrap();
        assert_eq!(&output[..], b"2  2\r\n", "output line 2");
    }

    #[test]
    fn enumerate_sets_current_line() {
        let mut output = Vec::new();
        let mut buffer =
            EditBuffer::from(vec!["1\r\n", "2", "3", "4", "5", "6", "7", "8", "9", "10"]);
        buffer.set_current_line(2);

        do_enumerate(&mut buffer, &mut output, Some(Address(6, 9))).unwrap();
    }

    #[test]
    fn enumerate_lg_buffer() {
        let mut output = Vec::new();
        let mut buffer =
            EditBuffer::from(vec!["1\r\n", "2", "3", "4", "5", "6", "7", "8", "9", "10"]);
        let mut input: Vec<u8> = Vec::new();
        for i in 11..=1024 {
            input.extend_from_slice(format!("{i}\r\n").as_bytes());
        }
        input.extend_from_slice(".\n".as_bytes());
        let mut input = &input[..];

        buffer
            .prepare_append(&mut input, Some(Address(buffer.len(), buffer.len())))
            .unwrap();
        buffer.set_current_line(2);
        assert_eq!(1024, buffer.len());
        output.clear();

        do_enumerate(&mut buffer, &mut output, Some(Address(4, 900))).unwrap();
        let expected = b"  4  4\r\n";
        assert_eq!(&expected[..], &output[0..expected.len()]);
        output.clear();

        do_enumerate(&mut buffer, &mut output, Some(Address(999, 999))).unwrap();
        let expected = b"999  999\r\n";
        assert_eq!(&expected[..], &output[0..expected.len()]);
    }

    #[test]
    fn do_global_no_matches() {
        let mut buffer = EditBuffer::from(vec!["one\n", "two", "three"]);
        let mut output = Vec::new();
        let mut prev_pattern: Option<Regex> = None;
        let pat = Regex::new("four").unwrap();
        let commands = "p\n".to_owned();
        do_global(
            &mut buffer,
            &mut output,
            None,
            &pat,
            &commands,
            &mut prev_pattern,
        )
        .unwrap();
        assert!(output.is_empty());
    }

    #[test]
    fn do_global_illegal_nested_gobal() {
        let mut buffer = EditBuffer::from(vec!["one\r\n", "two", "three"]);
        buffer.set_current_line(1);
        let mut output = Vec::new();
        let mut prev_pattern: Option<Regex> = None;
        let pat = Regex::new("t..").unwrap();
        let commands = "1,2g/ee/n\n".to_owned();
        let res = do_global(
            &mut buffer,
            &mut output,
            None,
            &pat,
            &commands,
            &mut prev_pattern,
        )
        .expect_err("nested global");
        assert!(matches!(res, Error::NestedGlobalCmd));
    }

    #[test]
    fn do_global_blank_command_print() {
        let mut buffer = EditBuffer::from(vec!["one\r\n", "two", "three", "tweedle dee"]);
        buffer.set_current_line(3);
        let mut output = Vec::new();
        let mut prev_pattern: Option<Regex> = None;
        let pat = Regex::new("t..").unwrap();
        let commands = "\n".to_owned();
        do_global(
            &mut buffer,
            &mut output,
            Some(Address(1, 3)),
            &pat,
            &commands,
            &mut prev_pattern,
        )
        .unwrap();
        assert_eq!(str::from_utf8(&output[..]).unwrap(), "two\r\nthree\r\n");
    }

    #[test]
    fn do_global_print() {
        let mut buffer = EditBuffer::from(vec!["one\n", "two", "three"]);
        buffer.set_current_line(1);
        let mut output = Vec::new();
        let mut prev_pattern: Option<Regex> = None;
        let pat = Regex::new("t..").unwrap();
        let commands = "p\r\n".to_owned();
        do_global(
            &mut buffer,
            &mut output,
            None,
            &pat,
            &commands,
            &mut prev_pattern,
        )
        .unwrap();
        assert_eq!(str::from_utf8(&output[..]).unwrap(), "two\nthree\n");
    }

    #[test]
    fn do_global_enumerate() {
        let mut buffer = EditBuffer::from(vec!["one\n", "two", "three"]);
        buffer.set_current_line(1);
        let mut output = Vec::new();
        let mut prev_pattern: Option<Regex> = None;
        let pat = Regex::new("t..").unwrap();
        let commands = "n\r\n".to_owned();
        do_global(
            &mut buffer,
            &mut output,
            Some(Address(1, 3)),
            &pat,
            &commands,
            &mut prev_pattern,
        )
        .unwrap();
        assert_eq!(str::from_utf8(&output[..]).unwrap(), "2  two\n3  three\n");
    }

    #[test]
    fn do_global_enumerate_with_addresses() {
        let mut buffer = EditBuffer::from(vec!["one\n", "two", "three", "four", "five", "six"]);
        buffer.set_current_line(6);
        let mut output = Vec::new();
        let mut prev_pattern: Option<Regex> = None;
        let pat = Regex::new("e$").unwrap();
        let commands = "-1,.n\r\n".to_owned();
        do_global(
            &mut buffer,
            &mut output,
            Some(Address(2, 5)),
            &pat,
            &commands,
            &mut prev_pattern,
        )
        .unwrap();
        assert_eq!(
            str::from_utf8(&output[..]).unwrap(),
            "2  two\n3  three\n4  four\n5  five\n"
        );
    }

    #[test]
    fn do_global_unsupported_commands() {
        let mut buffer = EditBuffer::from(vec!["one\r\n", "two", "three"]);
        buffer.set_current_line(1);
        let mut output = Vec::new();
        let mut prev_pattern: Option<Regex> = None;
        let pat = Regex::new(r"t..").unwrap();
        let commands = "e filename.txt\n".to_owned();
        let res = do_global(
            &mut buffer,
            &mut output,
            Some(Address(1, 3)),
            &pat,
            &commands,
            &mut prev_pattern,
        )
        .expect_err("unsupported global command");
        assert!(matches!(res, Error::UnsupportedGlobalCmd));
    }

    #[test]
    fn do_print_no_addr() {
        let mut output = Vec::new();
        let mut buffer = EditBuffer::from(vec!["1\r\n", "2", "3"]);
        buffer.set_current_line(2);
        do_print(&mut buffer, &mut output, None).unwrap();
        assert_eq!(&output[..], b"2\r\n");
    }

    #[test]
    fn do_print_single_line() {
        let mut output = Vec::new();
        let mut buffer = EditBuffer::from(vec!["1\r\n", "2", "3"]);
        buffer.set_current_line(2);
        do_print(&mut buffer, &mut output, Some(Address(3, 3))).unwrap();
        assert_eq!(&output[..], b"3\r\n");
    }

    #[test]
    fn do_print_span() {
        let mut output = Vec::new();
        let mut buffer = EditBuffer::from(vec!["1\r\n", "2", "3", "4", "5", "6"]);
        buffer.set_current_line(5);
        do_print(&mut buffer, &mut output, Some(Address(2, 4))).unwrap();
        assert_eq!(&output[..], b"2\r\n3\r\n4\r\n");
    }

    #[test]
    fn do_print_sets_current_line() {
        let mut output = Vec::new();
        let mut buffer = EditBuffer::from(vec!["1\r\n", "2", "3", "4", "5", "6"]);
        buffer.set_current_line(5);
        do_print(&mut buffer, &mut output, Some(Address(2, 4))).unwrap();
        assert_eq!(4, buffer.current_line());
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
    fn do_print_empty_buffer_gives_error() {
        let mut output = Vec::new();
        let mut buffer = EditBuffer::new();
        let res = do_print(&mut buffer, &mut output, None).expect_err("invalid address");
        assert!(matches!(res, Error::InvalidAddress));
        let res =
            do_print(&mut buffer, &mut output, Some(Address(0, 0))).expect_err("invalid address");
        assert!(matches!(res, Error::InvalidAddress));
    }

    #[test]
    #[ignore]
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
        assert_eq!(&output[..], &b":invalid address\n:"[..],);
    }

    #[test]
    #[ignore]
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
    #[ignore]
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
    #[ignore]
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
            file: None,
            //            file: Some(PathBuf::from("test/assets/text_with_final_eol.txt")),
        };
        run(&mut &input[..], &mut output, &args).unwrap();
        let output = str::from_utf8(&output[..]).unwrap();
        //        assert!(output.contains("test/assets/text_with_final_eol.txt"));
        assert!(output.contains("No current filename"));
        assert!(output.contains("new_file_name.txt"));
    }

    #[test]
    fn insert_cmd_dispatch() {
        let input = b"i\none\ntwo\nthree\n.\n2p\nq\nq\n";
        let mut output = Vec::new();
        run(&mut &input[..], &mut output, &CmdArgs::default()).unwrap();
        let output = str::from_utf8(&output[..]).unwrap();
        assert!(output.contains("two\n"));
        assert!(output.contains("Unwritten changes"));
        assert!(!output.contains("one"));
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
