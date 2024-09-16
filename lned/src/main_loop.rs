use std::collections::VecDeque;
use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::{self, prelude::*, BufReader};
use std::path::Path;

use regex::Regex;

use crate::cli;
use crate::command::{self, Address, Cmd};
use crate::edit_buffer::EditBuffer;
use crate::num_utils::NumUtils;

use line_reader::LineRead;

#[derive(Debug)]
pub enum Error {
    ParseCmd(command::Error),
    InvalidAddress,
    NestedGlobalCmd,
    UnsupportedGlobalCmd,
    ReadGlobalCmd,
    NoFilename,
    EditFileOpen { source: std::io::Error },
    WriteFileOpen { source: std::io::Error },
    WriteLines { source: std::io::Error },
    ReadLines { source: std::io::Error },
    QuitUnwrittenChanges,
    EditUnwrittenChanges,
    EditFileNotFound(String),
    DestinationIntersectsSource,
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match *self {
            Error::ParseCmd(_)
            | Error::EditFileNotFound(_)
            | Error::QuitUnwrittenChanges
            | Error::EditUnwrittenChanges
            | Error::InvalidAddress
            | Error::NestedGlobalCmd
            | Error::UnsupportedGlobalCmd
            | Error::ReadGlobalCmd
            | Error::DestinationIntersectsSource
            | Error::NoFilename => None,
            Error::EditFileOpen { ref source }
            | Error::WriteFileOpen { ref source }
            | Error::WriteLines { ref source }
            | Error::ReadLines { ref source } => Some(source),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::ParseCmd(e) => write!(f, "{e}"),
            Error::InvalidAddress => write!(f, "invalid address"),
            Error::NestedGlobalCmd => {
                write!(f, "invalid nested global command")
            }
            Error::UnsupportedGlobalCmd => {
                write!(f, "unsupported global command")
            }
            Error::ReadGlobalCmd => write!(f, "error reading global command"),
            Error::NoFilename => write!(f, "no filename"),
            Error::EditFileOpen { .. } => {
                write!(f, "error opening file to edit")
            }
            Error::WriteFileOpen { .. } => {
                write!(f, "error opening file for writing")
            }
            Error::WriteLines { .. } => write!(f, "error writing lines"),
            Error::ReadLines { .. } => write!(f, "error reading input lines"),
            Error::QuitUnwrittenChanges => {
                write!(
                    f,
                    "unwritten changes - repeat quit command to discard changes"
                )
            }
            Error::EditUnwrittenChanges => {
                write!(
                    f,
                    "unwritten changes - repeat edit command to discard changes"
                )
            }
            Error::EditFileNotFound(filename) => {
                write!(f, "{filename} not found")
            }
            Error::DestinationIntersectsSource => {
                write!(f, "destination intersects source")
            }
        }
    }
}

/// Main event loop.
///
/// Handles prompting, command input, command dispatch, and error display.
pub fn run(
    mut input: impl LineRead,
    mut output: impl Write,
    args: &cli::CmdArgs,
) -> Result<(), Error> {
    let mut buffer = EditBuffer::new();

    let mut previous_cmd: Option<Cmd> = None;
    let mut previous_pattern: Option<regex::Regex> = None;

    if let Some(file) = &args.file {
        edit_cmd(&mut buffer, &mut output, Some(file), &previous_cmd)
            .or_else(|e| writeln!(output, "{e}"))
            .unwrap();
    }

    // Accept and process commands until fatal error or exit
    let mut done = false;
    while !done {
        Cmd::read(&mut input, &mut buffer, &mut previous_pattern)
            .map_err(Error::ParseCmd)
            .and_then(|cmd| {
                let res = match &cmd {
                    // dispatch editor commands
                    Cmd::Append(address) => {
                        append_cmd(&mut buffer, &mut input, *address)
                    }
                    Cmd::Delete(address) => delete_cmd(&mut buffer, *address),
                    Cmd::Change(address) => {
                        change_cmd(&mut buffer, &mut input, *address)
                    }
                    Cmd::Edit(filename) => edit_cmd(
                        &mut buffer,
                        &mut output,
                        filename.as_deref(),
                        &previous_cmd,
                    ),
                    Cmd::Enumerate(address) => {
                        enumerate_cmd(&mut buffer, &mut output, *address)
                    }
                    Cmd::File(filename) => {
                        file_cmd(&mut buffer, &mut output, filename.as_deref());
                        Ok(())
                    }
                    Cmd::Global(address, pattern, commands) => global_cmd(
                        &mut buffer,
                        &mut output,
                        *address,
                        pattern,
                        commands,
                        &mut previous_pattern,
                    ),
                    Cmd::Insert(address) => {
                        insert_cmd(&mut buffer, &mut input, *address)
                    }
                    Cmd::Null(address) => {
                        null_cmd(&mut buffer, &mut output, *address)
                    }
                    Cmd::Print(address) => {
                        print_cmd(&mut buffer, &mut output, *address)
                    }
                    Cmd::Quit => {
                        quit_cmd(&buffer, &previous_cmd).map(|()| done = true)
                    }
                    Cmd::Redo => {
                        buffer.do_redo();
                        Ok(())
                    }
                    Cmd::Transfer(address, destination) => {
                        transfer_cmd(&mut buffer, *address, *destination)
                    }
                    Cmd::Undo => {
                        buffer.do_undo();
                        Ok(())
                    }
                    Cmd::Write(address, filename) => write_cmd(
                        &mut buffer,
                        &mut output,
                        *address,
                        filename.as_deref(),
                    ),
                };
                previous_cmd = Some(cmd);
                res
            })
            .or_else(|e| {
                writeln!(output, "{e}").unwrap();
                output.flush().unwrap();
                Ok(())
            })?;
    }
    Ok(())
}

