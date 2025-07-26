use std::collections::VecDeque;
use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::{self, BufReader, prelude::*};
use std::path::Path;

use regex::Regex;
use unicode_segmentation::UnicodeSegmentation;

use crate::cli;
use crate::command::{self, Address, Cmd, PrintSuffix, SubstitutionScope};
use crate::edit_buffer::{Change, ChangeSet, Diff, EditBuffer};

use line_reader::LineRead;

#[derive(Debug)]
pub enum Error {
    ParseCmd(command::Error),
    InvalidAddress,
    NestedGlobalCmd,
    UnsupportedGlobalCmd,
    ReadGlobalCmd { source: command::Error },
    NoFilename,
    EditFileOpen { source: std::io::Error },
    WriteFileOpen { source: std::io::Error },
    WriteLines { source: std::io::Error },
    ReadLines { source: std::io::Error },
    QuitUnwrittenChanges,
    EditUnwrittenChanges,
    FileNotFound(String),
    DestinationIntersectsSource,
    NoMatch,
    NothingToUndo,
    NothingToRedo,
    GlobalCmdErrorStop { source: Box<Error>, changes: Option<ChangeSet> },
    ReadFileOpen { source: std::io::Error },
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match *self {
            Error::ParseCmd(_)
            | Error::FileNotFound(_)
            | Error::QuitUnwrittenChanges
            | Error::EditUnwrittenChanges
            | Error::InvalidAddress
            | Error::NestedGlobalCmd
            | Error::UnsupportedGlobalCmd
            | Error::DestinationIntersectsSource
            | Error::NoMatch
            | Error::NothingToUndo
            | Error::NothingToRedo
            | Error::NoFilename => None,
            Error::EditFileOpen { ref source }
            | Error::WriteFileOpen { ref source }
            | Error::WriteLines { ref source }
            | Error::ReadFileOpen { ref source }
            | Error::ReadLines { ref source } => Some(source),
            Error::ReadGlobalCmd { ref source } => Some(source),
            Error::GlobalCmdErrorStop { ref source, .. } => Some(source),
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
            Error::ReadGlobalCmd { .. } => {
                write!(f, "error reading global command")
            }
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
            Error::FileNotFound(filename) => {
                write!(f, "{filename} not found")
            }
            Error::DestinationIntersectsSource => {
                write!(f, "destination intersects source")
            }
            Error::NoMatch => {
                write!(f, "no matches found")
            }
            Error::NothingToUndo => write!(f, "nothing to undo"),
            Error::NothingToRedo => write!(f, "nothing to redo"),
            Error::GlobalCmdErrorStop { .. } => {
                write!(f, "error executing global command")
            }
            Error::ReadFileOpen { .. } => {
                write!(f, "error opening file to read")
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

    if let Some(file) = &args.file
        && let Err(e) = edit_cmd(
            &mut buffer,
            &mut output,
            Some(file),
            previous_cmd.as_ref(),
        )
    {
        writeln!(output, "{e}").unwrap();
    }

    // Accept and process commands until fatal error or exit
    let mut done = false;
    while !done {
        Cmd::read(&mut input, &mut buffer, &mut previous_pattern)
            .map_err(Error::ParseCmd)
            .and_then(|res| match res {
                Some((cmd, sfx)) => {
                    let res = dispatch_cmd(
                        &cmd,
                        &mut buffer,
                        &mut output,
                        &mut input,
                        &mut previous_cmd,
                        &mut previous_pattern,
                    );
                    previous_cmd = Some(cmd);
                    res.and_then(|exit| {
                        done = exit;
                        let cur_line_addr =
                            Some(Address::line(buffer.current_line()));
                        match sfx {
                            Some(PrintSuffix::Print) => print_cmd(
                                &mut buffer,
                                &mut output,
                                cur_line_addr,
                            ),
                            Some(PrintSuffix::Enumerate) => enumerate_cmd(
                                &mut buffer,
                                &mut output,
                                cur_line_addr,
                            ),
                            Some(PrintSuffix::List) => list_cmd(
                                &mut buffer,
                                &mut output,
                                cur_line_addr,
                            ),
                            None => Ok(None),
                        }
                    })
                }
                _ => Ok(None),
            })
            .or_else(|e| {
                writeln!(output, "{e}").unwrap();
                output.flush().unwrap();
                Ok(None)
            })?;
    }
    Ok(())
}

fn dispatch_cmd(
    cmd: &Cmd,
    buffer: &mut EditBuffer,
    output: &mut impl Write,
    input: &mut impl LineRead,
    previous_cmd: &mut Option<Cmd>,
    previous_pattern: &mut Option<regex::Regex>,
) -> Result<bool, Error> {
    let mut done = false;
    let res = match cmd {
        // dispatch editor commands
        Cmd::Append(address) => append_cmd(buffer, input, *address),
        Cmd::Delete(address) => delete_cmd(buffer, *address),
        Cmd::Change(address) => change_cmd(buffer, input, *address),
        Cmd::Edit(filename) => {
            edit_cmd(buffer, output, filename.as_deref(), previous_cmd.as_ref())
        }
        Cmd::Enumerate(address) => enumerate_cmd(buffer, output, *address),
        Cmd::File(filename) => {
            file_cmd(buffer, output, filename.as_deref());
            Ok(None)
        }
        Cmd::Global(address, pattern, commands) => global_cmd(
            buffer,
            output,
            *address,
            pattern,
            commands,
            previous_pattern,
        ),
        Cmd::Insert(address) => insert_cmd(buffer, input, *address),
        Cmd::Join(address) => join_cmd(buffer, *address),
        Cmd::LineNumber(address) => {
            Ok(line_number_cmd(buffer, output, *address))
        }
        Cmd::List(address) => list_cmd(buffer, output, *address),
        Cmd::Move(address, destination) => {
            move_cmd(buffer, *address, *destination)
        }
        Cmd::Null(address) => null_cmd(buffer, output, *address),
        Cmd::Print(address) => print_cmd(buffer, output, *address),
        Cmd::Quit => {
            quit_cmd(buffer, previous_cmd.as_ref()).inspect(|_| done = true)
        }
        Cmd::Read(address, filename) => {
            read_cmd(buffer, output, *address, filename.as_deref())
        }
        Cmd::Redo => buffer.do_redo().map(|()| None),
        Cmd::Substitute(address, pattern, replacement, scope) => {
            substitute_cmd(buffer, *address, pattern, replacement, *scope)
        }
        Cmd::Transfer(address, destination) => {
            transfer_cmd(buffer, *address, *destination)
        }
        Cmd::Undo => buffer.do_undo().map(|()| None),
        Cmd::Write(address, filename) => {
            write_cmd(buffer, output, *address, filename.as_deref())
        }
    };
    match res {
        Ok(Some(changes)) => buffer.push_undo(changes),
        Ok(None) => (),
        Err(Error::GlobalCmdErrorStop { source, changes }) => {
            if let Some(changes) = changes {
                buffer.push_undo(changes);
            }
            return Err(*source);
        }
        Err(e) => return Err(e),
    }
    Ok(done)
}

fn append_cmd(
    buffer: &mut EditBuffer,
    input: &mut impl LineRead,
    address: Option<Address>,
) -> Result<Option<ChangeSet>, Error> {
    if address.is_some_and(|a| a.end() > buffer.len()) {
        return Err(Error::InvalidAddress);
    }
    let mut lines = Vec::new();
    Cmd::read_input_lines(input, &mut lines)
        .map_err(|source| Error::ReadLines { source })?;
    Ok(buffer.do_append(address, lines))
}

fn change_cmd(
    buffer: &mut EditBuffer,
    input: &mut impl LineRead,
    address: Option<Address>,
) -> Result<Option<ChangeSet>, Error> {
    if address.is_some_and(|a| a.end() > buffer.len()) {
        return Err(Error::InvalidAddress);
    }

    let mut lines = Vec::new();
    Cmd::read_input_lines(input, &mut lines)
        .map_err(|source| Error::ReadLines { source })?;
    Ok(Some(buffer.do_change(address, lines)))
}

fn delete_cmd(
    buffer: &mut EditBuffer,
    address: Option<Address>,
) -> Result<Option<ChangeSet>, Error> {
    match address {
        Some(addr) if addr.start() == 0 => Err(Error::InvalidAddress),
        None if buffer.current_line() == 0 => Err(Error::InvalidAddress),
        _ => Ok(Some(buffer.do_delete(address))),
    }
}

fn edit_cmd(
    buffer: &mut EditBuffer,
    output: &mut impl Write,
    filename: Option<&Path>,
    previous_cmd: Option<&Cmd>,
) -> Result<Option<ChangeSet>, Error> {
    if buffer.is_dirty() {
        if matches!(previous_cmd, Some(Cmd::Edit(_))) {
            buffer.reset_clean_fingerprint();
        } else {
            return Err(Error::EditUnwrittenChanges);
        }
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
                    Err(Error::FileNotFound(missing_file))
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
    buffer.set_current_line(buffer.len());
    Ok(None)
}

fn enumerate_cmd(
    buffer: &mut EditBuffer,
    output: &mut impl Write,
    address: Option<Address>,
) -> Result<Option<ChangeSet>, Error> {
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

    let width = 1 + buffer.len().checked_ilog10().unwrap_or_default() as usize;
    let start = *span.start();
    buffer.set_current_line(*span.end());

    for (i, l) in buffer[span].iter().enumerate() {
        write!(output, "{:>width$}  ", start + i).unwrap();
        print_line(output, l);
    }
    output.flush().unwrap();
    Ok(None)
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
) -> Result<Option<ChangeSet>, Error> {
    let mut changes = ChangeSet::new(buffer.current_line());
    *previous_pattern = Some(pattern.clone());
    // make a list of matching lines
    let search_range = address.map_or_else(|| 1..=buffer.len(), Into::into);
    let matched_lines = (search_range)
        .filter(|&n| {
            buffer[n].lines().next().is_some_and(|l| pattern.is_match(l))
        })
        .collect::<VecDeque<usize>>();
    let res = do_global_cmds(
        buffer,
        output,
        commands,
        previous_pattern,
        matched_lines,
        &mut changes,
    );
    match res {
        Ok(()) => Ok(Some(changes)),
        Err(e) => match e {
            Error::NestedGlobalCmd => Err(Error::NestedGlobalCmd),
            Error::UnsupportedGlobalCmd => Err(Error::UnsupportedGlobalCmd),
            e => Err(Error::GlobalCmdErrorStop {
                source: Box::new(e),
                changes: Some(changes),
            }),
        },
    }
}

fn do_global_cmds(
    buffer: &mut EditBuffer,
    output: &mut impl Write,
    commands: &str,
    previous_pattern: &mut Option<Regex>,
    mut matched_lines: VecDeque<usize>,
    changes: &mut ChangeSet,
) -> Result<(), Error> {
    // iterate over list
    while let Some(line_num) = matched_lines.pop_front() {
        buffer.set_current_line(line_num);
        let mut input = commands.as_bytes();

        // parse and execute command list for line
        while let Some((cmd, sfx)) =
            Cmd::read(&mut input, buffer, previous_pattern)
                .map_err(|source| Error::ReadGlobalCmd { source })?
        {
            let cs = match cmd {
                Cmd::Append(address) => append_cmd(buffer, &mut input, address),
                Cmd::Change(address) => change_cmd(buffer, &mut input, address),
                Cmd::Delete(address) => delete_cmd(buffer, address),
                Cmd::Enumerate(address) => {
                    enumerate_cmd(buffer, output, address)
                }
                Cmd::Global(..) => return Err(Error::NestedGlobalCmd),
                Cmd::Insert(address) => insert_cmd(buffer, &mut input, address),
                Cmd::Join(address) => join_cmd(buffer, address),
                Cmd::Move(address, destination) => {
                    move_cmd(buffer, address, destination)
                }
                Cmd::Null(address) | Cmd::Print(address) => {
                    print_cmd(buffer, output, address)
                }
                Cmd::Substitute(address, pattern, replacement, scope) => {
                    substitute_cmd(
                        buffer,
                        address,
                        &pattern,
                        &replacement,
                        scope,
                    )
                }
                Cmd::Transfer(address, destination) => {
                    transfer_cmd(buffer, address, destination)
                }
                _ => Err(Error::UnsupportedGlobalCmd),
            }?;
            if let Some(mut cs) = cs {
                for change in cs.drain() {
                    adjust_global_list(&mut matched_lines, &change);
                    let cl_after = change.current_line_after;
                    changes.push(change, cl_after);
                }
            }
            if let Some(sfx) = sfx {
                let cur_line_addr = Some(Address::line(buffer.current_line()));
                match sfx {
                    PrintSuffix::Print => {
                        print_cmd(buffer, output, cur_line_addr)?;
                    }
                    PrintSuffix::Enumerate => {
                        enumerate_cmd(buffer, output, cur_line_addr)?;
                    }
                    PrintSuffix::List => {
                        list_cmd(buffer, output, cur_line_addr)?;
                    }
                }
            }
        }
    }
    Ok(())
}

fn adjust_global_list(list: &mut VecDeque<usize>, change: &Change) {
    for diff in change.diffs() {
        match diff {
            Diff::Remove(start, lines) => {
                let end = start + lines.len();
                list.retain_mut(|n| {
                    if *n < *start {
                        true
                    } else if *n > end {
                        *n -= lines.len();
                        true
                    } else {
                        false
                    }
                });
            }
            Diff::Add(start, lines) => {
                for n in list.iter_mut().filter(|n| **n > *start) {
                    *n += lines.len();
                }
            }
        }
    }
}

fn insert_cmd(
    buffer: &mut EditBuffer,
    input: &mut impl LineRead,
    address: Option<Address>,
) -> Result<Option<ChangeSet>, Error> {
    if address.is_some_and(|a| a.end() > buffer.len()) {
        return Err(Error::InvalidAddress);
    }
    let mut lines = Vec::new();
    Cmd::read_input_lines(input, &mut lines)
        .map_err(|source| Error::ReadLines { source })?;
    Ok(buffer.do_insert(address, lines))
}

fn join_cmd(
    buffer: &mut EditBuffer,
    address: Option<Address>,
) -> Result<Option<ChangeSet>, Error> {
    if buffer.is_empty() {
        return Err(Error::InvalidAddress);
    }
    match address {
        None if buffer.current_line() == buffer.len() => {
            Err(Error::InvalidAddress)
        }
        Some(a) if a.line_count() == 1 => Ok(None),
        _ => Ok(Some(buffer.do_join(address))),
    }
}

fn line_number_cmd(
    buffer: &mut EditBuffer,
    output: &mut impl Write,
    address: Option<Address>,
) -> Option<ChangeSet> {
    match address {
        None => {
            writeln!(output, "{}", buffer.len()).unwrap();
        }
        Some(address) => {
            writeln!(output, "{}", address.end()).unwrap();
        }
    }
    None
}

fn list_cmd(
    buffer: &mut EditBuffer,
    output: &mut impl Write,
    address: Option<Address>,
) -> Result<Option<ChangeSet>, Error> {
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
        let expanded: String = l
            .graphemes(true)
            .map(|gr| match gr {
                "\t" => "\\t",
                "$" => "\\$",
                "\n" => "$\n",
                "\r\n" => "$\r\n",
                gr => gr,
            })
            .collect();
        print_line(output, &expanded);
    }
    output.flush().unwrap();
    Ok(None)
}

fn move_cmd(
    buffer: &mut EditBuffer,
    mut address: Option<Address>,
    destination: Address,
) -> Result<Option<ChangeSet>, Error> {
    let source =
        address.get_or_insert_with(|| Address::line(buffer.current_line()));
    if destination.end() >= source.start() && destination.end() < source.end() {
        return Err(Error::DestinationIntersectsSource);
    }
    Ok(Some(buffer.do_move(address, destination)))
}

fn null_cmd(
    buffer: &mut EditBuffer,
    output: &mut impl Write,
    address: Option<Address>,
) -> Result<Option<ChangeSet>, Error> {
    let address = Some(Address::line(
        address.map_or_else(|| buffer.current_line() + 1, |a| a.end()),
    ));
    print_cmd(buffer, output, address)
}

fn print_cmd(
    buffer: &mut EditBuffer,
    output: &mut impl Write,
    address: Option<Address>,
) -> Result<Option<ChangeSet>, Error> {
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
        print_line(output, l);
    }
    output.flush().unwrap();
    Ok(None)
}

fn print_line(output: &mut impl Write, line: &str) {
    let mut cols = 0;
    for c in line.chars() {
        let c_width = if c == '\t' {
            8 - (cols % 8)
        } else {
            use unicode_width::UnicodeWidthChar;
            c.width().unwrap_or(0)
        };
        if c == '\t' {
            write!(output, "{}", &"        "[..c_width]).unwrap();
        } else {
            write!(output, "{c}").unwrap();
        }
        cols += c_width;
    }
}

/// Implements quit command.
///
/// Displays warning and doesn't actually exit if unwritten
/// buffer changes are detected.
fn quit_cmd(
    buffer: &EditBuffer,
    previous_cmd: Option<&Cmd>,
) -> Result<Option<ChangeSet>, Error> {
    match previous_cmd {
        Some(Cmd::Quit) => Ok(None),
        _ if !buffer.is_dirty() => Ok(None),
        _ => Err(Error::QuitUnwrittenChanges),
    }
}

fn read_cmd(
    buffer: &mut EditBuffer,
    output: &mut impl Write,
    address: Option<Address>,
    filename: Option<&Path>,
) -> Result<Option<ChangeSet>, Error> {
    let address = if let Some(address) = address {
        if address.end() > buffer.len() {
            return Err(Error::InvalidAddress);
        }
        address
    } else {
        Address::line(buffer.current_line())
    };

    if buffer.filename().is_none() && filename.is_some() {
        buffer.set_filename(filename.map(ToOwned::to_owned));
    }
    let filename = filename.or(buffer.filename()).ok_or(Error::NoFilename)?;

    let file = File::open(filename);
    let mut source = match file {
        Ok(f) => BufReader::new(f),
        Err(e) => {
            return match e.kind() {
                io::ErrorKind::NotFound => {
                    let missing_file = String::from(filename.to_string_lossy());
                    Err(Error::FileNotFound(missing_file))
                }
                _ => Err(Error::ReadFileOpen { source: e }),
            };
        }
    };

    let mut lines = Vec::new();
    let (lines_read, bytes_read) = read_lines(&mut source, &mut lines)?;
    writeln!(output, "{lines_read} lines ({bytes_read} bytes) read").unwrap();
    let mut changes = ChangeSet::new(buffer.current_line());
    let mut change = Change::new(buffer.current_line());
    change.push_add(address.end(), lines.clone());
    let lines_added = lines.len();
    if buffer.append(address.end(), lines) {
        output.flush().unwrap();
        writeln!(output, "missing line terminator appended").unwrap();
    }
    buffer.set_current_line(address.end() + lines_added);
    changes.push(change, buffer.current_line());
    Ok(Some(changes))
}

fn substitute_cmd(
    buffer: &mut EditBuffer,
    address: Option<Address>,
    pattern: &Regex,
    replacement: &str,
    scope: SubstitutionScope,
) -> Result<Option<ChangeSet>, Error> {
    let address =
        address.unwrap_or_else(|| Address::line(buffer.current_line()));
    if address.start() == 0
        || address.start() > address.end()
        || address.end() > buffer.len()
    {
        return Err(Error::InvalidAddress);
    }

    let mut line_num = address.start();
    let mut last_line = address.end();
    let (target_match, limit) = if let SubstitutionScope::Single(n) = scope {
        (n - 1, 1)
    } else {
        (0, 0)
    };

    let mut changes = ChangeSet::new(buffer.current_line());
    let mut replacement_lines = Vec::new();
    let mut span_start: Option<usize> = None;
    loop {
        let line = &buffer[line_num];
        let eol_idx = line
            .rfind("\r\n")
            .or_else(|| line.rfind('\n'))
            .unwrap_or(line.len());
        let first_match = pattern.find_iter(&line[..eol_idx]).nth(target_match);
        let step = if let Some(first_match) = first_match {
            span_start.get_or_insert(line_num);
            let mut edited_line = line[..first_match.start()].to_owned();
            edited_line.push_str(&pattern.replacen(
                &line[first_match.start()..eol_idx],
                limit,
                replacement,
            ));
            edited_line.push_str(&line[eol_idx..]);
            replacement_lines.extend(
                edited_line
                    .split_terminator('\n')
                    .map(|l| l.trim_end_matches('\r'))
                    .map(|l| l.to_owned() + buffer.default_eol()),
            );
            1
        } else {
            // no match - apply span of matches up to this point,
            // if any
            if let Some(span_start) = span_start.take() {
                let step =
                    replacement_lines.len() - (line_num - span_start) + 1;
                let change = buffer.do_change(
                    Some(Address::span(span_start, line_num - 1)),
                    replacement_lines,
                );
                let change = Change::try_from(change)
                    .expect("do_change always returns single Change ChangeSet");
                changes.push(change, buffer.current_line());
                replacement_lines = Vec::new();
                step
            } else {
                1
            }
        };
        if line_num == last_line {
            if let Some(span_start) = span_start {
                let change = buffer.do_change(
                    Some(Address::span(span_start, line_num)),
                    replacement_lines,
                );
                let change = Change::try_from(change)
                    .expect("always returns single Change ChangeSet");
                changes.push(change, buffer.current_line());
            }
            break;
        }
        line_num += step;
        last_line = address.end() + step - 1;
    }

    if changes.is_empty() { Err(Error::NoMatch) } else { Ok(Some(changes)) }
}

fn transfer_cmd(
    buffer: &mut EditBuffer,
    mut address: Option<Address>,
    destination: Address,
) -> Result<Option<ChangeSet>, Error> {
    let source =
        address.get_or_insert_with(|| Address::line(buffer.current_line()));
    if destination.end() >= source.start() && destination.end() < source.end() {
        return Err(Error::DestinationIntersectsSource);
    }
    Ok(Some(buffer.do_transfer(address, destination)))
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
) -> Result<Option<ChangeSet>, Error> {
    if buffer.filename().is_none() && filename.is_some() {
        buffer.set_filename(filename.map(ToOwned::to_owned));
    }
    let filename = filename.or(buffer.filename()).ok_or(Error::NoFilename)?;

    let mut destination = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(filename)
        .map_err(|source| Error::WriteFileOpen { source })?;

    let (bytes_written, lines_written) =
        write_lines(&mut destination, buffer, address)?;
    writeln!(output, "{lines_written} lines ({bytes_written} bytes) written")
        .unwrap();
    output.flush().unwrap();
    Ok(None)
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
                        .write(&line.as_bytes()[bytes_written..])
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
    fn null_cmd_single_line() {
        let mut output = Vec::new();
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3"]);
        buffer.set_current_line(2);
        null_cmd(&mut buffer, &mut output, Some(Address::line(1))).unwrap();
        assert_eq!(buffer.current_line(), 1);
        assert_eq!(str::from_utf8(&output[..]).unwrap(), "1\n");
    }

    #[test]
    fn null_cmd_no_addr() {
        let mut output = Vec::new();
        let mut buffer = EditBuffer::from(vec!["1\r\n", "2", "3"]);
        buffer.set_current_line(2);
        null_cmd(&mut buffer, &mut output, None).unwrap();
        assert_eq!(str::from_utf8(&output[..]).unwrap(), "3\r\n");
        assert_eq!(buffer.current_line(), 3);
    }

    #[test]
    fn null_cmd_no_addr_last_line_gives_error() {
        let mut output = Vec::new();
        let mut buffer = EditBuffer::from(vec!["1\r\n", "2", "3"]);
        buffer.set_current_line(3);
        let res = null_cmd(&mut buffer, &mut output, None)
            .expect_err("invalid address");
        assert!(matches!(res, Error::InvalidAddress));
        assert_eq!(buffer.current_line(), 3);
    }

    #[test]
    fn null_cmd_span() {
        let mut output = Vec::new();
        let mut buffer =
            EditBuffer::from(vec!["1\r\n", "2", "3", "4", "5", "6"]);
        buffer.set_current_line(5);
        null_cmd(&mut buffer, &mut output, Some(Address::span(2, 4))).unwrap();
        let output = str::from_utf8(&output[..]).unwrap();
        assert_eq!(output, "4\r\n");
        assert_eq!(buffer.current_line(), 4);
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
        assert_eq!(str::from_utf8(&output[..]).unwrap(), " 2  2\r\n");
    }

    #[test]
    fn enumerate_sets_current_line() {
        let mut output = Vec::new();
        let mut buffer = EditBuffer::from(vec![
            "1\r\n", "2", "3", "4", "5", "6", "7", "8", "9", "10",
        ]);
        buffer.set_current_line(2);

        enumerate_cmd(&mut buffer, &mut output, Some(Address::span(6, 9)))
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

        enumerate_cmd(&mut buffer, &mut output, Some(Address::span(4, 900)))
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
        let res = global_cmd(
            &mut buffer,
            &mut output,
            None,
            &pat,
            &commands,
            &mut prev_pattern,
        );
        match res {
            Err(e) => panic!("unexpected error \"{e:?}\""),
            Ok(Some(changes)) => assert!(changes.is_empty()),
            Ok(None) => panic!("should have returned an empty ChangeSet"),
        }
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
        );
        assert!(matches!(res, Err(Error::NestedGlobalCmd)));
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
        let Ok(Some(changes)) = global_cmd(
            &mut buffer,
            &mut output,
            Some(Address::span(1, 3)),
            &pat,
            &commands,
            &mut prev_pattern,
        ) else {
            panic!("should have returned Ok(Some(ChangeSet))");
        };
        assert!(changes.is_empty());
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
        let Ok(Some(changes)) = global_cmd(
            &mut buffer,
            &mut output,
            None,
            &pat,
            &commands,
            &mut prev_pattern,
        ) else {
            panic!("should have returned Ok(Some(ChangeSet))");
        };
        assert!(changes.is_empty());
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
        let Ok(Some(changes)) = global_cmd(
            &mut buffer,
            &mut output,
            Some(Address::span(1, 3)),
            &pat,
            &commands,
            &mut prev_pattern,
        ) else {
            panic!("should have returned Ok(Some(ChangeSet))");
        };
        assert!(changes.is_empty());
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
        let Ok(Some(changes)) = global_cmd(
            &mut buffer,
            &mut output,
            Some(Address::span(2, 5)),
            &pat,
            &commands,
            &mut prev_pattern,
        ) else {
            panic!("unexpected global_cmd error!");
        };
        assert!(changes.is_empty());
        assert_eq!(
            str::from_utf8(&output[..]).unwrap(),
            "2  two\n3  three\n4  four\n5  five\n"
        );
    }

