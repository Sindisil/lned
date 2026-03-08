use std::borrow::Cow;
use std::cmp;
use std::collections::VecDeque;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, prelude::*};
use std::ops::RangeInclusive;
use std::path::Path;
use std::path::PathBuf;

use crossterm::terminal;
use regex::Regex;
use similar::TextDiff;
use unicode_segmentation::UnicodeSegmentation;

use crate::cli;
use crate::command::{self, Address, Cmd, PrintAttributes, SubstitutionScope};
use crate::edit_buffer::{Change, ChangeSet, EditBuffer, PrevailingEol};

use line_edit::LineEdit;

#[derive(Debug)]
pub enum LnedError {
    ParseCmd(command::Error),
    InvalidAddress,
    NestedGlobalCmd,
    UnsupportedGlobalCmd,
    ReadGlobalCmd {
        source: command::Error,
    },
    NoFilename,
    EditFileOpen {
        source: std::io::Error,
        filename: PathBuf,
    },
    WriteFileOpen {
        source: std::io::Error,
        filename: PathBuf,
    },
    WriteFile {
        source: std::io::Error,
        filename: PathBuf,
        backup_filename: Option<PathBuf>,
    },
    ReadLines {
        source: std::io::Error,
    },
    QuitUnwrittenChanges,
    EditUnwrittenChanges,
    FileNotFound(PathBuf),
    DestinationIntersectsSource,
    NoMatch,
    NothingToUndo,
    NothingToRedo,
    GlobalCmdErrorStop {
        source: Box<LnedError>,
        changes: Option<ChangeSet>,
    },
    ReadFileOpen {
        source: std::io::Error,
        file: PathBuf,
    },
    WriteBackupFileCreate {
        source: std::io::Error,
        filename: PathBuf,
        backup_filename: Option<PathBuf>,
    },
    WriteMakeBackup {
        source: std::io::Error,
        filename: PathBuf,
        backup_filename: Option<PathBuf>,
    },
    WriteRemoveBackup {
        source: std::io::Error,
        backup_filename: Option<PathBuf>,
    },
    DiffReadFile {
        source: std::io::Error,
        filename: PathBuf,
    },
    WriteWouldOverwrite(PathBuf),
}

impl std::error::Error for LnedError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match *self {
            LnedError::ParseCmd(_)
            | LnedError::FileNotFound(_)
            | LnedError::QuitUnwrittenChanges
            | LnedError::EditUnwrittenChanges
            | LnedError::InvalidAddress
            | LnedError::NestedGlobalCmd
            | LnedError::UnsupportedGlobalCmd
            | LnedError::DestinationIntersectsSource
            | LnedError::NoMatch
            | LnedError::NothingToUndo
            | LnedError::NothingToRedo
            | LnedError::WriteWouldOverwrite(_)
            | LnedError::NoFilename => None,
            LnedError::EditFileOpen { ref source, .. }
            | LnedError::DiffReadFile { ref source, .. }
            | LnedError::WriteMakeBackup { ref source, .. }
            | LnedError::WriteRemoveBackup { ref source, .. }
            | LnedError::WriteBackupFileCreate { ref source, .. }
            | LnedError::WriteFileOpen { ref source, .. }
            | LnedError::WriteFile { ref source, .. }
            | LnedError::ReadFileOpen { ref source, .. }
            | LnedError::ReadLines { ref source } => Some(source),
            LnedError::ReadGlobalCmd { ref source } => Some(source),
            LnedError::GlobalCmdErrorStop { ref source, .. } => Some(source),
        }
    }
}

impl fmt::Display for LnedError {
    #[allow(clippy::too_many_lines)]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LnedError::ParseCmd(e) => write!(f, "{e}"),
            LnedError::InvalidAddress => write!(f, "invalid address"),
            LnedError::NestedGlobalCmd => {
                write!(f, "invalid nested global command")
            }
            LnedError::UnsupportedGlobalCmd => {
                write!(f, "unsupported global command")
            }
            LnedError::ReadGlobalCmd { .. } => {
                write!(f, "error reading global command")
            }
            LnedError::NoFilename => write!(f, "no filename"),
            LnedError::EditFileOpen { filename, .. } => {
                write!(f, "error opening \"{}\" to edit", filename.display())
            }
            LnedError::WriteFileOpen { filename, .. } => {
                write!(
                    f,
                    "error opening \"{}\" for writing",
                    filename.display()
                )
            }
            LnedError::WriteFile { filename, backup_filename, .. } => {
                write!(
                    f,
                    "error writing buffer to \"{}\"",
                    filename.display(),
                )?;
                if let Some(backup_filename) = backup_filename {
                    write!(
                        f,
                        ", backup left in \"{}\"",
                        backup_filename.display()
                    )?;
                }
                Ok(())
            }
            LnedError::ReadLines { .. } => {
                write!(f, "error reading input lines")
            }
            LnedError::QuitUnwrittenChanges => {
                write!(
                    f,
                    "unwritten changes - repeat quit command to discard changes"
                )
            }
            LnedError::EditUnwrittenChanges => {
                write!(
                    f,
                    "unwritten changes - repeat edit command to discard changes"
                )
            }
            LnedError::FileNotFound(filename) => {
                write!(f, "{} not found", filename.display())
            }
            LnedError::DestinationIntersectsSource => {
                write!(f, "destination intersects source")
            }
            LnedError::NoMatch => {
                write!(f, "no matches found")
            }
            LnedError::NothingToUndo => write!(f, "nothing to undo"),
            LnedError::NothingToRedo => write!(f, "nothing to redo"),
            LnedError::GlobalCmdErrorStop { .. } => {
                write!(f, "error executing global command")
            }
            LnedError::ReadFileOpen { file, .. } => {
                write!(f, "error opening \"{}\" to read", file.display())
            }
            LnedError::WriteBackupFileCreate {
                filename,
                backup_filename,
                ..
            } => {
                write!(
                    f,
                    "error creating \"{}\" as backup for \"{}\"",
                    backup_filename
                        .as_ref()
                        .expect("backup path exists if this error produced")
                        .display(),
                    filename.display(),
                )
            }
            LnedError::WriteMakeBackup {
                filename, backup_filename, ..
            } => {
                write!(
                    f,
                    "error writing \"{}\" as backup of \"{}\"",
                    backup_filename
                        .as_ref()
                        .expect("backup path exists if this error produced")
                        .display(),
                    filename.display()
                )
            }
            LnedError::WriteRemoveBackup { backup_filename, .. } => {
                write!(
                    f,
                    "error removing \"{}\"",
                    backup_filename
                        .as_ref()
                        .expect("backup path exists if this error produced")
                        .display()
                )
            }
            LnedError::DiffReadFile { filename, .. } => {
                write!(f, "error reading {} for diff", filename.display())
            }
            LnedError::WriteWouldOverwrite(filename) => {
                write!(
                    f,
                    "'{}' exists - repeat write command to overwrite",
                    filename.display()
                )
            }
        }
    }
}

#[derive(Debug, Default)]
struct Editor {
    previous_warning: Warning,
    previous_pattern: Option<regex::Regex>,
    scroll_row_limit: Option<usize>,
    current_file: Option<PathBuf>,
    buffer: EditBuffer,
}

#[derive(Debug, Default, PartialEq)]
enum Warning {
    #[default]
    None,
    QuitUnsaved,
    EditUnsaved,
    WriteOverwrite(Option<Address>, Option<PathBuf>),
}

impl Editor {
    fn new() -> Editor {
        Editor { ..Default::default() }
    }

    #[allow(clippy::too_many_lines)]
    fn dispatch_cmd(
        &mut self,
        cmd: &Cmd,
        output: &mut impl Write,
        input: &mut impl LineEdit,
    ) -> Result<bool, LnedError> {
        let mut done = false;
        let res = match cmd {
            // dispatch editor commands
            Cmd::Append(address) => {
                self.append_cmd(input, *address, IndentMode::Auto)
            }
            Cmd::AppendRaw(address) => {
                self.append_cmd(input, *address, IndentMode::Raw)
            }
            Cmd::Delete(address) => self.delete_cmd(*address),
            Cmd::Change(address) => {
                self.change_cmd(input, *address, IndentMode::Auto)
            }
            Cmd::ChangeRaw(address) => {
                self.change_cmd(input, *address, IndentMode::Raw)
            }
            Cmd::Edit(filename) => self.edit_cmd(output, filename.as_deref()),
            Cmd::Enumerate(address) => self.enumerate_cmd(output, *address),
            Cmd::File(filename) => {
                self.file_cmd(output, filename.as_deref());
                Ok(None)
            }
            Cmd::Global(address, pattern, commands) => {
                self.global_cmd(output, *address, pattern, commands)
            }
            Cmd::Insert(address) => {
                self.insert_cmd(input, *address, IndentMode::Auto)
            }
            Cmd::InsertRaw(address) => {
                self.insert_cmd(input, *address, IndentMode::Raw)
            }
            Cmd::Join(address, separator) => {
                self.join_cmd(*address, separator.as_deref())
            }
            Cmd::LineNumber(address) => {
                Ok(self.line_number_cmd(output, *address))
            }
            Cmd::List(address) => self.list_cmd(output, *address),
            Cmd::Move(address, destination) => {
                self.move_cmd(*address, *destination)
            }
            Cmd::Newline(eol) => Ok(self.newline_cmd(output, *eol)),
            Cmd::Null(address) => self.null_cmd(output, *address),
            Cmd::Print(address) => self.print_cmd(output, *address),
            Cmd::Quit => self.quit_cmd().inspect(|_| done = true),
            Cmd::Read(address, filename) => {
                self.read_cmd(output, *address, filename.as_deref())
            }
            Cmd::Redo => self.buffer.do_redo().map(|()| None),
            Cmd::Scroll(address, cmd_rows, attrs) => {
                let (cols, term_rows): (usize, usize) = terminal::size()
                    .map_or((80, 24), |(cols, rows)| {
                        (cols.into(), rows.into())
                    });
                let rows = *match cmd_rows {
                    Some(rows) => self.scroll_row_limit.insert(*rows),
                    None => self
                        .scroll_row_limit
                        .get_or_insert_with(|| term_rows.saturating_sub(2)),
                };
                self.scroll_cmd(
                    output,
                    *address,
                    *attrs,
                    ScrollWindow { cols, rows },
                )
            }
            Cmd::ShowDiff(filename) => {
                self.show_diff_cmd(output, filename.as_deref())
            }
            Cmd::Substitute(address, pattern, replacement, scope) => {
                self.substitute_cmd(*address, pattern, replacement, *scope)
            }
            Cmd::Transfer(address, destination) => {
                self.transfer_cmd(*address, *destination)
            }
            Cmd::Undo => self.buffer.do_undo().map(|()| None),
            Cmd::Version => {
                version_cmd(output);
                Ok(None)
            }
            Cmd::Write(address, filename) => {
                self.write_cmd(output, *address, filename.as_deref())
            }
        };

        match res {
            Ok(Some(changes)) => self.buffer.push_undo(changes),
            Ok(None) => (),
            Err(LnedError::GlobalCmdErrorStop { source, changes }) => {
                if let Some(changes) = changes {
                    self.buffer.push_undo(changes);
                }
                return Err(*source);
            }
            Err(e) => return Err(e),
        }
        Ok(done)
    }

    fn append_cmd(
        &mut self,
        input: &mut impl LineEdit,
        address: Option<Address>,
        indent_mode: IndentMode,
    ) -> Result<Option<ChangeSet>, LnedError> {
        self.previous_warning = Warning::None;
        if address.is_some_and(|a| a.end() > self.buffer.len()) {
            return Err(LnedError::InvalidAddress);
        }
        let indent = match indent_mode {
            IndentMode::Auto => self.buffer[..=address
                .map_or_else(|| self.buffer.current_line(), |a| a.end())]
                .iter()
                .rfind(|l| l.contains(|c: char| !c.is_whitespace()))
                .and_then(|l| command::INDENT.captures(l))
                .and_then(|c| c.get(1))
                .map(|m| m.as_str().to_owned()),
            IndentMode::Raw => None,
        };
        let mut lines = Vec::new();
        Cmd::read_input_lines(input, &mut lines, indent)
            .map_err(|source| LnedError::ReadLines { source })?;
        Ok(self.buffer.do_append(address, lines))
    }

    fn change_cmd(
        &mut self,
        input: &mut impl LineEdit,
        address: Option<Address>,
        indent_mode: IndentMode,
    ) -> Result<Option<ChangeSet>, LnedError> {
        self.previous_warning = Warning::None;
        if address.is_some_and(|a| a.end() > self.buffer.len()) {
            return Err(LnedError::InvalidAddress);
        }
        let to_change = address.map_or_else(
            || Address::line(cmp::max(self.buffer.current_line(), 1)),
            |a| Address::span(cmp::max(a.start(), 1), cmp::max(a.end(), 1)),
        );
        let indent = match indent_mode {
            IndentMode::Auto => self.buffer[RangeInclusive::from(to_change)]
                .iter()
                .find(|l| l.contains(|c: char| !c.is_whitespace()))
                .or_else(|| {
                    self.buffer[..to_change.start()]
                        .iter()
                        .rfind(|l| l.contains(|c: char| !c.is_whitespace()))
                })
                .and_then(|l| command::INDENT.captures(l))
                .and_then(|c| c.get(1))
                .map(|m| m.as_str().to_owned()),
            IndentMode::Raw => None,
        };

        let mut lines = Vec::new();
        Cmd::read_input_lines(input, &mut lines, indent)
            .map_err(|source| LnedError::ReadLines { source })?;
        Ok(Some(self.buffer.do_change(address, lines)))
    }

    fn delete_cmd(
        &mut self,
        address: Option<Address>,
    ) -> Result<Option<ChangeSet>, LnedError> {
        self.previous_warning = Warning::None;
        match address {
            Some(addr) if addr.start() == 0 => Err(LnedError::InvalidAddress),
            None if self.buffer.current_line() == 0 => {
                Err(LnedError::InvalidAddress)
            }
            _ => Ok(Some(self.buffer.do_delete(address))),
        }
    }

    fn scroll_cmd(
        &mut self,
        output: &mut impl Write,
        address: Option<Address>,
        attrs: Option<PrintAttributes>,
        window: ScrollWindow,
    ) -> Result<Option<ChangeSet>, LnedError> {
        self.previous_warning = Warning::None;
        // create addressed span to print from specified address
        // and max_rows
        let start = if let Some(addr) = address {
            addr.end()
        } else {
            self.buffer
                .current_line()
                .checked_add(1)
                .ok_or(LnedError::InvalidAddress)?
        };
        let end = cmp::min(self.buffer.len(), start + window.rows);
        let address = Address::span(start, end);

        let attrs = attrs.unwrap_or_default();
        let last_printed =
            print_lines(output, &self.buffer, address, attrs, Some(&window))?;
        self.buffer.set_current_line(cmp::min(last_printed, self.buffer.len()));
        Ok(None)
    }

    fn show_diff_cmd(
        &mut self,
        output: &mut impl Write,
        filename: Option<&Path>,
    ) -> Result<Option<ChangeSet>, LnedError> {
        self.previous_warning = Warning::None;
        let filename = filename
            .or(self.current_file.as_deref())
            .ok_or(LnedError::NoFilename)?;
        let file = fs::read(filename).map_err(|source| {
            LnedError::DiffReadFile { source, filename: filename.to_owned() }
        })?;
        let file = String::from_utf8_lossy(&file);
        let mem = Cow::from(self.buffer[..].concat());
        TextDiff::from_lines(&file, &mem)
            .unified_diff()
            .header(&filename.as_os_str().to_string_lossy(), "current buffer")
            .to_writer(output)
            .expect("reliable stdout");
        Ok(None)
    }