fn append_cmd(
    buffer: &mut EditBuffer,
    input: &mut impl LineRead,
    address: Option<Address>,
) -> Result<(), Error> {
    if address.is_some_and(|a| a.end() > buffer.len()) {
        return Err(Error::InvalidAddress);
    }
    let mut lines = Vec::new();
    Cmd::read_lines(input, &mut lines)
        .map_err(|source| Error::ReadLines { source })?;
    //    let location = address.map_or(buffer.current_line(), |addr| addr.1);
    buffer.do_append(address, lines);
    Ok(())
}

fn change_cmd(
    buffer: &mut EditBuffer,
    input: &mut impl LineRead,
    address: Option<Address>,
) -> Result<(), Error> {
    if address.is_some_and(|a| a.end() > buffer.len()) {
        return Err(Error::InvalidAddress);
    }

    let mut lines = Vec::new();
    Cmd::read_lines(input, &mut lines)
        .map_err(|source| Error::ReadLines { source })?;
    buffer.do_change(address, lines);
    Ok(())
}

fn delete_cmd(
    buffer: &mut EditBuffer,
    address: Option<Address>,
) -> Result<(), Error> {
    match address {
        Some(addr) if addr.start() == 0 => Err(Error::InvalidAddress),
        None if buffer.current_line() == 0 => Err(Error::InvalidAddress),
        _ => {
            buffer.do_delete(address);
            Ok(())
        }
    }
}

fn edit_cmd(
    buffer: &mut EditBuffer,
    output: &mut impl Write,
    filename: Option<&Path>,
    previous_cmd: &Option<Cmd>,
) -> Result<(), Error> {
    if buffer.is_dirty() && !matches!(previous_cmd, Some(Cmd::Edit(_))) {
        return Err(Error::EditUnwrittenChanges);
    }

    if let Some(filename) = filename {
        buffer.set_filename(Some(filename.to_owned()));
    }
    let filename = buffer.filename();
    let filename = filename.as_ref().ok_or(Error::NoFilename)?;

    let file = File::open(filename);
    let mut source = match file {
        Ok(f) => BufReader::new(f),
        Err(e) => {
            return match e.kind() {
                io::ErrorKind::NotFound => {
                    let missing_file = String::from(filename.to_string_lossy());
                    buffer.clear_text();
                    Err(Error::EditFileNotFound(missing_file))
                }
                _ => Err(Error::EditFileOpen { source: e }),
            };
        }
    };

    let mut lines = Vec::new();
    let (lines_read, bytes_read) = read_lines(&mut source, &mut lines)?;
    writeln!(output, "{lines_read} lines ({bytes_read} bytes) read").unwrap();

    buffer.clear_text();
    if buffer.append(0, lines) {
        output.flush().unwrap();
        writeln!(output, "missing line terminator appended").unwrap();
    }
    Ok(())
}