    #[test]
    fn global_cmd_append() {
        let mut buffer = EditBuffer::from(vec![
            "one\n", "two", "three", "four", "five", "six",
        ]);
        let orig = buffer.clone();
        let expected = EditBuffer::from(vec![
            "one\n", "append", "two", "three", "append", "four", "five",
            "append", "six",
        ]);
        let mut output = Vec::new();
        let mut prev_pattern: Option<Regex> = None;
        let pat = Regex::new("e$").unwrap();
        let commands = "a\nappend\n.\n".to_owned();
        let Ok(Some(changes)) = global_cmd(
            &mut buffer,
            &mut output,
            Some(Address::span(1, 6)),
            &pat,
            &commands,
            &mut prev_pattern,
        ) else {
            panic!("global_cmd's err return was Some() rather than None!")
        };
        assert!(!changes.is_empty());
        assert_eq!(&buffer[..], &expected[..]);
        assert_eq!(buffer.current_line(), 8);
        buffer.push_undo(changes);

        // now undo
        buffer.do_undo().expect("something there to undo");
        assert_eq!(&buffer[..], &orig[..]);
        assert_eq!(buffer.current_line(), orig.current_line());

        // redo
        buffer.do_redo().expect("something there to undo");
        assert_eq!(&buffer[..], &expected[..]);
        assert_eq!(buffer.current_line(), 8);
    }