    fn edit_cmd(
        &mut self,
        output: &mut impl Write,
        filename: Option<&Path>,
    ) -> Result<Option<ChangeSet>, LnedError> {
        if self.buffer.is_dirty() {
            if self.previous_warning == Warning::EditUnsaved {
                self.previous_warning = Warning::None;
                self.buffer.reset_clean_fingerprint();
            } else {
                self.previous_warning = Warning::EditUnsaved;
                return Err(LnedError::EditUnwrittenChanges);
            }
        }

        if let Some(filename) = filename {
            self.current_file = Some(filename.to_owned());
        }
        let filename =
            self.current_file.as_ref().ok_or(LnedError::NoFilename)?;

        let file = File::open(filename);
        let mut source = match file {
            Ok(f) => BufReader::new(f),
            Err(e) => {
                return match e.kind() {
                    io::ErrorKind::NotFound => {
                        let err = Err(LnedError::FileNotFound(filename.into()));
                        self.buffer.clear_text();
                        err
                    }
                    _ => Err(LnedError::EditFileOpen {
                        source: e,
                        filename: filename.into(),
                    }),
                };
            }
        };

        let mut lines = Vec::new();
        let bytes_read = read_lines(&mut source, &mut lines)?;
        let lines_read = lines.len();
        self.buffer.clear_text();
        let missing_eol = self.buffer.append(0, lines);
        write!(output, "{lines_read} lines ({bytes_read} bytes) read",)
            .unwrap();
        let prevailing_eol = self
            .buffer
            .prevailing_eol()
            .expect("prevailing_eol set after append");
        writeln!(output, " [{prevailing_eol}]").unwrap();
        if missing_eol {
            writeln!(output, "missing newline appended").unwrap();
        }
        output.flush().unwrap();
        self.buffer.set_current_line(self.buffer.len());
        Ok(None)
    }

    fn read_cmd(
        &mut self,
        output: &mut impl Write,
        address: Option<Address>,
        filename: Option<&Path>,
    ) -> Result<Option<ChangeSet>, LnedError> {
        self.previous_warning = Warning::None;
        let address = if let Some(address) = address {
            if address.end() > self.buffer.len() {
                return Err(LnedError::InvalidAddress);
            }
            address
        } else {
            Address::line(self.buffer.current_line())
        };

        // read shouldn't set the remembered filename
        let filename = filename
            .or(self.current_file.as_deref())
            .ok_or(LnedError::NoFilename)?;

        let file = File::open(filename);
        let mut source = match file {
            Ok(f) => BufReader::new(f),
            Err(e) => {
                return match e.kind() {
                    io::ErrorKind::NotFound => {
                        Err(LnedError::FileNotFound(filename.into()))
                    }
                    _ => Err(LnedError::ReadFileOpen {
                        source: e,
                        file: filename.into(),
                    }),
                };
            }
        };

        let mut lines = Vec::new();
        let bytes_read = read_lines(&mut source, &mut lines)?;
        let lines_read = lines.len();
        writeln!(output, "{} lines ({bytes_read} bytes) read", lines.len())
            .unwrap();
        let mut changes = ChangeSet::new(
            self.buffer.current_line(),
            self.buffer.prevailing_eol(),
        );
        changes.push(Change::Add(address.end(), lines.clone()));
        if self.buffer.append(address.end(), lines) {
            output.flush().unwrap();
            writeln!(output, "missing newline appended").unwrap();
        }
        self.buffer.set_current_line(address.end() + lines_read);
        Ok(Some(changes))
    }

    fn substitute_cmd(
        &mut self,
        address: Option<Address>,
        pattern: &Regex,
        replacement: &str,
        scope: SubstitutionScope,
    ) -> Result<Option<ChangeSet>, LnedError> {
        self.previous_warning = Warning::None;
        let address = address
            .unwrap_or_else(|| Address::line(self.buffer.current_line()));
        if address.start() == 0
            || address.start() > address.end()
            || address.end() > self.buffer.len()
        {
            return Err(LnedError::InvalidAddress);
        }

        let prevailing_eol = self
            .buffer
            .prevailing_eol()
            .expect("non-empty buffer has valid EOL")
            .eol
            .as_str();
        let mut line_num = address.start();
        let mut last_line = address.end();
        let (target_match, limit) = if let SubstitutionScope::Single(n) = scope
        {
            (n - 1, 1)
        } else {
            (0, 0)
        };

        let mut changes = ChangeSet::new(
            self.buffer.current_line(),
            self.buffer.prevailing_eol(),
        );
        let mut replacement_lines = Vec::new();
        let mut span_start: Option<usize> = None;
        loop {
            let line = &self.buffer[line_num];
            let eol_idx = line
                .rfind("\r\n")
                .or_else(|| line.rfind('\n'))
                .unwrap_or(line.len());
            let first_match =
                pattern.find_iter(&line[..eol_idx]).nth(target_match);
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
                        .map(|l| l.to_owned() + prevailing_eol),
                );
                1
            } else {
                // no match - apply span of matches up to this point,
                // if any
                if let Some(span_start) = span_start.take() {
                    let step =
                        replacement_lines.len() - (line_num - span_start) + 1;
                    for change in self
                        .buffer
                        .do_change(
                            Some(Address::span(span_start, line_num - 1)),
                            replacement_lines,
                        )
                        .drain()
                    {
                        changes.push(change);
                    }
                    replacement_lines = Vec::new();
                    step
                } else {
                    1
                }
            };
            if line_num == last_line {
                if let Some(span_start) = span_start {
                    for change in self
                        .buffer
                        .do_change(
                            Some(Address::span(span_start, line_num)),
                            replacement_lines,
                        )
                        .drain()
                    {
                        changes.push(change);
                    }
                }
                break;
            }
            line_num += step;
            last_line = address.end() + step - 1;
        }

        if changes.is_empty() {
            Err(LnedError::NoMatch)
        } else {
            Ok(Some(changes))
        }
    }

    fn transfer_cmd(
        &mut self,
        mut address: Option<Address>,
        destination: Address,
    ) -> Result<Option<ChangeSet>, LnedError> {
        self.previous_warning = Warning::None;
        if destination.end() > self.buffer.len() {
            return Err(LnedError::InvalidAddress);
        }
        let source = address
            .get_or_insert_with(|| Address::line(self.buffer.current_line()));
        if destination.end() >= source.start()
            && destination.end() < source.end()
        {
            return Err(LnedError::DestinationIntersectsSource);
        }
        Ok(Some(self.buffer.do_transfer(address, destination)))
    }

    fn enumerate_cmd(
        &mut self,
        output: &mut impl Write,
        address: Option<Address>,
    ) -> Result<Option<ChangeSet>, LnedError> {
        self.previous_warning = Warning::None;
        let address = address
            .or_else(|| {
                if self.buffer.current_line() == 0 {
                    return None;
                }
                Some(Address::line(self.buffer.current_line()))
            })
            .ok_or(LnedError::InvalidAddress)?;
        let attrs = PrintAttributes { enumerate: true, ..Default::default() };
        let last_printed =
            print_lines(output, &self.buffer, address, attrs, None)?;
        self.buffer.set_current_line(last_printed);
        Ok(None)
    }

    fn file_cmd(&mut self, output: &mut impl Write, filename: Option<&Path>) {
        self.previous_warning = Warning::None;
        if let Some(filename) = filename {
            self.current_file = Some(filename.to_owned());
        }

        if let Some(filename) = &self.current_file {
            write!(output, "{}", filename.display()).unwrap();
        } else {
            write!(output, "no filename set").unwrap();
        }

        if let Some(eol) = self.buffer.prevailing_eol() {
            writeln!(output, " [{eol}]").unwrap();
        } else {
            writeln!(output, " [None]").unwrap();
        }

        output.flush().unwrap();
    }

    fn global_cmd(
        &mut self,
        output: &mut impl Write,
        address: Option<Address>,
        pattern: &Regex,
        commands: &str,
    ) -> Result<Option<ChangeSet>, LnedError> {
        self.previous_warning = Warning::None;
        let mut changes = ChangeSet::new(
            self.buffer.current_line(),
            self.buffer.prevailing_eol(),
        );
        self.previous_pattern = Some(pattern.clone());
        // make a list of matching lines
        let search_range =
            address.map_or_else(|| 1..=self.buffer.len(), Into::into);
        let matched_lines = (search_range)
            .filter(|&n| {
                self.buffer[n]
                    .lines()
                    .next()
                    .is_some_and(|l| pattern.is_match(l))
            })
            .collect::<VecDeque<usize>>();
        let res =
            self.do_global_cmds(output, commands, matched_lines, &mut changes);
        let changes = if changes.is_empty() { None } else { Some(changes) };
        match res {
            Ok(()) => Ok(changes),
            Err(e) => match e {
                LnedError::NestedGlobalCmd => Err(LnedError::NestedGlobalCmd),
                LnedError::UnsupportedGlobalCmd => {
                    Err(LnedError::UnsupportedGlobalCmd)
                }
                e => Err(LnedError::GlobalCmdErrorStop {
                    source: Box::new(e),
                    changes,
                }),
            },
        }
    }

    fn do_global_cmds(
        &mut self,
        output: &mut impl Write,
        commands: &str,
        mut matched_lines: VecDeque<usize>,
        changes: &mut ChangeSet,
    ) -> Result<(), LnedError> {
        // iterate over list
        while let Some(line_num) = matched_lines.pop_front() {
            self.buffer.set_current_line(line_num);
            let mut input = commands.as_bytes();

            // parse and execute command list for line
            while let Some((cmd, sfx)) = Cmd::read(
                &mut input,
                &mut self.buffer,
                &mut self.previous_pattern,
            )
            .map_err(|source| LnedError::ReadGlobalCmd { source })?
            {
                let cs = match cmd {
                    Cmd::Append(address) => {
                        self.append_cmd(&mut input, address, IndentMode::Auto)
                    }
                    Cmd::Change(address) => {
                        self.change_cmd(&mut input, address, IndentMode::Auto)
                    }
                    Cmd::Delete(address) => self.delete_cmd(address),
                    Cmd::Enumerate(address) => {
                        self.enumerate_cmd(output, address)
                    }
                    Cmd::Global(..) => return Err(LnedError::NestedGlobalCmd),
                    Cmd::Insert(address) => {
                        self.insert_cmd(&mut input, address, IndentMode::Auto)
                    }
                    Cmd::Join(address, separator) => {
                        self.join_cmd(address, separator.as_deref())
                    }
                    Cmd::Move(address, destination) => {
                        self.move_cmd(address, destination)
                    }
                    Cmd::List(address) => self.list_cmd(output, address),
                    Cmd::Null(address) | Cmd::Print(address) => {
                        self.print_cmd(output, address)
                    }
                    Cmd::Substitute(address, pattern, replacement, scope) => {
                        self.substitute_cmd(
                            address,
                            &pattern,
                            &replacement,
                            scope,
                        )
                    }
                    Cmd::Transfer(address, destination) => {
                        self.transfer_cmd(address, destination)
                    }
                    _ => Err(LnedError::UnsupportedGlobalCmd),
                }?;
                if let Some(mut cs) = cs {
                    for change in cs.drain() {
                        adjust_global_list(&mut matched_lines, &change);
                        changes.push(change);
                    }
                    if let Some(attrs) = sfx {
                        print_lines(
                            output,
                            &self.buffer,
                            Address::line(self.buffer.current_line()),
                            attrs,
                            None,
                        )?;
                    }
                }
            }
        }
        Ok(())
    }

    fn newline_cmd(
        &mut self,
        output: &mut impl Write,
        eol: Option<PrevailingEol>,
    ) -> Option<ChangeSet> {
        self.previous_warning = Warning::None;
        let changes =
            eol.and_then(|eol| self.buffer.set_prevailing_eol(eol.eol));

        writeln!(
            output,
            "prevailing newline: {}",
            self.buffer
                .prevailing_eol()
                .map_or("None", |eol| eol.display_str())
        )
        .unwrap();

        changes
    }

    fn null_cmd(
        &mut self,
        output: &mut impl Write,
        address: Option<Address>,
    ) -> Result<Option<ChangeSet>, LnedError> {
        self.previous_warning = Warning::None;
        let address = Some(Address::line(
            address.map_or_else(|| self.buffer.current_line() + 1, |a| a.end()),
        ));
        self.print_cmd(output, address)
    }

    fn print_cmd(
        &mut self,
        output: &mut impl Write,
        address: Option<Address>,
    ) -> Result<Option<ChangeSet>, LnedError> {
        self.previous_warning = Warning::None;
        let address = address
            .or_else(|| {
                if self.buffer.current_line() == 0 {
                    None
                } else {
                    Some(Address::line(self.buffer.current_line()))
                }
            })
            .ok_or(LnedError::InvalidAddress)?;
        let attrs = PrintAttributes { ..Default::default() };
        let last_printed =
            print_lines(output, &self.buffer, address, attrs, None)?;
        self.buffer.set_current_line(last_printed);
        Ok(None)
    }

    fn list_cmd(
        &mut self,
        output: &mut impl Write,
        address: Option<Address>,
    ) -> Result<Option<ChangeSet>, LnedError> {
        self.previous_warning = Warning::None;
        let address = address
            .or_else(|| {
                if self.buffer.current_line() == 0 {
                    None
                } else {
                    Some(Address::line(self.buffer.current_line()))
                }
            })
            .ok_or(LnedError::InvalidAddress)?;
        let attrs =
            PrintAttributes { expand_escapes: true, ..Default::default() };
        let last_printed =
            print_lines(output, &self.buffer, address, attrs, None)?;
        self.buffer.set_current_line(last_printed);
        Ok(None)
    }

    fn move_cmd(
        &mut self,
        mut address: Option<Address>,
        destination: Address,
    ) -> Result<Option<ChangeSet>, LnedError> {
        self.previous_warning = Warning::None;
        if destination.end() > self.buffer.len() {
            return Err(LnedError::InvalidAddress);
        }
        let source = address
            .get_or_insert_with(|| Address::line(self.buffer.current_line()));
        if destination.end() >= source.start()
            && destination.end() < source.end()
        {
            return Err(LnedError::DestinationIntersectsSource);
        }
        Ok(Some(self.buffer.do_move(address, destination)))
    }

    fn write_cmd(
        // "w" with no address range or filename is "save"
        //     Sets sync and file fingerprints equal to buffer's on success.
        //
        //     Warnings:
        //         * If file contents changed since last sync
        //     Errors:
        //         * If no associated filename
        //
        // "[addr]w [filename]" is "save as"
        //     Sets associated filename if not already set, otherwise
        //     associated filename is unchanged.
        //
        //     If full buffer is addressed and filename is same as associated
        //     filename, overwrite warning is given, but command is otherwise
        //     treated as a "save" with respect to fingerprint updates and the
        //     like.
        //
        //     Warnings:
        //         * If file already exists (overwwrite)
        //     Errors:
        //         * I/O errors
        //
        &mut self,
        output: &mut impl Write,
        address: Option<Address>,
        filename: Option<&Path>,
    ) -> Result<Option<ChangeSet>, LnedError> {
        let cmd_filename = filename;
        let safe_to_overwrite = filename.is_none()
            || matches!(
                &self.previous_warning,
                Warning::WriteOverwrite(a, f)
                    if *a == address && f.as_deref() == filename,
            );

        let filename = filename
            .or(self.current_file.as_deref())
            .ok_or(LnedError::NoFilename)?;

        let mut writer = EditedFile::open_or_create(filename)?;
        if !(writer.created || safe_to_overwrite) {
            if let Err(e) = writer.remove_backup().map_err(|source| {
                LnedError::WriteRemoveBackup {
                    source,
                    backup_filename: writer
                        .backup_name()
                        .map(ToOwned::to_owned),
                }
            }) {
                writeln!(output, "{e}").expect("reliable stdout");
            }
            self.previous_warning = Warning::WriteOverwrite(
                address,
                cmd_filename.map(ToOwned::to_owned),
            );
            return Err(LnedError::WriteWouldOverwrite(filename.to_owned()));
        }

        write_file(&mut self.buffer, output, address, &mut writer)?;

        if self.current_file.is_none() {
            self.current_file = Some(filename.to_owned());
        }
        self.previous_warning = Warning::None;
        Ok(None)
    }

    fn insert_cmd(
        &mut self,
        input: &mut impl LineEdit,
        address: Option<Address>,
        indent_mode: IndentMode,
    ) -> Result<Option<ChangeSet>, LnedError> {
        self.previous_warning = Warning::None;
        if address.is_some_and(|a| a.end() > self.buffer.len()) {
            return Err(LnedError::InvalidAddress);
        }
        let indent = match indent_mode {
            IndentMode::Auto => self.buffer[address.map_or_else(
                || cmp::max(self.buffer.current_line(), 1),
                |a| a.end(),
            )..]
                .iter()
                .find(|l| l.contains(|c: char| !c.is_whitespace()))
                .and_then(|l| command::INDENT.captures(l))
                .and_then(|c| c.get(1))
                .map(|m| m.as_str().to_owned()),
            IndentMode::Raw => None,
        };
        let mut lines = Vec::new();
        Cmd::read_input_lines(input, &mut lines, indent)
            .map_err(|source| LnedError::ReadLines { source })?;
        Ok(self.buffer.do_insert(address, lines))
    }

    fn join_cmd(
        &mut self,
        address: Option<Address>,
        separator: Option<&str>,
    ) -> Result<Option<ChangeSet>, LnedError> {
        self.previous_warning = Warning::None;
        match address {
            None if self.buffer.current_line() == self.buffer.len() => {
                Err(LnedError::InvalidAddress)
            }
            Some(a) if a.line_count() == 1 && a.end() == self.buffer.len() => {
                Err(LnedError::InvalidAddress)
            }
            _ => Ok(Some(self.buffer.do_join(address, separator))),
        }
    }

    fn line_number_cmd(
        &mut self,
        output: &mut impl Write,
        address: Option<Address>,
    ) -> Option<ChangeSet> {
        self.previous_warning = Warning::None;
        match address {
            None => {
                writeln!(output, "{}", self.buffer.len()).unwrap();
            }
            Some(address) => {
                writeln!(output, "{}", address.end()).unwrap();
            }
        }
        None
    }

    /// Implements quit command.
    ///
    /// Displays warning and doesn't actually exit if unwritten
    /// buffer changes are detected.
    fn quit_cmd(&mut self) -> Result<Option<ChangeSet>, LnedError> {
        match self.previous_warning {
            Warning::QuitUnsaved => Ok(None),
            _ if !self.buffer.is_dirty() => Ok(None),
            _ => {
                self.previous_warning = Warning::QuitUnsaved;
                Err(LnedError::QuitUnwrittenChanges)
            }
        }
    }
}