fn enumerate_cmd(
    buffer: &mut EditBuffer,
    output: &mut impl Write,
    address: Option<Address>,
) -> Result<(), Error> {
    let span = if let Some(addr) = address {
        addr.into()
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

    let width = buffer.len().decimal_digits();
    let start = *span.start();
    buffer.set_current_line(*span.end());

    for (i, l) in buffer[span].iter().enumerate() {
        output
            .write_all(format!("{:>width$}  {l}", start + i).as_bytes())
            .unwrap();
    }
    output.flush().unwrap();
    Ok(())
}

fn file_cmd(
    buffer: &mut EditBuffer,
    output: &mut impl Write,
    filename: Option<&Path>,
) {
    if let Some(filename) = filename {
        buffer.set_filename(Some(filename.to_owned()));
    }

    match buffer.filename() {
        None => writeln!(output, "no current filename").unwrap(),
        Some(f) => writeln!(output, "{}", f.display()).unwrap(),
    }
    output.flush().unwrap();
}

fn global_cmd(
    buffer: &mut EditBuffer,
    output: &mut impl Write,
    address: Option<Address>,
    pattern: &Regex,
    commands: &str,
    previous_pattern: &mut Option<Regex>,
) -> Result<(), Error> {
    // make a list of matching lines
    let search_range = address.map_or_else(|| 1..=buffer.len(), Into::into);
    let mut matched_lines = (search_range)
        .filter(|&n| {
            buffer[n].lines().next().map_or(false, |l| pattern.is_match(l))
        })
        .collect::<VecDeque<usize>>();

    // iterate over list
    while let Some(line_num) = matched_lines.pop_front() {
        buffer.set_current_line(line_num);
        let mut input = commands.as_bytes();

        // parse and execute command list for line
        let cmd = Cmd::read(&mut input, buffer, previous_pattern)
            .map_err(|_| Error::ReadGlobalCmd)?;
        match cmd {
            Cmd::Enumerate(address) => enumerate_cmd(buffer, output, address)?,
            Cmd::Global(..) => return Err(Error::NestedGlobalCmd),
            Cmd::Null(address) | Cmd::Print(address) => {
                print_cmd(buffer, output, address)?;
            }
            _ => return Err(Error::UnsupportedGlobalCmd),
        }
    }

    Ok(())
}

fn insert_cmd(
    buffer: &mut EditBuffer,
    input: &mut impl LineRead,
    address: Option<Address>,
) -> Result<(), Error> {
    if address.is_some_and(|a| a.end() > buffer.len()) {
        return Err(Error::InvalidAddress);
    }
    let mut lines = Vec::new();
    Cmd::read_lines(input, &mut lines)
        .map_err(|source| Error::ReadLines { source })?;
    buffer.do_insert(address, lines);
    Ok(())
}

fn null_cmd(
    buffer: &mut EditBuffer,
    output: &mut impl Write,
    address: Option<Address>,
) -> Result<(), Error> {
    match address {
        None => {
            if buffer.is_empty() || buffer.current_line() == buffer.len() {
                return Err(Error::InvalidAddress);
            }
            print_cmd(
                buffer,
                output,
                Some(Address::line(buffer.current_line() + 1)),
            )
        }
        _ => print_cmd(buffer, output, address),
    }
}

fn print_cmd(
    buffer: &mut EditBuffer,
    output: &mut impl Write,
    address: Option<Address>,
) -> Result<(), Error> {
    let span = if let Some(addr) = address {
        addr.into()
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
        output.write_all(l.as_bytes()).unwrap();
    }
    output.flush().unwrap();
    Ok(())
}

/// Implements quit command.
///
/// Displays warning and doesn't actually exit if unwritten
/// buffer changes are detected.
fn quit_cmd(
    buffer: &EditBuffer,
    previous_cmd: &Option<Cmd>,
) -> Result<(), Error> {
    match previous_cmd {
        Some(Cmd::Quit) => Ok(()),
        _ if !buffer.is_dirty() => Ok(()),
        _ => Err(Error::QuitUnwrittenChanges),
    }
}

fn transfer_cmd(
    buffer: &mut EditBuffer,
    address: Option<Address>,
    destination: Address,
) -> Result<(), Error> {
    if !destination.is_disjoint(
        address.unwrap_or_else(|| Address::line(buffer.current_line())),
    ) {
        return Err(Error::DestinationIntersectsSource);
    }
    buffer.do_transfer(address, destination);
    Ok(())
}

fn read_lines(
    source: &mut impl BufRead,
    lines: &mut Vec<String>,
) -> Result<(usize, usize), Error> {
    let mut line = String::new();
    let mut bytes_read = 0;
    let mut lines_read = 0;
    loop {
        let len = source
            .read_line(&mut line)
            .map_err(|source| Error::ReadLines { source })?;
        if len == 0 {
            break;
        }
        bytes_read += len;
        lines_read += 1;
        line.shrink_to_fit();
        lines.push(line);
        line = String::new();
    }

    Ok((lines_read, bytes_read))
}

fn write_cmd(
    buffer: &mut EditBuffer,
    output: &mut impl Write,
    address: Option<Address>,
    filename: Option<&Path>,
) -> Result<(), Error> {
    if buffer.filename().is_none() {
        if filename.is_none() {
            return Err(Error::NoFilename);
        }
        buffer.set_filename(filename.map(ToOwned::to_owned));
    }

    let mut destination = OpenOptions::new()
        .write(true)
        .create(true)
        .open(buffer.filename().as_ref().unwrap())
        .map_err(|source| Error::WriteFileOpen { source })?;

    let (bytes_written, lines_written) =
        write_lines(&mut destination, buffer, address)?;
    writeln!(output, "{bytes_written} bytes written ({lines_written} lines)")
        .unwrap();
    output.flush().unwrap();
    Ok(())
}

fn write_lines(
    destination: &mut impl Write,
    buffer: &mut EditBuffer,
    address: Option<Address>,
) -> Result<(usize, usize), Error> {
    let line_span = address.map_or_else(|| 1usize..=buffer.len(), Into::into);
    let full_buffer_write = line_span == (1usize..=buffer.len());

    let mut total_bytes_written = 0;
    let mut lines_written = 0;

    if !line_span.is_empty() {
        for line in &buffer[line_span] {
            let bytes_to_write = line.len();
            let mut bytes_written = 0;
            while bytes_written < bytes_to_write {
                bytes_written = bytes_written
                    + destination
                        .write(line[bytes_written..].as_bytes())
                        .map_err(|source| Error::WriteLines { source })?;
            }
            total_bytes_written += bytes_written;
            lines_written += 1;
        }
    }

    if full_buffer_write {
        buffer.reset_clean_fingerprint();
    }
    Ok((total_bytes_written, lines_written))
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::cli::CmdArgs;
    use std::path::PathBuf;
    use std::str;

    use similar_asserts::assert_eq;

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
    #[test]
    fn null_cmd_no_addr() {
        let mut output = Vec::new();
        let mut buffer = EditBuffer::from(vec!["1\r\n", "2", "3"]);
        buffer.set_current_line(2);
        null_cmd(&mut buffer, &mut output, None).unwrap();
        assert_eq!(&output[..], b"3\r\n");
    }

    #[test]
    fn null_cmd_single_line() {
        let mut output = Vec::new();
        let mut buffer = EditBuffer::from(vec!["1\r\n", "2", "3"]);
        buffer.set_current_line(2);
        null_cmd(&mut buffer, &mut output, Some(Address::line(3))).unwrap();
        assert_eq!(&output[..], b"3\r\n");
    }

    #[test]
    fn null_cmd_span() {
        let mut output = Vec::new();
        let mut buffer =
            EditBuffer::from(vec!["1\r\n", "2", "3", "4", "5", "6"]);
        buffer.set_current_line(5);
        null_cmd(&mut buffer, &mut output, Some(Address::span(2, 4).unwrap()))
            .unwrap();
        assert_eq!(&output[..], b"2\r\n3\r\n4\r\n");
    }

    #[test]
    fn null_cmd_sets_current_line() {
        let mut output = Vec::new();
        let mut buffer =
            EditBuffer::from(vec!["1\r\n", "2", "3", "4", "5", "6"]);
        buffer.set_current_line(5);
        null_cmd(&mut buffer, &mut output, Some(Address::span(2, 4).unwrap()))
            .unwrap();
        assert_eq!(4, buffer.current_line());
    }

    #[test]
    fn null_cmd_empty_buffer_gives_error() {
        let mut output = Vec::new();
        let mut buffer = EditBuffer::new();
        let res = null_cmd(&mut buffer, &mut output, None)
            .expect_err("invalid address");
        assert!(matches!(res, Error::InvalidAddress));
        let res = null_cmd(&mut buffer, &mut output, Some(Address::line(0)))
            .expect_err("invalid address");
        assert!(matches!(res, Error::InvalidAddress));
    }

    #[test]
    fn enumerate_empty_buffer_error() {
        let mut output = Vec::new();
        let mut buffer = EditBuffer::new();
        let res = enumerate_cmd(&mut buffer, &mut output, None)
            .expect_err("invalid address");
        assert!(matches!(res, Error::InvalidAddress));
        let res =
            enumerate_cmd(&mut buffer, &mut output, Some(Address::line(1)))
                .expect_err("invalid address");
        assert!(matches!(res, Error::InvalidAddress));
    }

    #[test]
    fn enumerate_sm_buffer() {
        let mut output = Vec::new();
        let mut buffer = EditBuffer::from(vec![
            "1\r\n", "2", "3", "4", "5", "6", "7", "8", "9", "10",
        ]);
        buffer.set_current_line(2);
        enumerate_cmd(&mut buffer, &mut output, None).unwrap();
        assert_eq!(&output[..], b" 2  2\r\n", "output line 2");
    }

    #[test]
    fn enumerate_sets_current_line() {
        let mut output = Vec::new();
        let mut buffer = EditBuffer::from(vec![
            "1\r\n", "2", "3", "4", "5", "6", "7", "8", "9", "10",
        ]);
        buffer.set_current_line(2);

        enumerate_cmd(
            &mut buffer,
            &mut output,
            Some(Address::span(6, 9).unwrap()),
        )
        .unwrap();
    }

    #[test]
    fn enumerate_lg_buffer() {
        let mut output = Vec::new();
        let mut buffer = EditBuffer::from(vec![
            "1\r\n", "2", "3", "4", "5", "6", "7", "8", "9", "10",
        ]);
        let mut input: Vec<u8> = Vec::new();
        for i in 11..=1024 {
            input.extend_from_slice(format!("{i}\r\n").as_bytes());
        }
        input.extend_from_slice(".\n".as_bytes());
        let mut input = &input[..];
        let address = Some(Address::line(buffer.len()));
        append_cmd(&mut buffer, &mut input, address).unwrap();
        buffer.set_current_line(2);
        assert_eq!(1024, buffer.len());
        output.clear();

        enumerate_cmd(
            &mut buffer,
            &mut output,
            Some(Address::span(4, 900).unwrap()),
        )
        .unwrap();
        let expected = b"   4  4\r\n";
        assert_eq!(&expected[..], &output[0..expected.len()]);
        output.clear();

        enumerate_cmd(&mut buffer, &mut output, Some(Address::line(999)))
            .unwrap();
        let expected = b" 999  999\r\n";
        assert_eq!(&expected[..], &output[0..expected.len()]);
    }

    #[test]
    fn print_filename_none_set() {
        let mut buffer = EditBuffer::from(vec!["1\r\n", "2", "3"]);
        let mut output = Vec::new();
        file_cmd(&mut buffer, &mut output, None);
        assert_eq!(
            str::from_utf8(&output[..]).unwrap(),
            "no current filename\n"
        );
        assert_eq!(None, buffer.filename());
    }

    #[test]
    fn set_filename() {
        let new_filename = "a_new_filename.txt\n";
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3"]);
        let mut output = Vec::new();
        assert_eq!(None, buffer.filename());
        file_cmd(
            &mut buffer,
            &mut output,
            Some(Path::new(new_filename.trim())),
        );
        assert_eq!(str::from_utf8(&output[..]).unwrap(), new_filename);
        assert_eq!(Some(Path::new(new_filename.trim())), buffer.filename());
    }

    #[test]
    fn print_filename() {
        let new_filename = "a_new_filename.txt\n";
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3"]);
        let mut output = Vec::new();
        assert_eq!(None, buffer.filename());
        file_cmd(
            &mut buffer,
            &mut output,
            Some(Path::new(new_filename.trim())),
        );
        assert_eq!(Some(Path::new(new_filename.trim())), buffer.filename());
        output.clear();
        file_cmd(&mut buffer, &mut output, None);
        assert_eq!(str::from_utf8(&output[..]).unwrap(), new_filename);
    }

    #[test]
    fn change_filename() {
        let orig_filename = "a_filename.md";
        let new_filename = "a_new_filename.txt\n";
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3"]);
        let mut output = Vec::new();
        file_cmd(&mut buffer, &mut output, Some(Path::new(orig_filename)));
        output.clear();
        file_cmd(
            &mut buffer,
            &mut output,
            Some(Path::new(new_filename.trim())),
        );
        assert_eq!(str::from_utf8(&output[..]).unwrap(), new_filename);
        assert_eq!(Some(Path::new(new_filename.trim())), buffer.filename());
    }

    #[test]
    fn global_cmd_no_matches() {
        let mut buffer = EditBuffer::from(vec!["one\n", "two", "three"]);
        let mut output = Vec::new();
        let mut prev_pattern: Option<Regex> = None;
        let pat = Regex::new("four").unwrap();
        let commands = "p\n".to_owned();
        global_cmd(
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
    fn global_cmd_illegal_nested_gobal() {
        let mut buffer = EditBuffer::from(vec!["one\r\n", "two", "three"]);
        buffer.set_current_line(1);
        let mut output = Vec::new();
        let mut prev_pattern: Option<Regex> = None;
        let pat = Regex::new("t..").unwrap();
        let commands = "1,2g/ee/n\n".to_owned();
        let res = global_cmd(
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
    fn global_cmd_blank_command_print() {
        let mut buffer =
            EditBuffer::from(vec!["one\r\n", "two", "three", "tweedle dee"]);
        buffer.set_current_line(3);
        let mut output = Vec::new();
        let mut prev_pattern: Option<Regex> = None;
        let pat = Regex::new("t..").unwrap();
        let commands = "\n".to_owned();
        global_cmd(
            &mut buffer,
            &mut output,
            Some(Address::span(1, 3).unwrap()),
            &pat,
            &commands,
            &mut prev_pattern,
        )
        .unwrap();
        assert_eq!(str::from_utf8(&output[..]).unwrap(), "two\r\nthree\r\n");
    }

    #[test]
    fn global_cmd_print() {
        let mut buffer = EditBuffer::from(vec!["one\n", "two", "three"]);
        buffer.set_current_line(1);
        let mut output = Vec::new();
        let mut prev_pattern: Option<Regex> = None;
        let pat = Regex::new("t..").unwrap();
        let commands = "p\r\n".to_owned();
        global_cmd(
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
    fn global_cmd_enumerate() {
        let mut buffer = EditBuffer::from(vec!["one\n", "two", "three"]);
        buffer.set_current_line(1);
        let mut output = Vec::new();
        let mut prev_pattern: Option<Regex> = None;
        let pat = Regex::new("t..").unwrap();
        let commands = "n\r\n".to_owned();
        global_cmd(
            &mut buffer,
            &mut output,
            Some(Address::span(1, 3).unwrap()),
            &pat,
            &commands,
            &mut prev_pattern,
        )
        .unwrap();
        assert_eq!(str::from_utf8(&output[..]).unwrap(), "2  two\n3  three\n");
    }

    #[test]
    fn global_cmd_enumerate_with_addresses() {
        let mut buffer = EditBuffer::from(vec![
            "one\n", "two", "three", "four", "five", "six",
        ]);
        buffer.set_current_line(6);
        let mut output = Vec::new();
        let mut prev_pattern: Option<Regex> = None;
        let pat = Regex::new("e$").unwrap();
        let commands = "-1,.n\r\n".to_owned();
        global_cmd(
            &mut buffer,
            &mut output,
            Some(Address::span(2, 5).unwrap()),
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
    fn global_cmd_unsupported_commands() {
        let mut buffer = EditBuffer::from(vec!["one\r\n", "two", "three"]);
        buffer.set_current_line(1);
        let mut output = Vec::new();
        let mut prev_pattern: Option<Regex> = None;
        let pat = Regex::new(r"t..").unwrap();
        let commands = "e filename.txt\n".to_owned();
        let res = global_cmd(
            &mut buffer,
            &mut output,
            Some(Address::span(1, 3).unwrap()),
            &pat,
            &commands,
            &mut prev_pattern,
        )
        .expect_err("unsupported global command");
        assert!(matches!(res, Error::UnsupportedGlobalCmd));
    }

    #[test]
    fn print_cmd_no_addr() {
        let mut output = Vec::new();
        let mut buffer = EditBuffer::from(vec!["1\r\n", "2", "3"]);
        buffer.set_current_line(2);
        print_cmd(&mut buffer, &mut output, None).unwrap();
        assert_eq!(&output[..], b"2\r\n");
    }

    #[test]
    fn print_cmd_single_line() {
        let mut output = Vec::new();
        let mut buffer = EditBuffer::from(vec!["1\r\n", "2", "3"]);
        buffer.set_current_line(2);
        print_cmd(&mut buffer, &mut output, Some(Address::line(3))).unwrap();
        assert_eq!(&output[..], b"3\r\n");
    }

    #[test]
    fn print_cmd_span() {
        let mut output = Vec::new();
        let mut buffer =
            EditBuffer::from(vec!["1\r\n", "2", "3", "4", "5", "6"]);
        buffer.set_current_line(5);
        print_cmd(&mut buffer, &mut output, Some(Address::span(2, 4).unwrap()))
            .unwrap();
        assert_eq!(&output[..], b"2\r\n3\r\n4\r\n");
    }

    #[test]
    fn print_cmd_sets_current_line() {
        let mut output = Vec::new();
        let mut buffer =
            EditBuffer::from(vec!["1\r\n", "2", "3", "4", "5", "6"]);
        buffer.set_current_line(5);
        print_cmd(&mut buffer, &mut output, Some(Address::span(2, 4).unwrap()))
            .unwrap();
        assert_eq!(4, buffer.current_line());
    }

    #[test]
    fn quit_cmd_twice_exits() {
        let input = b"a\n1\n2\n3\n.\nq\nq\n";
        let mut output = Vec::new();

        run(&input[..], &mut output, &CmdArgs::default()).unwrap();
        let output = str::from_utf8(&output[..]).unwrap();
        assert!(output.contains("unwritten changes - repeat quit"));
    }

    #[test]
    fn print_cmd_empty_buffer_gives_error() {
        let mut output = Vec::new();
        let mut buffer = EditBuffer::new();
        let res = print_cmd(&mut buffer, &mut output, None)
            .expect_err("invalid address");
        assert!(matches!(res, Error::InvalidAddress));
        let res = print_cmd(&mut buffer, &mut output, Some(Address::line(0)))
            .expect_err("invalid address");
        assert!(matches!(res, Error::InvalidAddress));
    }

    #[test]
    fn edit_cmd_twice_overrides_warning() {
        let input =
            b"a\n1\n2\n3\n.\ne a_file_that_is_not_there.ext\ne a_file_that_is_not_there.ext\nq\nq\n";
        let mut output = Vec::new();

        run(&input[..], &mut output, &CmdArgs::default()).unwrap();
        assert!(str::from_utf8(&output[..]).unwrap().contains(
            "unwritten changes - repeat edit command to discard changes"
        ));
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
        run(&input[..], &mut output, &args).unwrap();
        let output = str::from_utf8(&output[..]).unwrap();
        assert!(output.contains("312"));
    }

    #[test]
    fn file_on_cmd_line_not_found() {
        let args = cli::CmdArgs { file: Some(PathBuf::from("not_a_file")) };
        let input = b"q\n";
        let mut output = Vec::new();
        run(&input[..], &mut output, &args).unwrap();
        let output = str::from_utf8(&output[..]).unwrap();
        assert!(output.contains("not found"));
    }

    #[test]
    fn append_cmd_dispatch() {
        let input = b"a\none\ntwo\nthree\n.\n2p\nq\nq\n";
        let mut output = Vec::new();
        run(&input[..], &mut output, &CmdArgs::default()).unwrap();
        let output = str::from_utf8(&output[..]).unwrap();
        assert!(output.contains("two\n"));
        assert!(output.contains("unwritten changes"));
        assert!(!output.contains("one"));
    }

    #[test]
    fn delete_cmd_dispatch() {
        let input = b"a\none\ntwo\nthree\n.\n1,2d\np\nd\np\nq\nq\n";
        let mut output = Vec::new();
        run(&input[..], &mut output, &CmdArgs::default()).unwrap();
        let output = str::from_utf8(&output[..]).unwrap();
        assert!(output.contains("three"));
        assert!(output.contains("invalid address"));
    }

    #[test]
    fn change_cmd_dispatch() {
        let input = b"a\n1\n2\n3\n4\n.\n2,3c\na\nb\n.\n1,$p\nq\nq\n";
        let mut output = Vec::new();
        run(&input[..], &mut output, &CmdArgs::default()).unwrap();
        let output = str::from_utf8(&output[..]).unwrap();
        assert!(output.contains("1\na\nb\n4\n"));
    }

    #[test]
    fn edit_cmd_dispatch() {
        let input = b"e test/assets/text_with_final_eol.txt\nq\n";
        let mut output = Vec::new();
        run(&input[..], &mut output, &CmdArgs::default()).unwrap();
        let output = str::from_utf8(&output[..]).unwrap();
        assert!(output.contains("312"));
    }

    #[test]
    fn enumerate_cmd_dispatch() {
        let input = b"a\none\ntwo\nthree\n.\n2,3n\nq\nq\n";
        let mut output = Vec::new();
        run(&input[..], &mut output, &CmdArgs::default()).unwrap();
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
        run(&input[..], &mut output, &args).unwrap();
        let output = str::from_utf8(&output[..]).unwrap();
        //        assert!(output.contains("test/assets/text_with_final_eol.txt"));
        assert!(output.contains("no current filename"));
        assert!(output.contains("new_file_name.txt"));
    }

    #[test]
    fn insert_cmd_dispatch() {
        let input = b"i\none\ntwo\nthree\n.\n2p\nq\nq\n";
        let mut output = Vec::new();
        run(&input[..], &mut output, &CmdArgs::default()).unwrap();
        let output = str::from_utf8(&output[..]).unwrap();
        assert!(output.contains("two\n"));
        assert!(output.contains("unwritten changes"));
        assert!(!output.contains("one"));
    }

    #[test]
    fn global_cmd_dispatch() {
        let input = b"a\none\ntwo\nthree\nfour\nfive\n.\ng/e$/n\nq\nq\n";
        let mut output = Vec::new();
        run(&input[..], &mut output, &CmdArgs::default()).unwrap();
        let output = str::from_utf8(&output[..]).unwrap();
        assert!(output.contains("1  one\n3  three\n5  five\n"));
    }

    #[test]
    fn null_cmd_dispatch() {
        let input = b"a\r\none\r\ntwo\r\nthree\r\n.\r\n1\r\n\r\nq\r\nq\r\n";
        let mut output = Vec::new();
        run(&input[..], &mut output, &CmdArgs::default()).unwrap();
        let output = str::from_utf8(&output[..]).unwrap();
        assert!(output.contains("two"));
    }

    #[test]
    fn print_cmd_dispatch() {
        let input = b"a\none\ntwo\nthree\n.\n1,2p\nq\nq\n";
        let mut output = Vec::new();
        run(&input[..], &mut output, &CmdArgs::default()).unwrap();
        let output = str::from_utf8(&output[..]).unwrap();
        assert!(output.contains("one\ntwo\n"));
    }

    #[test]
    fn quit_cmd_dispatch() {
        let input = b"q\r\n";
        let mut output = Vec::new();
        run(&input[..], &mut output, &CmdArgs::default()).unwrap();
    }

    #[test]
    fn write_cmd_dispatch() {
        let input = b"a\none\n.\nw\nq\nq\n";
        let mut output = Vec::new();
        run(&input[..], &mut output, &CmdArgs::default()).unwrap();
        let output = str::from_utf8(&output[..]).unwrap();
        assert!(output.contains("no filename"));
    }

    #[test]
    fn undo_cmd_dispatch() {
        let input = b"a\n1\n2\n3\n.\n2,3d\np\nu\np\nu\nq\n";
        let mut output = Vec::new();
        run(&input[..], &mut output, &CmdArgs::default()).unwrap();
        let output = str::from_utf8(&output[..]).unwrap();
        assert!(output.contains("1\n"));
        assert!(output.contains("3\n"));
    }

    #[test]
    fn redo_cmd_dispatch() {
        let input = b"a\n1\n2\n3\n.\n2,3d\nu\nU\n3p\nq\nq\n";
        let mut output = Vec::new();
        run(&input[..], &mut output, &CmdArgs::default()).unwrap();
        let output = str::from_utf8(&output[..]).unwrap();
        assert!(output.contains("invalid address"));
        assert!(output.contains("unwritten changes"));
    }

    #[test]
    fn transfer_cmd_dispatch() {
        let input = b"a\n3\n4\n5\n1\n2\n.\n4,5t0\n1,3p\nq\nq\n";
        let mut output = Vec::new();
        run(&input[..], &mut output, &CmdArgs::default()).unwrap();
        let output = str::from_utf8(&output[..]).unwrap();
        assert!(output.contains("1\n2\n3\n"));
    }

    #[test]
    fn transfer_cmd_destination_intersects_source_give_error() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        let source = Address::span(3, 5).unwrap();
        let destination = Address::line(5);
        let res = transfer_cmd(&mut buffer, Some(source), destination)
            .expect_err("should fail");
        assert!(matches!(res, Error::DestinationIntersectsSource));
    }

    #[test]
    fn write_propegates_errors() {
        let mut buffer = EditBuffer::from(vec!["1\r\n", "2", "3"]);
        let mut dummy_file = BadWriter {};
        let res = write_lines(
            &mut dummy_file,
            &mut buffer,
            Some(Address::span(1, 2).unwrap()),
        )
        .expect_err("io error");
        assert!(matches!(res, Error::WriteLines { .. }));
    }

    #[test]
    fn write_one_line() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3"]);
        let mut dummy_file = Vec::new();
        let (bytes, lines) =
            write_lines(&mut dummy_file, &mut buffer, Some(Address::line(2)))
                .unwrap();
        assert_eq!(bytes, 2);
        assert_eq!(lines, 1);
    }

    #[test]
    fn write_many_lines() {
        let mut buffer =
            EditBuffer::from(vec!["1\r\n", "2", "3", "4", "5", "6"]);
        let mut dummy_file = Vec::new();
        let (bytes, lines) = write_lines(
            &mut dummy_file,
            &mut buffer,
            Some(Address::span(1, 6).unwrap()),
        )
        .unwrap();
        assert_eq!(bytes, 18);
        assert_eq!(lines, 6);
    }

    #[test]
    fn write_empty_buffer() {
        let mut buffer = EditBuffer::new();
        let mut dummy_file = Vec::new();
        let (bytes, lines) =
            write_lines(&mut dummy_file, &mut buffer, None).unwrap();
        assert_eq!(bytes, 0);
        assert_eq!(lines, 0);
    }

    #[test]
    fn write_no_addr_leaves_clean_buffer() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3"]);
        assert!(!buffer.is_dirty());
        let mut input = "one more line\n.\n".as_bytes();
        append_cmd(&mut buffer, &mut input, Some(Address::line(0))).unwrap();
        assert!(buffer.is_dirty());
        let mut dummy_file = Vec::new();
        let (bytes, lines) =
            write_lines(&mut dummy_file, &mut buffer, None).unwrap();
        assert_eq!(bytes, 20);
        assert_eq!(lines, 4);
        assert!(!buffer.is_dirty());
    }

    #[test]
    fn write_full_buffer_leaves_clean_buffer() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3"]);
        assert!(!buffer.is_dirty());
        let mut input = "one more line\n.\n".as_bytes();
        append_cmd(&mut buffer, &mut input, Some(Address::line(0))).unwrap();
        assert!(buffer.is_dirty());
        let mut dummy_file = Vec::new();
        let address = Some(Address::span(1, buffer.len()).unwrap());
        let (bytes, lines) =
            write_lines(&mut dummy_file, &mut buffer, address).unwrap();
        assert_eq!(bytes, 20);
        assert_eq!(lines, 4);
        assert!(!buffer.is_dirty());
    }

    #[test]
    fn write_partial_buffer_leaves_dirty_buffer() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3"]);
        assert!(!buffer.is_dirty());
        let mut input = "one more line\n.\n".as_bytes();
        append_cmd(&mut buffer, &mut input, Some(Address::line(0))).unwrap();
        assert!(buffer.is_dirty());
        let mut dummy_file = Vec::new();
        let (bytes, lines) = write_lines(
            &mut dummy_file,
            &mut buffer,
            Some(Address::span(1, 2).unwrap()),
        )
        .unwrap();
        assert_eq!(bytes, 16);
        assert_eq!(lines, 2);
        assert!(buffer.is_dirty());
    }

    #[test]
    fn append_cmd_past_end_gives_error_before_input() {
        let mut buffer = EditBuffer::new();
        let mut input = "one\n.\n".as_bytes();
        let expected = "one\n.\n".as_bytes();
        let res = append_cmd(&mut buffer, &mut input, Some(Address::line(2)))
            .expect_err("invalid addr");
        assert_eq!(0, buffer.len());
        assert_eq!(input, expected);
        assert!(matches!(res, Error::InvalidAddress));
    }

    #[test]
    fn insert_cmd_past_end_gives_error_before_input() {
        let mut buffer = EditBuffer::new();
        let mut input = "one\n.\n".as_bytes();
        let expected = "one\n.\n".as_bytes();
        let res = insert_cmd(&mut buffer, &mut input, Some(Address::line(2)))
            .expect_err("invalid addr");
        assert_eq!(0, buffer.len());
        assert_eq!(input, expected);
        assert!(matches!(res, Error::InvalidAddress));
    }

    #[test]
    fn delete_cmd_empty_buffer() {
        let mut buffer = EditBuffer::new();
        let res = delete_cmd(&mut buffer, None).expect_err("invalid address");
        assert!(matches!(res, Error::InvalidAddress));
    }

    #[test]
    fn delete_cmd_line_zero() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3"]);
        let res = delete_cmd(&mut buffer, Some(Address::line(0)))
            .expect_err("invalid address");
        assert!(matches!(res, Error::InvalidAddress));
    }

    #[test]
    fn delete_cmd_span_starting_at_zero() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5"]);
        let res = delete_cmd(&mut buffer, Some(Address::span(0, 3).unwrap()))
            .expect_err("invalid address");
        assert!(matches!(res, Error::InvalidAddress));
    }

    #[test]
    fn edit_cmd_no_filename_error() {
        let mut buffer = EditBuffer::new();
        let res = edit_cmd(&mut buffer, &mut Vec::new(), None, &None)
            .expect_err("no filename");
        assert!(matches!(res, Error::NoFilename));
    }

    #[test]
    fn edit_cmd_missing_file_clears_buffer_sets_filename() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3"]);
        assert_eq!(buffer.len(), 3);
        let mut output = Vec::new();
        let not_a_file = Some(Path::new("non-existant_file.txt"));
        let res = edit_cmd(&mut buffer, &mut output, not_a_file, &None)
            .expect_err("EditFileNotFound");
        assert!(matches!(res, Error::EditFileNotFound(_)));
        assert_eq!(buffer.filename(), not_a_file);
        assert!(buffer.is_empty());
    }

    #[test]
    fn read_lines_returns_correct_counts() {
        let source = b"one\r\ntwo\r\nthree\r\nfour\r\n";
        let source_bytes = source.len();
        let mut lines = Vec::new();
        let (line_count, byte_count) =
            read_lines(&mut &source[..], &mut lines).expect("no error");
        assert_eq!(byte_count, source_bytes);
        assert_eq!(line_count, lines.len());
    }

    #[test]
    fn read_lines_io_error() {
        let mut source = BufReader::new(BadReader {});
        let res =
            read_lines(&mut source, &mut Vec::new()).expect_err("io error");
        assert!(matches!(res, Error::ReadLines { .. }));
    }

    #[test]
    fn edit_cmd_reads_file() {
        let mut buffer = EditBuffer::new();
        let mut output = Vec::new();
        let filename1 = Some(Path::new(r"test/assets/text_with_final_eol.txt"));
        let filename2 =
            Some(Path::new(r"test/assets/text_with_no_final_eol.txt"));

        edit_cmd(&mut buffer, &mut output, filename1, &None).unwrap();
        assert_eq!(buffer.len(), 10);
        let out_text = str::from_utf8(&output[..]).unwrap();
        assert!(
            out_text.contains("10 lines") && out_text.contains("312 bytes")
        );

        output.clear();
        edit_cmd(&mut buffer, &mut output, filename2, &None).unwrap();
        assert_eq!(buffer.len(), 10);
        let out_text = str::from_utf8(&output[..]).unwrap();
        assert!(
            out_text.contains("10 lines") && out_text.contains("318 bytes")
        );
    }

    #[test]
    fn change_cmd_addr_starting_after_buffer_end_gives_error() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3"]);
        let res = change_cmd(
            &mut buffer,
            &mut &b".\n"[..],
            Some(Address::span(5, 6).unwrap()),
        )
        .expect_err("illegal address");
        assert!(matches!(res, Error::InvalidAddress));
    }

    #[test]
    fn change_cmd_addr_ending_past_buffer_end_gives_error() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3"]);
        let res = change_cmd(
            &mut buffer,
            &mut &b".\n"[..],
            Some(Address::span(2, 4).unwrap()),
        )
        .expect_err("illegal address");
        assert!(matches!(res, Error::InvalidAddress));
    }
}