    #[test]
    fn global_cmd_change() {
        let mut buffer = EditBuffer::from(vec![
            "one\n", "one", "two", "two", "three", "three", "four", "four",
            "five", "five", "six", "six",
        ]);
        let orig = buffer.clone();
        let expected = EditBuffer::from(vec![
            "change 1\n",
            "change 2",
            "change 3",
            "two",
            "two",
            "change 1",
            "change 2",
            "change 3",
            "four",
            "four",
            "five",
            "five",
            "six",
            "six",
        ]);
        let mut output = Vec::new();
        let mut prev_pattern: Option<Regex> = None;
        let pat = Regex::new("([a-z]*e)$").unwrap();
        let commands = ".,+c\nchange 1\nchange 2\nchange 3\n.\n".to_owned();
        let Ok(Some(changes)) = global_cmd(
            &mut buffer,
            &mut output,
            Some(Address::span(1, 6)),
            &pat,
            &commands,
            &mut prev_pattern,
        ) else {
            panic!("global_cmd's err return wasn't None!")
        };
        assert!(!changes.is_empty());
        buffer.push_undo(changes);
        assert_eq!(&buffer[..], &expected[..]);
        assert_eq!(buffer.current_line(), 8);

        // now undo
        buffer.do_undo().expect("something there to undo");
        assert_eq!(&buffer[..], &orig[..]);
        assert_eq!(buffer.current_line(), orig.current_line());

        // redo
        buffer.do_redo().expect("something there to undo");
        assert_eq!(&buffer[..], &expected[..]);
        assert_eq!(buffer.current_line(), 8);
    }