#[derive(Debug, PartialEq, Copy, Clone)]
enum IndentMode {
    Auto,
    Raw,
}

/// Main event loop.
///
/// Handles prompting, command input, command dispatch, and error display.
pub fn run(
    mut input: impl LineEdit,
    mut output: impl Write,
    args: &cli::CmdArgs,
) -> Result<(), LnedError> {
    let mut editor = Editor::new();

    if let Some(file) = &args.file
        && let Err(e) = editor.edit_cmd(&mut output, Some(file))
    {
        writeln!(output, "{e}").unwrap();
    }

    // Accept and process commands until fatal error or exit
    let mut done = false;
    while !done {
        Cmd::read(&mut input, &mut editor.buffer, &mut editor.previous_pattern)
            .map_err(LnedError::ParseCmd)
            .and_then(|res| match res {
                Some((cmd, sfx)) => {
                    let res =
                        editor.dispatch_cmd(&cmd, &mut output, &mut input);
                    res.and_then(|exit| {
                        done = exit;
                        if let Some(attrs) = sfx {
                            let cur_line_addr =
                                Address::line(editor.buffer.current_line());
                            print_lines(
                                &mut output,
                                &editor.buffer,
                                cur_line_addr,
                                attrs,
                                None,
                            )?;
                        }
                        Ok(())
                    })
                }
                _ => Ok(()),
            })
            .or_else(|e| {
                writeln!(output, "{e}").unwrap();
                write_backtrace(&mut output, &e);
                output.flush().unwrap();
                Ok(())
            })?;
    }
    Ok(())
}

fn write_backtrace(output: &mut impl Write, mut err: &dyn std::error::Error) {
    if err.source().is_none() {
        return;
    }
    writeln!(output, "\nCaused by:").unwrap();
    let mut n = 0;
    while let Some(source) = err.source() {
        writeln!(output, "  {n}: {source}").unwrap();
        err = source;
        n += 1;
    }
}

#[derive(Debug, Copy, Clone)]
struct ScrollWindow {
    cols: usize,
    rows: usize,
}

fn adjust_global_list(list: &mut VecDeque<usize>, change: &Change) {
    match change {
        Change::Remove(start, lines) => {
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
        Change::Add(start, lines) => {
            for n in list.iter_mut().filter(|n| **n > *start) {
                *n += lines.len();
            }
        }
        Change::SetEol(..) => (), // SetEol doesn't change list
    }
}

/// Prints the addressed lines to ouput, applying the
/// specified print attributes. If a window is specified,
/// printing is stopped after the window has been filled.
/// Since a single line may exceed the window size, output
/// will overrun the window if the final printed line is
/// longer than the specified window width.
///
/// Returns the last line number printed.
fn print_lines(
    output: &mut impl Write,
    buffer: &EditBuffer,
    address: Address,
    attributes: PrintAttributes,
    window: Option<&ScrollWindow>,
) -> Result<usize, LnedError> {
    if address.start() < 1
        || address.start() > buffer.len()
        || address.start() > address.end()
    {
        return Err(LnedError::InvalidAddress);
    }

    let ln_num_cols =
        usize::try_from(1 + buffer.len().checked_ilog10().unwrap_or_default())
            .unwrap();
    let mut rows = 0;

    for (n, l) in
        (address.into_iter()).zip(&buffer[RangeInclusive::from(address)])
    {
        if attributes.enumerate {
            write!(output, "{n:>ln_num_cols$}  ").expect("reliable stdout");
        }
        let graphs = l.graphemes(true).map(|gr| {
            if attributes.expand_escapes { expand_escapes(gr) } else { gr }
        });
        let mut cols = 0;
        for gr in graphs {
            cols += if gr == "\t" {
                let gr_width = 8 - (cols % 8);
                write!(output, "{}", &"        "[..gr_width])
                    .expect("reliable stdout");
                gr_width
            } else {
                use unicode_width::UnicodeWidthStr;
                write!(output, "{gr}").expect("reliable stdout");
                gr.width()
            };
        }

        if let Some(window) = window {
            let rows_printed = cols.div_ceil(window.cols);
            if window.rows - rows <= rows_printed {
                return Ok(n);
            }
            rows += rows_printed;
        }
    }
    Ok(address.end())
}

fn expand_escapes(s: &str) -> &str {
    match s {
        "\t" => "\\t",
        "$" => "\\$",
        "\r" => "\\r",
        "\n" => "\\n$\n",
        "\r\n" => "\\r\\n$\r\n",
        s => s,
    }
}

fn read_lines(
    source: &mut impl BufRead,
    lines: &mut Vec<String>,
) -> Result<usize, LnedError> {
    let mut line = String::new();
    let mut bytes_read = 0;
    loop {
        let len = source
            .read_line(&mut line)
            .map_err(|source| LnedError::ReadLines { source })?;
        if len == 0 {
            break;
        }
        bytes_read += len;
        line.shrink_to_fit();
        lines.push(line);
        line = String::new();
    }

    Ok(bytes_read)
}

trait FileWrite {
    fn write(
        &mut self,
        buffer: &mut EditBuffer,
        span: Option<Address>,
    ) -> io::Result<(usize, usize)>;

    fn backup(&mut self) -> io::Result<()>;
    fn remove_backup(&self) -> io::Result<()>;
    fn name(&self) -> &Path;
    fn backup_name(&self) -> Option<&Path>;
}

#[derive(Debug)]
struct EditedFile {
    filename: PathBuf,
    file: File,
    created: bool,
    backup_filename: Option<PathBuf>,
    backup: Option<File>,
}

impl EditedFile {
    fn open_or_create(filename: &Path) -> Result<EditedFile, LnedError> {
        match OpenOptions::new().read(true).write(true).open(filename) {
            Ok(file) => {
                let mut backup_filename = filename.to_path_buf();
                backup_filename.as_mut_os_string().push(".bak");
                let backup = File::create_new(backup_filename.as_path())
                    .map_err(|source| LnedError::WriteBackupFileCreate {
                        source,
                        filename: filename.to_path_buf(),
                        backup_filename: Some(backup_filename.clone()),
                    })?;
                Ok(EditedFile {
                    filename: filename.to_path_buf(),
                    file,
                    created: false,
                    backup_filename: Some(backup_filename),
                    backup: Some(backup),
                })
            }
            Err(source) => {
                if source.kind() == io::ErrorKind::NotFound {
                    let file =
                        File::create_new(filename).map_err(|source| {
                            LnedError::WriteFileOpen {
                                source,
                                filename: filename.to_path_buf(),
                            }
                        })?;
                    return Ok(EditedFile {
                        filename: filename.to_path_buf(),
                        file,
                        created: true,
                        backup_filename: None,
                        backup: None,
                    });
                }
                Err(LnedError::WriteFileOpen {
                    source,
                    filename: filename.to_path_buf(),
                })
            }
        }
    }
}

impl FileWrite for EditedFile {
    fn write(
        &mut self,
        buffer: &mut EditBuffer,
        span: Option<Address>,
    ) -> io::Result<(usize, usize)> {
        self.file.rewind()?;
        let (bytes_written, lines_written) =
            write_lines(&mut self.file, buffer, span)?;
        self.file.set_len(bytes_written.try_into().unwrap())?;
        self.file.sync_all()?;
        Ok((bytes_written, lines_written))
    }

    fn backup(&mut self) -> io::Result<()> {
        if let Some(backup) = &mut self.backup {
            self.file.rewind()?;
            backup.rewind()?;

            let _ = io::copy(&mut self.file, backup)?;
            backup.flush()?;
            backup.sync_all()?;
        }
        Ok(())
    }

    fn remove_backup(&self) -> io::Result<()> {
        if let Some(backup_filename) = &self.backup_filename {
            fs::remove_file(backup_filename)?;
        }
        Ok(())
    }

    fn name(&self) -> &Path {
        self.filename.as_path()
    }

    fn backup_name(&self) -> Option<&Path> {
        self.backup_filename.as_deref()
    }
}

fn version_cmd(output: &mut impl Write) {
    writeln!(output, "{} {}", cli::APP_NAME, cli::APP_VERSION)
        .expect("reliable stdout");
}

fn write_file(
    buffer: &mut EditBuffer,
    output: &mut impl Write,
    address: Option<Address>,
    writer: &mut impl FileWrite,
) -> Result<(), LnedError> {
    writer
        .backup()
        .map_err(|source| LnedError::WriteMakeBackup {
            source,
            filename: writer.name().to_owned(),
            backup_filename: writer.backup_name().map(Path::to_owned),
        })
        .inspect_err(|_| {
            let _ = writer.remove_backup();
        })?;
    let (bytes, lines) = writer.write(buffer, address).map_err(|source| {
        LnedError::WriteFile {
            source,
            filename: writer.name().to_owned(),
            backup_filename: writer.backup_name().map(Path::to_owned),
        }
    })?;

    write!(output, "{lines} lines ({bytes} bytes) written ",)
        .expect("stdout failure is fatal");
    if let Some(eol) = buffer.prevailing_eol() {
        writeln!(output, "[{eol}]").unwrap();
    } else {
        writeln!(output, "[None]").unwrap();
    }

    output.flush().expect("stdout failure is fatal");
    writer.remove_backup().map_err(|source| LnedError::WriteRemoveBackup {
        source,
        backup_filename: writer.backup_name().map(Path::to_path_buf),
    })
}

fn write_lines(
    destination: &mut impl Write,
    buffer: &mut EditBuffer,
    address: Option<Address>,
) -> Result<(usize, usize), io::Error> {
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
                    + destination.write(&line.as_bytes()[bytes_written..])?;
            }
            total_bytes_written += bytes_written;
            lines_written += 1;
        }
    }
    destination.flush()?;

    if full_buffer_write {
        buffer.reset_clean_fingerprint();
    }
    Ok((total_bytes_written, lines_written))
}

#[cfg(test)]
mod tests {
    use super::*;

    use cli::CmdArgs;
    use line_edit::EditorOptions;
    use std::path::PathBuf;
    use std::str;

    use similar_asserts::assert_eq;
    use tempfile::tempdir;

    use crate::eol::Eol;

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

    struct IndentReader {
        input: VecDeque<String>,
    }

    impl<const N: usize> From<&[&str; N]> for IndentReader {
        fn from(value: &[&str; N]) -> Self {
            IndentReader {
                input: value.as_slice().iter().map(|&s| s.to_owned()).collect(),
            }
        }
    }

    impl LineEdit for IndentReader {
        fn read_line(
            &mut self,
            buffer: &mut String,
            options: Option<&EditorOptions>,
        ) -> io::Result<usize> {
            let input = self.input.pop_front().unwrap_or_default();
            if !input.is_empty() {
                if let Some(indent) = options.and_then(|o| o.prefill.as_ref()) {
                    buffer.push_str(indent);
                }
                buffer.push_str(&input);
            }
            Ok(input.len())
        }
    }

    /////
    #[test]
    fn null_cmd_single_line() {
        let mut output = Vec::new();
        let mut editor = Editor::new();
        editor.buffer = EditBuffer::with_text(&["1\n", "2", "3"]);
        editor.buffer.set_current_line(2);
        editor.null_cmd(&mut output, Some(Address::line(1))).unwrap();
        assert_eq!(editor.buffer.current_line(), 1);
        assert_eq!(str::from_utf8(&output[..]).unwrap(), "1\n");
    }

    #[test]
    fn null_cmd_no_addr() {
        let mut output = Vec::new();
        let mut editor = Editor::new();
        editor.buffer = EditBuffer::with_text(&["1\r\n", "2", "3"]);
        editor.buffer.set_current_line(2);
        editor.null_cmd(&mut output, None).unwrap();
        assert_eq!(str::from_utf8(&output[..]).unwrap(), "3\r\n");
        assert_eq!(editor.buffer.current_line(), 3);
    }

    #[test]
    fn null_cmd_no_addr_last_line_gives_error() {
        let mut output = Vec::new();
        let mut editor = Editor::new();
        editor.buffer = EditBuffer::with_text(&["1\r\n", "2", "3"]);
        editor.buffer.set_current_line(3);
        let res =
            editor.null_cmd(&mut output, None).expect_err("invalid address");
        assert!(matches!(res, LnedError::InvalidAddress));
        assert_eq!(editor.buffer.current_line(), 3);
    }