    #[test]
    fn global_cmd_delete() {
        let mut buffer = EditBuffer::from(vec![
            "one\n", "two", "three", "four", "five", "six",
        ]);
        let orig = buffer.clone();
        let expected = EditBuffer::from(vec!["two\n", "four", "six"]);
        let mut output = Vec::new();
        let mut prev_pattern: Option<Regex> = None;
        let pat = Regex::new("e$").unwrap();
        let commands = "dn\n".to_owned();
        let Ok(Some(changes)) = global_cmd(
            &mut buffer,
            &mut output,
            Some(Address::span(1, 6)),
            &pat,
            &commands,
            &mut prev_pattern,
        ) else {
            panic!("global_cmd err return wasn't None!");
        };
        assert!(!changes.is_empty());
        buffer.push_undo(changes);
        assert_eq!(
            str::from_utf8(&output[..]).unwrap(),
            "1  two\n2  four\n3  six\n"
        );
        assert_eq!(&buffer[..], &expected[..]);
        assert_eq!(buffer.current_line(), 3);

        // now undo
        buffer.do_undo().expect("something there to undo");
        assert_eq!(&buffer[..], &orig[..]);
        assert_eq!(buffer.current_line(), orig.current_line());

        // redo
        buffer.do_redo().expect("something there to undo");
        assert_eq!(&buffer[..], &expected[..]);
        assert_eq!(buffer.current_line(), 3);
    }

    #[test]
    fn global_cmd_insert() {
        let mut buffer = EditBuffer::from(vec![
            "one\r\n", "two", "three", "four", "five", "six",
        ]);
        let orig = buffer.clone();
        let expected = EditBuffer::from(vec![
            "insert\r\n",
            "one",
            "two",
            "insert",
            "three",
            "four",
            "insert",
            "five",
            "six",
        ]);
        let mut output = Vec::new();
        let mut prev_pattern: Option<Regex> = None;
        let pat = Regex::new("e$").unwrap();
        let commands = "i\r\ninsert\r\n.\r\n".to_owned();
        let Ok(Some(changes)) = global_cmd(
            &mut buffer,
            &mut output,
            Some(Address::span(1, 6)),
            &pat,
            &commands,
            &mut prev_pattern,
        ) else {
            panic!("global_cmd returned an unexpected error!");
        };
        assert!(!changes.is_empty());
        buffer.push_undo(changes);
        assert_eq!(&buffer[..], &expected[..]);
        assert_eq!(buffer.current_line(), 7);

        // now undo
        buffer.do_undo().expect("something there to undo");
        assert_eq!(&buffer[..], &orig[..]);
        assert_eq!(buffer.current_line(), orig.current_line());

        // redo
        buffer.do_redo().expect("something there to undo");
        assert_eq!(&buffer[..], &expected[..]);
        assert_eq!(buffer.current_line(), 7);
    }

    #[test]
    fn global_cmd_join() {
        let mut buffer = EditBuffer::from(vec![
            "one\n", "two", "three", "four", "five", "six",
        ]);
        let orig = buffer.clone();
        let mut expected =
            EditBuffer::from(vec!["onetwo\n", "threefour", "fivesix"]);
        expected.set_current_line(3);
        let mut output = Vec::new();
        let mut prev_pattern: Option<Regex> = None;
        let pat = Regex::new("e$").unwrap();
        let commands = "jn\n".to_owned();
        let res = global_cmd(
            &mut buffer,
            &mut output,
            Some(Address::span(1, 6)),
            &pat,
            &commands,
            &mut prev_pattern,
        );
        let changes = match res {
            Err(e) => panic!("unexpected error {e:?}"),
            Ok(None) => panic!("should have returned Some(ChangeSet)"),
            Ok(Some(changes)) => changes,
        };
        assert!(!changes.is_empty());
        buffer.push_undo(changes);
        assert_eq!(
            str::from_utf8(&output[..]).unwrap(),
            "1  onetwo\n2  threefour\n3  fivesix\n"
        );
        assert_eq!(&buffer[..], &expected[..]);
        assert_eq!(buffer.current_line(), 3);

        // now undo
        buffer.do_undo().expect("something there to undo");
        assert_eq!(&buffer[..], &orig[..]);
        assert_eq!(buffer.current_line(), orig.current_line());

        // redo
        buffer.do_redo().expect("something there to undo");
        assert_eq!(&buffer[..], &expected[..]);
        assert_eq!(buffer.current_line(), 3);
    }

    #[test]
    fn global_cmd_move() {
        let mut buffer = EditBuffer::from(vec![
            "one\r\n", "two", "three", "four", "five", "six",
        ]);
        let orig = buffer.clone();
        let mut expected = EditBuffer::from(vec![
            "three\r\n",
            "two",
            "one",
            "four",
            "five",
            "six",
        ]);
        expected.set_current_line(1);
        let mut output = Vec::new();
        let mut prev_pattern: Option<Regex> = None;
        let pat = Regex::new("^t").unwrap();
        let commands = "m0\r\n".to_owned();
        let Some(changes) = global_cmd(
            &mut buffer,
            &mut output,
            Some(Address::span(1, 6)),
            &pat,
            &commands,
            &mut prev_pattern,
        )
        .expect("should have been Ok!") else {
            panic!("should have been Some(changes)!");
        };
        assert!(!changes.is_empty());
        buffer.push_undo(changes);
        assert_eq!(&buffer[..], &expected[..]);
        assert_eq!(buffer.current_line(), expected.current_line());

        // now undo
        buffer.do_undo().expect("something there to undo");
        assert_eq!(&buffer[..], &orig[..]);
        assert_eq!(buffer.current_line(), orig.current_line());

        // redo
        buffer.do_redo().expect("something there to undo");
        assert_eq!(&buffer[..], &expected[..]);
        assert_eq!(buffer.current_line(), expected.current_line());
    }

    #[test]
    fn global_cmd_move_with_overlap() {
        let mut buffer = EditBuffer::from(vec![
            "one\r\n", "two", "three", "four", "five", "six",
        ]);
        let orig = buffer.clone();
        let mut expected = EditBuffer::from(vec![
            "two\r\n", "three", "one", "four", "five", "six",
        ]);
        expected.set_current_line(2);
        let mut output = Vec::new();
        let mut prev_pattern: Option<Regex> = None;
        let pat = Regex::new("^t").unwrap();
        let commands = ".,+m0\r\n".to_owned();
        let Some(changes) = global_cmd(
            &mut buffer,
            &mut output,
            Some(Address::span(1, 6)),
            &pat,
            &commands,
            &mut prev_pattern,
        )
        .expect("should have been Ok!") else {
            panic!("should have been Some(changes)!");
        };
        assert!(!changes.is_empty());
        buffer.push_undo(changes);
        assert_eq!(&buffer[..], &expected[..]);
        assert_eq!(buffer.current_line(), expected.current_line());

        // now undo
        buffer.do_undo().expect("something there to undo");
        assert_eq!(&buffer[..], &orig[..]);
        assert_eq!(buffer.current_line(), orig.current_line());

        // redo
        buffer.do_redo().expect("something there to undo");
        assert_eq!(&buffer[..], &expected[..]);
        assert_eq!(buffer.current_line(), expected.current_line());
    }

    #[test]
    fn global_cmd_substitute_with_error() {
        let mut buffer = EditBuffer::from(vec![
            "1:one two three four\n",
            "2:five six seven eight",
            "3:nine ten eleven twelve",
            "4:thirteen fourteen fifteen sixteen",
            "5:seventeen eighteen nineteen twenty",
            "6:thirteen fourteen fifteen sixteen",
            "7:nine ten eleven twelve",
            "8:five six seven eight",
            "9:one two three four\n",
        ]);
        buffer.set_current_line(5);
        let before = buffer.clone();
        let mut expected = EditBuffer::from(vec![
            "1:one two three four\n",
            "2:five ",
            "'x seven eight",
            "3:nine ten eleven twelve",
            "4:thirteen fourteen fifteen ",
            "'xteen",
            "5:",
            "'venteen eighteen nineteen twenty",
            "6:thirteen fourteen fifteen ",
            "'xteen",
            "7:nine ten eleven twelve",
            "8:five six seven eight",
            "9:one two three four\n",
        ]);
        expected.set_current_line(12);
        let expected_output = " 6  'xteen\n10  'xteen\n";

        let mut output = Vec::new();
        let mut prev_pattern: Option<Regex> = None;
        let pat = Regex::new("s[aeiou]").unwrap();
        let commands = ".,+2s//\\\n'/n".to_string();
        let Err(Error::GlobalCmdErrorStop { source, changes }) = global_cmd(
            &mut buffer,
            &mut output,
            None,
            &pat,
            &commands,
            &mut prev_pattern,
        ) else {
            panic!("should have returned GlobalCmdErrorStop");
        };
        assert!(matches!(*source, Error::InvalidAddress));
        let Some(changes) = changes else {
            panic!("changes was None!");
        };
        let output = str::from_utf8(&output[..]).unwrap();
        assert_eq!(output, expected_output);
        buffer.push_undo(changes);
        assert_eq!(buffer.current_line(), expected.current_line());
        assert_eq!(&buffer[..], &expected[..]);
        buffer.do_undo().unwrap();
        assert_eq!(buffer.current_line(), before.current_line());
        assert_eq!(&before[..], &buffer[..]);
        buffer.do_redo().unwrap();
        assert_eq!(buffer.current_line(), expected.current_line());
        assert_eq!(&buffer[..], &expected[..]);
    }

    #[test]
    fn global_cmd_substitute() {
        let mut buffer = EditBuffer::from(vec![
            "1:one two three four\n",
            "2:five six seven eight",
            "3:nine ten eleven twelve",
            "4:thirteen fourteen fifteen sixteen",
            "5:seventeen eighteen nineteen twenty",
            "6:thirteen fourteen fifteen sixteen",
            "7:nine ten eleven twelve",
            "8:five six seven eight",
            "9:one two three four\n",
        ]);
        buffer.set_current_line(5);
        let before = buffer.clone();
        let mut expected = EditBuffer::from(vec![
            "1:one two three four\n",
            "2:five ",
            "'x seven eight",
            "3:nine ten eleven twelve",
            "4:thirteen fourteen fifteen ",
            "'xteen",
            "5:",
            "'venteen eighteen nineteen twenty",
            "6:thirteen fourteen fifteen ",
            "'xteen",
            "7:nine ten eleven twelve",
            "8:five ",
            "'x seven eight",
            "9:one two three four\n",
        ]);
        expected.set_current_line(13);
        let expected_output = " 3  'x seven eight\n 6  'xteen\n 8  'venteen eighteen nineteen twenty\n10  'xteen\n13  'x seven eight\n";

        let mut output = Vec::new();
        let mut prev_pattern: Option<Regex> = None;
        let pat = Regex::new("s[aeiou]").unwrap();
        let commands = "s//\\\n'/n".to_string();
        let Some(changes) = global_cmd(
            &mut buffer,
            &mut output,
            None,
            &pat,
            &commands,
            &mut prev_pattern,
        )
        .expect("should have been Ok") else {
            panic!("should have been Some(changes)!");
        };
        let output = str::from_utf8(&output[..]).unwrap();
        assert_eq!(output, expected_output);
        assert!(!changes.is_empty());
        buffer.push_undo(changes);
        assert_eq!(buffer.current_line(), expected.current_line());
        assert_eq!(&buffer[..], &expected[..]);
        buffer.do_undo().unwrap();
        assert_eq!(buffer.current_line(), before.current_line());
        assert_eq!(&before[..], &buffer[..]);
        buffer.do_redo().unwrap();
        assert_eq!(buffer.current_line(), expected.current_line());
        assert_eq!(&buffer[..], &expected[..]);
    }