    #[test]
    fn null_cmd_span() {
        let mut output = Vec::new();
        let mut editor = Editor::new();
        editor.buffer =
            EditBuffer::with_text(&["1\r\n", "2", "3", "4", "5", "6"]);
        editor.buffer.set_current_line(5);
        editor.null_cmd(&mut output, Some(Address::span(2, 4))).unwrap();
        let output = str::from_utf8(&output[..]).unwrap();
        assert_eq!(output, "4\r\n");
        assert_eq!(editor.buffer.current_line(), 4);
    }

    #[test]
    fn null_cmd_empty_buffer_gives_error() {
        let mut output = Vec::new();
        let mut editor = Editor::new();
        let res =
            editor.null_cmd(&mut output, None).expect_err("invalid address");
        assert!(matches!(res, LnedError::InvalidAddress));
        let res = editor
            .null_cmd(&mut output, Some(Address::line(0)))
            .expect_err("invalid address");
        assert!(matches!(res, LnedError::InvalidAddress));
    }

    #[test]
    fn enumerate_empty_buffer_error() {
        let mut output = Vec::new();
        let mut editor = Editor::new();
        let res = editor
            .enumerate_cmd(&mut output, None)
            .expect_err("invalid address");
        assert!(matches!(res, LnedError::InvalidAddress));
        let res = editor
            .enumerate_cmd(&mut output, Some(Address::line(1)))
            .expect_err("invalid address");
        assert!(matches!(res, LnedError::InvalidAddress));
    }

    #[test]
    fn enumerate_sm_buffer() {
        let mut output = Vec::new();
        let mut editor = Editor::new();
        editor.buffer = EditBuffer::with_text(&[
            "1\r\n", "2", "3", "4", "5", "6", "7", "8", "9", "10",
        ]);
        editor.buffer.set_current_line(2);
        editor.enumerate_cmd(&mut output, None).unwrap();
        assert_eq!(str::from_utf8(&output[..]).unwrap(), " 2  2\r\n");
    }

    #[test]
    fn enumerate_sets_current_line() {
        let mut output = Vec::new();
        let mut editor = Editor::new();
        editor.buffer = EditBuffer::with_text(&[
            "1\r\n", "2", "3", "4", "5", "6", "7", "8", "9", "10",
        ]);
        editor.buffer.set_current_line(2);

        editor.enumerate_cmd(&mut output, Some(Address::span(6, 9))).unwrap();
    }

    #[test]
    fn enumerate_lg_buffer() {
        let mut output = Vec::new();
        let mut editor = Editor::new();
        editor.buffer = EditBuffer::with_text(&[
            "1\r\n", "2", "3", "4", "5", "6", "7", "8", "9", "10",
        ]);
        let mut input: Vec<u8> = Vec::new();
        for i in 11..=1024 {
            input.extend_from_slice(format!("{i}\r\n").as_bytes());
        }
        input.extend_from_slice(".\n".as_bytes());
        let mut input = &input[..];
        let address = Some(Address::line(editor.buffer.len()));
        editor.append_cmd(&mut input, address, IndentMode::Raw).unwrap();
        editor.buffer.set_current_line(2);
        assert_eq!(1024, editor.buffer.len());
        output.clear();

        editor.enumerate_cmd(&mut output, Some(Address::span(4, 900))).unwrap();
        let expected = b"   4  4\r\n";
        assert_eq!(&expected[..], &output[0..expected.len()]);
        output.clear();

        editor.enumerate_cmd(&mut output, Some(Address::line(999))).unwrap();
        let expected = b" 999  999\r\n";
        assert_eq!(&expected[..], &output[0..expected.len()]);
    }

    #[test]
    fn print_filename_none_set() {
        let mut editor = Editor::new();
        editor.buffer = EditBuffer::with_text(&["1\r\n", "2", "3"]);
        let mut output = Vec::new();
        editor.file_cmd(&mut output, None);
        let expected = "no filename set [CRLF]\n";
        assert_eq!(str::from_utf8(&output[..]).unwrap(), expected);
        assert!(editor.current_file.is_none());
    }

    #[test]
    fn set_filename() {
        let mut editor = Editor::new();
        let new_filename = "a_new_filename.txt";
        editor.buffer = EditBuffer::with_text(&["1\n", "2", "3"]);
        let mut output = Vec::new();
        editor.file_cmd(&mut output, Some(Path::new(new_filename)));
        let expected = format!("{new_filename} [LF]\n");
        assert_eq!(str::from_utf8(&output[..]).unwrap(), &expected);
        assert_eq!(
            Some(Path::new(new_filename.trim())),
            editor.current_file.as_deref()
        );
    }

    #[test]
    fn print_filename() {
        let new_filename = "a_new_filename.txt";
        let mut editor = Editor::new();
        editor.buffer = EditBuffer::with_text(&["1\n", "2", "3"]);
        let mut output = Vec::new();
        editor.file_cmd(&mut output, Some(Path::new(new_filename)));
        assert_eq!(
            Some(Path::new(new_filename)),
            editor.current_file.as_deref()
        );
        output.clear();
        editor.file_cmd(&mut output, None);
        let expected = "a_new_filename.txt [LF]\n";
        assert_eq!(str::from_utf8(&output[..]).unwrap(), expected);
    }

    #[test]
    fn change_filename() {
        let mut editor = Editor::new();
        let orig_filename = "a_filename.md";
        let new_filename = "a_new_filename.txt";
        editor.buffer = EditBuffer::with_text(&["1\n", "2", "3"]);
        let mut output = Vec::new();
        editor.file_cmd(&mut output, Some(Path::new(orig_filename)));
        output.clear();
        editor.file_cmd(&mut output, Some(Path::new(new_filename)));
        let expected = "a_new_filename.txt [LF]\n";
        assert_eq!(str::from_utf8(&output[..]).unwrap(), expected);
        assert_eq!(
            Some(Path::new(new_filename.trim())),
            editor.current_file.as_deref()
        );
    }

    #[test]
    fn global_cmd_no_matches() {
        let mut editor = Editor::new();
        editor.buffer = EditBuffer::with_text(&["one\n", "two", "three"]);
        let mut output = Vec::new();
        let pat = Regex::new("four").unwrap();
        let commands = "p\n".to_owned();
        let res = editor
            .global_cmd(&mut output, None, &pat, &commands)
            .expect("no error");
        assert!(res.is_none(), "should be no changes");
        assert!(output.is_empty());
    }

    #[test]
    fn global_cmd_illegal_nested_gobal() {
        let mut editor = Editor::new();
        editor.buffer = EditBuffer::with_text(&["one\r\n", "two", "three"]);
        editor.buffer.set_current_line(1);
        let mut output = Vec::new();
        let pat = Regex::new("t..").unwrap();
        let commands = "1,2g/ee/n\n".to_owned();
        let res = editor.global_cmd(&mut output, None, &pat, &commands);
        assert!(matches!(res, Err(LnedError::NestedGlobalCmd)));
    }

    #[test]
    fn global_cmd_blank_command_print() {
        let mut editor = Editor::new();
        editor.buffer =
            EditBuffer::with_text(&["one\r\n", "two", "three", "tweedle dee"]);
        editor.buffer.set_current_line(3);
        let mut output = Vec::new();
        let pat = Regex::new("t..").unwrap();
        let commands = "\n".to_owned();
        let res = editor
            .global_cmd(&mut output, Some(Address::span(1, 3)), &pat, &commands)
            .unwrap();
        assert!(res.is_none(), "should be no changes");
        assert_eq!(str::from_utf8(&output[..]).unwrap(), "two\r\nthree\r\n");
    }

    #[test]
    fn global_cmd_print() {
        let mut editor = Editor::new();
        editor.buffer = EditBuffer::with_text(&["one\n", "two", "three"]);
        editor.buffer.set_current_line(1);
        let mut output = Vec::new();
        let pat = Regex::new("t..").unwrap();
        let commands = "p\r\n".to_owned();
        let res = editor
            .global_cmd(&mut output, None, &pat, &commands)
            .expect("no errors");
        assert!(res.is_none(), "should be no changes");
        assert_eq!(str::from_utf8(&output[..]).unwrap(), "two\nthree\n");
    }

    #[test]
    fn global_cmd_enumerate() {
        let mut editor = Editor::new();
        editor.buffer = EditBuffer::with_text(&["one\n", "two", "three"]);
        editor.buffer.set_current_line(1);
        let mut output = Vec::new();
        let pat = Regex::new("t..").unwrap();
        let commands = "n\r\n".to_owned();
        let res = editor
            .global_cmd(&mut output, Some(Address::span(1, 3)), &pat, &commands)
            .expect("no error");
        assert!(res.is_none(), "should be no changes");
        assert_eq!(str::from_utf8(&output[..]).unwrap(), "2  two\n3  three\n");
    }

    #[test]
    fn global_cmd_enumerate_with_addresses() {
        let mut editor = Editor::new();
        editor.buffer = EditBuffer::with_text(&[
            "one\n", "two", "three", "four", "five", "six",
        ]);
        editor.buffer.set_current_line(6);
        let mut output = Vec::new();
        let pat = Regex::new("e$").unwrap();
        let commands = "-1,.n\r\n".to_owned();
        let res = editor
            .global_cmd(&mut output, Some(Address::span(2, 5)), &pat, &commands)
            .expect("no error");
        assert!(res.is_none(), "should be no changes");
        assert_eq!(
            str::from_utf8(&output[..]).unwrap(),
            "2  two\n3  three\n4  four\n5  five\n"
        );
    }

    #[test]
    fn global_cmd_list() {
        let mut editor = Editor::new();
        editor.buffer = EditBuffer::with_text(&["one\n", "two", "three"]);
        editor.buffer.set_current_line(1);
        let mut output = Vec::new();
        let pat = Regex::new("t..").unwrap();
        let commands = "l\r\n".to_owned();
        let res = editor
            .global_cmd(&mut output, Some(Address::span(1, 3)), &pat, &commands)
            .expect("no error");
        assert!(res.is_none(), "should be no changes");
        assert_eq!(
            str::from_utf8(&output[..]).unwrap(),
            "two\\n$\nthree\\n$\n"
        );
    }

    #[test]
    fn global_cmd_list_with_addresses() {
        let mut editor = Editor::new();
        editor.buffer = EditBuffer::with_text(&[
            "one\n", "two", "three", "four", "five", "six",
        ]);
        editor.buffer.set_current_line(6);
        let mut output = Vec::new();
        let pat = Regex::new("e$").unwrap();
        let commands = "-1,.l\r\n".to_owned();
        let res = editor
            .global_cmd(&mut output, Some(Address::span(2, 5)), &pat, &commands)
            .expect("no error");
        assert!(res.is_none(), "should be no changes");
        assert_eq!(
            str::from_utf8(&output[..]).unwrap(),
            "two\\n$\nthree\\n$\nfour\\n$\nfive\\n$\n"
        );
    }

    #[test]
    fn global_cmd_append() {
        let mut editor = Editor::new();
        editor.buffer = EditBuffer::with_text(&[
            "one\n", "two", "three", "four", "five", "six",
        ]);
        let orig = editor.buffer.clone();
        let expected = EditBuffer::with_text(&[
            "one\n", "append", "two", "three", "append", "four", "five",
            "append", "six",
        ]);
        let mut output = Vec::new();
        let pat = Regex::new("e$").unwrap();
        let commands = "a\nappend\n.\n".to_owned();
        let changes = editor
            .global_cmd(&mut output, Some(Address::span(1, 6)), &pat, &commands)
            .expect("no error")
            .expect("some changes");
        assert!(!changes.is_empty());
        assert_eq!(&editor.buffer[..], &expected[..]);
        assert_eq!(editor.buffer.current_line(), 8);
        editor.buffer.push_undo(changes);

        // now undo
        editor.buffer.do_undo().expect("something there to undo");
        assert_eq!(&editor.buffer[..], &orig[..]);
        assert_eq!(editor.buffer.current_line(), orig.current_line());

        // redo
        editor.buffer.do_redo().expect("something there to undo");
        assert_eq!(&editor.buffer[..], &expected[..]);
        assert_eq!(editor.buffer.current_line(), 8);
    }