    #[test]
    fn global_cmd_transfer() {
        let mut buffer = EditBuffer::from(vec![
            "one\r\n", "two", "three", "four", "five", "six",
        ]);
        let orig = buffer.clone();
        let expected = EditBuffer::from(vec![
            "one\r\n", "two", "three", "four", "five", "six", "one", "three",
            "five",
        ]);
        let mut output = Vec::new();
        let mut prev_pattern: Option<Regex> = None;
        let pat = Regex::new("e$").unwrap();
        let commands = "t$\r\n".to_owned();
        let Some(changes) = global_cmd(
            &mut buffer,
            &mut output,
            Some(Address::span(1, 6)),
            &pat,
            &commands,
            &mut prev_pattern,
        )
        .expect("should have been Ok!") else {
            panic!("should have been Some(changes)!");
        };
        assert!(!changes.is_empty());
        buffer.push_undo(changes);
        assert_eq!(&buffer[..], &expected[..]);
        assert_eq!(buffer.current_line(), 9);

        // now undo
        buffer.do_undo().expect("something there to undo");
        assert_eq!(&buffer[..], &orig[..]);
        assert_eq!(buffer.current_line(), orig.current_line());

        // redo
        buffer.do_redo().expect("something there to undo");
        assert_eq!(&buffer[..], &expected[..]);
        assert_eq!(buffer.current_line(), 9);
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
            Some(Address::span(1, 3)),
            &pat,
            &commands,
            &mut prev_pattern,
        );
        assert!(matches!(res, Err(Error::UnsupportedGlobalCmd)));
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
        print_cmd(&mut buffer, &mut output, Some(Address::span(2, 4))).unwrap();
        assert_eq!(&output[..], b"2\r\n3\r\n4\r\n");
    }

    #[test]
    fn print_cmd_sets_current_line() {
        let mut output = Vec::new();
        let mut buffer =
            EditBuffer::from(vec!["1\r\n", "2", "3", "4", "5", "6"]);
        buffer.set_current_line(5);
        print_cmd(&mut buffer, &mut output, Some(Address::span(2, 4))).unwrap();
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
        let output = str::from_utf8(&output[..]).unwrap();
        assert!(output.contains(
            "unwritten changes - repeat edit command to discard changes"
        ));
        assert!(!output.contains(
            "unwritten changes - repeat quit command to discard changes"
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
    fn append_cmd_dispatch_p_print_sfx() {
        let input = b"ap\none\ntwo\nthree\n.\n2p\nq\nq\n";
        let mut output = Vec::new();
        run(&input[..], &mut output, &CmdArgs::default()).unwrap();
        let output = str::from_utf8(&output[..]).unwrap();
        assert!(output.contains("two\n"));
        assert!(output.contains("unwritten changes"));
        assert!(!output.contains("one"));
        assert!(output.contains("three\n"));
    }

    #[test]
    fn append_cmd_dispatch_n_print_sfx() {
        let input = b"an\none\ntwo\nthree\n.\n2p\nq\nq\n";
        let mut output = Vec::new();
        run(&input[..], &mut output, &CmdArgs::default()).unwrap();
        let output = str::from_utf8(&output[..]).unwrap();
        assert!(output.contains("two\n"));
        assert!(output.contains("unwritten changes"));
        assert!(!output.contains("one"));
        assert!(output.contains("3  three\n"));
    }

    #[test]
    fn append_cmd_dispatch_l_print_sfx() {
        let input = b"al\none\ntwo\nthree\n.\n2p\nq\nq\n";
        let mut output = Vec::new();
        run(&input[..], &mut output, &CmdArgs::default()).unwrap();
        let output = str::from_utf8(&output[..]).unwrap();
        assert!(output.contains("two\n"));
        assert!(output.contains("unwritten changes"));
        assert!(!output.contains("one"));
        assert!(output.contains("three$\n"));
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
        let args = CmdArgs { file: None };
        run(&input[..], &mut output, &args).unwrap();
        let output = str::from_utf8(&output[..]).unwrap();
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
    fn join_cmd_dispatch() {
        let input = b"a\n1\n2\n3\n4\n.\n1,2j\n1,$p\nq\nq\n";
        let mut output = Vec::new();
        run(&input[..], &mut output, &CmdArgs::default()).unwrap();
        let output = str::from_utf8(&output[..]).unwrap();
        assert!(output.contains("12\n3\n4\n"));
    }

    #[test]
    fn list_cmd_dispatch() {
        let input = b"a\n1\n2\n3\n4\n.\n1,2l\nq\nq\n";
        let mut output = Vec::new();
        run(&input[..], &mut output, &CmdArgs::default()).unwrap();
        let output = str::from_utf8(&output[..]).unwrap();
        assert!(output.contains("1$\n2$\n"));
    }

    #[test]
    fn line_number_cmd_dispatch() {
        let input = b"a\n1\n2\n3\n4\n.\n2n\n=\n.=\nq\nq\n";
        let mut output = Vec::new();
        run(&input[..], &mut output, &CmdArgs::default()).unwrap();
        let output = str::from_utf8(&output[..]).unwrap();
        assert!(output.contains("\n2\n"));
        assert!(output.contains("\n4\n"));
    }

    #[test]
    fn move_cmd_dispatch() {
        let input = b"a\n3\n4\n5\n1\n2\n.\n3,4m0\n1,$p\nq\nq\n";
        let mut output = Vec::new();
        run(&input[..], &mut output, &CmdArgs::default()).unwrap();
        let output = str::from_utf8(&output[..]).unwrap();
        assert!(output.contains("5\n1\n3\n4\n2\n"));
    }

    #[test]
    fn null_cmd_dispatch() {
        let input = b"a\r\none\r\ntwo\r\nthree\r\n.\r\n1\r\n\r\nq\r\nq\r\n";
        let mut output = Vec::new();
        run(&input[..], &mut output, &CmdArgs::default()).unwrap();
        let output = str::from_utf8(&output[..]).unwrap();
        assert!(output.contains("one"));
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
    fn read_cmd_dispatch() {
        let input = b"a\npre 1\npre 2\npost 1\npost 2\n.\n2r test/assets/text_with_final_eol.txt\nq\nq\n";
        let mut output = Vec::new();
        run(&input[..], &mut output, &CmdArgs::default()).unwrap();
        let output = str::from_utf8(&output[..]).unwrap();
        assert!(output.contains("312"));
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
    fn substitute_cmd_dispatch() {
        let input = b"a\n11231145611\n.\n1s/[^01]+/./g\n1p\nq\nq\n";
        let mut output = Vec::new();
        run(&input[..], &mut output, &CmdArgs::default()).unwrap();
        let output = str::from_utf8(&output[..]).unwrap();
        assert!(output.contains("11.11.11\n"));
    }

    #[test]
    fn transfer_cmd_dispatch() {
        let input = b"a\n3\n4\n5\n1\n2\n.\n4,5t0\n1,$p\nq\nq\n";
        let mut output = Vec::new();
        run(&input[..], &mut output, &CmdArgs::default()).unwrap();
        let output = str::from_utf8(&output[..]).unwrap();
        assert!(output.contains("1\n2\n3\n4\n5\n1\n2\n"));
    }

    #[test]
    fn substitute_cmd_no_matches() {
        let mut buffer = EditBuffer::from(vec![
            "one two three four\r\n",
            "five six seven eight",
            "nine ten eleven twelve",
            "thirteen fourteen fifteen sixteen",
            "seventeen eighteen nineteen twenty",
        ]);
        buffer.set_current_line(5);
        let res = substitute_cmd(
            &mut buffer,
            Some(Address::span(1, 5)),
            &Regex::new("won't match").unwrap(),
            "",
            SubstitutionScope::Global,
        )
        .expect_err("should give error");
        assert!(matches!(res, Error::NoMatch));
    }

    #[test]
    fn substitute_cmd_current_line_global() {
        let mut buffer = EditBuffer::from(vec![
            "one two three four\r\n",
            "five six seven eight",
            "nine ten eleven twelve",
            "thirteen fourteen fifteen sixteen",
            "seventeen eighteen nineteen twenty",
        ]);
        buffer.set_current_line(5);
        substitute_cmd(
            &mut buffer,
            None,
            &Regex::new("e+n").unwrap(),
            "'",
            SubstitutionScope::Global,
        )
        .unwrap();
        assert_eq!(buffer[5], "sev't' eight' ninet' tw'ty\r\n");
    }

    #[test]
    fn substitute_cmd_current_line_at_eol() {
        let mut buffer = EditBuffer::from(vec!["some text\n"]);
        let expected = EditBuffer::from(vec!["some text!\n"]);
        substitute_cmd(
            &mut buffer,
            None,
            &Regex::new("$").unwrap(),
            "!",
            SubstitutionScope::Single(1),
        )
        .unwrap();
        assert_eq!(&buffer[..], &expected[..]);
    }

    #[test]
    fn substitute_cmd_current_line_single_first() {
        let mut buffer = EditBuffer::from(vec![
            "one two three four\r\n",
            "five six seven eight",
            "nine ten eleven twelve",
            "thirteen fourteen fifteen sixteen",
            "seventeen eighteen nineteen twenty",
        ]);
        buffer.set_current_line(5);
        substitute_cmd(
            &mut buffer,
            None,
            &Regex::new("e+n").unwrap(),
            "'",
            SubstitutionScope::Single(1),
        )
        .unwrap();
        assert_eq!(buffer[5], "sev'teen eighteen nineteen twenty\r\n");
    }

    #[test]
    fn substitute_cmd_current_line_single() {
        let mut buffer = EditBuffer::from(vec![
            "one two three four\r\n",
            "five six seven eight",
            "nine ten eleven twelve",
            "thirteen fourteen fifteen sixteen",
            "seventeen eighteen nineteen twenty",
        ]);
        buffer.set_current_line(5);
        substitute_cmd(
            &mut buffer,
            None,
            &Regex::new("e+n").unwrap(),
            "'",
            SubstitutionScope::Single(4),
        )
        .unwrap();
        assert_eq!(buffer[5], "seventeen eighteen ninet' twenty\r\n");
    }

    #[test]
    fn substitute_split_line() {
        let mut buffer = EditBuffer::from(vec!["a line, to split\r\n"]);
        buffer.set_current_line(1);
        let cmd_line = "s/, /\\\r\n/";
        let mut input = cmd_line.as_bytes();
        let Some((Cmd::Substitute(address, pattern, replacement, scope), None)) =
            Cmd::read(&mut input, &mut buffer, &mut None).unwrap()
        else {
            panic!("{cmd_line} didn't parse as Cmd::Substitute");
        };
        substitute_cmd(
            &mut buffer,
            address,
            &pattern,
            replacement.as_str(),
            scope,
        )
        .unwrap();
        let mut expected = EditBuffer::from(vec!["a line\r\n", "to split"]);
        expected.set_current_line(2);
        assert_eq!(buffer, expected);
    }

    #[test]
    fn substitute_split_line_no_end_delimiter() {
        let mut buffer = EditBuffer::from(vec!["a line, to split\n"]);
        buffer.set_current_line(1);
        let mut cmd_line = "/, /\\\n".graphemes(true).peekable();
        let mut input = "\n".as_bytes();
        let Ok(Some((
            Cmd::Substitute(address, pattern, replacement, scope),
            None,
        ))) = command::parse_substitute_cmd(
            &mut cmd_line,
            None,
            &mut None,
            &mut input,
        )
        else {
            panic!("should have parsed to Cmd::Substitute!");
        };
        substitute_cmd(
            &mut buffer,
            address,
            &pattern,
            replacement.as_str(),
            scope,
        )
        .unwrap();
        let mut expected = EditBuffer::from(vec!["a line\n", "to split"]);
        expected.set_current_line(2);
        assert_eq!(buffer[..], expected[..]);
        assert_eq!(buffer.current_line(), expected.current_line());
    }

    #[test]
    fn substitute_cmd_multi_line_single() {
        let mut buffer = EditBuffer::from(vec![
            "1:one two three four\n",
            "2:five six seven eight",
            "3:nine ten eleven twelve",
            "4:thirteen fourteen fifteen sixteen",
            "5:seventeen eighteen nineteen twenty",
            "6:thirteen fourteen fifteen sixteen",
            "7:nine ten eleven twelve",
            "8:five six seven eight",
            "9:one two three four\n",
        ]);
        buffer.set_current_line(5);
        let mut expected = EditBuffer::from(vec![
            "1:one two three four\n",
            "2:five 'x seven eight",
            "3:nine ten eleven twelve",
            "4:thirteen fourteen fifteen 'xteen",
            "5:'venteen eighteen nineteen twenty",
            "6:thirteen fourteen fifteen 'xteen",
            "7:nine ten eleven twelve",
            "8:five 'x seven eight",
            "9:one two three four\n",
        ]);
        expected.set_current_line(8);
        substitute_cmd(
            &mut buffer,
            Some(Address::span(2, 9)),
            &Regex::new("s[aeiou]").unwrap(),
            "'",
            SubstitutionScope::Single(1),
        )
        .unwrap();
        assert_eq!(buffer.current_line(), expected.current_line());
        assert_eq!(&buffer[..], &expected[..]);
    }

    #[test]
    fn undo_redo_substitute_cmd_multi_line_single() {
        let mut buffer = EditBuffer::from(vec![
            "1:one two three four\n",
            "2:five six seven eight",
            "3:nine ten eleven twelve",
            "4:thirteen fourteen fifteen sixteen",
            "5:seventeen eighteen nineteen twenty",
            "6:thirteen fourteen fifteen sixteen",
            "7:nine ten eleven twelve",
            "8:five six seven eight",
            "9:one two three four\n",
        ]);
        buffer.set_current_line(5);
        let before = buffer.clone();
        let mut expected = EditBuffer::from(vec![
            "1:one two three four\n",
            "2:five 'x seven eight",
            "3:nine ten eleven twelve",
            "4:thirteen fourteen fifteen 'xteen",
            "5:'venteen eighteen nineteen twenty",
            "6:thirteen fourteen fifteen 'xteen",
            "7:nine ten eleven twelve",
            "8:five 'x seven eight",
            "9:one two three four\n",
        ]);
        expected.set_current_line(8);
        let Some(changes) = substitute_cmd(
            &mut buffer,
            Some(Address::span(2, 9)),
            &Regex::new("s[aeiou]").unwrap(),
            "'",
            SubstitutionScope::Single(1),
        )
        .unwrap() else {
            panic!("expected Some(ChangeSet)!");
        };
        assert!(!changes.is_empty());
        buffer.push_undo(changes);
        assert_eq!(buffer.current_line(), expected.current_line());
        assert_eq!(&buffer[..], &expected[..]);
        buffer.do_undo().unwrap();
        assert_eq!(buffer.current_line(), before.current_line());
        assert_eq!(&before[..], &buffer[..]);
        buffer.do_redo().unwrap();
        assert_eq!(buffer.current_line(), expected.current_line());
        assert_eq!(&buffer[..], &expected[..]);
    }

    #[test]
    fn substitute_cmd_multi_line_single_first() {
        let mut buffer = EditBuffer::from(vec![
            "one two three four\r\n",
            "five six seven eight",
            "nine ten eleven twelve",
            "thirteen fourteen fifteen sixteen",
            "seventeen eighteen nineteen twenty",
        ]);
        buffer.set_current_line(5);
        substitute_cmd(
            &mut buffer,
            Some(Address::span(2, 3)),
            &Regex::new("e+n").unwrap(),
            "'",
            SubstitutionScope::Single(1),
        )
        .unwrap();
        assert_eq!(
            buffer[2..4],
            ["five six sev' eight\r\n", "nine t' eleven twelve\r\n"]
        );
    }

    #[test]
    fn substitute_cmd_multi_line_capture() {
        let mut buffer = EditBuffer::from(vec![
            "one two three four\r\n",
            "five six seven eight",
            "nine ten eleven twelve",
            "thirteen fourteen fifteen sixteen",
            "seventeen eighteen nineteen twenty",
        ]);
        buffer.set_current_line(5);
        substitute_cmd(
            &mut buffer,
            Some(Address::span(2, 4)),
            &Regex::new("[a-z]+?(e+n)[^ ]*").unwrap(),
            "$1 ($0)",
            SubstitutionScope::Single(2),
        )
        .unwrap();
        assert_eq!(
            buffer[2..5],
            [
                "five six seven eight\r\n",
                "nine ten en (eleven) twelve\r\n",
                "thirteen een (fourteen) fifteen sixteen\r\n"
            ]
        );
    }

    #[test]
    fn undo_redo_substitute_cmd_multi_line_capture() {
        let mut buffer = EditBuffer::from(vec![
            "one two three four\r\n",
            "five six seven eight",
            "nine ten eleven twelve",
            "thirteen fourteen fifteen sixteen",
            "seventeen eighteen nineteen twenty",
        ]);
        buffer.set_current_line(5);
        let before = buffer.clone();
        let Ok(Some(changes)) = substitute_cmd(
            &mut buffer,
            Some(Address::span(2, 4)),
            &Regex::new("[a-z]+?(e+n)[^ ]*").unwrap(),
            "$1 ($0)",
            SubstitutionScope::Single(2),
        ) else {
            panic!("expected Ok(Some(ChangeSet))!");
        };
        assert!(!changes.is_empty());
        buffer.push_undo(changes);
        assert_eq!(
            buffer[2..5],
            [
                "five six seven eight\r\n",
                "nine ten en (eleven) twelve\r\n",
                "thirteen een (fourteen) fifteen sixteen\r\n"
            ]
        );
        let after = buffer.clone();

        buffer.do_undo().unwrap();
        assert_eq!(&buffer[..], &before[..]);

        buffer.do_redo().unwrap();
        assert_eq!(&buffer[..], &after[..]);
    }

    #[test]
    fn transfer_cmd_destination_intersects_source_give_error() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        let source = Address::span(3, 5);
        let destination = Address::line(5);
        transfer_cmd(&mut buffer, Some(source), destination).unwrap();
        assert_eq!(
            &buffer[..],
            &["1\n", "2\n", "3\n", "4\n", "5\n", "3\n", "4\n", "5\n", "6\n"]
        );
        let destination = Address::line(4);
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
            Some(Address::span(1, 2)),
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
            Some(Address::span(1, 6)),
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
        let Some(change) =
            append_cmd(&mut buffer, &mut input, Some(Address::line(0)))
                .unwrap()
        else {
            panic!("expected Some(ChangeSet) from append_cmd!");
        };
        assert!(!change.is_empty());
        buffer.push_undo(change);
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
        let Some(change) =
            append_cmd(&mut buffer, &mut input, Some(Address::line(0)))
                .unwrap()
        else {
            panic!("expected append_cmd to return Some(ChangeSet)!");
        };
        assert!(!change.is_empty());
        buffer.push_undo(change);
        assert!(buffer.is_dirty());
        let mut dummy_file = Vec::new();
        let address = Some(Address::span(1, buffer.len()));
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
        let Some(change) =
            append_cmd(&mut buffer, &mut input, Some(Address::line(0)))
                .unwrap()
        else {
            panic!("expected Some(ChangeSet) from append_cmd!");
        };
        assert!(!change.is_empty());
        buffer.push_undo(change);
        assert!(buffer.is_dirty());
        let mut dummy_file = Vec::new();
        let (bytes, lines) = write_lines(
            &mut dummy_file,
            &mut buffer,
            Some(Address::span(1, 2)),
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
        let res = delete_cmd(&mut buffer, Some(Address::span(0, 3)))
            .expect_err("invalid address");
        assert!(matches!(res, Error::InvalidAddress));
    }

    #[test]
    fn edit_cmd_no_filename_error() {
        let mut buffer = EditBuffer::new();
        let res = edit_cmd(&mut buffer, &mut Vec::new(), None, None)
            .expect_err("no filename");
        assert!(matches!(res, Error::NoFilename));
    }

    #[test]
    fn edit_cmd_missing_file_clears_buffer_sets_filename() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3"]);
        assert_eq!(buffer.len(), 3);
        let mut output = Vec::new();
        let not_a_file = Some(Path::new("non-existant_file.txt"));
        let res = edit_cmd(&mut buffer, &mut output, not_a_file, None)
            .expect_err("FileNotFound");
        assert!(matches!(res, Error::FileNotFound(_)));
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

        edit_cmd(&mut buffer, &mut output, filename1, None).unwrap();
        assert_eq!(buffer.len(), 10);
        let out_text = str::from_utf8(&output[..]).unwrap();
        assert!(
            out_text.contains("10 lines") && out_text.contains("312 bytes")
        );

        output.clear();
        edit_cmd(&mut buffer, &mut output, filename2, None).unwrap();
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
            Some(Address::span(5, 6)),
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
            Some(Address::span(2, 4)),
        )
        .expect_err("illegal address");
        assert!(matches!(res, Error::InvalidAddress));
    }

    #[test]
    fn join_cmd_empty_buffer() {
        let mut buffer = EditBuffer::new();
        let res = join_cmd(&mut buffer, None).expect_err("should fail");
        assert!(matches!(res, Error::InvalidAddress));
    }

    #[test]
    fn join_cmd_single_line_addr() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3"]);
        let expected = buffer.clone();
        join_cmd(&mut buffer, Some(Address::line(2))).unwrap();
        assert_eq!(buffer, expected);
    }

    #[test]
    fn join_cmd_default_on_last_line() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3"]);
        let res = join_cmd(&mut buffer, None).expect_err("should fail");
        assert!(matches!(res, Error::InvalidAddress));
    }

    #[test]
    fn move_cmd_destination_intersects_source_give_error() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        let source = Address::span(3, 5);
        let res = move_cmd(&mut buffer, Some(source), Address::line(4))
            .expect_err("should fail");
        assert!(matches!(res, Error::DestinationIntersectsSource));
        move_cmd(&mut buffer, Some(source), Address::line(5))
            .expect("shouldn't fail");
    }

    #[test]
    fn line_number_cmd_with_and_without_address() {
        let mut output = Vec::new();
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        buffer.set_current_line(2);
        let res = line_number_cmd(&mut buffer, &mut output, None);
        let out_text = str::from_utf8(&output[..]).unwrap();
        assert_eq!(out_text, "6\n");
        assert!(res.is_none());
        output.clear();
        let res =
            line_number_cmd(&mut buffer, &mut output, Some(Address::line(2)));
        let out_text = str::from_utf8(&output[..]).unwrap();
        assert!(res.is_none());
        assert_eq!(out_text, "2\n");
    }

    #[test]
    fn read_cmd_no_filename_error() {
        let mut buffer = EditBuffer::new();
        let res = read_cmd(&mut buffer, &mut Vec::new(), None, None)
            .expect_err("no filename");
        assert!(matches!(res, Error::NoFilename));
    }

    #[test]
    fn read_cmd_reads_file() {
        let mut buffer =
            EditBuffer::from(vec!["one\n", "two", "three", "four"]);
        buffer.set_current_line(2);
        let orig = buffer.clone();
        let mut expected = EditBuffer::from(vec![
            "one\n",
            "two",
            "This is a test file with several lines of",
            "text. It is for unit testing, so it's not long,",
            "but it will suffice to test commands that",
            "read",
            "and",
            "edit files. The lines",
            "are of various lengths, and",
            "end and begin with ",
            "\"special\" characters (i.e., non-alpha characters).",
            "Critically, it ends with a final line terminator.",
            "three",
            "four",
        ]);
        expected.set_current_line(12);
        let mut output = Vec::new();
        let filename1 = Some(Path::new(r"test/assets/text_with_final_eol.txt"));

        let changes = read_cmd(&mut buffer, &mut output, None, filename1)
            .expect("no error")
            .expect("Some(ChangeSet)");
        let out_text = str::from_utf8(&output[..]).unwrap();
        assert!(
            out_text.contains("10 lines") && out_text.contains("312 bytes")
        );
        buffer.push_undo(changes);

        assert_eq!(buffer[..], expected[..]);
        assert_eq!(buffer.current_line(), expected.current_line());

        buffer.do_undo().expect("something to undo");
        assert_eq!(buffer[..], orig[..]);
        assert_eq!(buffer.current_line(), orig.current_line());

        buffer.do_redo().expect("something to redo");
        assert_eq!(buffer[..], expected[..]);
        assert_eq!(buffer.current_line(), expected.current_line());
    }

    #[test]
    fn list_cmd_bad_addr() {
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3"]);
        let mut output = Vec::new();
        let res = list_cmd(&mut buffer, &mut output, Some(Address::line(4)))
            .expect_err("invalid addr");
        assert!(matches!(res, Error::InvalidAddress));

        buffer = EditBuffer::new();
        let res =
            list_cmd(&mut buffer, &mut output, None).expect_err("invalid addr");
        assert!(matches!(res, Error::InvalidAddress));
    }

    #[test]
    fn list_cmd_no_addr() {
        let mut output = Vec::new();
        let mut buffer = EditBuffer::from(vec!["1\r\n", "2", "3"]);
        buffer.set_current_line(2);
        list_cmd(&mut buffer, &mut output, None).unwrap();
        assert_eq!(str::from_utf8(&output[..]).unwrap(), "2$\r\n");
    }

    #[test]
    fn list_cmd_single_line() {
        let mut output = Vec::new();
        let mut buffer = EditBuffer::from(vec!["1\r\n", "2", "3"]);
        buffer.set_current_line(2);
        list_cmd(&mut buffer, &mut output, Some(Address::line(3))).unwrap();
        assert_eq!(str::from_utf8(&output[..]).unwrap(), "3$\r\n");
    }

    #[test]
    fn list_cmd_span() {
        let mut output = Vec::new();
        let mut buffer =
            EditBuffer::from(vec!["1\r\n", "2\t2", "3", "4", "5", "6"]);
        buffer.set_current_line(5);
        list_cmd(&mut buffer, &mut output, Some(Address::span(2, 4))).unwrap();
        assert_eq!(
            str::from_utf8(&output[..]).unwrap(),
            "2\\t2$\r\n3$\r\n4$\r\n"
        );
    }

    #[test]
    fn list_cmd_sets_current_line() {
        let mut output = Vec::new();
        let mut buffer =
            EditBuffer::from(vec!["1\r\n", "2", "3", "4", "5", "6"]);
        buffer.set_current_line(5);
        list_cmd(&mut buffer, &mut output, Some(Address::span(2, 4))).unwrap();
        assert_eq!(4, buffer.current_line());
    }
}