    #[test]
    fn global_cmd_change() {
        let mut editor = Editor::new();
        editor.buffer = EditBuffer::with_text(&[
            "one\n", "one", "two", "two", "three", "three", "four", "four",
            "five", "five", "six", "six",
        ]);
        let orig = editor.buffer.clone();
        let expected = EditBuffer::with_text(&[
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
        let pat = Regex::new("([a-z]*e)$").unwrap();
        let commands = ".,+c\nchange 1\nchange 2\nchange 3\n.\n".to_owned();
        let Ok(Some(changes)) = editor.global_cmd(
            &mut output,
            Some(Address::span(1, 6)),
            &pat,
            &commands,
        ) else {
            panic!("global_cmd's err return wasn't None!")
        };
        assert!(!changes.is_empty());
        editor.buffer.push_undo(changes);
        assert_eq!(&editor.buffer[..], &expected[..]);
        assert_eq!(editor.buffer.current_line(), 8);

        // now undo
        editor.buffer.do_undo().expect("something there to undo");
        assert_eq!(&editor.buffer[..], &orig[..]);
        assert_eq!(editor.buffer.current_line(), orig.current_line());

        // redo
        editor.buffer.do_redo().expect("something there to undo");
        assert_eq!(&editor.buffer[..], &expected[..]);
        assert_eq!(editor.buffer.current_line(), 8);
    }

    #[test]
    fn global_cmd_delete() {
        let mut editor = Editor::new();
        editor.buffer = EditBuffer::with_text(&[
            "one\n", "two", "three", "four", "five", "six",
        ]);
        let orig = editor.buffer.clone();
        let expected = EditBuffer::with_text(&["two\n", "four", "six"]);
        let mut output = Vec::new();
        let pat = Regex::new("e$").unwrap();
        let commands = "dn\n".to_owned();
        let Ok(Some(changes)) = editor.global_cmd(
            &mut output,
            Some(Address::span(1, 6)),
            &pat,
            &commands,
        ) else {
            panic!("global_cmd err return wasn't None!");
        };
        assert!(!changes.is_empty());
        editor.buffer.push_undo(changes);
        assert_eq!(
            str::from_utf8(&output[..]).unwrap(),
            "1  two\n2  four\n3  six\n"
        );
        assert_eq!(&editor.buffer[..], &expected[..]);
        assert_eq!(editor.buffer.current_line(), 3);

        // now undo
        editor.buffer.do_undo().expect("something there to undo");
        assert_eq!(&editor.buffer[..], &orig[..]);
        assert_eq!(editor.buffer.current_line(), orig.current_line());

        // redo
        editor.buffer.do_redo().expect("something there to undo");
        assert_eq!(&editor.buffer[..], &expected[..]);
        assert_eq!(editor.buffer.current_line(), 3);
    }

    #[test]
    fn global_cmd_insert() {
        let mut editor = Editor::new();
        editor.buffer = EditBuffer::with_text(&[
            "one\r\n", "two", "three", "four", "five", "six",
        ]);
        let orig = editor.buffer.clone();
        let expected = EditBuffer::with_text(&[
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
        let pat = Regex::new("e$").unwrap();
        let commands = "i\r\ninsert\r\n.\r\n".to_owned();
        let Ok(Some(changes)) = editor.global_cmd(
            &mut output,
            Some(Address::span(1, 6)),
            &pat,
            &commands,
        ) else {
            panic!("global_cmd returned an unexpected error!");
        };
        assert!(!changes.is_empty());
        editor.buffer.push_undo(changes);
        assert_eq!(&editor.buffer[..], &expected[..]);
        assert_eq!(editor.buffer.current_line(), 7);

        // now undo
        editor.buffer.do_undo().expect("something there to undo");
        assert_eq!(&editor.buffer[..], &orig[..]);
        assert_eq!(editor.buffer.current_line(), orig.current_line());

        // redo
        editor.buffer.do_redo().expect("something there to undo");
        assert_eq!(&editor.buffer[..], &expected[..]);
        assert_eq!(editor.buffer.current_line(), 7);
    }

    #[test]
    fn global_cmd_join() {
        let mut editor = Editor::new();
        editor.buffer = EditBuffer::with_text(&[
            "one\n", "two", "three", "four", "five", "six",
        ]);
        let orig = editor.buffer.clone();
        let mut expected =
            EditBuffer::with_text(&["onetwo\n", "threefour", "fivesix"]);
        expected.set_current_line(3);
        let mut output = Vec::new();
        let pat = Regex::new("e$").unwrap();
        let commands = "jn\n".to_owned();
        let res = editor.global_cmd(
            &mut output,
            Some(Address::span(1, 6)),
            &pat,
            &commands,
        );
        let changes = match res {
            Err(e) => panic!("unexpected error {e:?}"),
            Ok(None) => panic!("should have returned Some(ChangeSet)"),
            Ok(Some(changes)) => changes,
        };
        assert!(!changes.is_empty());
        editor.buffer.push_undo(changes);
        assert_eq!(
            str::from_utf8(&output[..]).unwrap(),
            "1  onetwo\n2  threefour\n3  fivesix\n"
        );
        assert_eq!(&editor.buffer[..], &expected[..]);
        assert_eq!(editor.buffer.current_line(), 3);

        // now undo
        editor.buffer.do_undo().expect("something there to undo");
        assert_eq!(&editor.buffer[..], &orig[..]);
        assert_eq!(editor.buffer.current_line(), orig.current_line());

        // redo
        editor.buffer.do_redo().expect("something there to undo");
        assert_eq!(&editor.buffer[..], &expected[..]);
        assert_eq!(editor.buffer.current_line(), 3);
    }

    #[test]
    fn global_cmd_move() {
        let mut editor = Editor::new();
        editor.buffer = EditBuffer::with_text(&[
            "one\r\n", "two", "three", "four", "five", "six",
        ]);
        let orig = editor.buffer.clone();
        let mut expected = EditBuffer::with_text(&[
            "three\r\n",
            "two",
            "one",
            "four",
            "five",
            "six",
        ]);
        expected.set_current_line(1);
        let mut output = Vec::new();
        let pat = Regex::new("^t").unwrap();
        let commands = "m0\r\n".to_owned();
        let Some(changes) = editor
            .global_cmd(&mut output, Some(Address::span(1, 6)), &pat, &commands)
            .expect("should have been Ok!")
        else {
            panic!("should have been Some(changes)!");
        };
        assert!(!changes.is_empty());
        editor.buffer.push_undo(changes);
        assert_eq!(&editor.buffer[..], &expected[..]);
        assert_eq!(editor.buffer.current_line(), expected.current_line());

        // now undo
        editor.buffer.do_undo().expect("something there to undo");
        assert_eq!(&editor.buffer[..], &orig[..]);
        assert_eq!(editor.buffer.current_line(), orig.current_line());

        // redo
        editor.buffer.do_redo().expect("something there to undo");
        assert_eq!(&editor.buffer[..], &expected[..]);
        assert_eq!(editor.buffer.current_line(), expected.current_line());
    }

    #[test]
    fn global_cmd_move_with_overlap() {
        let mut editor = Editor::new();
        editor.buffer = EditBuffer::with_text(&[
            "one\r\n", "two", "three", "four", "five", "six",
        ]);
        let orig = editor.buffer.clone();
        let mut expected = EditBuffer::with_text(&[
            "two\r\n", "three", "one", "four", "five", "six",
        ]);
        expected.set_current_line(2);
        let mut output = Vec::new();
        let pat = Regex::new("^t").unwrap();
        let commands = ".,+m0\r\n".to_owned();
        let Some(changes) = editor
            .global_cmd(&mut output, Some(Address::span(1, 6)), &pat, &commands)
            .expect("should have been Ok!")
        else {
            panic!("should have been Some(changes)!");
        };
        assert!(!changes.is_empty());
        editor.buffer.push_undo(changes);
        assert_eq!(&editor.buffer[..], &expected[..]);
        assert_eq!(editor.buffer.current_line(), expected.current_line());

        // now undo
        editor.buffer.do_undo().expect("something there to undo");
        assert_eq!(&editor.buffer[..], &orig[..]);
        assert_eq!(editor.buffer.current_line(), orig.current_line());

        // redo
        editor.buffer.do_redo().expect("something there to undo");
        assert_eq!(&editor.buffer[..], &expected[..]);
        assert_eq!(editor.buffer.current_line(), expected.current_line());
    }

    #[test]
    fn global_cmd_substitute_with_error() {
        let mut editor = Editor::new();
        editor.buffer = EditBuffer::with_text(&[
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
        editor.buffer.set_current_line(5);
        let before = editor.buffer.clone();
        let mut expected = EditBuffer::with_text(&[
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
        let pat = Regex::new("s[aeiou]").unwrap();
        let commands = ".,+2s//\\\n'/n".to_string();
        let Err(LnedError::GlobalCmdErrorStop { source, changes }) =
            editor.global_cmd(&mut output, None, &pat, &commands)
        else {
            panic!("should have returned GlobalCmdErrorStop");
        };
        assert!(matches!(
            *source,
            LnedError::ReadGlobalCmd {
                source: command::Error::AddressTooLarge
            }
        ));
        let Some(changes) = changes else {
            panic!("changes was None!");
        };
        let output = str::from_utf8(&output[..]).unwrap();
        assert_eq!(output, expected_output);
        editor.buffer.push_undo(changes);
        assert_eq!(editor.buffer.current_line(), expected.current_line());
        assert_eq!(&editor.buffer[..], &expected[..]);
        editor.buffer.do_undo().unwrap();
        assert_eq!(editor.buffer.current_line(), before.current_line());
        assert_eq!(&before[..], &editor.buffer[..]);
        editor.buffer.do_redo().unwrap();
        assert_eq!(editor.buffer.current_line(), expected.current_line());
        assert_eq!(&editor.buffer[..], &expected[..]);
    }

    #[test]
    fn global_cmd_substitute() {
        let mut editor = Editor::new();
        editor.buffer = EditBuffer::with_text(&[
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
        editor.buffer.set_current_line(5);
        let before = editor.buffer.clone();
        let mut expected = EditBuffer::with_text(&[
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
        let pat = Regex::new("s[aeiou]").unwrap();
        let commands = "s//\\\n'/n".to_string();
        let Some(changes) = editor
            .global_cmd(&mut output, None, &pat, &commands)
            .expect("should have been Ok")
        else {
            panic!("should have been Some(changes)!");
        };
        let output = str::from_utf8(&output[..]).unwrap();
        assert_eq!(output, expected_output);
        assert!(!changes.is_empty());
        editor.buffer.push_undo(changes);
        assert_eq!(editor.buffer.current_line(), expected.current_line());
        assert_eq!(&editor.buffer[..], &expected[..]);
        editor.buffer.do_undo().unwrap();
        assert_eq!(editor.buffer.current_line(), before.current_line());
        assert_eq!(&before[..], &editor.buffer[..]);
        editor.buffer.do_redo().unwrap();
        assert_eq!(editor.buffer.current_line(), expected.current_line());
        assert_eq!(&editor.buffer[..], &expected[..]);
    }

    #[test]
    fn global_cmd_transfer() {
        let mut editor = Editor::new();
        editor.buffer = EditBuffer::with_text(&[
            "one\r\n", "two", "three", "four", "five", "six",
        ]);
        let orig = editor.buffer.clone();
        let expected = EditBuffer::with_text(&[
            "one\r\n", "two", "three", "four", "five", "six", "one", "three",
            "five",
        ]);
        let mut output = Vec::new();
        let pat = Regex::new("e$").unwrap();
        let commands = "t$\r\n".to_owned();
        let Some(changes) = editor
            .global_cmd(&mut output, Some(Address::span(1, 6)), &pat, &commands)
            .expect("should have been Ok!")
        else {
            panic!("should have been Some(changes)!");
        };
        assert!(!changes.is_empty());
        editor.buffer.push_undo(changes);
        assert_eq!(&editor.buffer[..], &expected[..]);
        assert_eq!(editor.buffer.current_line(), 9);

        // now undo
        editor.buffer.do_undo().expect("something there to undo");
        assert_eq!(&editor.buffer[..], &orig[..]);
        assert_eq!(editor.buffer.current_line(), orig.current_line());

        // redo
        editor.buffer.do_redo().expect("something there to undo");
        assert_eq!(&editor.buffer[..], &expected[..]);
        assert_eq!(editor.buffer.current_line(), 9);
    }

    #[test]
    fn global_cmd_unsupported_commands() {
        let mut editor = Editor::new();
        editor.buffer = EditBuffer::with_text(&["one\r\n", "two", "three"]);
        editor.buffer.set_current_line(1);
        let mut output = Vec::new();
        let pat = Regex::new(r"t..").unwrap();
        let commands = "e filename.txt\n".to_owned();
        let res = editor.global_cmd(
            &mut output,
            Some(Address::span(1, 3)),
            &pat,
            &commands,
        );
        assert!(matches!(res, Err(LnedError::UnsupportedGlobalCmd)));
    }

    #[test]
    fn print_cmd_no_addr() {
        let mut output = Vec::new();
        let mut editor = Editor::new();
        editor.buffer = EditBuffer::with_text(&["1\r\n", "2", "3"]);
        editor.buffer.set_current_line(2);
        editor.print_cmd(&mut output, None).unwrap();
        assert_eq!(&output[..], b"2\r\n");
    }

    #[test]
    fn print_cmd_single_line() {
        let mut output = Vec::new();
        let mut editor = Editor::new();
        editor.buffer = EditBuffer::with_text(&["1\r\n", "2", "3"]);
        editor.buffer.set_current_line(2);
        editor.print_cmd(&mut output, Some(Address::line(3))).unwrap();
        assert_eq!(&output[..], b"3\r\n");
    }

    #[test]
    fn print_cmd_span() {
        let mut output = Vec::new();
        let mut editor = Editor::new();
        editor.buffer =
            EditBuffer::with_text(&["1\r\n", "2", "3", "4", "5", "6"]);
        editor.buffer.set_current_line(5);
        editor.print_cmd(&mut output, Some(Address::span(2, 4))).unwrap();
        assert_eq!(&output[..], b"2\r\n3\r\n4\r\n");
    }

    #[test]
    fn print_cmd_sets_current_line() {
        let mut output = Vec::new();
        let mut editor = Editor::new();
        editor.buffer =
            EditBuffer::with_text(&["1\r\n", "2", "3", "4", "5", "6"]);
        editor.buffer.set_current_line(5);
        editor.print_cmd(&mut output, Some(Address::span(2, 4))).unwrap();
        assert_eq!(4, editor.buffer.current_line());
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
        let mut editor = Editor::new();
        editor.buffer = EditBuffer::new();
        let res =
            editor.print_cmd(&mut output, None).expect_err("invalid address");
        assert!(matches!(res, LnedError::InvalidAddress));
        let res = editor
            .print_cmd(&mut output, Some(Address::line(0)))
            .expect_err("invalid address");
        assert!(matches!(res, LnedError::InvalidAddress));
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
    fn append_raw_cmd_dispatch() {
        let input = b"a\n    one\n    two\n    three\n.\n2A\nappended\n.\n2p\n3p\nq\nq\n";
        let mut output = Vec::new();
        run(&input[..], &mut output, &CmdArgs::default()).unwrap();
        let output = str::from_utf8(&output[..]).unwrap();
        assert!(output.contains("    two\n"));
        assert!(output.contains("\nappended"));
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
        assert!(output.contains("three\\n$\n"));
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
    fn change_raw_cmd_dispatch() {
        let input =
            b"a\n    1\n    2\n    3\n    4\n.\n2,3C\na\nb\n.\n1,$p\nq\nq\n";
        let mut output = Vec::new();
        run(&input[..], &mut output, &CmdArgs::default()).unwrap();
        let output = str::from_utf8(&output[..]).unwrap();
        assert!(output.contains("    1\na\nb\n    4\n"));
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
        let input = b"f\nf new_filename.txt\nq\n";
        let mut output = Vec::new();
        let args = CmdArgs { file: None };
        run(&input[..], &mut output, &args).unwrap();
        let output = str::from_utf8(&output[..]).unwrap();
        assert!(output.contains("no filename set"));
        assert!(output.contains("new_filename.txt"));
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
    fn insert_raw_cmd_dispatch() {
        let input = b"a\n    one\n    two\n    three\n.\n3I\ninserted\n.\n2p\n3p\nq\nq\n";
        let mut output = Vec::new();
        run(&input[..], &mut output, &CmdArgs::default()).unwrap();
        let output = str::from_utf8(&output[..]).unwrap();
        assert!(output.contains("two\n"));
        assert!(output.contains("unwritten changes"));
        assert!(output.contains("\ninserted"));
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
        assert!(output.contains("1\\n$\n2\\n$\n"));
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
    fn newline_cmd_dispatch() {
        let input = b"a\n1\n2\n3\n.\nN\nq\nq\n";
        let mut output = Vec::new();
        run(&input[..], &mut output, &CmdArgs::default()).unwrap();
        let output = str::from_utf8(&output[..]).unwrap();
        assert!(output.contains("prevailing newline: LF"));
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
    fn version_cmd_dispatch() {
        let input = b"#\nq";
        let mut output = Vec::new();
        run(&input[..], &mut output, &CmdArgs::default()).unwrap();
        let output = str::from_utf8(&output[..]).unwrap();
        assert!(output.contains(cli::APP_VERSION));
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
        assert!(output.contains("address too large"));
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
        let mut editor = Editor::new();
        editor.buffer = EditBuffer::with_text(&[
            "one two three four\r\n",
            "five six seven eight",
            "nine ten eleven twelve",
            "thirteen fourteen fifteen sixteen",
            "seventeen eighteen nineteen twenty",
        ]);
        editor.buffer.set_current_line(5);
        let res = editor
            .substitute_cmd(
                Some(Address::span(1, 5)),
                &Regex::new("won't match").unwrap(),
                "",
                SubstitutionScope::Global,
            )
            .expect_err("should give error");
        assert!(matches!(res, LnedError::NoMatch));
    }

    #[test]
    fn substitute_cmd_current_line_global() {
        let mut editor = Editor::new();
        editor.buffer = EditBuffer::with_text(&[
            "one two three four\r\n",
            "five six seven eight",
            "nine ten eleven twelve",
            "thirteen fourteen fifteen sixteen",
            "seventeen eighteen nineteen twenty",
        ]);
        editor.buffer.set_current_line(5);
        editor
            .substitute_cmd(
                None,
                &Regex::new("e+n").unwrap(),
                "'",
                SubstitutionScope::Global,
            )
            .unwrap();
        assert_eq!(editor.buffer[5], "sev't' eight' ninet' tw'ty\r\n");
    }

    #[test]
    fn substitute_cmd_current_line_at_eol() {
        let mut editor = Editor::new();
        editor.buffer = EditBuffer::with_text(&["some text\n"]);
        let expected = EditBuffer::with_text(&["some text!\n"]);
        editor
            .substitute_cmd(
                None,
                &Regex::new("$").unwrap(),
                "!",
                SubstitutionScope::Single(1),
            )
            .unwrap();
        assert_eq!(&editor.buffer[..], &expected[..]);
    }

    #[test]
    fn substitute_cmd_current_line_single_first() {
        let mut editor = Editor::new();
        editor.buffer = EditBuffer::with_text(&[
            "one two three four\r\n",
            "five six seven eight",
            "nine ten eleven twelve",
            "thirteen fourteen fifteen sixteen",
            "seventeen eighteen nineteen twenty",
        ]);
        editor.buffer.set_current_line(5);
        editor
            .substitute_cmd(
                None,
                &Regex::new("e+n").unwrap(),
                "'",
                SubstitutionScope::Single(1),
            )
            .unwrap();
        assert_eq!(editor.buffer[5], "sev'teen eighteen nineteen twenty\r\n");
    }

    #[test]
    fn substitute_cmd_current_line_single() {
        let mut editor = Editor::new();
        editor.buffer = EditBuffer::with_text(&[
            "one two three four\r\n",
            "five six seven eight",
            "nine ten eleven twelve",
            "thirteen fourteen fifteen sixteen",
            "seventeen eighteen nineteen twenty",
        ]);
        editor.buffer.set_current_line(5);
        editor
            .substitute_cmd(
                None,
                &Regex::new("e+n").unwrap(),
                "'",
                SubstitutionScope::Single(4),
            )
            .unwrap();
        assert_eq!(editor.buffer[5], "seventeen eighteen ninet' twenty\r\n");
    }

    #[test]
    fn substitute_split_line() {
        let mut editor = Editor::new();
        editor.buffer = EditBuffer::with_text(&["a line, to split\r\n"]);
        editor.buffer.set_current_line(1);
        let cmd_line = "s/, /\\\r\n/";
        let mut input = cmd_line.as_bytes();
        let Some((Cmd::Substitute(address, pattern, replacement, scope), None)) =
            Cmd::read(&mut input, &mut editor.buffer, &mut None).unwrap()
        else {
            panic!("{cmd_line} didn't parse as Cmd::Substitute");
        };
        editor
            .substitute_cmd(address, &pattern, replacement.as_str(), scope)
            .unwrap();
        let mut expected = EditBuffer::with_text(&["a line\r\n", "to split"]);
        expected.set_current_line(2);
        assert_eq!(editor.buffer, expected);
    }

    #[test]
    fn substitute_split_line_no_end_delimiter() {
        let mut editor = Editor::new();
        editor.buffer = EditBuffer::with_text(&["a line, to split\n"]);
        editor.buffer.set_current_line(1);
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
        editor
            .substitute_cmd(address, &pattern, replacement.as_str(), scope)
            .unwrap();
        let mut expected = EditBuffer::with_text(&["a line\n", "to split"]);
        expected.set_current_line(2);
        assert_eq!(editor.buffer[..], expected[..]);
        assert_eq!(editor.buffer.current_line(), expected.current_line());
    }

    #[test]
    fn substitute_cmd_multi_line_single() {
        let mut editor = Editor::new();
        editor.buffer = EditBuffer::with_text(&[
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
        editor.buffer.set_current_line(5);
        let mut expected = EditBuffer::with_text(&[
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
        editor
            .substitute_cmd(
                Some(Address::span(2, 9)),
                &Regex::new("s[aeiou]").unwrap(),
                "'",
                SubstitutionScope::Single(1),
            )
            .unwrap();
        assert_eq!(editor.buffer.current_line(), expected.current_line());
        assert_eq!(&editor.buffer[..], &expected[..]);
    }

    #[test]
    fn undo_redo_substitute_cmd_multi_line_single() {
        let mut editor = Editor::new();
        editor.buffer = EditBuffer::with_text(&[
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
        editor.buffer.set_current_line(5);
        let before = editor.buffer.clone();
        let mut expected = EditBuffer::with_text(&[
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
        let Some(changes) = editor
            .substitute_cmd(
                Some(Address::span(2, 9)),
                &Regex::new("s[aeiou]").unwrap(),
                "'",
                SubstitutionScope::Single(1),
            )
            .unwrap()
        else {
            panic!("expected Some(ChangeSet)!");
        };
        assert!(!changes.is_empty());
        editor.buffer.push_undo(changes);
        assert_eq!(editor.buffer.current_line(), expected.current_line());
        assert_eq!(&editor.buffer[..], &expected[..]);
        editor.buffer.do_undo().unwrap();
        assert_eq!(editor.buffer.current_line(), before.current_line());
        assert_eq!(&before[..], &editor.buffer[..]);
        editor.buffer.do_redo().unwrap();
        assert_eq!(editor.buffer.current_line(), expected.current_line());
        assert_eq!(&editor.buffer[..], &expected[..]);
    }

    #[test]
    fn substitute_cmd_multi_line_single_first() {
        let mut editor = Editor::new();
        editor.buffer = EditBuffer::with_text(&[
            "one two three four\r\n",
            "five six seven eight",
            "nine ten eleven twelve",
            "thirteen fourteen fifteen sixteen",
            "seventeen eighteen nineteen twenty",
        ]);
        editor.buffer.set_current_line(5);
        editor
            .substitute_cmd(
                Some(Address::span(2, 3)),
                &Regex::new("e+n").unwrap(),
                "'",
                SubstitutionScope::Single(1),
            )
            .unwrap();
        assert_eq!(
            editor.buffer[2..4],
            ["five six sev' eight\r\n", "nine t' eleven twelve\r\n"]
        );
    }

    #[test]
    fn substitute_cmd_multi_line_capture() {
        let mut editor = Editor::new();
        editor.buffer = EditBuffer::with_text(&[
            "one two three four\r\n",
            "five six seven eight",
            "nine ten eleven twelve",
            "thirteen fourteen fifteen sixteen",
            "seventeen eighteen nineteen twenty",
        ]);
        editor.buffer.set_current_line(5);
        editor
            .substitute_cmd(
                Some(Address::span(2, 4)),
                &Regex::new("[a-z]+?(e+n)[^ ]*").unwrap(),
                "$1 ($0)",
                SubstitutionScope::Single(2),
            )
            .unwrap();
        assert_eq!(
            editor.buffer[2..5],
            [
                "five six seven eight\r\n",
                "nine ten en (eleven) twelve\r\n",
                "thirteen een (fourteen) fifteen sixteen\r\n"
            ]
        );
    }

    #[test]
    fn undo_redo_substitute_cmd_multi_line_capture() {
        let mut editor = Editor::new();
        editor.buffer = EditBuffer::with_text(&[
            "one two three four\r\n",
            "five six seven eight",
            "nine ten eleven twelve",
            "thirteen fourteen fifteen sixteen",
            "seventeen eighteen nineteen twenty",
        ]);
        editor.buffer.set_current_line(5);
        let before = editor.buffer.clone();
        let Ok(Some(changes)) = editor.substitute_cmd(
            Some(Address::span(2, 4)),
            &Regex::new("[a-z]+?(e+n)[^ ]*").unwrap(),
            "$1 ($0)",
            SubstitutionScope::Single(2),
        ) else {
            panic!("expected Ok(Some(ChangeSet))!");
        };
        assert!(!changes.is_empty());
        editor.buffer.push_undo(changes);
        assert_eq!(
            editor.buffer[2..5],
            [
                "five six seven eight\r\n",
                "nine ten en (eleven) twelve\r\n",
                "thirteen een (fourteen) fifteen sixteen\r\n"
            ]
        );
        let after = editor.buffer.clone();

        editor.buffer.do_undo().unwrap();
        assert_eq!(&editor.buffer[..], &before[..]);

        editor.buffer.do_redo().unwrap();
        assert_eq!(&editor.buffer[..], &after[..]);
    }

    #[test]
    fn transfer_cmd_destination_invalid() {
        let mut editor = Editor::new();
        editor.buffer =
            EditBuffer::with_text(&["1\n", "2", "3", "4", "5", "6"]);
        let source = Address::span(3, 5);
        let destination = Address::line(7);
        let res = editor
            .transfer_cmd(Some(source), destination)
            .expect_err("should fail");
        assert!(matches!(res, LnedError::InvalidAddress));
    }

    #[test]
    fn transfer_cmd_destination_intersects_source_give_error() {
        let mut editor = Editor::new();
        editor.buffer =
            EditBuffer::with_text(&["1\n", "2", "3", "4", "5", "6"]);
        let source = Address::span(3, 5);
        let destination = Address::line(4);
        let res = editor
            .transfer_cmd(Some(source), destination)
            .expect_err("should fail");
        assert!(matches!(res, LnedError::DestinationIntersectsSource));
    }

    #[test]
    fn write_propegates_errors() {
        let mut editor = Editor::new();
        editor.buffer = EditBuffer::with_text(&["1\r\n", "2", "3"]);
        let mut dummy_file = BadWriter {};
        write_lines(
            &mut dummy_file,
            &mut editor.buffer,
            Some(Address::span(1, 2)),
        )
        .expect_err("io error");
    }

    #[test]
    fn write_one_line() {
        let mut editor = Editor::new();
        editor.buffer = EditBuffer::with_text(&["1\n", "2", "3"]);
        let mut dummy_file = Vec::new();
        let (bytes, lines) = write_lines(
            &mut dummy_file,
            &mut editor.buffer,
            Some(Address::line(2)),
        )
        .unwrap();
        assert_eq!(bytes, 2);
        assert_eq!(lines, 1);
    }

    #[test]
    fn write_many_lines() {
        let mut editor = Editor::new();
        editor.buffer =
            EditBuffer::with_text(&["1\r\n", "2", "3", "4", "5", "6"]);
        let mut dummy_file = Vec::new();
        let (bytes, lines) = write_lines(
            &mut dummy_file,
            &mut editor.buffer,
            Some(Address::span(1, 6)),
        )
        .unwrap();
        assert_eq!(bytes, 18);
        assert_eq!(lines, 6);
    }

    #[test]
    fn write_empty_buffer() {
        let mut editor = Editor::new();
        editor.buffer = EditBuffer::new();
        let mut dummy_file = Vec::new();
        let (bytes, lines) =
            write_lines(&mut dummy_file, &mut editor.buffer, None).unwrap();
        assert_eq!(bytes, 0);
        assert_eq!(lines, 0);
    }

    #[test]
    fn write_no_addr_leaves_clean_buffer() {
        let mut editor = Editor::new();
        editor.buffer = EditBuffer::with_text(&["1\n", "2", "3"]);
        assert!(!editor.buffer.is_dirty());
        let mut input = "one more line\n.\n".as_bytes();
        let Some(change) = editor
            .append_cmd(&mut input, Some(Address::line(0)), IndentMode::Raw)
            .unwrap()
        else {
            panic!("expected Some(ChangeSet) from append_cmd!");
        };
        assert!(!change.is_empty());
        editor.buffer.push_undo(change);
        assert!(editor.buffer.is_dirty());
        let mut dummy_file = Vec::new();
        let (bytes, lines) =
            write_lines(&mut dummy_file, &mut editor.buffer, None).unwrap();
        assert_eq!(bytes, 20);
        assert_eq!(lines, 4);
        assert!(!editor.buffer.is_dirty());
    }

    #[test]
    fn write_full_buffer_leaves_clean_buffer() {
        let mut editor = Editor::new();
        editor.buffer = EditBuffer::with_text(&["1\n", "2", "3"]);
        assert!(!editor.buffer.is_dirty());
        let mut input = "one more line\n.\n".as_bytes();
        let Some(change) = editor
            .append_cmd(&mut input, Some(Address::line(0)), IndentMode::Raw)
            .unwrap()
        else {
            panic!("expected append_cmd to return Some(ChangeSet)!");
        };
        assert!(!change.is_empty());
        editor.buffer.push_undo(change);
        assert!(editor.buffer.is_dirty());
        let mut dummy_file = Vec::new();
        let address = Some(Address::span(1, editor.buffer.len()));
        let (bytes, lines) =
            write_lines(&mut dummy_file, &mut editor.buffer, address).unwrap();
        assert_eq!(bytes, 20);
        assert_eq!(lines, 4);
        assert!(!editor.buffer.is_dirty());
    }

    #[test]
    fn write_partial_buffer_leaves_dirty_buffer() {
        let mut editor = Editor::new();
        editor.buffer = EditBuffer::with_text(&["1\n", "2", "3"]);
        assert!(!editor.buffer.is_dirty());
        let mut input = "one more line\n.\n".as_bytes();
        let Some(change) = editor
            .append_cmd(&mut input, Some(Address::line(0)), IndentMode::Raw)
            .unwrap()
        else {
            panic!("expected Some(ChangeSet) from append_cmd!");
        };
        assert!(!change.is_empty());
        editor.buffer.push_undo(change);
        assert!(editor.buffer.is_dirty());
        let mut dummy_file = Vec::new();
        let (bytes, lines) = write_lines(
            &mut dummy_file,
            &mut editor.buffer,
            Some(Address::span(1, 2)),
        )
        .unwrap();
        assert_eq!(bytes, 16);
        assert_eq!(lines, 2);
        assert!(editor.buffer.is_dirty());
    }

    #[test]
    fn append_cmd_past_end_gives_error_before_input() {
        let mut editor = Editor::new();
        editor.buffer = EditBuffer::new();
        let mut input = "one\n.\n".as_bytes();
        let expected = "one\n.\n".as_bytes();
        let res = editor
            .append_cmd(&mut input, Some(Address::line(2)), IndentMode::Auto)
            .expect_err("invalid addr");
        assert_eq!(0, editor.buffer.len());
        assert_eq!(input, expected);
        assert!(matches!(res, LnedError::InvalidAddress));
    }

    #[test]
    fn append_cmd_auto_indent() {
        let mut editor = Editor::new();
        editor.buffer = EditBuffer::with_text(&["one\n", "    two", "three"]);
        let mut input = IndentReader::from(&["indented\n", "    further\n"]);
        let expected = [
            "one\n",
            "    two\n",
            "    indented\n",
            "        further\n",
            "three\n",
        ];
        let _ = editor
            .append_cmd(&mut input, Some(Address::line(2)), IndentMode::Auto)
            .expect("lines appended");
        assert_eq!(&editor.buffer[..], expected);
    }

    #[test]
    fn insert_cmd_past_end_gives_error_before_input() {
        let mut editor = Editor::new();
        editor.buffer = EditBuffer::new();
        let mut input = "one\n.\n".as_bytes();
        let expected = "one\n.\n".as_bytes();
        let res = editor
            .insert_cmd(&mut input, Some(Address::line(2)), IndentMode::Auto)
            .expect_err("invalid addr");
        assert_eq!(0, editor.buffer.len());
        assert_eq!(input, expected);
        assert!(matches!(res, LnedError::InvalidAddress));
    }

    #[test]
    fn delete_cmd_empty_buffer() {
        let mut editor = Editor::new();
        editor.buffer = EditBuffer::new();
        let res = editor.delete_cmd(None).expect_err("invalid address");
        assert!(matches!(res, LnedError::InvalidAddress));
    }

    #[test]
    fn insert_cmd_auto_indent() {
        let mut editor = Editor::new();
        editor.buffer = EditBuffer::with_text(&["one\n", "    two", "three"]);
        let mut input = IndentReader::from(&["indented\n", "    further\n"]);
        let expected = [
            "one\n",
            "    indented\n",
            "        further\n",
            "    two\n",
            "three\n",
        ];
        let _ = editor
            .insert_cmd(&mut input, Some(Address::line(2)), IndentMode::Auto)
            .expect("lines inserted");
        assert_eq!(&editor.buffer[..], expected);
    }

    #[test]
    fn delete_cmd_line_zero() {
        let mut editor = Editor::new();
        editor.buffer = EditBuffer::with_text(&["1\n", "2", "3"]);
        let res = editor
            .delete_cmd(Some(Address::line(0)))
            .expect_err("invalid address");
        assert!(matches!(res, LnedError::InvalidAddress));
    }

    #[test]
    fn delete_cmd_span_starting_at_zero() {
        let mut editor = Editor::new();
        editor.buffer = EditBuffer::with_text(&["1\n", "2", "3", "4", "5"]);
        let res = editor
            .delete_cmd(Some(Address::span(0, 3)))
            .expect_err("invalid address");
        assert!(matches!(res, LnedError::InvalidAddress));
    }

    #[test]
    fn edit_cmd_no_filename_error() {
        let mut editor = Editor::new();
        let res =
            editor.edit_cmd(&mut Vec::new(), None).expect_err("no filename");
        assert!(matches!(res, LnedError::NoFilename));
    }

    #[test]
    fn edit_cmd_missing_file_clears_buffer_sets_filename() {
        let mut editor = Editor::new();
        editor.buffer = EditBuffer::with_text(&["1\n", "2", "3"]);
        assert_eq!(editor.buffer.len(), 3);
        let mut output = Vec::new();
        let not_a_file = Some(Path::new("non-existant_file.txt"));
        let res =
            editor.edit_cmd(&mut output, not_a_file).expect_err("FileNotFound");
        assert!(matches!(res, LnedError::FileNotFound(_)));
        assert_eq!(editor.current_file.as_deref(), not_a_file);
        assert!(editor.buffer.is_empty());
    }

    #[test]
    fn read_lines_returns_correct_count() {
        let source = b"one\r\ntwo\r\nthree\r\nfour\r\n";
        let source_bytes = source.len();
        let mut lines = Vec::new();
        let byte_count =
            read_lines(&mut &source[..], &mut lines).expect("no error");
        assert_eq!(byte_count, source_bytes);
        assert_eq!(lines.len(), 4);
    }

    #[test]
    fn read_lines_io_error() {
        let mut source = BufReader::new(BadReader {});
        let res =
            read_lines(&mut source, &mut Vec::new()).expect_err("io error");
        assert!(matches!(res, LnedError::ReadLines { .. }));
    }

    #[test]
    fn edit_cmd_reads_file() {
        let mut editor = Editor::new();
        let mut output = Vec::new();
        let filename1 = Some(Path::new(r"test/assets/text_with_final_eol.txt"));
        let filename2 =
            Some(Path::new(r"test/assets/text_with_no_final_eol.txt"));

        editor.edit_cmd(&mut output, filename1).unwrap();
        assert_eq!(editor.buffer.len(), 10);
        let out_text = str::from_utf8(&output[..]).unwrap();
        assert!(
            out_text.contains("10 lines") && out_text.contains("312 bytes")
        );

        output.clear();
        editor.edit_cmd(&mut output, filename2).unwrap();
        assert_eq!(editor.buffer.len(), 10);
        let out_text = str::from_utf8(&output[..]).unwrap();
        assert!(
            out_text.contains("10 lines") && out_text.contains("318 bytes")
        );
    }

    #[test]
    fn change_cmd_addr_starting_after_buffer_end_gives_error() {
        let mut editor = Editor::new();
        editor.buffer = EditBuffer::with_text(&["1\n", "2", "3"]);
        let res = editor
            .change_cmd(
                &mut &b".\n"[..],
                Some(Address::span(5, 6)),
                IndentMode::Auto,
            )
            .expect_err("illegal address");
        assert!(matches!(res, LnedError::InvalidAddress));
    }

    #[test]
    fn change_cmd_addr_ending_past_buffer_end_gives_error() {
        let mut editor = Editor::new();
        editor.buffer = EditBuffer::with_text(&["1\n", "2", "3"]);
        let res = editor
            .change_cmd(
                &mut &b".\n"[..],
                Some(Address::span(2, 4)),
                IndentMode::Auto,
            )
            .expect_err("illegal address");
        assert!(matches!(res, LnedError::InvalidAddress));
    }

    #[test]
    fn change_cmd_auto_indent() {
        let mut editor = Editor::new();
        editor.buffer = EditBuffer::with_text(&[
            "one\n",
            "\n",
            "\n",
            "    two",
            "three",
            "    four",
            "        five",
            "\n",
            "\n",
            "\n",
            "    six",
        ]);
        let mut input = IndentReader::from(&["replacing blanks\n"]);
        let expected = [
            "one\n",
            "\n",
            "\n",
            "    two\n",
            "three\n",
            "    four\n",
            "        five\n",
            "        replacing blanks\n",
            "    six\n",
        ];
        let _ = editor
            .change_cmd(
                &mut input,
                Some(Address::span(8, 10)),
                IndentMode::Auto,
            )
            .expect("blanks replaced");
        assert_eq!(&editor.buffer[..], expected);

        let mut input = IndentReader::from(&["indented\n", "    further\n"]);
        let expected = [
            "one\n",
            "    indented\n",
            "        further\n",
            "    four\n",
            "        five\n",
            "        replacing blanks\n",
            "    six\n",
        ];
        let _ = editor
            .change_cmd(&mut input, Some(Address::span(2, 5)), IndentMode::Auto)
            .expect("lines changed");
        assert_eq!(&editor.buffer[..], expected);

        let mut input = IndentReader::from(&["second"]);
        let expected = [
            "second\n",
            "one\n",
            "    indented\n",
            "        further\n",
            "    four\n",
            "        five\n",
            "        replacing blanks\n",
            "    six\n",
        ];
        let _ = editor
            .change_cmd(&mut input, Some(Address::line(0)), IndentMode::Auto)
            .expect("line changed");
        assert_eq!(&editor.buffer[..], expected);
    }

    #[test]
    fn join_cmd_empty_buffer() {
        let mut editor = Editor::new();
        editor.buffer = EditBuffer::new();
        let res = editor.join_cmd(None, None).expect_err("should fail");
        assert!(matches!(res, LnedError::InvalidAddress));
    }

    #[test]
    fn join_cmd_single_line_addr() {
        let mut editor = Editor::new();
        editor.buffer = EditBuffer::with_text(&["1\n", "2", "3"]);
        let expected = editor.buffer.clone();
        let res = editor
            .join_cmd(Some(Address::line(3)), None)
            .expect_err("invalid address");
        assert!(matches!(res, LnedError::InvalidAddress));
        assert_eq!(editor.buffer, expected);
        let expected = EditBuffer::with_text(&["1\n", "23"]);
        editor.join_cmd(Some(Address::line(2)), None).unwrap();
        assert_eq!(editor.buffer, expected);
    }

    #[test]
    fn join_cmd_default_on_last_line() {
        let mut editor = Editor::new();
        editor.buffer = EditBuffer::with_text(&["1\n", "2", "3"]);
        let res = editor.join_cmd(None, None).expect_err("should fail");
        assert!(matches!(res, LnedError::InvalidAddress));
    }

    #[test]
    fn move_cmd_destination_invalid() {
        let mut editor = Editor::new();
        editor.buffer =
            EditBuffer::with_text(&["1\n", "2", "3", "4", "5", "6"]);
        let source = Address::span(3, 5);
        let destination = Address::line(7);
        let res = editor
            .move_cmd(Some(source), destination)
            .expect_err("should fail");
        assert!(matches!(res, LnedError::InvalidAddress));
    }

    #[test]
    fn move_cmd_destination_intersects_source_give_error() {
        let mut editor = Editor::new();
        editor.buffer =
            EditBuffer::with_text(&["1\n", "2", "3", "4", "5", "6"]);
        let source = Address::span(3, 5);
        let res = editor
            .move_cmd(Some(source), Address::line(4))
            .expect_err("should fail");
        assert!(matches!(res, LnedError::DestinationIntersectsSource));
        editor
            .move_cmd(Some(source), Address::line(5))
            .expect("shouldn't fail");
    }

    #[test]
    fn line_number_cmd_with_and_without_address() {
        let mut output = Vec::new();
        let mut editor = Editor::new();
        editor.buffer =
            EditBuffer::with_text(&["1\n", "2", "3", "4", "5", "6"]);
        editor.buffer.set_current_line(2);
        let res = editor.line_number_cmd(&mut output, None);
        let out_text = str::from_utf8(&output[..]).unwrap();
        assert_eq!(out_text, "6\n");
        assert!(res.is_none());
        output.clear();
        let res = editor.line_number_cmd(&mut output, Some(Address::line(2)));
        let out_text = str::from_utf8(&output[..]).unwrap();
        assert!(res.is_none());
        assert_eq!(out_text, "2\n");
    }

    #[test]
    fn read_cmd_no_filename_error() {
        let mut editor = Editor::new();
        let res = editor
            .read_cmd(&mut Vec::new(), None, None)
            .expect_err("no filename");
        assert!(matches!(res, LnedError::NoFilename));
    }

    #[test]
    fn read_cmd_reads_file() {
        let mut editor = Editor::new();
        editor.buffer =
            EditBuffer::with_text(&["one\n", "two", "three", "four"]);
        editor.buffer.set_current_line(2);
        let orig = editor.buffer.clone();
        let mut expected = EditBuffer::with_text(&[
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

        let changes = editor
            .read_cmd(&mut output, None, filename1)
            .expect("no error")
            .expect("Some(ChangeSet)");
        let out_text = str::from_utf8(&output[..]).unwrap();
        assert!(
            out_text.contains("10 lines") && out_text.contains("312 bytes")
        );
        editor.buffer.push_undo(changes);

        assert_eq!(editor.buffer[..], expected[..]);
        assert_eq!(editor.buffer.current_line(), expected.current_line());

        editor.buffer.do_undo().expect("something to undo");
        assert_eq!(editor.buffer[..], orig[..]);
        assert_eq!(editor.buffer.current_line(), orig.current_line());

        editor.buffer.do_redo().expect("something to redo");
        assert_eq!(editor.buffer[..], expected[..]);
        assert_eq!(editor.buffer.current_line(), expected.current_line());
    }

    #[test]
    fn write_cmd_no_filename() {
        let mut output = Vec::new();
        let input = b"a\n1\n.\nw\nq\nq\n";

        run(&input[..], &mut output, &CmdArgs::default()).unwrap();
        let output = str::from_utf8(&output[..]).unwrap();
        assert!(output.contains("no filename"));
    }

    #[test]
    fn write_cmd_new_filename() {
        let mut output = Vec::new();
        let tmp_dir = tempdir().expect("tmp dir created");
        let current_filename = tmp_dir.path().join("new_filename");
        let new_filename = tmp_dir.path().join("new_filename");
        let backup_filename = new_filename.clone().with_added_extension("bak");
        let mut editor = Editor::new();
        editor.buffer = EditBuffer::with_text(&["1\n", "2", "3"]);
        editor.current_file = Some(current_filename.clone());
        let _res = editor
            .write_cmd(&mut output, None, Some(&new_filename))
            .expect("successful write to new_filename");
        assert!(matches!(fs::exists(&new_filename), Ok(true)));
        assert_eq!(editor.current_file, Some(current_filename));
        assert!(matches!(fs::exists(&backup_filename), Ok(false)));
    }

    #[test]
    fn write_cmd_overwrite() {
        let tmp_dir = tempdir().expect("tmp dir created");
        let name = tmp_dir.path().join("filename.txt");
        let mut editor = Editor::new();
        editor.previous_warning = Warning::None;
        editor.current_file = Some(name.clone());
        let expected_warning =
            Warning::WriteOverwrite(None, Some(name.clone()));
        editor.buffer = EditBuffer::with_text(&["1\r\n", "2\r\n", "3\r\n"]);
        let mut output = Vec::new();
        fs::copy(
            Path::new(r"test/assets/text_with_final_eol.txt"),
            name.as_path(),
        )
        .expect("copy file for test");

        let res = editor
            .write_cmd(&mut output, None, Some(&name))
            .expect_err("overwrite warning");
        assert!(matches!(res, LnedError::WriteWouldOverwrite(_)));
        assert_eq!(editor.previous_warning, expected_warning);
        let _ = editor
            .write_cmd(&mut output, None, Some(&name))
            .expect("successful overwrite on second try");
        assert_eq!(editor.previous_warning, Warning::None);
        let new_content = fs::read(&name).expect("successful read");
        assert_eq!(
            new_content,
            editor.buffer[..]
                .iter()
                .fold(String::new(), |mut acc, x| {
                    acc.push_str(x);
                    acc
                })
                .as_bytes()
        );
        assert!(
            str::from_utf8(&output[..])
                .unwrap()
                .contains("3 lines (9 bytes) written")
        );
    }

    #[test]
    fn write_cmd_default_overwrite() {
        let tmp_dir = tempdir().expect("tmp dir created");
        let name = tmp_dir.path().join("filename.txt");
        let mut editor = Editor::new();
        editor.buffer = EditBuffer::with_text(&["1\r\n", "2\r\n", "3\r\n"]);
        editor.current_file = Some(name.clone());
        editor.previous_warning =
            Warning::WriteOverwrite(None, Some(name.clone()));
        let mut output = Vec::new();
        fs::copy(
            Path::new(r"test/assets/text_with_final_eol.txt"),
            name.as_path(),
        )
        .expect("copy file for test");

        let _ = editor
            .write_cmd(&mut output, None, None)
            .expect("successful overwrite because default filename");
        let new_content = fs::read(&name).expect("successful read");
        assert_eq!(editor.previous_warning, Warning::None);
        assert_eq!(
            new_content,
            editor.buffer[..]
                .iter()
                .fold(String::new(), |mut acc, x| {
                    acc.push_str(x);
                    acc
                })
                .as_bytes()
        );
        assert!(
            str::from_utf8(&output[..])
                .unwrap()
                .contains("3 lines (9 bytes) written")
        );
    }

    #[test]
    fn write_cmd_backup_exists() {
        let tmp_dir = tempdir().expect("tmp dir created");
        let mut editor = Editor::new();
        editor.buffer = EditBuffer::with_text(&["1\n", "2", "3"]);
        let name = tmp_dir.path().join("filename.txt");
        let backup_name = name.with_added_extension("bak");
        editor.current_file = Some(name.clone());
        let mut output = Vec::new();
        fs::copy(Path::new(r"test/assets/text_with_final_eol.txt"), &name)
            .expect("copy file for test");
        fs::copy(
            Path::new(r"test/assets/text_with_final_eol.txt"),
            &backup_name,
        )
        .expect("copy file for backup");
        if let LnedError::WriteBackupFileCreate {
            source,
            filename,
            backup_filename,
        } = editor
            .write_cmd(&mut output, None, Some(&name))
            .expect_err("backup file create fail")
        {
            assert_eq!(source.kind(), io::ErrorKind::AlreadyExists);
            assert_eq!(filename, name);
            assert_eq!(backup_filename, Some(backup_name));
        } else {
            panic!("expected error creating \"{}\"", backup_name.display());
        }
    }

    #[test]
    fn write_file_error_writing_file() {
        struct BadWriter {
            inner: EditedFile,
        }

        impl FileWrite for BadWriter {
            fn write(
                &mut self,
                _buffer: &mut EditBuffer,
                _span: Option<Address>,
            ) -> io::Result<(usize, usize)> {
                Err(io::Error::new(
                    io::ErrorKind::StorageFull,
                    "no room at the in!",
                ))
            }
            fn backup(&mut self) -> io::Result<()> {
                self.inner.backup()
            }
            fn remove_backup(&self) -> io::Result<()> {
                self.inner.remove_backup()
            }
            fn name(&self) -> &Path {
                self.inner.name()
            }
            fn backup_name(&self) -> Option<&Path> {
                self.inner.backup_name()
            }
        }

        let tmp_dir = tempdir().expect("tmp dir created");
        let mut editor = Editor::new();
        editor.buffer = EditBuffer::with_text(&["1\n", "2", "3"]);
        let name = tmp_dir.path().join("filename.txt");
        let backup_name = name.with_added_extension("bak");
        let mut output = Vec::new();
        fs::copy(
            Path::new(r"test/assets/text_with_final_eol.txt"),
            name.as_path(),
        )
        .expect("copy file for test");
        let file_content = fs::read(&name).expect("successful read");
        let edited_file =
            EditedFile::open_or_create(&name).expect("EditedFile");
        let mut writer = BadWriter { inner: edited_file };
        if let Err(LnedError::WriteFile {
            source,
            filename: _,
            backup_filename,
        }) = write_file(&mut editor.buffer, &mut output, None, &mut writer)
        {
            assert_eq!(source.kind(), io::ErrorKind::StorageFull);
            assert!(fs::exists(backup_filename.unwrap()).unwrap());
            let backup_content =
                fs::read(&backup_name).expect("successful read");
            assert_eq!(backup_content, file_content);
        }
    }

    #[test]
    fn write_file_error_making_backup() {
        struct BadWriter {
            inner: EditedFile,
        }

        impl FileWrite for BadWriter {
            fn write(
                &mut self,
                buffer: &mut EditBuffer,
                span: Option<Address>,
            ) -> io::Result<(usize, usize)> {
                self.inner.write(buffer, span)
            }
            fn backup(&mut self) -> io::Result<()> {
                Err(io::Error::new(
                    io::ErrorKind::StorageFull,
                    "no room at the in!",
                ))
            }
            fn remove_backup(&self) -> io::Result<()> {
                self.inner.remove_backup()
            }
            fn name(&self) -> &Path {
                self.inner.name()
            }
            fn backup_name(&self) -> Option<&Path> {
                self.inner.backup_name()
            }
        }

        let tmp_dir = tempdir().expect("tmp dir created");
        let mut editor = Editor::new();
        editor.buffer = EditBuffer::with_text(&["1\n", "2", "3"]);
        let name = tmp_dir.path().join("filename.txt");
        let mut output = Vec::new();
        fs::copy(Path::new(r"test/assets/text_with_final_eol.txt"), &name)
            .expect("copy file for test");
        let edited_file =
            EditedFile::open_or_create(&name).expect("EditedFile");
        let mut writer = BadWriter { inner: edited_file };
        if let Err(LnedError::WriteMakeBackup {
            source,
            filename: _,
            backup_filename,
        }) = write_file(&mut editor.buffer, &mut output, None, &mut writer)
        {
            assert_eq!(source.kind(), io::ErrorKind::StorageFull);
            assert!(!fs::exists(backup_filename.unwrap()).unwrap());
        }
    }

    #[test]
    fn list_cmd_bad_addr() {
        let mut editor = Editor::new();
        editor.buffer = EditBuffer::with_text(&["1\n", "2", "3"]);
        let mut output = Vec::new();
        let res = editor
            .list_cmd(&mut output, Some(Address::line(4)))
            .expect_err("invalid addr");
        assert!(matches!(res, LnedError::InvalidAddress));

        editor.buffer = EditBuffer::new();
        let res = editor.list_cmd(&mut output, None).expect_err("invalid addr");
        assert!(matches!(res, LnedError::InvalidAddress));
    }

    #[test]
    fn list_cmd_no_addr() {
        let mut output = Vec::new();
        let mut editor = Editor::new();
        editor.buffer = EditBuffer::with_text(&["1\r\n", "2", "3"]);
        editor.buffer.set_current_line(2);
        editor.list_cmd(&mut output, None).unwrap();
        assert_eq!(str::from_utf8(&output[..]).unwrap(), "2\\r\\n$\r\n");
    }

    #[test]
    fn list_cmd_single_line() {
        let mut output = Vec::new();
        let mut editor = Editor::new();
        editor.buffer = EditBuffer::with_text(&["1\r\n", "2", "3"]);
        editor.buffer.set_current_line(2);
        editor.list_cmd(&mut output, Some(Address::line(3))).unwrap();
        assert_eq!(str::from_utf8(&output[..]).unwrap(), "3\\r\\n$\r\n");
    }

    #[test]
    fn list_cmd_span() {
        let mut output = Vec::new();
        let mut editor = Editor::new();
        editor.buffer =
            EditBuffer::with_text(&["1\r\n", "2\t2", "3", "4", "5", "6"]);
        editor.buffer.set_current_line(5);
        editor.list_cmd(&mut output, Some(Address::span(2, 4))).unwrap();
        assert_eq!(
            str::from_utf8(&output[..]).unwrap(),
            "2\\t2\\r\\n$\r\n3\\r\\n$\r\n4\\r\\n$\r\n"
        );
    }

    #[test]
    fn list_cmd_sets_current_line() {
        let mut output = Vec::new();
        let mut editor = Editor::new();
        editor.buffer =
            EditBuffer::with_text(&["1\r\n", "2", "3", "4", "5", "6"]);
        editor.buffer.set_current_line(5);
        editor.list_cmd(&mut output, Some(Address::span(2, 4))).unwrap();
    }

    #[test]
    fn scroll_cmd_dispatch() {
        let input = b"a\n1\n2\n3\n4\n\n.\n1\nz2\nq\nq\n";
        let mut output = Vec::new();
        let args = CmdArgs { file: None };
        run(&input[..], &mut output, &args).unwrap();
        let output = str::from_utf8(&output[..]).unwrap();
        assert!(output.contains("2\n3\n"));
        assert!(!output.contains("4\n"));
    }

    #[test]
    fn show_diff_cmd_dispatch() {
        let input = b"S\nq\n";
        let mut output = Vec::new();
        let args = CmdArgs { file: None };
        run(&input[..], &mut output, &args).unwrap();
        let output = str::from_utf8(&output[..]).unwrap();
        assert!(output.contains("no filename"));
    }

    #[test]
    fn scroll_cmd_at_end() {
        let mut editor = Editor::new();
        let lines: Vec<String> = (1..=64).map(|n| format!("{n}\r\n")).collect();
        editor.buffer = EditBuffer::from(lines);
        let mut output = Vec::new();
        let res = editor
            .scroll_cmd(
                &mut output,
                Some(Address::line(60)),
                None,
                ScrollWindow { cols: 80, rows: 24 },
            )
            .expect("scroll to end");
        assert!(res.is_none());
        let output = str::from_utf8(&output[..]).unwrap();
        assert!(output.contains("60\r\n61\r\n62\r\n63\r\n64\r\n"));
        assert_eq!(editor.buffer.current_line(), 64);
    }

    #[test]
    fn scroll_cmd_saves_windows() {
        let mut editor = Editor::new();
        let lines: Vec<String> = (1..=64).map(|n| format!("{n}\r\n")).collect();
        editor.buffer = EditBuffer::from(lines);
        let mut output = Vec::new();
        let mut input = b"" as &[u8];
        editor
            .dispatch_cmd(
                &Cmd::Scroll(Some(Address::line(10)), Some(3), None),
                &mut output,
                &mut input,
            )
            .expect("scroll 10..12");
        assert_eq!(editor.buffer.current_line(), 12);
        assert_eq!(editor.scroll_row_limit, Some(3));
        editor
            .dispatch_cmd(
                &Cmd::Scroll(None, None, None),
                &mut output,
                &mut input,
            )
            .expect("scroll 13..15");
        assert_eq!(editor.buffer.current_line(), 15);
        assert_eq!(editor.scroll_row_limit, Some(3));
    }

    #[test]
    fn scroll_cmd_with_print_sfx() {
        let mut editor = Editor::new();
        let lines: Vec<String> = (1..=64).map(|n| format!("{n}\n")).collect();
        editor.buffer = EditBuffer::from(lines);
        let mut output = Vec::new();
        let mut input = b"" as &[u8];
        editor
            .dispatch_cmd(
                &Cmd::Scroll(
                    Some(Address::line(10)),
                    Some(3),
                    Some(PrintAttributes {
                        enumerate: true,
                        ..Default::default()
                    }),
                ),
                &mut output,
                &mut input,
            )
            .expect("scroll 10..12");
        assert_eq!(editor.buffer.current_line(), 12);
        assert!(
            str::from_utf8(&output[..])
                .unwrap()
                .contains("10  10\n11  11\n12  12\n")
        );
        assert!(!str::from_utf8(&output[..]).unwrap().contains("13"));
        editor
            .dispatch_cmd(
                &Cmd::Scroll(
                    None,
                    None,
                    Some(PrintAttributes {
                        expand_escapes: true,
                        ..Default::default()
                    }),
                ),
                &mut output,
                &mut input,
            )
            .expect("scroll 13..15");
        assert_eq!(editor.buffer.current_line(), 15);
        assert!(
            str::from_utf8(&output[..])
                .unwrap()
                .contains("13\\n$\n14\\n$\n15\\n$\n")
        );
        assert!(!str::from_utf8(&output[..]).unwrap().contains("16"));
    }

    #[test]
    fn show_diff_cmd_diffs_current_file() {
        let mut editor = Editor::new();
        let mut output = Vec::new();
        let name = Path::new(r"test/assets/text_with_final_eol.txt");
        let _ = editor.edit_cmd(&mut output, Some(name)).expect("no error");
        assert_eq!(editor.current_file.as_deref(), Some(name));

        let _ = editor.delete_cmd(Some(Address::line(6))).expect("no error");
        let _ = editor.show_diff_cmd(&mut output, None).expect("no error");
        let output = str::from_utf8(&output).unwrap();
        let expected = "10 lines (312 bytes) read [LF]\n--- test/assets/text_with_final_eol.txt\n+++ current buffer\n@@ -3,7 +3,6 @@\n but it will suffice to test commands that\n read\n and\n-edit files. The lines\n are of various lengths, and\n end and begin with \n \"special\" characters (i.e., non-alpha characters).\n";
        assert_eq!(output, expected);
    }

    #[test]
    fn show_diff_cmd_with_filename_diffs_filename() {
        let mut editor = Editor::new();
        let mut output = Vec::new();
        let name = Path::new(r"test/assets/text_with_final_eol.txt");
        let _ =
            editor.read_cmd(&mut output, None, Some(name)).expect("no error");
        let _ = editor.delete_cmd(Some(Address::line(6))).expect("no error");
        let _ =
            editor.show_diff_cmd(&mut output, Some(name)).expect("no error");
        let output = str::from_utf8(&output).unwrap();
        let expected = "10 lines (312 bytes) read\n--- test/assets/text_with_final_eol.txt\n+++ current buffer\n@@ -3,7 +3,6 @@\n but it will suffice to test commands that\n read\n and\n-edit files. The lines\n are of various lengths, and\n end and begin with \n \"special\" characters (i.e., non-alpha characters).\n";
        assert_eq!(output, expected);
    }

    #[test]
    fn show_diff_cmd_error_reading_file_fails() {
        let mut editor = Editor::new();
        editor.buffer = EditBuffer::with_text(&["1\n", "2", "3"]);
        let mut output = Vec::new();
        let name = Path::new("file_not_found");
        let Err(LnedError::DiffReadFile { source, filename }) =
            editor.show_diff_cmd(&mut output, Some(name))
        else {
            panic!("error expected");
        };
        assert!(matches!(source.kind(), io::ErrorKind::NotFound));
        assert_eq!(filename, name);
    }

    #[test]
    fn show_diff_cmd_no_filename_no_current_file_fails() {
        let mut editor = Editor::new();
        editor.buffer = EditBuffer::with_text(&["1\n", "2", "3"]);
        let mut output = Vec::new();
        let res =
            editor.show_diff_cmd(&mut output, None).expect_err("no filename");
        assert!(matches!(res, LnedError::NoFilename));
    }

    #[test]
    fn newline_cmd_same_eol_not_mixed_does_nothing() {
        let mut editor = Editor::new();
        editor.buffer = EditBuffer::with_text(&["1\r\n", "2", "3"]);
        let mut output = Vec::new();
        let res =
            editor.newline_cmd(&mut output, Some(PrevailingEol::crlf(false)));
        assert!(res.is_none());
    }

    #[test]
    fn newline_cmd_no_arg_prints_prevailing_eol() {
        let mut editor = Editor::new();
        editor.buffer = EditBuffer::with_text(&["1\n", "2", "3"]);
        let mut output = Vec::new();
        let res = editor.newline_cmd(&mut output, None);
        assert!(res.is_none());
        let output = str::from_utf8(&output[..]).unwrap();
        assert!(output.contains("prevailing newline: LF"));
    }

    #[test]
    fn newline_cmd_invalid_newline_prints_error() {
        let input = b"a\n1\n2\n3\n.\nN HT\nq\nq\n";
        let mut output = Vec::new();
        run(&input[..], &mut output, &CmdArgs::default()).unwrap();
        let output = str::from_utf8(&output[..]).unwrap();
        assert!(output.contains("invalid newline"));
    }

    #[test]
    fn newline_cmd_with_arg_normalizes_and_prints_prevailing_eol() {
        let mut editor = Editor::new();
        editor.buffer = EditBuffer::with_text(&["1\n", "2\r\n", "3\n"]);
        let mut output = Vec::new();
        let res = editor.newline_cmd(&mut output, None);
        assert!(res.is_none());
        let text = str::from_utf8(&output[..]).unwrap();
        assert!(text.contains("prevailing newline: LF/mixed"));
        output.clear();
        let res =
            editor.newline_cmd(&mut output, Some(PrevailingEol::crlf(false)));
        assert!(res.is_some());
        let text = str::from_utf8(&output[..]).unwrap();
        assert!(text.contains("prevailing newline: CRLF"));
        assert_eq!(
            editor.buffer.prevailing_eol(),
            Some(PrevailingEol::crlf(false)),
        );
        output.clear();
        let res = editor.newline_cmd(
            &mut output,
            Some(PrevailingEol { eol: Eol::Lf, mixed: false }),
        );
        assert!(res.is_some());
        let text = str::from_utf8(&output[..]).unwrap();
        assert!(text.contains("prevailing newline: LF"));
        assert_eq!(
            editor.buffer.prevailing_eol(),
            Some(PrevailingEol::lf(false)),
        );
    }

    #[test]
    fn newline_cmd_undo_redo_restores_prevailing_eol() {
        let mut editor = Editor::new();
        editor.buffer = EditBuffer::with_text(&["1\n", "2\r\n", "3\n"]);
        editor.buffer.set_current_line(1);
        let orig_buffer = editor.buffer.clone();
        let mut output = Vec::new();
        let mut expected = EditBuffer::with_text(&["1\r\n", "2", "3"]);
        expected.set_current_line(1);

        let res =
            editor.newline_cmd(&mut output, Some(PrevailingEol::crlf(false)));
        editor.buffer.push_undo(res.unwrap());
        assert_eq!(editor.buffer[..], expected[..]);
        assert_eq!(editor.buffer.current_line(), expected.current_line());
        assert_eq!(editor.buffer.prevailing_eol(), expected.prevailing_eol());

        editor.buffer.do_undo().unwrap();
        assert_eq!(editor.buffer[..], orig_buffer[..]);
        assert_eq!(editor.buffer.current_line(), orig_buffer.current_line());
        assert_eq!(
            editor.buffer.prevailing_eol(),
            orig_buffer.prevailing_eol()
        );
        editor.buffer.do_redo().unwrap();
        assert_eq!(editor.buffer[..], expected[..]);
        assert_eq!(editor.buffer.current_line(), expected.current_line());
        assert_eq!(editor.buffer.prevailing_eol(), expected.prevailing_eol());
    }
}
