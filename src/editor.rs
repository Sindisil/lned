use std::borrow::Cow;
use std::cmp;
use std::collections::VecDeque;
use std::fmt::{self, Write as _};
use std::fs::{self, File, OpenOptions};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::{self, BufRead, BufReader, Cursor, prelude::*};
use std::num::NonZero;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::SystemTime;

use crossterm::{ExecutableCommand, terminal};
use regex::Regex;
use similar::TextDiff;
use unicode_segmentation::UnicodeSegmentation;

use crate::cli;
use crate::command::{
    Cmd, InputMode, InputSource, PrintSuffix, SubstitutionScope,
};
use crate::edit_buffer::EditBuffer;
use crate::eol::{Eol, Eols};
use crate::error::{Error, Warning};
use crate::undo_stack::{Change, ChangeSet};

use line_edit::{self, EditorOptions, LineEdit};

static INDENT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^([[:blank:]]*)").expect("indent regex"));
#[derive(Debug)]
struct Editor {
    previous_warning: Option<Warning>,
    previous_pattern: Option<regex::Regex>,
    page_length: Option<NonZero<usize>>,
    current_file: Option<PathBuf>,
    file_metadata: Option<FileMetadata>,
    file_hash: Option<u64>,
    buffer_sync_hash: u64,
    buffer: EditBuffer,
    page_buffer: Vec<String>,
    clipboard: String,
    output_target: OutputTarget,
}

#[derive(Debug, Copy, Clone)]
pub enum OutputTarget {
    Terminal,
    Other,
}

#[derive(Debug, PartialEq)]
struct FileMetadata {
    len: u64,
    modified: Option<SystemTime>,
}

impl Editor {
    fn new(output_target: OutputTarget) -> Editor {
        let mut buffer = EditBuffer::new();
        let buffer_sync_hash = buffer.content_hash();
        let (_, term_rows) = match output_target {
            OutputTarget::Terminal => terminal_size(),
            OutputTarget::Other => DEFAULT_TERMINAL_SIZE,
        };
        let mut page_buffer = Vec::new();
        page_buffer.resize_with(term_rows, String::new);
        let clipboard = String::new();
        Editor {
            previous_warning: None,
            previous_pattern: None,
            page_length: None,
            current_file: None,
            file_metadata: None,
            file_hash: None,
            buffer,
            buffer_sync_hash,
            clipboard,
            page_buffer,
            output_target,
        }
    }

    fn terminal_size(&self) -> (usize, usize) {
        match self.output_target {
            OutputTarget::Terminal => terminal_size(),
            OutputTarget::Other => DEFAULT_TERMINAL_SIZE,
        }
    }

    fn buffer_is_unsaved(&mut self) -> bool {
        self.buffer_sync_hash != self.buffer.content_hash()
    }

    fn size_page_buffer(&mut self, rows: usize) {
        if self.page_buffer.len() < rows {
            self.page_buffer.resize_with(rows, String::new);
        }
    }

    #[allow(clippy::too_many_lines)]
    fn dispatch_cmd(
        &mut self,
        cmd: Cmd,
        output: &mut impl Write,
        input: &mut impl LineEdit,
    ) -> Result<Option<ChangeSet>, Error> {
        let mut output = FmtWriter(output);
        let res = match cmd {
            // dispatch editor commands
            Cmd::Append { index, source, mode } => {
                self.append_cmd(input, &mut output, index, source, mode)
            }
            Cmd::Copy(span) => {
                self.copy_cmd(span);
                Ok(None)
            }
            Cmd::Cut(span) => Ok(Some(self.cut_cmd(span))),
            Cmd::Delete(span) => self.delete_cmd(span),
            Cmd::Edit(filename) => {
                self.edit_cmd(&mut output, filename.as_path())
            }
            Cmd::Enumerate(span) => Ok(self.enumerate_cmd(&mut output, span)),
            Cmd::File => {
                self.file_cmd(&mut output);
                Ok(None)
            }
            Cmd::Global(span, pattern, commands) => {
                self.global_cmd(&mut output, span, &pattern, &commands)
            }
            Cmd::Insert { index, source, mode } => {
                self.insert_cmd(input, &mut output, index, source, mode)
            }
            Cmd::Join(span, separator) => {
                self.join_cmd(span, separator.as_deref())
            }
            Cmd::LineNumber(index) => {
                self.line_number_cmd(&mut output, index);
                Ok(None)
            }
            Cmd::List(span) => Ok(self.list_cmd(&mut output, span)),
            Cmd::New => self.new_cmd(),
            Cmd::Newline(eol) => Ok(self.newline_cmd(&mut output, eol)),
            Cmd::Null(index) => self.null_cmd(&mut output, index),
            Cmd::Overwrite { span, source, mode } => {
                self.overwrite_cmd(input, &mut output, span, source, mode)
            }
            Cmd::PageDown(index, page_length, pr_sfx) => {
                let (cols, term_rows) = self.terminal_size();
                if let Some(n) = page_length {
                    self.page_length = NonZero::new(n.clamp(0, term_rows - 1));
                }
                let rows = self
                    .page_length
                    .map_or(term_rows.saturating_sub(3) / 2, usize::from);
                self.page_down_cmd(
                    &mut output,
                    index,
                    pr_sfx,
                    PageBounds { cols, rows },
                );
                Ok(None)
            }
            Cmd::PageUp(index, page_length, pr_sfx) => {
                let (cols, term_rows) = self.terminal_size();
                if let Some(n) = page_length {
                    self.page_length = NonZero::new(n.clamp(0, term_rows - 1));
                }
                let rows = self
                    .page_length
                    .map_or(term_rows.saturating_sub(3) / 2, usize::from);
                self.page_up_cmd(
                    &mut output,
                    index,
                    pr_sfx,
                    PageBounds { cols, rows },
                );
                Ok(None)
            }
            Cmd::Print(span) => Ok(self.print_cmd(&mut output, span)),
            Cmd::Quit => self.quit_cmd(),
            Cmd::Redo => self.buffer.redo().map(|()| None),
            Cmd::Reload => self.reload_cmd(&mut output),
            Cmd::ShowDiff(filename) => {
                self.show_diff_cmd(&mut output.0, filename.as_deref())
            }
            Cmd::Substitute(span, pattern, replacement, scope) => {
                self.substitute_cmd(span, &pattern, &replacement, scope)
            }
            Cmd::Undo => self.buffer.undo().map(|()| None),
            Cmd::Version => {
                version_cmd(&mut output);
                Ok(None)
            }
            Cmd::Write => self.write_cmd(&mut output),
            Cmd::WriteAs(span, filename) => {
                self.write_as_cmd(&mut output, span, filename.as_path())
            }
        };

        res.map_err(|e| {
            if let Error::GlobalCmdErrorStop { source, changes } = e {
                if let Some(changes) = changes {
                    self.buffer.push_undo(changes);
                }
                *source
            } else {
                e
            }
        })
    }

    fn update_file_metadata(&mut self) {
        self.file_metadata = self
            .current_file
            .as_ref()
            .and_then(|cf| fs::metadata(cf).ok())
            .map(|md| FileMetadata {
                len: md.len(),
                modified: md.modified().ok(),
            });
    }

    // Read lines of input into buf, stopping when a '.' alone on a line
    // is read. Clears previous content of buf, but doesn't shrink capacity.
    // If prevailing_eol is provided, ensures all lines
    // are terminated with that newline sequence.
    // Returns number of bytes read or Error::Readlines if an error is
    // encountered.
    fn append_cmd(
        &mut self,
        input: &mut impl LineEdit,
        output: &mut impl fmt::Write,
        index: Option<usize>,
        source: InputSource,
        mode: InputMode,
    ) -> Result<Option<ChangeSet>, Error> {
        let index = match index {
            Some(index) => index + 1,
            None if self.buffer.is_empty() => 0,
            None => self.buffer.current_index() + 1,
        };
        let (indent, eol) = match mode {
            InputMode::Cooked => (
                self.buffer[..index]
                    .iter()
                    .rfind(|l| l.contains(|c: char| !c.is_whitespace()))
                    .and_then(|l| INDENT.captures(l))
                    .and_then(|c| c.get(1))
                    .map(|m| m.as_str().to_owned()),
                Some(self.buffer.eols().prevailing()),
            ),
            InputMode::Raw => (None, None),
        };
        let mut lines = Vec::new();
        match source {
            InputSource::Clipboard => {
                read_lines(&mut lines, &mut Cursor::new(&self.clipboard), eol)?;
            }
            InputSource::File(filename) => {
                read_file_lines(&mut lines, &filename, eol, output)?;
            }
            InputSource::StdIn => {
                read_input_lines(&mut lines, input, indent, eol)?;
            }
        }
        let changes = self.buffer.insert(index, lines);
        Ok((!changes.is_empty()).then_some(changes))
    }

    fn overwrite_cmd(
        &mut self,
        input: &mut impl LineEdit,
        output: &mut impl fmt::Write,
        span: Option<Range<usize>>,
        source: InputSource,
        mode: InputMode,
    ) -> Result<Option<ChangeSet>, Error> {
        if self.buffer.is_empty() {
            return Err(Error::NothingToOverwrite);
        }
        let span = span.unwrap_or_else(|| self.buffer.current_index_as_range());
        let (indent, eol) = match mode {
            // Auto-indent is same as first non-blank line in changed
            // span, or first previous non-blank line if changed span is
            // all blank.
            InputMode::Cooked => (
                self.buffer[span.clone()]
                    .iter()
                    .find(|l| l.contains(|c: char| !c.is_whitespace()))
                    .or_else(|| {
                        self.buffer[..span.start]
                            .iter()
                            .rfind(|l| l.contains(|c: char| !c.is_whitespace()))
                    })
                    .and_then(|l| INDENT.captures(l))
                    .and_then(|c| c.get(1))
                    .map(|m| m.as_str().to_owned()),
                Some(self.buffer.eols().prevailing()),
            ),
            InputMode::Raw => (None, None),
        };

        let mut lines = Vec::new();
        match source {
            InputSource::Clipboard => {
                read_lines(&mut lines, &mut Cursor::new(&self.clipboard), eol)?;
            }
            InputSource::File(filename) => {
                read_file_lines(&mut lines, &filename, eol, output)?;
            }
            InputSource::StdIn => {
                read_input_lines(&mut lines, input, indent, eol)?;
            }
        }
        let mut changes = self.buffer.remove(span.clone());
        changes.extend(self.buffer.insert(span.start, lines));
        Ok((!changes.is_empty()).then_some(changes))
    }

    fn delete_cmd(
        &mut self,
        span: Option<Range<usize>>,
    ) -> Result<Option<ChangeSet>, Error> {
        if span.is_none() && self.buffer.is_empty() {
            return Err(Error::NothingToDelete);
        }

        let span = span.unwrap_or_else(|| self.buffer.current_index_as_range());
        let changes = self.buffer.remove(span);
        Ok((!changes.is_empty()).then_some(changes))
    }

    fn copy_cmd(&mut self, span: Option<Range<usize>>) {
        let span = span.unwrap_or(self.buffer.current_index_as_range());
        self.clipboard.clear();
        self.clipboard.extend(self.buffer[span].iter().cloned());
    }

    fn cut_cmd(&mut self, span: Option<Range<usize>>) -> ChangeSet {
        let span = span.unwrap_or(self.buffer.current_index_as_range());
        self.clipboard.clear();
        self.clipboard.extend(self.buffer[span.clone()].iter().cloned());
        self.buffer.remove(span)
    }

    fn page_down_cmd(
        &mut self,
        output: &mut impl fmt::Write,
        index: Option<usize>,
        pr_sfx: Option<PrintSuffix>,
        bounds: PageBounds,
    ) {
        if index.is_none()
            && self.buffer.current_index()
                == self.buffer.len().saturating_sub(1)
        {
            return;
        }
        let start = index.unwrap_or(self.buffer.current_index() + 1);
        let end = cmp::min(self.buffer.len(), start + bounds.rows + 1);

        self.size_page_buffer(bounds.rows);
        let pr_sfx = pr_sfx.unwrap_or_default();

        let mut rows: usize = 0;
        let mut pb_end = 0;
        for i in start..end {
            self.page_buffer[pb_end].clear();
            let cols = write_line(
                &mut self.page_buffer[pb_end],
                &self.buffer,
                i,
                pr_sfx,
            )
            .unwrap();
            pb_end += 1;
            rows = rows.saturating_add(cols.div_ceil(bounds.cols));
            if rows >= bounds.rows {
                break;
            }
        }
        for line in &self.page_buffer[0..pb_end] {
            output.write_str(line).unwrap();
        }
        self.buffer.set_current_index(start + pb_end.saturating_sub(1));
    }

    fn page_up_cmd(
        &mut self,
        output: &mut impl fmt::Write,
        index: Option<usize>,
        pr_sfx: Option<PrintSuffix>,
        bounds: PageBounds,
    ) {
        let end = index.map_or(self.buffer.current_index(), |i| i + 1);
        if end == 0 {
            return;
        }

        let start = end.saturating_sub(bounds.rows);

        self.size_page_buffer(bounds.rows);
        let pr_sfx = pr_sfx.unwrap_or_default();

        let mut rows: usize = 0;
        let mut pb_start = bounds.rows - 1;
        for i in (start..end).rev() {
            self.page_buffer[pb_start].clear();
            let cols = write_line(
                &mut self.page_buffer[pb_start],
                &self.buffer,
                i,
                pr_sfx,
            )
            .unwrap();
            rows = rows.saturating_add(cols.div_ceil(bounds.cols));
            if rows >= bounds.rows {
                break;
            }
            pb_start -= 1;
        }
        for line in &self.page_buffer[pb_start..] {
            output.write_str(line).unwrap();
        }
        self.buffer
            .set_current_index(end.saturating_sub(bounds.rows - pb_start));
    }

    fn show_diff_cmd(
        &mut self,
        output: &mut impl Write,
        filename: Option<&Path>,
    ) -> Result<Option<ChangeSet>, Error> {
        let filename = filename
            .or(self.current_file.as_deref())
            .ok_or(Error::NoFilename)?;
        let file = fs::read(filename).map_err(|e| Error::DiffReadFile {
            source: Some(Box::new(e)),
            filename: filename.to_owned(),
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

    fn reload_cmd(
        &mut self,
        output: &mut impl fmt::Write,
    ) -> Result<Option<ChangeSet>, Error> {
        let unsaved = self.buffer_is_unsaved();

        // make sure current_file set
        let Some(filename) = self.current_file.as_ref() else {
            return Err(Error::NoFilename);
        };

        // warn if there are unsaved changes
        if self.previous_warning != Some(Warning::ReloadUnsaved) && unsaved {
            return Err(Error::Warning(Warning::ReloadUnsaved));
        }

        // load current_file into buffer
        let file = File::open(filename).map_err(|e| {
            if e.kind() == io::ErrorKind::NotFound {
                self.buffer.clear();
                Error::FileNotFound(filename.into())
            } else {
                Error::EditFileOpen {
                    source: Some(Box::new(e)),
                    filename: filename.into(),
                }
            }
        })?;
        let mut source = BufReader::new(file);
        let mut lines = Vec::new();
        let eol = Some(self.buffer.eols().prevailing());
        let (bytes_read, eol_added) = read_lines(&mut lines, &mut source, eol)?;
        let lines_read = lines.len();
        self.buffer.clear();
        self.buffer.insert(0, lines);

        // Update metadata & hashes
        self.update_file_metadata();
        self.file_hash = Some(self.buffer.content_hash());
        self.buffer_sync_hash = self.buffer.content_hash();

        // report info on load
        write!(
            output,
            "{} lines ({} bytes) read",
            format_number(lines_read),
            format_number(bytes_read)
        )
        .unwrap();
        let prevailing_eol = self.buffer.eols().prevailing();
        writeln!(output, " [{prevailing_eol}]").unwrap();
        if eol_added {
            writeln!(output, "missing newline appended").unwrap();
        }
        Ok(None)
    }

    fn edit_cmd(
        &mut self,
        output: &mut impl fmt::Write,
        filename: &Path,
    ) -> Result<Option<ChangeSet>, Error> {
        // warn if there are unsaved changes
        let warning = Warning::EditUnsaved(filename.to_owned());
        if self.previous_warning.as_ref() != Some(&warning)
            && self.buffer_is_unsaved()
        {
            return Err(Error::Warning(warning));
        }

        // load filename into buffer
        let file = File::open(filename).map_err(|e| {
            if e.kind() == io::ErrorKind::NotFound {
                self.buffer.clear();
                self.current_file = Some(filename.to_owned());
                self.update_file_metadata();
                self.file_hash = None;
                self.buffer_sync_hash = self.buffer.content_hash();
                Error::FileNotFound(filename.into())
            } else {
                Error::EditFileOpen {
                    source: Some(Box::new(e)),
                    filename: filename.into(),
                }
            }
        })?;
        let mut source = BufReader::new(file);
        let mut lines = Vec::new();
        let eol = Some(self.buffer.eols().prevailing());
        let (bytes_read, eol_added) = read_lines(&mut lines, &mut source, eol)?;
        let lines_read = lines.len();
        self.buffer.clear();
        self.buffer.insert(0, lines);

        // set new current_file
        self.current_file = Some(filename.to_owned());

        // Update metadata & hashes
        self.update_file_metadata();
        self.file_hash = Some(self.buffer.content_hash());
        self.buffer_sync_hash = self.buffer.content_hash();

        // report info on load
        write!(
            output,
            "{} lines ({} bytes) read",
            format_number(lines_read),
            format_number(bytes_read)
        )
        .unwrap();
        let eol = self.buffer.eols().prevailing();
        writeln!(output, " [{eol}]").unwrap();
        if eol_added {
            writeln!(output, "missing newline appended").unwrap();
        }

        Ok(None)
    }

    fn substitute_cmd(
        &mut self,
        span: Option<Range<usize>>,
        pattern: &Regex,
        replacement: &str,
        scope: SubstitutionScope,
    ) -> Result<Option<ChangeSet>, Error> {
        if self.buffer.is_empty() && span.is_none() {
            return Err(Error::NoMatch);
        }

        // Handle default
        let mut span =
            span.unwrap_or_else(|| self.buffer.current_index_as_range());
        let prevailing_eol = self.buffer.eols().prevailing();

        let mut index = span.start;
        let (target_match, limit) = if let SubstitutionScope::Single(n) = scope
        {
            (n - 1, 1)
        } else {
            (0, 0)
        };

        let mut changes =
            ChangeSet::new(self.buffer.current_index(), self.buffer.eols());
        let mut replacement_lines = Vec::new();
        let mut span_start: Option<usize> = None;
        loop {
            if index == span.end {
                if let Some(span_start) = span_start {
                    changes.extend(self.buffer.remove(span_start..index));
                    changes.extend(
                        self.buffer.insert(span_start, replacement_lines),
                    );
                }
                break;
            }
            let line = &self.buffer[index];
            let eol_idx = line.len()
                - Eol::from_line(line)
                    .expect("buffer lines are terminated")
                    .str_value()
                    .len();
            let first_match =
                pattern.find_iter(&line[..eol_idx]).nth(target_match);
            let step = if let Some(first_match) = first_match {
                // Note start of span of matches
                span_start.get_or_insert(index);
                let mut edited_line = line[..first_match.start()].to_owned();
                edited_line.push_str(&pattern.replacen(
                    &line[first_match.start()..eol_idx],
                    limit,
                    replacement,
                ));
                edited_line.push_str(&line[eol_idx..]);
                replacement_lines.extend(
                    edited_line
                        .split_inclusive(prevailing_eol.str_value())
                        .map(ToOwned::to_owned),
                );
                1
            } else {
                // no match - apply span of matches up to this point,
                // if any
                if let Some(span_start) = span_start.take() {
                    let step =
                        replacement_lines.len() - (index - span_start) + 1;
                    changes.extend(self.buffer.remove(span_start..index));
                    changes.extend(
                        self.buffer.insert(span_start, replacement_lines),
                    );
                    replacement_lines = Vec::new();
                    step
                } else {
                    1
                }
            };
            span.end += step - 1;
            index += step;
        }

        if changes.is_empty() { Err(Error::NoMatch) } else { Ok(Some(changes)) }
    }

    fn enumerate_cmd(
        &mut self,
        output: &mut impl fmt::Write,
        span: Option<Range<usize>>,
    ) -> Option<ChangeSet> {
        if span.is_none() && self.buffer.is_empty() {
            return None;
        }

        // Handle default
        let span = span.unwrap_or_else(|| self.buffer.current_index_as_range());
        self.buffer.set_current_index(span.end - 1);
        let pr_sfx = PrintSuffix { enumerate: true, ..Default::default() };
        for i in span {
            write_line(output, &self.buffer, i, pr_sfx).unwrap();
        }
        None
    }

    fn file_cmd(&mut self, output: &mut impl fmt::Write) {
        let mut msg = String::new();
        self.format_file_info(&mut msg);
        writeln!(output, "{msg}").unwrap();
    }

    fn format_file_info(&mut self, buf: &mut String) {
        if let Some(f) = &self.current_file {
            write!(buf, "{}", f.display()).unwrap();
        } else {
            buf.push_str("no filename set");
        }

        if self.buffer_is_unsaved() {
            buf.push_str(" [unsaved]");
        }

        if self.buffer.is_empty() {
            write!(buf, " [empty]").unwrap();
        } else {
            write!(buf, " [{}]", self.buffer.eols()).unwrap();
        }
    }

    fn global_cmd(
        &mut self,
        output: &mut impl fmt::Write,
        span: Option<Range<usize>>,

        pattern: &Regex,
        commands: &str,
    ) -> Result<Option<ChangeSet>, Error> {
        self.previous_pattern = Some(pattern.clone());
        // Compile indices of lines that match pattern
        // (not including EOL)
        let search_span = span.unwrap_or_else(|| 0..self.buffer.len());
        let matched_lines = (search_span)
            .filter(|&n| {
                self.buffer[n]
                    .lines()
                    .next()
                    .is_some_and(|l| pattern.is_match(l))
            })
            .collect::<VecDeque<usize>>();

        if matched_lines.is_empty() {
            return Err(Error::NoMatch);
        }

        let mut changes =
            ChangeSet::new(self.buffer.current_index(), self.buffer.eols());
        let res =
            self.do_global_cmds(output, commands, matched_lines, &mut changes);
        let changes = if changes.is_empty() { None } else { Some(changes) };
        match res {
            Ok(()) => Ok(changes),
            Err(e) => match e {
                Error::NestedGlobalCmd => Err(Error::NestedGlobalCmd),
                Error::UnsupportedGlobalCmd => Err(Error::UnsupportedGlobalCmd),
                e => Err(Error::GlobalCmdErrorStop {
                    source: Box::new(e),
                    changes,
                }),
            },
        }
    }

    fn do_global_cmds(
        &mut self,
        output: &mut impl fmt::Write,
        commands: &str,
        mut matched_lines: VecDeque<usize>,
        changes: &mut ChangeSet,
    ) -> Result<(), Error> {
        // iterate over list
        while let Some(index) = matched_lines.pop_front() {
            self.buffer.set_current_index(index);
            let mut input = commands.as_bytes();

            // parse and execute command list for line
            while let Some((cmd, sfx)) = Cmd::read(
                &mut input,
                &mut self.buffer,
                &mut self.previous_pattern,
            )
            .map_err(|e| Error::ReadGlobalCmd { source: Some(Box::new(e)) })?
            {
                let cs = match cmd {
                    Cmd::Append { index, source, mode } => {
                        self.append_cmd(&mut input, output, index, source, mode)
                    }
                    Cmd::Overwrite { span, source, mode } => self
                        .overwrite_cmd(&mut input, output, span, source, mode),
                    Cmd::Copy(span) => {
                        self.copy_cmd(span);
                        Ok(None)
                    }
                    Cmd::Cut(span) => Ok(Some(self.cut_cmd(span))),
                    Cmd::Delete(span) => self.delete_cmd(span),
                    Cmd::Enumerate(span) => {
                        Ok(self.enumerate_cmd(output, span))
                    }
                    Cmd::Global(..) => return Err(Error::NestedGlobalCmd),
                    Cmd::Insert { index, source, mode } => {
                        self.insert_cmd(&mut input, output, index, source, mode)
                    }
                    Cmd::Join(span, separator) => {
                        self.join_cmd(span, separator.as_deref())
                    }
                    Cmd::List(span) => Ok(self.list_cmd(output, span)),
                    Cmd::Null(index) => {
                        Ok(self.print_cmd(output, index.map(|i| i..i + 1)))
                    }
                    Cmd::Print(span) => Ok(self.print_cmd(output, span)),
                    Cmd::Substitute(span, pattern, replacement, scope) => {
                        self.substitute_cmd(span, &pattern, &replacement, scope)
                    }
                    _ => Err(Error::UnsupportedGlobalCmd),
                }?;
                if let Some(mut cs) = cs {
                    for change in cs.drain() {
                        adjust_global_list(&mut matched_lines, &change);
                        changes.push(change);
                    }
                    if let Some(pr_sfx) = sfx {
                        write_line(
                            output,
                            &self.buffer,
                            self.buffer.current_index(),
                            pr_sfx,
                        )
                        .unwrap();
                    }
                }
            }
        }
        Ok(())
    }

    fn newline_cmd(
        &mut self,
        output: &mut impl fmt::Write,
        eol: Option<Eol>,
    ) -> Option<ChangeSet> {
        let ret = eol.and_then(|eol| self.buffer.set_eols(eol));

        // Output current prevailing EOL
        if self.buffer.is_empty() {
            writeln!(output, "empty buffer").unwrap();
        } else {
            writeln!(output, "prevailing newline: {}", self.buffer.eols())
                .unwrap();
        }

        ret
    }

    fn null_cmd(
        &mut self,
        output: &mut impl fmt::Write,
        index: Option<usize>,
    ) -> Result<Option<ChangeSet>, Error> {
        if index.is_none() {
            if self.buffer.is_empty() {
                return Ok(None);
            }

            if self.buffer.current_index() == self.buffer.len() - 1 {
                return Err(Error::InvalidAddress);
            }
        }

        // Handle default
        let index = index.unwrap_or(self.buffer.current_index() + 1);

        self.buffer.set_current_index(index);
        let pr_attrs = PrintSuffix::default();
        write_line(output, &self.buffer, index, pr_attrs).unwrap();
        Ok(None)
    }

    fn print_cmd(
        &mut self,
        output: &mut impl fmt::Write,
        span: Option<Range<usize>>,
    ) -> Option<ChangeSet> {
        if span.is_none() && self.buffer.is_empty() {
            return None;
        }
        // Handle default
        let span = span.unwrap_or_else(|| self.buffer.current_index_as_range());
        self.buffer.set_current_index(span.end - 1);
        let attributes = PrintSuffix::default();
        for i in span {
            write_line(output, &self.buffer, i, attributes).unwrap();
        }
        None
    }

    fn list_cmd(
        &mut self,
        output: &mut impl fmt::Write,
        span: Option<Range<usize>>,
    ) -> Option<ChangeSet> {
        if span.is_none() && self.buffer.is_empty() {
            return None;
        }

        // Handle default
        let span = span.unwrap_or_else(|| self.buffer.current_index_as_range());
        self.buffer.set_current_index(span.end - 1);
        let attributes =
            PrintSuffix { expand_escapes: true, ..Default::default() };
        for i in span {
            write_line(output, &self.buffer, i, attributes).unwrap();
        }
        None
    }

    fn insert_cmd(
        &mut self,
        input: &mut impl LineEdit,
        output: &mut impl fmt::Write,
        index: Option<usize>,
        source: InputSource,
        mode: InputMode,
    ) -> Result<Option<ChangeSet>, Error> {
        // Handle default
        let index = index.unwrap_or_else(|| self.buffer.current_index());
        let (indent, eol) = match mode {
            // Auto-indent is same as first non-blank line after index.
            InputMode::Cooked => (
                self.buffer[index..]
                    .iter()
                    .find(|l| l.contains(|c: char| !c.is_whitespace()))
                    .and_then(|l| INDENT.captures(l))
                    .and_then(|c| c.get(1))
                    .map(|m| m.as_str().to_owned()),
                Some(self.buffer.eols().prevailing()),
            ),
            InputMode::Raw => (None, None),
        };
        let mut lines = Vec::new();
        match source {
            InputSource::Clipboard => {
                read_lines(&mut lines, &mut Cursor::new(&self.clipboard), eol)?;
            }
            InputSource::File(filename) => {
                read_file_lines(&mut lines, &filename, eol, output)?;
            }
            InputSource::StdIn => {
                read_input_lines(&mut lines, input, indent, eol)?;
            }
        }
        let changes = self.buffer.insert(index, lines);
        Ok((!changes.is_empty()).then_some(changes))
    }

    fn join_cmd(
        &mut self,
        span: Option<Range<usize>>,
        separator: Option<&str>,
    ) -> Result<Option<ChangeSet>, Error> {
        fn simple_join(buffer: &EditBuffer, mut span: Range<usize>) -> String {
            let joined_len =
                buffer[..].iter().fold(0usize, |acc, l| acc + l.len());
            let mut joined_line = String::with_capacity(joined_len);
            let i = span.next().expect("at least 2 lines to join");
            joined_line.push_str(&buffer[i]);
            for line in &buffer[span] {
                let joined_eol = Eol::from_line(&joined_line)
                    .expect("all buffer lines should be terminated");
                let trimmed_len =
                    joined_line.len() - joined_eol.str_value().len();
                joined_line.truncate(trimmed_len);
                joined_line.push_str(line);
            }
            joined_line
        }

        fn separated_join(
            buffer: &EditBuffer,
            mut span: Range<usize>,
            separator: &str,
        ) -> String {
            let joined_len = buffer[span.clone()]
                .iter()
                .fold(0usize, |acc, l| acc + l.len() + separator.len());
            let mut joined_line = String::with_capacity(joined_len);
            let i = span.next().expect("at least 2 lines to join");
            joined_line.push_str(&buffer[i]);
            for line in &buffer[span] {
                let trimmed_len = joined_line.trim_end().len();
                joined_line.truncate(trimmed_len);
                joined_line.push_str(separator);
                joined_line.push_str(line.trim_start());
            }
            joined_line
        }

        // Handle default
        if self.buffer.is_empty() {
            return Err(Error::NothingToJoin);
        }
        let mut span =
            span.unwrap_or_else(|| self.buffer.current_index_as_range());
        // If only one line addressed, join with next line
        if span.end - span.start == 1 {
            span.end += 1;
        }
        // Ensure range within buffer
        if span.contains(&self.buffer.len()) {
            return Err(Error::InvalidAddress);
        }

        let joined_line = if let Some(separator) = separator {
            separated_join(&self.buffer, span.clone(), separator)
        } else {
            simple_join(&self.buffer, span.clone())
        };
        let mut changes =
            ChangeSet::new(self.buffer.current_index(), self.buffer.eols());
        changes.extend(self.buffer.remove(span.clone()));
        changes.extend(self.buffer.insert(span.start, vec![joined_line]));

        Ok(Some(changes))
    }

    fn line_number_cmd(
        &mut self,
        output: &mut impl fmt::Write,
        index: Option<usize>,
    ) {
        match index {
            None if self.buffer.is_empty() => {
                writeln!(output, "empty buffer").unwrap();
            }
            None => {
                writeln!(output, "{}", self.buffer.len()).unwrap();
            }
            Some(index) => {
                writeln!(output, "{}", index + 1).unwrap();
            }
        }
    }

    /// Implements quit command.
    ///
    /// Displays warning and doesn't actually exit if unsaved
    /// buffer changes are detected.
    fn quit_cmd(&mut self) -> Result<Option<ChangeSet>, Error> {
        if self.previous_warning != Some(Warning::QuitUnsaved)
            && self.buffer_is_unsaved()
        {
            return Err(Error::Warning(Warning::QuitUnsaved));
        }
        Err(Error::Quit)
    }

    // New discards the buffer contents and unsets current file
    fn new_cmd(&mut self) -> Result<Option<ChangeSet>, Error> {
        if self.previous_warning == Some(Warning::NewUnsaved)
            && self.buffer_is_unsaved()
        {
            return Err(Error::Warning(Warning::NewUnsaved));
        }

        self.buffer.clear();
        self.current_file = None;
        Ok(None)
    }

    fn write_cmd(
        &mut self,
        output: &mut impl fmt::Write,
    ) -> Result<Option<ChangeSet>, Error> {
        let Some(filename) = self.current_file.as_deref() else {
            return Err(Error::NoFilename);
        };

        if self.previous_warning != Some(Warning::WriteOverwrite) {
            let new_file_md = fs::metadata(filename).ok().map(|md| {
                FileMetadata { len: md.len(), modified: md.modified().ok() }
            });

            if self.file_metadata.is_none() || self.file_metadata != new_file_md
            {
                // metadata changed or unknown, compute new file hash
                let (hash, metadata) = compute_file_hash(filename, new_file_md);
                if hash != self.file_hash {
                    if hash.is_some() {
                        self.file_hash = hash;
                    }
                    if metadata.is_some() {
                        self.file_metadata = metadata;
                    }
                    return Err(Error::Warning(Warning::WriteOverwrite));
                }
            }
        }

        let mut writer = EditedFile::open_or_create(filename)?;
        let span = 0..self.buffer.len();
        write_file(&mut self.buffer, output, span, &mut writer)?;

        // Update metadata & hashes
        self.update_file_metadata();
        self.file_hash = Some(self.buffer.content_hash());
        self.buffer_sync_hash = self.buffer.content_hash();
        Ok(None)
    }

    fn write_as_cmd(
        &mut self,
        output: &mut impl fmt::Write,
        span: Option<Range<usize>>,
        filename: &Path,
    ) -> Result<Option<ChangeSet>, Error> {
        if self.current_file.as_deref() == Some(filename) {
            return Err(Error::WriteAsCurrentFile);
        }

        let overwrite_warning =
            Warning::WriteAsOverwrite(span.clone(), filename.to_owned());
        let mut writer = EditedFile::open_or_create(filename)?;
        if !writer.new_file
            && self.previous_warning.as_ref() != Some(&overwrite_warning)
        {
            if let Err(e) =
                writer.remove_backup().map_err(|e| Error::WriteRemoveBackup {
                    source: Some(Box::new(e)),
                    backup_filename: writer
                        .backup_name()
                        .map(ToOwned::to_owned),
                })
            {
                // write backup file remove error out so not lost
                writeln!(output, "{e}").expect("reliable stdout");
            }
            return Err(Error::Warning(overwrite_warning));
        }

        // Handle default
        let span = span.unwrap_or(0..self.buffer.len());
        let whole_buffer_write = span.end - span.start == self.buffer.len();
        write_file(&mut self.buffer, output, span, &mut writer)?;

        if self.current_file.is_none() && whole_buffer_write {
            // Saving whole buffer for first time
            self.current_file = Some(filename.to_owned());
            self.update_file_metadata();
            self.file_hash = Some(self.buffer.content_hash());
            self.buffer_sync_hash = self.buffer.content_hash();
        }

        Ok(None)
    }
}

/// Main event loop.
///
/// Handles prompting, command input, command dispatch, and error display.
pub fn run(
    mut input: impl LineEdit,
    output: impl Write,
    output_target: OutputTarget,
    args: &cli::CmdArgs,
) -> Result<(), Error> {
    let mut output = FmtWriter(output);
    let mut editor = Editor::new(OutputTarget::Terminal);

    if let Some(file) = &args.file
        && let Err(e) = editor.edit_cmd(&mut output, file)
    {
        writeln!(output, "{e}").unwrap();
    }

    // Accept and process commands until fatal error or exit
    let mut done = false;
    let mut title = String::new();
    while !done {
        if let OutputTarget::Terminal = output_target {
            title.clear();
            title.push_str("lned - ");
            editor.format_file_info(&mut title);
            output.0.execute(terminal::SetTitle(&title)).unwrap();
        }

        Cmd::read(&mut input, &mut editor.buffer, &mut editor.previous_pattern)
            .and_then(|res| match res {
                Some((cmd, pr_sfx)) => {
                    let res =
                        editor.dispatch_cmd(cmd, &mut output.0, &mut input);
                    res.map(|cs| {
                        if let Some(cs) = cs {
                            editor.buffer.push_undo(cs);
                        }
                        editor.previous_warning = None;
                        if let Some(pr_sfx) = pr_sfx {
                            write_line(
                                &mut output,
                                &editor.buffer,
                                editor.buffer.current_index(),
                                pr_sfx,
                            )
                            .unwrap();
                        }
                    })
                }
                _ => Ok(()),
            })
            .or_else(|e| {
                writeln!(output, "{e}").unwrap();
                write_backtrace(&mut output, &e);
                match e {
                    Error::Warning(warning) => {
                        editor.previous_warning = Some(warning);
                    }
                    Error::Quit => done = true,
                    _ => (),
                }
                Ok(())
            })?;
    }
    Ok(())
}

fn write_backtrace(
    output: &mut impl fmt::Write,
    mut err: &dyn std::error::Error,
) {
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
struct PageBounds {
    cols: usize,
    rows: usize,
}

fn adjust_global_list(list: &mut VecDeque<usize>, change: &Change) {
    match change {
        Change::Remove { index, lines } => {
            let end = *index + lines.len();
            list.retain_mut(|n| {
                if *n < *index {
                    true
                } else if *n >= end {
                    *n -= lines.len();
                    true
                } else {
                    false
                }
            });
        }
        Change::Insert { index, lines } => {
            for n in list.iter_mut().filter(|n| **n >= *index) {
                *n += lines.len();
            }
        }
        Change::SetEols { .. } => (), // SetEols doesn't change list
    }
}

fn write_line(
    output: &mut impl fmt::Write,
    buffer: &EditBuffer,
    index: usize,
    attributes: PrintSuffix,
) -> Result<usize, fmt::Error> {
    let line_number = index + 1;
    let mut columns = 0;

    if attributes.enumerate {
        columns = usize::try_from(
            1 + buffer.len().checked_ilog10().unwrap_or_default(),
        )
        .unwrap();
        write!(output, "{line_number:>columns$}  ")?;
        columns += 2;
    }

    if attributes.expand_escapes {
        let graphemes = buffer[index].graphemes(true).map(expand_escapes);
        for gr in graphemes {
            output.write_str(gr)?;
            if gr != "\n" && gr != "\r\n" {
                use unicode_width::UnicodeWidthStr;
                columns += gr.width();
            }
        }
    } else {
        let graphemes = buffer[index].graphemes(true);
        for gr in graphemes {
            if gr == "\t" {
                let tab_width = 8 - (columns % 8);
                write!(output, "{}", &"        "[..tab_width])?;
                columns += tab_width;
            } else {
                output.write_str(gr)?;
                if gr != "\n" && gr != "\r\n" {
                    use unicode_width::UnicodeWidthStr;
                    columns += gr.width();
                }
            }
        }
    }
    Ok(columns)
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

fn read_input_lines(
    buf: &mut Vec<String>,
    input: &mut impl LineEdit,
    indent: Option<String>,
    prevailing_eol: Option<Eol>,
) -> Result<(), Error> {
    let mut text_read_options =
        EditorOptions { prompt: None, history: false, prefill: indent };
    buf.clear();
    loop {
        let mut line = String::new();
        let n = input
            .read_line(&mut line, Some(&text_read_options))
            .map_err(|e| Error::ReadLines { source: Some(Box::new(e)) })?;
        if n == 0 || line == ".\n" || line == ".\r\n" {
            return Ok(());
        }
        if let Some(indent) = text_read_options.prefill.as_mut() {
            indent.replace_range(
                ..,
                INDENT
                    .captures(&line)
                    .and_then(|c| c.get(1))
                    .map_or("", |m| m.as_str()),
            );
        }
        if let Some(prevailing_eol) = prevailing_eol {
            let prevailing_eol = prevailing_eol.str_value();
            let input_eol = Eol::from_line(&line).map_or("", Into::into);
            if input_eol != prevailing_eol {
                line.truncate(line.len() - input_eol.len());
                line.push_str(prevailing_eol);
            }
        }
        buf.push(line);
    }
}

fn read_file_lines(
    lines: &mut Vec<String>,
    filename: &Path,
    eol: Option<Eol>,
    output: &mut impl fmt::Write,
) -> Result<(), Error> {
    let file = File::open(filename);
    let mut source = match file {
        Ok(f) => BufReader::new(f),
        Err(e) => {
            return match e.kind() {
                io::ErrorKind::NotFound => {
                    Err(Error::FileNotFound(filename.to_path_buf()))
                }
                _ => Err(Error::ReadFileOpen {
                    source: Some(Box::new(e)),
                    file: filename.to_path_buf(),
                }),
            };
        }
    };

    let (bytes_read, eol_added) = read_lines(lines, &mut source, eol)?;
    writeln!(output, "{} lines ({bytes_read} bytes) read", lines.len())
        .unwrap();
    if eol_added {
        writeln!(output, "missing newline appended").unwrap();
    }
    Ok(())
}

// Reads lines from `source` into `lines`, adding missing
// linefeed to final line if necessary.
//
// Added linefeed will be `eol` if Some, otherwise it will
// be the prevailing eol for the lines read.
//
// If successful, returns a tuple of the number of bytes read and
// a boolean indicating whether a final linefeed was appended.
//
// If there is an error, it is returned instead.
//
fn read_lines(
    lines: &mut Vec<String>,
    source: &mut impl BufRead,
    eol: Option<Eol>,
) -> Result<(usize, bool), Error> {
    let mut line = String::new();
    let mut bytes_read = 0;
    let mut eol_added = false;
    let mut eols = Eols::new(eol.unwrap_or(Eol::Lf));
    loop {
        let len = source
            .read_line(&mut line)
            .map_err(|e| Error::ReadLines { source: Some(Box::new(e)) })?;
        if len == 0 {
            break;
        }
        bytes_read += len;
        if let Some(line_eol) = Eol::from_line(&line) {
            eols += line_eol;
        } else {
            let eol = eols.prevailing();
            eols += eol;
            line.push_str(eol.into());
            eol_added = true;
        }
        lines.push(line.clone());
        line.clear();
    }

    Ok((bytes_read, eol_added))
}

fn compute_file_hash(
    filename: &Path,
    mut metadata: Option<FileMetadata>,
) -> (Option<u64>, Option<FileMetadata>) {
    fn read_hash(filename: &Path) -> Option<u64> {
        let mut h = DefaultHasher::new();
        let mut line = String::new();
        let Ok(file) = File::open(filename) else {
            return None;
        };
        let mut file = BufReader::new(file);
        loop {
            let Ok(len) = BufRead::read_line(&mut file, &mut line) else {
                return None;
            };
            if len == 0 {
                break;
            }
            line.hash(&mut h);
            line.clear();
        }
        Some(h.finish())
    }

    for _ in 0..3 {
        // try up to 3 times to compute hash
        let hash = read_hash(filename);
        if hash.is_none() {
            continue;
        }
        let check_md = fs::metadata(filename).ok().map(|md| FileMetadata {
            len: md.len(),
            modified: md.modified().ok(),
        });
        if check_md == metadata {
            return (hash, metadata);
        }
        metadata = check_md;
    }
    (None, metadata)
}

fn format_number(val: usize) -> String {
    val.to_string()
        .as_bytes()
        .rchunks(3)
        .rev()
        .map(str::from_utf8)
        .collect::<Result<Vec<&str>, _>>()
        .unwrap()
        .join(",")
}

trait FileWrite {
    fn write(
        &mut self,
        buffer: &mut EditBuffer,
        span: Range<usize>,
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
    new_file: bool,
    backup_filename: Option<PathBuf>,
    backup: Option<File>,
}

impl EditedFile {
    fn open_or_create(filename: &Path) -> Result<EditedFile, Error> {
        match OpenOptions::new().read(true).write(true).open(filename) {
            Ok(file) => {
                let mut backup_filename = filename.to_path_buf();
                backup_filename.as_mut_os_string().push(".bak");
                let backup = File::create_new(backup_filename.as_path())
                    .map_err(|e| Error::WriteBackupFileCreate {
                        source: Some(Box::new(e)),
                        filename: filename.to_path_buf(),
                        backup_filename: Some(backup_filename.clone()),
                    })?;
                Ok(EditedFile {
                    filename: filename.to_path_buf(),
                    file,
                    new_file: false,
                    backup_filename: Some(backup_filename),
                    backup: Some(backup),
                })
            }
            Err(e) => {
                if e.kind() == io::ErrorKind::NotFound {
                    let file = File::create_new(filename).map_err(|e| {
                        Error::WriteFileOpen {
                            source: Some(Box::new(e)),
                            filename: filename.to_path_buf(),
                        }
                    })?;
                    return Ok(EditedFile {
                        filename: filename.to_path_buf(),
                        file,
                        new_file: true,
                        backup_filename: None,
                        backup: None,
                    });
                }
                Err(Error::WriteFileOpen {
                    source: Some(Box::new(e)),
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
        span: Range<usize>,
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

fn version_cmd(output: &mut impl fmt::Write) {
    writeln!(output, "{} {}", cli::APP_NAME, cli::APP_VERSION)
        .expect("reliable stdout");
}

fn write_file(
    buffer: &mut EditBuffer,
    output: &mut impl fmt::Write,
    span: Range<usize>,
    writer: &mut impl FileWrite,
) -> Result<(), Error> {
    writer
        .backup()
        .map_err(|e| Error::WriteMakeBackup {
            source: Some(Box::new(e)),
            filename: writer.name().to_owned(),
            backup_filename: writer.backup_name().map(Path::to_owned),
        })
        .inspect_err(|_| {
            let _ = writer.remove_backup();
        })?;
    let (bytes, lines) =
        writer.write(buffer, span).map_err(|e| Error::WriteFile {
            source: Some(Box::new(e)),
            filename: writer.name().to_owned(),
            backup_filename: writer.backup_name().map(Path::to_owned),
        })?;

    write!(
        output,
        "{} lines ({} bytes) written ",
        format_number(lines),
        format_number(bytes)
    )
    .expect("stdout failure is fatal");
    if buffer.is_empty() {
        writeln!(output, "[empty buffer]").unwrap();
    } else {
        writeln!(output, "[{}]", buffer.eols().prevailing()).unwrap();
    }

    writer.remove_backup().map_err(|e| Error::WriteRemoveBackup {
        source: Some(Box::new(e)),
        backup_filename: writer.backup_name().map(Path::to_path_buf),
    })
}

fn write_lines(
    destination: &mut impl Write,
    buffer: &mut EditBuffer,
    span: Range<usize>,
) -> Result<(usize, usize), io::Error> {
    let mut total_bytes_written = 0;
    let mut lines_written = 0;

    for line in &buffer[span] {
        let bytes_to_write = line.len();
        let mut bytes_written = 0;
        while bytes_written < bytes_to_write {
            bytes_written = bytes_written
                + destination.write(&line.as_bytes()[bytes_written..])?;
        }
        total_bytes_written += bytes_written;
        lines_written += 1;
    }
    destination.flush()?;

    Ok((total_bytes_written, lines_written))
}

struct FmtWriter<W: Write>(W);

impl<W: Write> fmt::Write for FmtWriter<W> {
    fn write_str(&mut self, s: &str) -> Result<(), fmt::Error> {
        self.0.write_all(s.as_bytes()).map_err(|_| std::fmt::Error)
    }
}

const DEFAULT_TERMINAL_SIZE: (usize, usize) =
    if cfg!(target_os = "windows") { (80, 25) } else { (80, 24) };

fn terminal_size() -> (usize, usize) {
    terminal::size().map_or(DEFAULT_TERMINAL_SIZE, |(cols, rows)| {
        (cols.into(), rows.into())
    })
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

    use crate::command;
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
        let mut output = String::new();
        let mut editor = Editor::new(OutputTarget::Other);
        editor.buffer = EditBuffer::with_lines(&["1\n", "2", "3"]);
        editor.buffer.set_current_index(2);
        editor.null_cmd(&mut output, Some(0)).unwrap();
        assert_eq!(editor.buffer.current_index(), 0);
        assert_eq!(&output, "1\n");
    }

    #[test]
    fn null_cmd_no_addr() {
        let mut output = String::new();
        let mut editor = Editor::new(OutputTarget::Other);
        editor.buffer = EditBuffer::with_lines(&["1\r\n", "2", "3"]);
        editor.buffer.set_current_index(1);
        editor.null_cmd(&mut output, None).unwrap();
        assert_eq!(&output, "3\r\n");
        assert_eq!(editor.buffer.current_index(), 2);
    }

    #[test]
    fn null_cmd_no_addr_last_line_gives_error() {
        let mut output = String::new();
        let mut editor = Editor::new(OutputTarget::Other);
        editor.buffer = EditBuffer::with_lines(&["1\r\n", "2", "3"]);
        editor.buffer.set_current_index(2);
        let res =
            editor.null_cmd(&mut output, None).expect_err("invalid address");
        assert!(matches!(res, Error::InvalidAddress));
        assert_eq!(editor.buffer.current_index(), 2);
    }

    #[test]
    fn null_cmd_span() {
        let mut output = String::new();
        let mut editor = Editor::new(OutputTarget::Other);
        editor.buffer =
            EditBuffer::with_lines(&["1\r\n", "2", "3", "4", "5", "6"]);
        editor.buffer.set_current_index(4);
        editor.null_cmd(&mut output, Some(3)).unwrap();
        assert_eq!(output, "4\r\n");
        assert_eq!(editor.buffer.current_index(), 3);
    }

    #[test]
    fn null_cmd_empty_buffer() {
        let mut output = String::new();
        let mut editor = Editor::new(OutputTarget::Other);
        editor.null_cmd(&mut output, None).unwrap();
        assert!(output.is_empty());
    }

    #[test]
    fn enumerate_empty_buffer() {
        let mut output = String::new();
        let mut editor = Editor::new(OutputTarget::Other);
        editor.enumerate_cmd(&mut output, None);
        assert!(output.is_empty());
    }

    #[test]
    fn enumerate_sm_buffer() {
        let mut output = String::new();
        let mut editor = Editor::new(OutputTarget::Other);
        editor.buffer = EditBuffer::with_lines(&[
            "1\r\n", "2", "3", "4", "5", "6", "7", "8", "9", "10",
        ]);
        editor.buffer.set_current_index(1);
        editor.enumerate_cmd(&mut output, None);
        assert_eq!(&output, " 2  2\r\n");
    }

    #[test]
    fn enumerate_sets_current_index() {
        let mut output = String::new();
        let mut editor = Editor::new(OutputTarget::Other);
        editor.buffer = EditBuffer::with_lines(&[
            "1\r\n", "2", "3", "4", "5", "6", "7", "8", "9", "10",
        ]);
        editor.buffer.set_current_index(2);

        editor.enumerate_cmd(&mut output, Some(5..9));
    }

    #[test]
    fn enumerate_lg_buffer() {
        let mut output = String::new();
        let mut editor = Editor::new(OutputTarget::Other);
        editor.buffer = EditBuffer::with_lines(&[
            "1\r\n", "2", "3", "4", "5", "6", "7", "8", "9", "10",
        ]);
        let mut input: Vec<u8> = Vec::new();
        for i in 11..=1024 {
            input.extend_from_slice(format!("{i}\r\n").as_bytes());
        }
        input.extend_from_slice(".\n".as_bytes());
        let mut input = &input[..];
        editor
            .append_cmd(
                &mut input,
                &mut output,
                Some(editor.buffer.len() - 1),
                InputSource::StdIn,
                InputMode::Raw,
            )
            .unwrap();
        editor.buffer.set_current_index(2);
        assert_eq!(1024, editor.buffer.len());
        output.clear();

        editor.enumerate_cmd(&mut output, Some(3..900));
        let expected = "   4  4\r\n";
        assert_eq!(expected, &output[0..expected.len()]);
        output.clear();

        editor.enumerate_cmd(&mut output, Some(998..999));
        let expected = " 999  999\r\n";
        assert_eq!(expected, &output[0..expected.len()]);
    }

    #[test]
    fn print_filename_none_set() {
        let mut editor = Editor::new(OutputTarget::Other);
        editor.buffer = EditBuffer::with_lines(&["1\r\n", "2", "3"]);
        let mut output = String::new();
        editor.file_cmd(&mut output);
        let expected = "no filename set [unsaved] [CRLF]\n";
        assert_eq!(&output, expected);
        assert!(editor.current_file.is_none());
    }

    #[test]
    fn print_filename() {
        let mut editor = Editor::new(OutputTarget::Other);
        editor.buffer = EditBuffer::with_lines(&["1\n", "2", "3"]);
        editor.current_file = Some(PathBuf::from("a_new_filename.txt"));
        let mut output = String::new();
        editor.file_cmd(&mut output);
        output.clear();
        editor.file_cmd(&mut output);
        let expected = "a_new_filename.txt [unsaved] [LF]\n";
        assert_eq!(&output, expected);
    }

    #[test]
    fn global_cmd_empty_buffer() {
        let mut editor = Editor::new(OutputTarget::Other);
        let mut output = String::new();
        let commands = "n\n".to_owned();
        let res = editor
            .global_cmd(
                &mut output,
                None,
                &Regex::new("no match").unwrap(),
                &commands,
            )
            .expect_err("no match");
        assert!(matches!(res, Error::NoMatch));
    }

    #[test]
    fn global_cmd_no_matches() {
        let mut editor = Editor::new(OutputTarget::Other);
        editor.buffer = EditBuffer::with_lines(&["one\n", "two", "three"]);
        let mut output = String::new();
        let pat = &Regex::new("four").unwrap();
        let commands = "p\n".to_owned();
        let res = editor
            .global_cmd(&mut output, None, pat, &commands)
            .expect_err("no match");
        assert!(matches!(res, Error::NoMatch));
        assert!(output.is_empty());
    }

    #[test]
    fn global_cmd_illegal_nested_gobal() {
        let mut editor = Editor::new(OutputTarget::Other);
        editor.buffer = EditBuffer::with_lines(&["one\r\n", "two", "three"]);
        editor.buffer.set_current_index(1);
        let mut output = String::new();
        let pat = &Regex::new("t..").unwrap();
        let commands = "1,2g/ee/n\n".to_owned();
        let res = editor.global_cmd(&mut output, None, pat, &commands);
        assert!(matches!(res, Err(Error::NestedGlobalCmd)));
    }

    #[test]
    fn global_cmd_blank_command_print() {
        let mut editor = Editor::new(OutputTarget::Other);
        editor.buffer =
            EditBuffer::with_lines(&["one\r\n", "two", "three", "tweedle dee"]);
        editor.buffer.set_current_index(3);
        let mut output = String::new();
        let pat = &Regex::new("t..").unwrap();
        let commands = "\n".to_owned();
        let res =
            editor.global_cmd(&mut output, Some(0..3), pat, &commands).unwrap();
        assert!(res.is_none(), "should be no changes");
        assert_eq!(&output, "two\r\nthree\r\n");
    }

    #[test]
    fn global_cmd_print() {
        let mut editor = Editor::new(OutputTarget::Other);
        editor.buffer = EditBuffer::with_lines(&["one\n", "two", "three"]);
        editor.buffer.set_current_index(1);
        let mut output = String::new();
        let pat = &Regex::new("t..").unwrap();
        let commands = "p\r\n".to_owned();
        let res = editor
            .global_cmd(&mut output, None, pat, &commands)
            .expect("no errors");
        assert!(res.is_none(), "should be no changes");
        assert_eq!(&output, "two\nthree\n");
    }

    #[test]
    fn global_cmd_enumerate() {
        let mut editor = Editor::new(OutputTarget::Other);
        editor.buffer = EditBuffer::with_lines(&["one\n", "two", "three"]);
        editor.buffer.set_current_index(0);
        let mut output = String::new();
        let pat = &Regex::new("t..").unwrap();
        let commands = "n\r\n".to_owned();
        let res = editor
            .global_cmd(&mut output, Some(0..3), pat, &commands)
            .expect("no error");
        assert!(res.is_none(), "should be no changes");
        assert_eq!(&output, "2  two\n3  three\n");
    }

    #[test]
    fn global_cmd_enumerate_with_addresses() {
        let mut editor = Editor::new(OutputTarget::Other);
        editor.buffer = EditBuffer::with_lines(&[
            "one\n", "two", "three", "four", "five", "six",
        ]);
        editor.buffer.set_current_index(5);
        let mut output = String::new();
        let pat = &Regex::new("e$").unwrap();
        let commands = "-1,.n\r\n".to_owned();
        let res = editor
            .global_cmd(&mut output, Some(1..5), pat, &commands)
            .expect("no error");
        assert!(res.is_none(), "should be no changes");
        assert_eq!(&output, "2  two\n3  three\n4  four\n5  five\n");
    }

    #[test]
    fn global_cmd_list() {
        let mut editor = Editor::new(OutputTarget::Other);
        editor.buffer = EditBuffer::with_lines(&["one\n", "two", "three"]);
        editor.buffer.set_current_index(1);
        let mut output = String::new();
        let pat = &Regex::new("t..").unwrap();
        let commands = "l\r\n".to_owned();
        let res = editor
            .global_cmd(&mut output, Some(0..3), pat, &commands)
            .expect("no error");
        assert!(res.is_none(), "should be no changes");
        assert_eq!(&output, "two\\n$\nthree\\n$\n");
    }

    #[test]
    fn global_cmd_list_with_addresses() {
        let mut editor = Editor::new(OutputTarget::Other);
        editor.buffer = EditBuffer::with_lines(&[
            "one\n", "two", "three", "four", "five", "six",
        ]);
        editor.buffer.set_current_index(5);
        let mut output = String::new();
        let pat = &Regex::new("e$").unwrap();
        let commands = "-1,.l\r\n".to_owned();
        let res = editor
            .global_cmd(&mut output, Some(1..5), pat, &commands)
            .expect("no error");
        assert!(res.is_none(), "should be no changes");
        assert_eq!(&output, "two\\n$\nthree\\n$\nfour\\n$\nfive\\n$\n");
    }
    #[test]
    fn global_cmd_copy() {
        let mut editor = Editor::new(OutputTarget::Other);
        editor.buffer = EditBuffer::with_lines(&[
            "one\n", "two", "three", "four", "five", "six",
        ]);
        let orig = editor.buffer.clone();
        let expected = EditBuffer::with_lines(&[
            "three\n", "two", "one", "two", "three", "four", "five", "six",
        ]);
        let mut output = String::new();
        let pat = Regex::new("^t").unwrap();
        let commands = "c\n1iv\n".to_owned();
        let changes = editor
            .global_cmd(&mut output, Some(0..6), &pat, &commands)
            .expect("no error")
            .expect("some changes");
        assert!(!changes.is_empty());
        assert_eq!(editor.clipboard, "three\n");
        assert_eq!(&editor.buffer[..], &expected[..]);
        assert_eq!(editor.buffer.current_index(), 0);
        editor.buffer.push_undo(changes);

        editor.buffer.undo().expect("something to undo");
        assert_eq!(&editor.buffer[..], &orig[..]);
        assert_eq!(&editor.clipboard, "three\n");
        assert_eq!(editor.buffer.current_index(), orig.current_index());

        editor.buffer.redo().expect("successful redo");
        assert_eq!(&editor.buffer[..], &expected[..]);
        assert_eq!(editor.buffer.current_index(), 0);
        assert_eq!(editor.clipboard, "three\n");
    }

    #[test]
    fn global_cmd_cut() {
        let mut editor = Editor::new(OutputTarget::Other);
        editor.buffer = EditBuffer::with_lines(&[
            "one\n", "two", "three", "four", "five", "six",
        ]);
        let orig = editor.buffer.clone();
        let expected = EditBuffer::with_lines(&[
            "three\n", "two", "one", "four", "five", "six",
        ]);
        let mut output = String::new();
        let pat = Regex::new("^t").unwrap();
        let commands = "x\n1iv\n".to_owned();
        let changes = editor
            .global_cmd(&mut output, Some(0..6), &pat, &commands)
            .expect("no error")
            .expect("some changes");
        assert!(!changes.is_empty());
        assert_eq!(editor.clipboard, "three\n");
        assert_eq!(&editor.buffer[..], &expected[..]);
        assert_eq!(editor.buffer.current_index(), 0);
        editor.buffer.push_undo(changes);

        editor.buffer.undo().expect("something to undo");
        assert_eq!(&editor.buffer[..], &orig[..]);
        assert_eq!(&editor.clipboard, "three\n");
        assert_eq!(editor.buffer.current_index(), orig.current_index());

        editor.buffer.redo().expect("successful redo");
        assert_eq!(&editor.buffer[..], &expected[..]);
        assert_eq!(editor.buffer.current_index(), 0);
        assert_eq!(editor.clipboard, "three\n");
    }

    #[test]
    fn global_cmd_append() {
        let mut editor = Editor::new(OutputTarget::Other);
        editor.buffer = EditBuffer::with_lines(&[
            "one\n", "two", "three", "four", "five", "six",
        ]);
        let orig = editor.buffer.clone();
        let expected = EditBuffer::with_lines(&[
            "one\n", "append", "two", "three", "append", "four", "five",
            "append", "six",
        ]);
        let mut output = String::new();
        let pat = &Regex::new("e$").unwrap();
        let commands = "a\nappend\n.\n".to_owned();
        let changes = editor
            .global_cmd(&mut output, Some(0..6), pat, &commands)
            .expect("no error")
            .expect("some changes");
        assert!(!changes.is_empty());
        assert_eq!(&editor.buffer[..], &expected[..]);
        assert_eq!(editor.buffer.current_index(), 7);
        editor.buffer.push_undo(changes);

        // now undo
        editor.buffer.undo().expect("something there to undo");
        assert_eq!(&editor.buffer[..], &orig[..]);
        assert_eq!(editor.buffer.current_index(), orig.current_index());

        // redo
        editor.buffer.redo().expect("something there to undo");
        assert_eq!(&editor.buffer[..], &expected[..]);
        assert_eq!(editor.buffer.current_index(), 7);
    }

    #[test]
    fn global_cmd_overwrite() {
        let mut editor = Editor::new(OutputTarget::Other);
        editor.buffer = EditBuffer::with_lines(&[
            "one\n", "one", "two", "two", "three", "three", "four", "four",
            "five", "five", "six", "six",
        ]);
        let orig = editor.buffer.clone();
        let expected = EditBuffer::with_lines(&[
            "overwrite 1\n",
            "overwrite 2",
            "overwrite 3",
            "two",
            "two",
            "overwrite 1",
            "overwrite 2",
            "overwrite 3",
            "four",
            "four",
            "five",
            "five",
            "six",
            "six",
        ]);
        let mut output = String::new();
        let pat = &Regex::new("([a-z]*e)$").unwrap();
        let commands =
            ".,+o\noverwrite 1\noverwrite 2\noverwrite 3\n.\n".to_owned();
        let Ok(Some(changes)) =
            editor.global_cmd(&mut output, Some(0..6), pat, &commands)
        else {
            panic!("global_cmd's err return wasn't None!")
        };
        assert!(!changes.is_empty());
        editor.buffer.push_undo(changes);
        assert_eq!(&editor.buffer[..], &expected[..]);
        assert_eq!(editor.buffer.current_index(), 7);

        // now undo
        editor.buffer.undo().expect("something there to undo");
        assert_eq!(&editor.buffer[..], &orig[..]);
        assert_eq!(editor.buffer.current_index(), orig.current_index());

        // redo
        editor.buffer.redo().expect("something there to undo");
        assert_eq!(&editor.buffer[..], &expected[..]);
        assert_eq!(editor.buffer.current_index(), 7);
    }

    #[test]
    fn global_cmd_delete() {
        let mut editor = Editor::new(OutputTarget::Other);
        editor.buffer = EditBuffer::with_lines(&[
            "one\n", "two", "three", "four", "five", "six",
        ]);
        let orig = editor.buffer.clone();
        let expected = EditBuffer::with_lines(&["two\n", "four", "six"]);
        let mut output = String::new();
        let pat = &Regex::new("e$").unwrap();
        let commands = "dn\n".to_owned();
        let Ok(Some(changes)) =
            editor.global_cmd(&mut output, Some(0..6), pat, &commands)
        else {
            panic!("global_cmd err return wasn't None!");
        };
        assert!(!changes.is_empty());
        editor.buffer.push_undo(changes);
        assert_eq!(&output, "1  two\n2  four\n3  six\n");
        assert_eq!(&editor.buffer[..], &expected[..]);
        assert_eq!(editor.buffer.current_index(), 2);

        // now undo
        editor.buffer.undo().expect("something there to undo");
        assert_eq!(&editor.buffer[..], &orig[..]);
        assert_eq!(editor.buffer.current_index(), orig.current_index());

        // redo
        editor.buffer.redo().expect("something there to undo");
        assert_eq!(&editor.buffer[..], &expected[..]);
        assert_eq!(editor.buffer.current_index(), 2);
    }

    #[test]
    fn global_cmd_insert() {
        let mut editor = Editor::new(OutputTarget::Other);
        editor.buffer = EditBuffer::with_lines(&[
            "one\r\n", "two", "three", "four", "five", "six",
        ]);
        let orig = editor.buffer.clone();
        let expected = EditBuffer::with_lines(&[
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
        let mut output = String::new();
        let pat = &Regex::new("e$").unwrap();
        let commands = "i\r\ninsert\r\n.\r\n".to_owned();
        let Ok(Some(changes)) =
            editor.global_cmd(&mut output, Some(0..6), pat, &commands)
        else {
            panic!("global_cmd returned an unexpected error!");
        };
        assert!(!changes.is_empty());
        editor.buffer.push_undo(changes);
        assert_eq!(&editor.buffer[..], &expected[..]);
        assert_eq!(editor.buffer.current_index(), 6);

        // now undo
        editor.buffer.undo().expect("something there to undo");
        assert_eq!(&editor.buffer[..], &orig[..]);
        assert_eq!(editor.buffer.current_index(), orig.current_index());

        // redo
        editor.buffer.redo().expect("something there to undo");
        assert_eq!(&editor.buffer[..], &expected[..]);
        assert_eq!(editor.buffer.current_index(), 6);
    }

    #[test]
    fn global_cmd_join() {
        let mut editor = Editor::new(OutputTarget::Other);
        editor.buffer = EditBuffer::with_lines(&[
            "one\n", "two", "three", "four", "five", "six",
        ]);
        let orig = editor.buffer.clone();
        let mut expected =
            EditBuffer::with_lines(&["onetwo\n", "threefour", "fivesix"]);
        expected.set_current_index(1);
        let mut output = String::new();
        let pat = &Regex::new("e$").unwrap();
        let commands = "jn\n".to_owned();
        let res = editor.global_cmd(&mut output, Some(0..6), pat, &commands);
        let changes = match res {
            Err(e) => panic!("unexpected error {e:?}"),
            Ok(None) => panic!("should have returned Some(ChangeSet)"),
            Ok(Some(changes)) => changes,
        };
        assert!(!changes.is_empty());
        editor.buffer.push_undo(changes);
        assert_eq!(&output, "1  onetwo\n2  threefour\n3  fivesix\n");
        assert_eq!(&editor.buffer[..], &expected[..]);
        assert_eq!(editor.buffer.current_index(), 2);

        // now undo
        editor.buffer.undo().expect("something there to undo");
        assert_eq!(&editor.buffer[..], &orig[..]);
        assert_eq!(editor.buffer.current_index(), orig.current_index());

        // redo
        editor.buffer.redo().expect("something there to undo");
        assert_eq!(&editor.buffer[..], &expected[..]);
        assert_eq!(editor.buffer.current_index(), 2);
    }

    #[test]
    fn global_cmd_substitute_with_error() {
        let mut editor = Editor::new(OutputTarget::Other);
        editor.buffer = EditBuffer::with_lines(&[
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
        editor.buffer.set_current_index(5);
        let before = editor.buffer.clone();
        let mut expected = EditBuffer::with_lines(&[
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
        expected.set_current_index(11);
        let expected_output = " 6  'xteen\n10  'xteen\n";

        let mut output = String::new();
        let pat = &Regex::new("s[aeiou]").unwrap();
        let commands = ".,+2s//\\\n'/n".to_string();
        let Err(Error::GlobalCmdErrorStop { source, changes }) =
            editor.global_cmd(&mut output, None, pat, &commands)
        else {
            panic!("should have returned GlobalCmdErrorStop");
        };
        if let Error::ReadGlobalCmd { source, .. } = *source {
            assert!(source.is_some_and(|e| matches!(
                *e.downcast::<Error>().unwrap(),
                Error::InvalidAddress
            )));
        } else {
            panic!("expected Error::ReadGlobalCmd");
        }
        let Some(changes) = changes else {
            panic!("changes was None!");
        };
        assert_eq!(output, expected_output);
        editor.buffer.push_undo(changes);
        assert_eq!(editor.buffer.current_index(), expected.current_index());
        assert_eq!(&editor.buffer[..], &expected[..]);
        editor.buffer.undo().unwrap();
        assert_eq!(editor.buffer.current_index(), before.current_index());
        assert_eq!(&before[..], &editor.buffer[..]);
        editor.buffer.redo().unwrap();
        assert_eq!(editor.buffer.current_index(), expected.current_index());
        assert_eq!(&editor.buffer[..], &expected[..]);
    }

    #[test]
    fn global_cmd_substitute() {
        let mut editor = Editor::new(OutputTarget::Other);
        editor.buffer = EditBuffer::with_lines(&[
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
        editor.buffer.set_current_index(5);
        let before = editor.buffer.clone();
        let mut expected = EditBuffer::with_lines(&[
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
        expected.set_current_index(12);
        let expected_output = " 3  'x seven eight\n 6  'xteen\n 8  'venteen eighteen nineteen twenty\n10  'xteen\n13  'x seven eight\n";

        let mut output = String::new();
        let pat = &Regex::new("s[aeiou]").unwrap();
        let commands = "s//\\\n'/n".to_string();
        let Some(changes) = editor
            .global_cmd(&mut output, None, pat, &commands)
            .expect("should have been Ok")
        else {
            panic!("should have been Some(changes)!");
        };
        assert_eq!(output, expected_output);
        assert!(!changes.is_empty());
        editor.buffer.push_undo(changes);
        assert_eq!(editor.buffer.current_index(), expected.current_index());
        assert_eq!(&editor.buffer[..], &expected[..]);
        editor.buffer.undo().unwrap();
        assert_eq!(editor.buffer.current_index(), before.current_index());
        assert_eq!(&before[..], &editor.buffer[..]);
        editor.buffer.redo().unwrap();
        assert_eq!(editor.buffer.current_index(), expected.current_index());
        assert_eq!(&editor.buffer[..], &expected[..]);
    }

    #[test]
    fn global_cmd_unsupported_commands() {
        let mut editor = Editor::new(OutputTarget::Other);
        editor.buffer = EditBuffer::with_lines(&["one\r\n", "two", "three"]);
        editor.buffer.set_current_index(1);
        let mut output = String::new();
        let pat = &Regex::new(r"t..").unwrap();
        let commands = "e filename.txt\n".to_owned();
        let res = editor.global_cmd(&mut output, Some(0..3), pat, &commands);
        assert!(matches!(res, Err(Error::UnsupportedGlobalCmd)));
    }

    #[test]
    fn print_cmd_no_addr() {
        let mut output = String::new();
        let mut editor = Editor::new(OutputTarget::Other);
        editor.buffer = EditBuffer::with_lines(&["1\r\n", "2", "3"]);
        editor.buffer.set_current_index(1);
        editor.print_cmd(&mut output, None);
        assert_eq!(&output[..], "2\r\n");
    }

    #[test]
    fn print_cmd_single_line() {
        let mut output = String::new();
        let mut editor = Editor::new(OutputTarget::Other);
        editor.buffer = EditBuffer::with_lines(&["1\r\n", "2", "3"]);
        editor.buffer.set_current_index(2);
        editor.print_cmd(&mut output, Some(2..3));
        assert_eq!(&output[..], "3\r\n");
    }

    #[test]
    fn print_cmd_span() {
        let mut output = String::new();
        let mut editor = Editor::new(OutputTarget::Other);
        editor.buffer =
            EditBuffer::with_lines(&["1\r\n", "2", "3", "4", "5", "6"]);
        editor.buffer.set_current_index(5);
        editor.print_cmd(&mut output, Some(1..4));
        assert_eq!(&output, "2\r\n3\r\n4\r\n");
    }

    #[test]
    fn print_cmd_sets_current_index() {
        let mut output = String::new();
        let mut editor = Editor::new(OutputTarget::Other);
        editor.buffer =
            EditBuffer::with_lines(&["1\r\n", "2", "3", "4", "5", "6"]);
        editor.buffer.set_current_index(0);
        editor.print_cmd(&mut output, Some(1..4));
        assert_eq!(3, editor.buffer.current_index());
    }

    #[test]
    fn quit_cmd_twice_exits() {
        let input = b"a\n1\n2\n3\n.\nq\nq\n";
        let mut output = Vec::new();

        run(&input[..], &mut output, OutputTarget::Other, &CmdArgs::default())
            .unwrap();
        let output = str::from_utf8(&output[..]).unwrap();
        assert!(
            output.contains(
                "unsaved changes - repeat command to discard changes"
            )
        );
        assert!(output.contains("exiting ..."));
    }

    #[test]
    fn print_cmd_empty_buffer() {
        let mut output = String::new();
        let mut editor = Editor::new(OutputTarget::Other);
        editor.buffer = EditBuffer::new();
        editor.print_cmd(&mut output, None);
        assert!(output.is_empty());
    }

    #[test]
    fn edit_cmd_twice_overrides_warning() {
        let input =
            b"a\n1\n2\n3\n.\ne test/assets/text_with_final_eol.txt\ne test/assets/text_with_final_eol.txt\nq\nq\n";
        let mut output = Vec::new();

        run(&input[..], &mut output, OutputTarget::Other, &CmdArgs::default())
            .unwrap();
        let output = str::from_utf8(&output[..]).unwrap();
        let warning_count = output
            .matches("unsaved changes - repeat command to discard changes")
            .count();
        assert_eq!(warning_count, 1);
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
        run(&input[..], &mut output, OutputTarget::Other, &args).unwrap();
        let output = str::from_utf8(&output[..]).unwrap();
        assert!(output.contains("312"));
    }

    #[test]
    fn file_on_cmd_line_not_found() {
        let args = cli::CmdArgs { file: Some(PathBuf::from("not_a_file")) };
        let input = b"q\n";
        let mut output = Vec::new();
        run(&input[..], &mut output, OutputTarget::Other, &args).unwrap();
        let output = str::from_utf8(&output[..]).unwrap();
        assert!(output.contains("not found"));
    }

    #[test]
    fn append_cmd_dispatch() {
        let input = b"a\none\ntwo\nthree\n.\n2p\nq\nq\n";
        let mut output = Vec::new();
        run(&input[..], &mut output, OutputTarget::Other, &CmdArgs::default())
            .unwrap();
        let output = str::from_utf8(&output[..]).unwrap();
        assert!(output.contains("two\n"));
        assert!(output.contains("unsaved changes"));
        assert!(!output.contains("one"));
    }

    #[test]
    fn append_raw_cmd_dispatch() {
        let input = b"a\n    one\n    two\n    three\n.\n2A\nappended\n.\n2p\n3p\nq\nq\n";
        let mut output = Vec::new();
        run(&input[..], &mut output, OutputTarget::Other, &CmdArgs::default())
            .unwrap();
        let output = str::from_utf8(&output[..]).unwrap();
        assert!(output.contains("    two\n"));
        assert!(output.contains("appended"));
        assert!(!output.contains(" appended"));
        assert!(output.contains("unsaved changes"));
        assert!(!output.contains("one"));
    }

    #[test]
    fn append_cmd_paste_dispatch() {
        let input = b"a\n1\n2\n3\n4\n5\n6\n.\n4,6x\n1Av\n1,$p\nq\nq\n";
        let mut output = Vec::new();
        run(&input[..], &mut output, OutputTarget::Other, &CmdArgs::default())
            .unwrap();
        let output = str::from_utf8(&output[..]).unwrap();
        assert!(output.contains("unsaved changes"));
        let expected = "1\n4\n5\n6\n2\n3\n";
        assert!(output.contains("unsaved changes"));
        assert!(
            output.contains(expected),
            "expected {expected:?}\n\tin {output:?}"
        );
    }

    #[test]
    fn append_cmd_dispatch_p_print_sfx() {
        let input = b"ap\none\ntwo\nthree\n.\n2p\nq\nq\n";
        let mut output = Vec::new();
        run(&input[..], &mut output, OutputTarget::Other, &CmdArgs::default())
            .unwrap();
        let output = str::from_utf8(&output[..]).unwrap();
        assert!(output.contains("two\n"));
        assert!(output.contains("unsaved changes"));
        assert!(!output.contains("one"));
        assert!(output.contains("three\n"));
    }

    #[test]
    fn append_cmd_dispatch_n_print_sfx() {
        let input = b"an\none\ntwo\nthree\n.\n2p\nq\nq\n";
        let mut output = Vec::new();
        run(&input[..], &mut output, OutputTarget::Other, &CmdArgs::default())
            .unwrap();
        let output = str::from_utf8(&output[..]).unwrap();
        assert!(output.contains("two\n"));
        assert!(output.contains("unsaved changes"));
        assert!(!output.contains("one"));
        assert!(output.contains("3  three\n"));
    }

    #[test]
    fn append_cmd_dispatch_l_print_sfx() {
        let input = b"al\none\ntwo\nthree\n.\n2p\nq\nq\n";
        let mut output = Vec::new();
        run(&input[..], &mut output, OutputTarget::Other, &CmdArgs::default())
            .unwrap();
        let output = str::from_utf8(&output[..]).unwrap();
        assert!(output.contains("two\n"));
        assert!(output.contains("unsaved changes"));
        assert!(!output.contains("one"));
        assert!(output.contains("three\\n$\n"));
    }

    #[test]
    fn delete_cmd_dispatch() {
        let input = b"a\none\ntwo\nthree\n.\n1,2d\np\nd\np\nq\nq\n";
        let mut output = Vec::new();
        run(&input[..], &mut output, OutputTarget::Other, &CmdArgs::default())
            .unwrap();
        let output = str::from_utf8(&output[..]).unwrap();
        assert!(output.contains("three"));
    }

    #[test]
    fn overwrite_cmd_dispatch() {
        let input = b"a\n1\n2\n3\n4\n.\n2,3o\na\nb\n.\n1,$p\nq\nq\n";
        let mut output = Vec::new();
        run(&input[..], &mut output, OutputTarget::Other, &CmdArgs::default())
            .unwrap();
        let output = str::from_utf8(&output[..]).unwrap();
        assert!(output.contains("1\na\nb\n4\n"));
    }

    #[test]
    fn overwrite_raw_cmd_dispatch() {
        let input =
            b"a\n    1\n    2\n    3\n    4\n.\n2,3O\na\nb\n.\n1,$p\nq\nq\n";
        let mut output = Vec::new();
        run(&input[..], &mut output, OutputTarget::Other, &CmdArgs::default())
            .unwrap();
        let output = str::from_utf8(&output[..]).unwrap();
        assert!(output.contains("    1\na\nb\n    4\n"));
    }

    #[test]
    fn overwrite_cmd_paste_dispatch() {
        let input = b"a\n1\n2\n3\n4\n5\n6\n.\n5,6c\n1,2Ov\n1,$p\nq\nq\n";
        let mut output = Vec::new();
        run(&input[..], &mut output, OutputTarget::Other, &CmdArgs::default())
            .unwrap();
        let output = str::from_utf8(&output[..]).unwrap();
        assert!(output.contains("unsaved changes"));
        let expected = "5\n6\n3\n4\n5\n6\n";
        assert!(output.contains("unsaved changes"));
        assert!(
            output.contains(expected),
            "expected {expected:?}\n\tin {output:?}"
        );
    }

    #[test]
    fn edit_cmd_dispatch() {
        let input = b"e test/assets/text_with_final_eol.txt\nq\n";
        let mut output = Vec::new();
        run(&input[..], &mut output, OutputTarget::Other, &CmdArgs::default())
            .unwrap();
        let output = str::from_utf8(&output[..]).unwrap();
        assert!(output.contains("312"));
    }

    #[test]
    fn enumerate_cmd_dispatch() {
        let input = b"a\none\ntwo\nthree\n.\n2,3n\nq\nq\n";
        let mut output = Vec::new();
        run(&input[..], &mut output, OutputTarget::Other, &CmdArgs::default())
            .unwrap();
        let output = str::from_utf8(&output[..]).unwrap();
        assert!(output.contains("2  two\n3  three\n"));
    }

    #[test]
    fn file_cmd_dispatch() {
        let input = b"f\nq\n";
        let mut output = Vec::new();
        let args = CmdArgs { file: None };
        run(&input[..], &mut output, OutputTarget::Other, &args).unwrap();
        let output = str::from_utf8(&output[..]).unwrap();
        assert!(output.contains("no filename set"));
    }

    #[test]
    fn insert_cmd_dispatch() {
        let input = b"i\none\ntwo\nthree\n.\n2p\nq\nq\n";
        let mut output = Vec::new();
        run(&input[..], &mut output, OutputTarget::Other, &CmdArgs::default())
            .unwrap();
        let output = str::from_utf8(&output[..]).unwrap();
        assert!(output.contains("two\n"));
        assert!(output.contains("unsaved changes"));
        assert!(!output.contains("one"));
    }

    #[test]
    fn insert_raw_cmd_dispatch() {
        let input = b"a\n    one\n    two\n    three\n.\n3I\ninserted\n.\n2p\n3p\nq\nq\n";
        let mut output = Vec::new();
        run(&input[..], &mut output, OutputTarget::Other, &CmdArgs::default())
            .unwrap();
        let output = str::from_utf8(&output[..]).unwrap();
        assert!(output.contains("two\n"));
        assert!(output.contains("unsaved changes"));
        assert!(output.contains("inserted"));
        assert!(!output.contains(" inserted"));
        assert!(!output.contains("one"));
    }

    #[test]
    fn insert_cmd_paste_dispatch() {
        let input = b"a\n1\n2\n3\n4\n5\n6\n.\n4,6x\n1Iv\n1,$p\nq\nq\n";
        let mut output = Vec::new();
        run(&input[..], &mut output, OutputTarget::Other, &CmdArgs::default())
            .unwrap();
        let output = str::from_utf8(&output[..]).unwrap();
        assert!(output.contains("unsaved changes"));
        let expected = "4\n5\n6\n1\n2\n3\n";
        assert!(output.contains("unsaved changes"));
        assert!(
            output.contains(expected),
            "expected {expected:?}\n\tin {output:?}"
        );
    }

    #[test]
    fn global_cmd_dispatch() {
        let input = b"a\none\ntwo\nthree\nfour\nfive\n.\ng/e$/n\nq\nq\n";
        let mut output = Vec::new();
        run(&input[..], &mut output, OutputTarget::Other, &CmdArgs::default())
            .unwrap();
        let output = str::from_utf8(&output[..]).unwrap();
        assert!(output.contains("1  one\n3  three\n5  five\n"));
    }

    #[test]
    fn join_cmd_dispatch() {
        let input = b"a\n1\n2\n3\n4\n.\n1,2j\n1,$p\nq\nq\n";
        let mut output = Vec::new();
        run(&input[..], &mut output, OutputTarget::Other, &CmdArgs::default())
            .unwrap();
        let output = str::from_utf8(&output[..]).unwrap();
        assert!(output.contains("12\n3\n4\n"));
    }

    #[test]
    fn list_cmd_dispatch() {
        let input = b"a\n1\n2\n3\n4\n.\n1,2l\nq\nq\n";
        let mut output = Vec::new();
        run(&input[..], &mut output, OutputTarget::Other, &CmdArgs::default())
            .unwrap();
        let output = str::from_utf8(&output[..]).unwrap();
        assert!(output.contains("1\\n$\n2\\n$\n"));
    }

    #[test]
    fn line_number_cmd_dispatch() {
        let input = b"a\none\ntwo\nthree\nfour\n.\n2n\n=\n.=\nq\nq\n";
        let mut output = Vec::new();
        run(&input[..], &mut output, OutputTarget::Other, &CmdArgs::default())
            .unwrap();
        let output = str::from_utf8(&output[..]).unwrap();
        assert!(output.contains("2\n"));
        assert!(output.contains("4\n"));
    }

    #[test]
    fn newline_cmd_dispatch() {
        let input = b"a\n1\n2\n3\n.\nL\nq\nq\n";
        let mut output = Vec::new();
        run(&input[..], &mut output, OutputTarget::Other, &CmdArgs::default())
            .unwrap();
        let output = str::from_utf8(&output[..]).unwrap();
        assert!(output.contains("prevailing newline: LF"));
    }

    #[test]
    fn null_cmd_dispatch() {
        let input = b"a\r\none\r\ntwo\r\nthree\r\n.\r\n1\r\n\r\nq\r\nq\r\n";
        let mut output = Vec::new();
        run(&input[..], &mut output, OutputTarget::Other, &CmdArgs::default())
            .unwrap();
        let output = str::from_utf8(&output[..]).unwrap();
        assert!(output.contains("one"));
    }

    #[test]
    fn print_cmd_dispatch() {
        let input = b"a\none\ntwo\nthree\n.\n1,2p\nq\nq\n";
        let mut output = Vec::new();
        run(&input[..], &mut output, OutputTarget::Other, &CmdArgs::default())
            .unwrap();
        let output = str::from_utf8(&output[..]).unwrap();
        assert!(output.contains("one\ntwo\n"));
    }

    #[test]
    fn quit_cmd_dispatch() {
        let input = b"q\r\n";
        let mut output = Vec::new();
        run(&input[..], &mut output, OutputTarget::Other, &CmdArgs::default())
            .unwrap();
    }

    #[test]
    fn append_cmd_from_file_dispatch() {
        let input = b"a\npre 1\npre 2\npost 1\npost 2\n.\n2A test/assets/text_with_final_eol.txt\nq\nq\n";
        let mut output = Vec::new();
        run(&input[..], &mut output, OutputTarget::Other, &CmdArgs::default())
            .unwrap();
        let output = str::from_utf8(&output[..]).unwrap();
        assert!(output.contains("312"));
    }

    #[test]
    fn version_cmd_dispatch() {
        let input = b"#\nq";
        let mut output = Vec::new();
        run(&input[..], &mut output, OutputTarget::Other, &CmdArgs::default())
            .unwrap();
        let output = str::from_utf8(&output[..]).unwrap();
        assert!(output.contains(cli::APP_VERSION));
    }

    #[test]
    fn write_cmd_dispatch() {
        let input = b"a\none\n.\nw\nq\nq\n";
        let mut output = Vec::new();
        run(&input[..], &mut output, OutputTarget::Other, &CmdArgs::default())
            .unwrap();
        let output = str::from_utf8(&output[..]).unwrap();
        assert!(output.contains("no filename"));
    }

    #[test]
    fn write_as_cmd_dispatch() {
        let input = b"a\none\n.\nW\nq\nq\n";
        let mut output = Vec::new();
        run(&input[..], &mut output, OutputTarget::Other, &CmdArgs::default())
            .unwrap();
        let output = str::from_utf8(&output[..]).unwrap();
        assert!(output.contains("no filename"));
    }

    #[test]
    fn undo_cmd_dispatch() {
        let input = b"a\n1\n2\n3\n.\n2,3d\np\nu\np\nu\nq\n";
        let mut output = Vec::new();
        run(&input[..], &mut output, OutputTarget::Other, &CmdArgs::default())
            .unwrap();
        let output = str::from_utf8(&output[..]).unwrap();
        assert!(output.contains("1\n"));
        assert!(output.contains("3\n"));
    }

    #[test]
    fn redo_cmd_dispatch() {
        let input = b"a\n1\n2\n3\n.\n2,3d\nu\nU\n3p\nq\nq\n";
        let mut output = Vec::new();
        run(&input[..], &mut output, OutputTarget::Other, &CmdArgs::default())
            .unwrap();
        let output = str::from_utf8(&output[..]).unwrap();
        assert!(output.contains("invalid address"));
        assert!(output.contains("unsaved changes"), "actual output {output:?}");
    }

    #[test]
    fn substitute_cmd_dispatch() {
        let input = b"a\n11231145611\n.\n1s/[^01]+/./g\n1p\nq\nq\n";
        let mut output = Vec::new();
        run(&input[..], &mut output, OutputTarget::Other, &CmdArgs::default())
            .unwrap();
        let output = str::from_utf8(&output[..]).unwrap();
        assert!(output.contains("11.11.11\n"));
    }

    #[test]
    fn substitute_cmd_empty_buffer() {
        let mut editor = Editor::new(OutputTarget::Other);
        let res = editor
            .substitute_cmd(
                None,
                &Regex::new("won't match").unwrap(),
                "",
                SubstitutionScope::Single(1),
            )
            .expect_err("no match");
        assert!(matches!(res, Error::NoMatch));
    }

    #[test]
    fn substitute_cmd_no_matches() {
        let mut editor = Editor::new(OutputTarget::Other);
        editor.buffer = EditBuffer::with_lines(&[
            "one two three four\r\n",
            "five six seven eight",
            "nine ten eleven twelve",
            "thirteen fourteen fifteen sixteen",
            "seventeen eighteen nineteen twenty",
        ]);
        editor.buffer.set_current_index(4);
        let res = editor
            .substitute_cmd(
                Some(0..5),
                &Regex::new("won't match").unwrap(),
                "",
                SubstitutionScope::Global,
            )
            .expect_err("should give error");
        assert!(matches!(res, Error::NoMatch));
    }

    #[test]
    fn substitute_cmd_current_index_global() {
        let mut editor = Editor::new(OutputTarget::Other);
        editor.buffer = EditBuffer::with_lines(&[
            "one two three four\r\n",
            "five six seven eight",
            "nine ten eleven twelve",
            "thirteen fourteen fifteen sixteen",
            "seventeen eighteen nineteen twenty",
        ]);
        editor.buffer.set_current_index(4);
        editor
            .substitute_cmd(
                None,
                &Regex::new("e+n").unwrap(),
                "'",
                SubstitutionScope::Global,
            )
            .unwrap();
        assert_eq!(editor.buffer[4], "sev't' eight' ninet' tw'ty\r\n");
    }

    #[test]
    fn substitute_cmd_current_index_at_eol() {
        let mut editor = Editor::new(OutputTarget::Other);
        editor.buffer = EditBuffer::with_lines(&["some text\n"]);
        let expected = EditBuffer::with_lines(&["some text!\n"]);
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
    fn substitute_cmd_current_index_single_first() {
        let mut editor = Editor::new(OutputTarget::Other);
        editor.buffer = EditBuffer::with_lines(&[
            "one two three four\r\n",
            "five six seven eight",
            "nine ten eleven twelve",
            "thirteen fourteen fifteen sixteen",
            "seventeen eighteen nineteen twenty",
        ]);
        editor.buffer.set_current_index(4);
        editor
            .substitute_cmd(
                None,
                &Regex::new("e+n").unwrap(),
                "'",
                SubstitutionScope::Single(1),
            )
            .unwrap();
        assert_eq!(editor.buffer[4], "sev'teen eighteen nineteen twenty\r\n");
    }

    #[test]
    fn substitute_cmd_current_index_single() {
        let mut editor = Editor::new(OutputTarget::Other);
        editor.buffer = EditBuffer::with_lines(&[
            "one two three four\r\n",
            "five six seven eight",
            "nine ten eleven twelve",
            "thirteen fourteen fifteen sixteen",
            "seventeen eighteen nineteen twenty",
        ]);
        editor.buffer.set_current_index(4);
        editor
            .substitute_cmd(
                None,
                &Regex::new("e+n").unwrap(),
                "'",
                SubstitutionScope::Single(4),
            )
            .unwrap();
        assert_eq!(editor.buffer[4], "seventeen eighteen ninet' twenty\r\n");
    }

    #[test]
    fn substitute_split_line() {
        let mut editor = Editor::new(OutputTarget::Other);
        editor.buffer = EditBuffer::with_lines(&["a line, to split\r\n"]);
        editor.buffer.set_current_index(0);
        let cmd_line = "s/, /\\\r\n/";
        let mut input = cmd_line.as_bytes();
        let Some((Cmd::Substitute(address, pattern, replacement, scope), None)) =
            Cmd::read(&mut input, &mut editor.buffer, &mut None).unwrap()
        else {
            panic!("{cmd_line} didn't parse as Cmd::Substitute");
        };
        editor.substitute_cmd(address, &pattern, &replacement, scope).unwrap();
        let mut expected = EditBuffer::with_lines(&["a line\r\n", "to split"]);
        expected.set_current_index(1);
        assert_eq!(&editor.buffer[..], &expected[..]);
        assert_eq!(editor.buffer, expected);
    }

    #[test]
    fn substitute_split_line_no_end_delimiter() {
        let mut editor = Editor::new(OutputTarget::Other);
        editor.buffer = EditBuffer::with_lines(&["a line, to split\n"]);
        editor.buffer.set_current_index(0);
        let mut cmd_line = "/, /\\\n".graphemes(true).peekable();
        let mut input = "\n".as_bytes();
        let Ok(Some((
            Cmd::Substitute(address, pattern, replacement, scope),
            None,
        ))) = command::parse_substitute_cmd(
            &mut cmd_line,
            &mut input,
            &editor.buffer,
            &mut None,
            None,
        )
        else {
            panic!("should have parsed to Cmd::Substitute!");
        };
        editor.substitute_cmd(address, &pattern, &replacement, scope).unwrap();
        let mut expected = EditBuffer::with_lines(&["a line\n", "to split"]);
        expected.set_current_index(1);
        assert_eq!(editor.buffer[..], expected[..]);
        assert_eq!(editor.buffer.current_index(), expected.current_index());
    }

    #[test]
    fn substitute_cmd_multi_line_single() {
        let mut editor = Editor::new(OutputTarget::Other);
        editor.buffer = EditBuffer::with_lines(&[
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
        editor.buffer.set_current_index(5);
        let mut expected = EditBuffer::with_lines(&[
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
        expected.set_current_index(7);
        editor
            .substitute_cmd(
                Some(1..9),
                &Regex::new("s[aeiou]").unwrap(),
                "'",
                SubstitutionScope::Single(1),
            )
            .unwrap();
        assert_eq!(&editor.buffer[..], &expected[..]);
        assert_eq!(editor.buffer.current_index(), expected.current_index());
    }

    #[test]
    fn undo_redo_substitute_cmd_multi_line_single() {
        let mut editor = Editor::new(OutputTarget::Other);
        editor.buffer = EditBuffer::with_lines(&[
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
        editor.buffer.set_current_index(5);
        let before = editor.buffer.clone();
        let mut expected = EditBuffer::with_lines(&[
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
        expected.set_current_index(7);
        let Some(changes) = editor
            .substitute_cmd(
                Some(1..9),
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
        assert_eq!(editor.buffer.current_index(), expected.current_index());
        assert_eq!(&editor.buffer[..], &expected[..]);
        editor.buffer.undo().unwrap();
        assert_eq!(editor.buffer.current_index(), before.current_index());
        assert_eq!(&before[..], &editor.buffer[..]);
        editor.buffer.redo().unwrap();
        assert_eq!(editor.buffer.current_index(), expected.current_index());
        assert_eq!(&editor.buffer[..], &expected[..]);
    }

    #[test]
    fn substitute_cmd_multi_line_single_first() {
        let mut editor = Editor::new(OutputTarget::Other);
        editor.buffer = EditBuffer::with_lines(&[
            "one two three four\r\n",
            "five six seven eight",
            "nine ten eleven twelve",
            "thirteen fourteen fifteen sixteen",
            "seventeen eighteen nineteen twenty",
        ]);
        editor.buffer.set_current_index(4);
        editor
            .substitute_cmd(
                Some(1..3),
                &Regex::new("e+n").unwrap(),
                "'",
                SubstitutionScope::Single(1),
            )
            .unwrap();
        assert_eq!(
            editor.buffer[1..3],
            ["five six sev' eight\r\n", "nine t' eleven twelve\r\n"]
        );
    }

    #[test]
    fn substitute_cmd_multi_line_capture() {
        let mut editor = Editor::new(OutputTarget::Other);
        editor.buffer = EditBuffer::with_lines(&[
            "one two three four\r\n",
            "five six seven eight",
            "nine ten eleven twelve",
            "thirteen fourteen fifteen sixteen",
            "seventeen eighteen nineteen twenty",
        ]);
        editor.buffer.set_current_index(4);
        editor
            .substitute_cmd(
                Some(1..4),
                &Regex::new("[a-z]+?(e+n)[^ ]*").unwrap(),
                "$1 ($0)",
                SubstitutionScope::Single(2),
            )
            .unwrap();
        assert_eq!(
            editor.buffer[1..4],
            [
                "five six seven eight\r\n",
                "nine ten en (eleven) twelve\r\n",
                "thirteen een (fourteen) fifteen sixteen\r\n"
            ]
        );
    }

    #[test]
    fn undo_redo_substitute_cmd_multi_line_capture() {
        let mut editor = Editor::new(OutputTarget::Other);
        editor.buffer = EditBuffer::with_lines(&[
            "one two three four\r\n",
            "five six seven eight",
            "nine ten eleven twelve",
            "thirteen fourteen fifteen sixteen",
            "seventeen eighteen nineteen twenty",
        ]);
        editor.buffer.set_current_index(4);
        let before = editor.buffer.clone();
        let Ok(Some(changes)) = editor.substitute_cmd(
            Some(1..4),
            &Regex::new("[a-z]+?(e+n)[^ ]*").unwrap(),
            "$1 ($0)",
            SubstitutionScope::Single(2),
        ) else {
            panic!("expected Ok(Some(ChangeSet))!");
        };
        assert!(!changes.is_empty());
        editor.buffer.push_undo(changes);
        assert_eq!(
            editor.buffer[1..4],
            [
                "five six seven eight\r\n",
                "nine ten en (eleven) twelve\r\n",
                "thirteen een (fourteen) fifteen sixteen\r\n"
            ]
        );
        let after = editor.buffer.clone();

        editor.buffer.undo().unwrap();
        assert_eq!(&editor.buffer[..], &before[..]);

        editor.buffer.redo().unwrap();
        assert_eq!(&editor.buffer[..], &after[..]);
    }

    #[test]
    fn write_propegates_errors() {
        let mut editor = Editor::new(OutputTarget::Other);
        editor.buffer = EditBuffer::with_lines(&["1\r\n", "2", "3"]);
        let mut dummy_file = BadWriter {};
        write_lines(&mut dummy_file, &mut editor.buffer, 0..3)
            .expect_err("io error");
    }

    #[test]
    fn write_one_line() {
        let mut editor = Editor::new(OutputTarget::Other);
        editor.buffer = EditBuffer::with_lines(&["1\n", "2", "3"]);
        let mut dummy_file = Vec::new();
        let (bytes, lines) =
            write_lines(&mut dummy_file, &mut editor.buffer, 2..3).unwrap();
        assert_eq!(bytes, 2);
        assert_eq!(lines, 1);
    }

    #[test]
    fn write_many_lines() {
        let mut editor = Editor::new(OutputTarget::Other);
        editor.buffer =
            EditBuffer::with_lines(&["1\r\n", "2", "3", "4", "5", "6"]);
        let mut dummy_file = Vec::new();
        let (bytes, lines) =
            write_lines(&mut dummy_file, &mut editor.buffer, 0..6).unwrap();
        assert_eq!(bytes, 18);
        assert_eq!(lines, 6);
    }

    #[test]
    fn write_empty_buffer() {
        let mut editor = Editor::new(OutputTarget::Other);
        editor.buffer = EditBuffer::new();
        let mut dummy_file = Vec::new();
        let (bytes, lines) =
            write_lines(&mut dummy_file, &mut editor.buffer, 0..0).unwrap();
        assert_eq!(bytes, 0);
        assert_eq!(lines, 0);
    }

    #[test]
    fn append_cmd_normalizes_eols() {
        let mut editor = Editor::new(OutputTarget::Other);
        editor.buffer = EditBuffer::with_lines(&["1\n", "2", "3"]);
        let expected = ["1\n", "2\n", "a\n", "b\n", "c\n", "3\n"];
        let mut input = IndentReader::from(&["a\r\n", "b\r\n", "c\r\n"]);
        let _ = editor
            .append_cmd(
                &mut input,
                &mut String::new(),
                Some(1),
                InputSource::StdIn,
                InputMode::Cooked,
            )
            .expect("lines appended");
        assert_eq!(editor.buffer[..], expected);
    }

    #[test]
    fn insert_cmd_normalizes_eols() {
        let mut editor = Editor::new(OutputTarget::Other);
        editor.buffer = EditBuffer::with_lines(&["1\n", "2", "3"]);
        let expected = ["1\n", "2\n", "a\n", "b\n", "c\n", "3\n"];
        let mut input = IndentReader::from(&["a\r\n", "b\r\n", "c\r\n"]);
        let _ = editor
            .insert_cmd(
                &mut input,
                &mut String::new(),
                Some(2),
                InputSource::StdIn,
                InputMode::Cooked,
            )
            .expect("lines appended");
        assert_eq!(editor.buffer[..], expected);
    }

    #[test]
    fn overwrite_cmd_empty_buffer() {
        let mut editor = Editor::new(OutputTarget::Other);
        let res = editor
            .overwrite_cmd(
                &mut "".as_bytes(),
                &mut String::new(),
                None,
                InputSource::StdIn,
                InputMode::Cooked,
            )
            .expect_err("nothing to overwrite");
        assert!(matches!(res, Error::NothingToOverwrite));
    }

    #[test]
    fn overwrite_cmd_normalizes_eols() {
        let mut editor = Editor::new(OutputTarget::Other);
        editor.buffer = EditBuffer::with_lines(&["1\n", "2", "3"]);
        let expected = ["1\n", "a\n", "b\n", "c\n", "3\n"];
        let mut input = IndentReader::from(&["a\r\n", "b\r\n", "c\r\n"]);
        let _ = editor
            .overwrite_cmd(
                &mut input,
                &mut String::new(),
                Some(1..2),
                InputSource::StdIn,
                InputMode::Cooked,
            )
            .expect("lines appended");
        assert_eq!(editor.buffer[..], expected);
    }

    #[test]
    fn append_cmd_auto_indent() {
        let mut editor = Editor::new(OutputTarget::Other);
        editor.buffer = EditBuffer::with_lines(&["one\n", "    two", "three"]);
        let mut input = IndentReader::from(&["indented\n", "    further\n"]);
        let expected = [
            "one\n",
            "    two\n",
            "    indented\n",
            "        further\n",
            "three\n",
        ];
        let _ = editor
            .append_cmd(
                &mut input,
                &mut String::new(),
                Some(1),
                InputSource::StdIn,
                InputMode::Cooked,
            )
            .expect("lines appended");
        assert_eq!(&editor.buffer[..], expected);
    }

    #[test]
    fn delete_cmd_empty_buffer() {
        let mut editor = Editor::new(OutputTarget::Other);
        editor.buffer = EditBuffer::new();
        let res = editor.delete_cmd(None).expect_err("nothing to delete");
        assert!(matches!(res, Error::NothingToDelete));
    }

    #[test]
    fn insert_cmd_auto_indent() {
        let mut editor = Editor::new(OutputTarget::Other);
        editor.buffer = EditBuffer::with_lines(&["one\n", "    two", "three"]);
        let mut input = IndentReader::from(&["indented\n", "    further\n"]);
        let expected = [
            "one\n",
            "    indented\n",
            "        further\n",
            "    two\n",
            "three\n",
        ];
        let _ = editor
            .insert_cmd(
                &mut input,
                &mut String::new(),
                Some(1),
                InputSource::StdIn,
                InputMode::Cooked,
            )
            .expect("lines inserted");
        assert_eq!(&editor.buffer[..], expected);
    }

    #[test]
    fn read_lines_returns_correct_count() {
        let source = b"one\r\ntwo\r\nthree\r\nfour\r\n";
        let source_bytes = source.len();
        let mut lines = Vec::new();
        let (byte_count, added) =
            read_lines(&mut lines, &mut &source[..], None).expect("no error");
        assert_eq!(byte_count, source_bytes);
        assert_eq!(lines.len(), 4);
        assert!(!added);
    }

    #[test]
    fn read_lines_io_error() {
        let mut source = BufReader::new(BadReader {});
        let res = read_lines(&mut Vec::new(), &mut source, None)
            .expect_err("io error");
        assert!(matches!(res, Error::ReadLines { .. }));
    }

    #[test]
    fn edit_cmd_reads_file() {
        let mut editor = Editor::new(OutputTarget::Other);
        let mut output = String::new();
        let filename1 = Path::new(r"test/assets/text_with_final_eol.txt");
        let filename2 = Path::new(r"test/assets/text_with_no_final_eol.txt");

        editor.edit_cmd(&mut output, filename1).unwrap();
        assert_eq!(editor.buffer.len(), 10);
        let out_text = &output;
        assert!(
            out_text.contains("10 lines") && out_text.contains("312 bytes")
        );

        output.clear();
        editor.edit_cmd(&mut output, filename2).unwrap();
        assert_eq!(editor.buffer.len(), 10);
        let out_text = &output;
        assert!(
            out_text.contains("10 lines") && out_text.contains("318 bytes")
        );
    }

    #[test]
    fn reload_cmd_reads_file() {
        let mut editor = Editor::new(OutputTarget::Other);
        let mut output = String::new();
        let filename1 = Path::new(r"test/assets/text_with_final_eol.txt");
        let filename2 = Path::new(r"test/assets/text_with_no_final_eol.txt");
        let tmp_dir = tempdir().unwrap();
        let current_filename = tmp_dir.path().join("file.txt");
        fs::copy(filename1, &current_filename).unwrap();

        editor.edit_cmd(&mut output, &current_filename).unwrap();
        assert_eq!(editor.buffer.len(), 10);
        let out_text = &output;
        assert!(
            out_text.contains("10 lines") && out_text.contains("312 bytes")
        );

        fs::copy(filename2, &current_filename).unwrap();
        output.clear();
        editor.reload_cmd(&mut output).unwrap();
        assert_eq!(editor.buffer.len(), 10);
        let out_text = &output;
        assert!(
            out_text.contains("10 lines") && out_text.contains("318 bytes")
        );
    }

    #[test]
    fn reload_warns_when_unsaved() {
        let mut editor = Editor::new(OutputTarget::Other);
        let mut output = String::new();
        let filename1 = Path::new(r"test/assets/text_with_final_eol.txt");

        editor.edit_cmd(&mut output, filename1).unwrap();
        assert_eq!(editor.buffer.len(), 10);
        let out_text = &output;
        assert!(
            out_text.contains("10 lines") && out_text.contains("312 bytes")
        );

        output.clear();
        editor.delete_cmd(Some(0..1)).unwrap();
        assert_eq!(editor.buffer.len(), 9);
        let ret = editor.reload_cmd(&mut output).expect_err("unsaved");
        assert!(matches!(ret, Error::Warning(Warning::ReloadUnsaved)));
        editor.previous_warning = Some(Warning::ReloadUnsaved);
        editor.reload_cmd(&mut output).unwrap();
        assert_eq!(editor.buffer.len(), 10);
        let out_text = &output;
        assert!(
            out_text.contains("10 lines") && out_text.contains("312 bytes")
        );
    }

    #[test]
    fn overwrite_cmd_auto_indent() {
        let mut editor = Editor::new(OutputTarget::Other);
        editor.buffer = EditBuffer::with_lines(&[
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
            .overwrite_cmd(
                &mut input,
                &mut String::new(),
                Some(7..10),
                InputSource::StdIn,
                InputMode::Cooked,
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
            .overwrite_cmd(
                &mut input,
                &mut String::new(),
                Some(1..5),
                InputSource::StdIn,
                InputMode::Cooked,
            )
            .expect("lines changed");
        assert_eq!(&editor.buffer[..], expected);

        let mut input = IndentReader::from(&["second\n"]);
        let expected = [
            "second\n",
            "    indented\n",
            "        further\n",
            "    four\n",
            "        five\n",
            "        replacing blanks\n",
            "    six\n",
        ];
        let _ = editor
            .overwrite_cmd(
                &mut input,
                &mut String::new(),
                Some(0..1),
                InputSource::StdIn,
                InputMode::Cooked,
            )
            .expect("line changed");
        assert_eq!(&editor.buffer[..], expected);
    }

    #[test]
    fn join_cmd_empty_buffer() {
        let mut editor = Editor::new(OutputTarget::Other);
        editor.buffer = EditBuffer::new();
        let res = editor.join_cmd(None, None).expect_err("should fail");
        assert!(matches!(res, Error::NothingToJoin));
    }

    #[test]
    fn join_cmd_single_line_addr() {
        let mut editor = Editor::new(OutputTarget::Other);
        editor.buffer = EditBuffer::with_lines(&["1\n", "2", "3"]);
        let expected = editor.buffer.clone();
        let res =
            editor.join_cmd(Some(2..3), None).expect_err("invalid address");
        assert!(matches!(res, Error::InvalidAddress));
        assert_eq!(editor.buffer, expected);
        let expected = EditBuffer::with_lines(&["1\n", "23"]);
        editor.join_cmd(Some(1..2), None).unwrap();
        assert_eq!(editor.buffer, expected);
    }

    #[test]
    fn join_cmd_default_on_last_line() {
        let mut editor = Editor::new(OutputTarget::Other);
        editor.buffer = EditBuffer::with_lines(&["1\n", "2", "3"]);
        let res = editor.join_cmd(None, None).expect_err("should fail");
        assert!(matches!(res, Error::InvalidAddress));
    }

    #[test]
    fn line_number_cmd_with_and_without_address() {
        let mut output = String::new();
        let mut editor = Editor::new(OutputTarget::Other);
        editor.buffer =
            EditBuffer::with_lines(&["1\n", "2", "3", "4", "5", "6"]);
        editor.buffer.set_current_index(2);
        editor.line_number_cmd(&mut output, None);
        assert_eq!(&output, "6\n");
        output.clear();
        editor.line_number_cmd(&mut output, Some(1));
        assert_eq!(&output, "2\n");
    }

    #[test]
    fn line_number_cmd_with_empty_buffer() {
        let mut editor = Editor::new(OutputTarget::Other);
        let mut output = String::new();
        editor.line_number_cmd(&mut output, None);
        assert_eq!(output.trim(), "empty buffer");
    }

    #[test]
    fn append_cmd_reads_file() {
        let mut editor = Editor::new(OutputTarget::Other);
        editor.buffer =
            EditBuffer::with_lines(&["one\n", "two", "three", "four"]);
        editor.buffer.set_current_index(2);
        let orig = editor.buffer.clone();
        let mut expected = EditBuffer::with_lines(&[
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
        expected.set_current_index(11);
        let mut output = String::new();
        let mut input = "".as_bytes();
        let filename1 = Path::new(r"test/assets/text_with_final_eol.txt");

        let changes = editor
            .append_cmd(
                &mut input,
                &mut output,
                Some(1),
                InputSource::File(filename1.to_path_buf()),
                InputMode::Raw,
            )
            .expect("no error")
            .expect("Some(ChangeSet)");
        let out_text = &output;
        assert!(
            out_text.contains("10 lines") && out_text.contains("312 bytes")
        );
        editor.buffer.push_undo(changes);

        assert_eq!(editor.buffer[..], expected[..]);
        assert_eq!(editor.buffer.current_index(), expected.current_index());

        editor.buffer.undo().expect("something to undo");
        assert_eq!(editor.buffer[..], orig[..]);
        assert_eq!(editor.buffer.current_index(), orig.current_index());

        editor.buffer.redo().expect("something to redo");
        assert_eq!(editor.buffer[..], expected[..]);
        assert_eq!(editor.buffer.current_index(), expected.current_index());
    }

    #[test]
    fn insert_cmd_reads_file() {
        let mut editor = Editor::new(OutputTarget::Other);
        editor.buffer =
            EditBuffer::with_lines(&["one\n", "two", "three", "four"]);
        editor.buffer.set_current_index(2);
        let orig = editor.buffer.clone();
        let mut expected = EditBuffer::with_lines(&[
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
        expected.set_current_index(11);
        let mut output = String::new();
        let mut input = "".as_bytes();
        let filename1 = Path::new(r"test/assets/text_with_final_eol.txt");

        let changes = editor
            .insert_cmd(
                &mut input,
                &mut output,
                Some(2),
                InputSource::File(filename1.to_path_buf()),
                InputMode::Raw,
            )
            .expect("no error")
            .expect("Some(ChangeSet)");
        let out_text = &output;
        assert!(
            out_text.contains("10 lines") && out_text.contains("312 bytes")
        );
        editor.buffer.push_undo(changes);

        assert_eq!(editor.buffer[..], expected[..]);
        assert_eq!(editor.buffer.current_index(), expected.current_index());

        editor.buffer.undo().expect("something to undo");
        assert_eq!(editor.buffer[..], orig[..]);
        assert_eq!(editor.buffer.current_index(), orig.current_index());

        editor.buffer.redo().expect("something to redo");
        assert_eq!(editor.buffer[..], expected[..]);
        assert_eq!(editor.buffer.current_index(), expected.current_index());
    }

    #[test]
    fn overwrite_cmd_reads_file() {
        let mut editor = Editor::new(OutputTarget::Other);
        editor.buffer =
            EditBuffer::with_lines(&["one\n", "two", "three", "four"]);
        editor.buffer.set_current_index(2);
        let orig = editor.buffer.clone();
        let mut expected = EditBuffer::with_lines(&[
            "one\n",
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
            "four",
        ]);
        expected.set_current_index(10);
        let mut output = String::new();
        let mut input = "".as_bytes();
        let filename1 = Path::new(r"test/assets/text_with_final_eol.txt");

        let changes = editor
            .overwrite_cmd(
                &mut input,
                &mut output,
                Some(1..3),
                InputSource::File(filename1.to_path_buf()),
                InputMode::Raw,
            )
            .expect("no error")
            .expect("Some(ChangeSet)");
        let out_text = &output;
        assert!(
            out_text.contains("10 lines") && out_text.contains("312 bytes")
        );
        editor.buffer.push_undo(changes);

        assert_eq!(editor.buffer[..], expected[..]);
        assert_eq!(editor.buffer.current_index(), expected.current_index());

        editor.buffer.undo().expect("something to undo");
        assert_eq!(editor.buffer[..], orig[..]);
        assert_eq!(editor.buffer.current_index(), orig.current_index());

        editor.buffer.redo().expect("something to redo");
        assert_eq!(editor.buffer[..], expected[..]);
        assert_eq!(editor.buffer.current_index(), expected.current_index());
    }

    #[test]
    fn write_as_cmd_no_filename() {
        let mut output = Vec::new();
        let input = b"a\n1\n.\nw\nq\nq\n";

        run(&input[..], &mut output, OutputTarget::Other, &CmdArgs::default())
            .unwrap();
        let output = str::from_utf8(&output[..]).unwrap();
        assert!(output.contains("no filename"));
    }

    #[test]
    fn write_as_cmd_new_filename() {
        let mut output = String::new();
        let tmp_dir = tempdir().expect("tmp dir created");
        let current_filename = tmp_dir.path().join("old_filename");
        let new_filename = tmp_dir.path().join("new_filename");
        let backup_filename = new_filename.clone().with_added_extension("bak");
        let mut editor = Editor::new(OutputTarget::Other);
        editor.buffer = EditBuffer::with_lines(&["1\n", "2", "3"]);
        editor.current_file = Some(current_filename.clone());
        let _res = editor
            .write_as_cmd(&mut output, None, &new_filename)
            .expect("successful write to new_filename");
        assert!(matches!(fs::exists(&new_filename), Ok(true)));
        assert_eq!(editor.current_file, Some(current_filename));
        assert!(matches!(fs::exists(&backup_filename), Ok(false)));
    }

    #[test]
    fn write_as_cmd_overwrite() {
        let tmp_dir = tempdir().expect("tmp dir created");
        let name = tmp_dir.path().join("filename.txt");
        let mut editor = Editor::new(OutputTarget::Other);
        editor.previous_warning = None;
        editor.current_file = Some(PathBuf::from("current_file"));
        let expected_warning = Warning::WriteAsOverwrite(None, name.clone());
        editor.buffer = EditBuffer::with_lines(&["1\r\n", "2\r\n", "3\r\n"]);
        let mut output = String::new();
        fs::copy(
            Path::new(r"test/assets/text_with_final_eol.txt"),
            name.as_path(),
        )
        .expect("copy file for test");

        let res = editor
            .write_as_cmd(&mut output, None, &name)
            .expect_err("overwrite warning");
        let Error::Warning(new_warning) = res else {
            panic!("expected Error::Warning(_), got {res:?}");
        };
        assert_eq!(new_warning, expected_warning);
        editor.previous_warning = Some(new_warning);
        let _ = editor
            .write_as_cmd(&mut output, None, &name)
            .expect("successful overwrite on second try");
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
        assert!(output.contains("3 lines (9 bytes) written"));
    }

    #[test]
    fn write_cmd_success() {
        let tmp_dir = tempdir().expect("tmp dir created");
        let name = tmp_dir.path().join("filename.txt");
        fs::copy(
            Path::new(r"test/assets/text_with_final_eol.txt"),
            name.as_path(),
        )
        .expect("copy file for test");
        let mut output = String::new();
        let mut editor = Editor::new(OutputTarget::Other);
        let _ = editor.edit_cmd(&mut output, &name).expect("successful open");
        editor.buffer = EditBuffer::with_lines(&["1\r\n", "2\r\n", "3\r\n"]);

        let _ = editor.write_cmd(&mut output).expect("successful overwrite");
        let new_content = fs::read(&name).expect("successful read");
        assert_eq!(editor.previous_warning, None);
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
        assert!(output.contains("3 lines (9 bytes) written"));
    }

    #[test]
    fn write_cmd_external_changes() {
        let tmp_dir = tempdir().expect("tmp dir created");
        let name = tmp_dir.path().join("filename.txt");
        fs::copy(
            Path::new(r"test/assets/text_with_final_eol.txt"),
            name.as_path(),
        )
        .expect("copy file for test");
        let mut output = String::new();
        let mut editor = Editor::new(OutputTarget::Other);
        let _ = editor.edit_cmd(&mut output, &name).expect("opened");
        fs::copy(
            Path::new(r"test/assets/text_with_no_final_eol.txt"),
            name.as_path(),
        )
        .expect("overwrite file");
        editor.buffer = EditBuffer::with_lines(&["1\r\n", "2\r\n", "3\r\n"]);

        let error = editor
            .write_cmd(&mut output)
            .expect_err("should get Error::Warning");
        assert!(matches!(error, Error::Warning(Warning::WriteOverwrite)));
    }

    #[test]
    fn write_as_cmd_backup_exists() {
        let tmp_dir = tempdir().expect("tmp dir created");
        let mut editor = Editor::new(OutputTarget::Other);
        editor.buffer = EditBuffer::with_lines(&["1\n", "2", "3"]);
        let name = tmp_dir.path().join("filename.txt");
        let backup_name = name.with_added_extension("bak");
        let mut output = String::new();
        fs::copy(Path::new(r"test/assets/text_with_final_eol.txt"), &name)
            .expect("copy file for test");
        fs::copy(
            Path::new(r"test/assets/text_with_final_eol.txt"),
            &backup_name,
        )
        .expect("copy file for backup");

        let ret = editor
            .write_as_cmd(&mut output, None, &name)
            .expect_err("backup file create fail");
        if let Error::WriteBackupFileCreate {
            source,
            filename,
            backup_filename,
        } = ret
        {
            assert_eq!(
                source.unwrap().downcast::<std::io::Error>().unwrap().kind(),
                io::ErrorKind::AlreadyExists
            );
            assert_eq!(filename, name);
            assert_eq!(backup_filename, Some(backup_name));
        } else {
            panic!("expected error creating \"{}\"", backup_name.display());
        }
    }

    #[test]
    fn write_as_cmd_filename_eq_current_file() {
        let tmp_dir = tempdir().expect("tmp dir created");
        let mut editor = Editor::new(OutputTarget::Other);
        editor.buffer = EditBuffer::with_lines(&["1\n", "2", "3"]);
        let name = tmp_dir.path().join("filename.txt");
        editor.current_file = Some(name.clone());
        let mut output = String::new();
        fs::copy(Path::new(r"test/assets/text_with_final_eol.txt"), &name)
            .expect("copy file for test");

        let ret = editor
            .write_as_cmd(&mut output, None, &name)
            .expect_err("filename same as current_file");
        assert!(matches!(ret, Error::WriteAsCurrentFile));
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
                _span: Range<usize>,
            ) -> io::Result<(usize, usize)> {
                Err(io::Error::new(
                    io::ErrorKind::StorageFull,
                    "no room at the inn!",
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
        let mut editor = Editor::new(OutputTarget::Other);
        editor.buffer = EditBuffer::with_lines(&["1\n", "2", "3"]);
        let name = tmp_dir.path().join("filename.txt");
        let backup_name = name.with_added_extension("bak");
        let mut output = String::new();
        fs::copy(
            Path::new(r"test/assets/text_with_final_eol.txt"),
            name.as_path(),
        )
        .expect("copy file for test");
        let file_content = fs::read(&name).expect("successful read");
        let edited_file =
            EditedFile::open_or_create(&name).expect("EditedFile");
        let mut writer = BadWriter { inner: edited_file };
        if let Err(Error::WriteFile { source, filename: _, backup_filename }) =
            write_file(&mut editor.buffer, &mut output, 0..3, &mut writer)
        {
            assert_eq!(
                source.unwrap().downcast::<std::io::Error>().unwrap().kind(),
                io::ErrorKind::StorageFull
            );
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
                span: Range<usize>,
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
        let mut editor = Editor::new(OutputTarget::Other);
        editor.buffer = EditBuffer::with_lines(&["1\n", "2", "3"]);
        let name = tmp_dir.path().join("filename.txt");
        let mut output = String::new();
        fs::copy(Path::new(r"test/assets/text_with_final_eol.txt"), &name)
            .expect("copy file for test");
        let edited_file =
            EditedFile::open_or_create(&name).expect("EditedFile");
        let mut writer = BadWriter { inner: edited_file };
        if let Err(Error::WriteMakeBackup {
            source,
            filename: _,
            backup_filename,
        }) = write_file(&mut editor.buffer, &mut output, 0..3, &mut writer)
        {
            assert_eq!(
                source.unwrap().downcast::<std::io::Error>().unwrap().kind(),
                io::ErrorKind::StorageFull
            );
            assert!(!fs::exists(backup_filename.unwrap()).unwrap());
        }
    }

    #[test]
    fn list_cmd_no_addr() {
        let mut output = String::new();
        let mut editor = Editor::new(OutputTarget::Other);
        editor.buffer = EditBuffer::with_lines(&["1\r\n", "2", "3"]);
        editor.buffer.set_current_index(1);
        editor.list_cmd(&mut output, None);
        assert_eq!(&output, "2\\r\\n$\r\n");
    }

    #[test]
    fn list_cmd_single_line() {
        let mut output = String::new();
        let mut editor = Editor::new(OutputTarget::Other);
        editor.buffer = EditBuffer::with_lines(&["1\r\n", "2", "3"]);
        editor.buffer.set_current_index(2);
        editor.list_cmd(&mut output, Some(2..3));
        assert_eq!(&output, "3\\r\\n$\r\n");
    }

    #[test]
    fn list_cmd_span() {
        let mut output = String::new();
        let mut editor = Editor::new(OutputTarget::Other);
        editor.buffer =
            EditBuffer::with_lines(&["1\r\n", "2\t2", "3", "4", "5", "6"]);
        editor.buffer.set_current_index(5);
        editor.list_cmd(&mut output, Some(1..4));
        assert_eq!(&output, "2\\t2\\r\\n$\r\n3\\r\\n$\r\n4\\r\\n$\r\n");
    }

    #[test]
    fn list_cmd_sets_current_index() {
        let mut output = String::new();
        let mut editor = Editor::new(OutputTarget::Other);
        editor.buffer =
            EditBuffer::with_lines(&["1\r\n", "2", "3", "4", "5", "6"]);
        editor.buffer.set_current_index(5);
        editor.list_cmd(&mut output, Some(1..4));
        assert_eq!(editor.buffer.current_index(), 3);
    }

    #[test]
    fn page_down_cmd_dispatch() {
        let input = b"a\n1\n2\n3\n4\n\n.\n1\nz2\nq\nq\n";
        let mut output = Vec::new();
        let args = CmdArgs { file: None };
        run(&input[..], &mut output, OutputTarget::Other, &args).unwrap();
        let output = str::from_utf8(&output[..]).unwrap();
        assert!(output.contains("2\n3\n"));
        assert!(!output.contains("4\n"));
    }

    #[test]
    fn page_up_cmd_dispatch() {
        let input = b"a\n1\n2\n3\n4\n\n.\n4\nZ2\nq\nq\n";
        let mut output = Vec::new();
        let args = CmdArgs { file: None };
        run(&input[..], &mut output, OutputTarget::Other, &args).unwrap();
        let output = str::from_utf8(&output[..]).unwrap();
        assert!(output.contains("2\n3\n"));
        assert_eq!(output.matches("4\n").count(), 1);
        assert!(!output.contains("1\n"));
    }

    #[test]
    fn show_diff_cmd_dispatch() {
        let input = b"S\nq\n";
        let mut output = Vec::new();
        let args = CmdArgs { file: None };
        run(&input[..], &mut output, OutputTarget::Other, &args).unwrap();
        let output = str::from_utf8(&output[..]).unwrap();
        assert!(output.contains("no filename"));
    }

    #[test]
    fn page_down_cmd_empty_buffer() {
        let mut editor = Editor::new(OutputTarget::Other);
        let mut output = String::new();
        editor.page_down_cmd(
            &mut output,
            None,
            None,
            PageBounds { cols: 80, rows: 24 },
        );
        assert!(output.is_empty());
    }

    #[test]
    fn page_up_cmd_empty_buffer() {
        let mut editor = Editor::new(OutputTarget::Other);
        let mut output = String::new();
        editor.page_up_cmd(
            &mut output,
            None,
            None,
            PageBounds { cols: 80, rows: 24 },
        );
        assert!(output.is_empty());
    }

    #[test]
    fn page_down_cmd_to_end() {
        let mut editor = Editor::new(OutputTarget::Other);
        let lines: Vec<String> = (1..=64).map(|n| format!("{n}\r\n")).collect();
        editor.buffer = EditBuffer::from(lines);
        let mut output = String::new();
        editor.page_down_cmd(
            &mut output,
            Some(59),
            None,
            PageBounds { cols: 80, rows: 24 },
        );
        assert!(output.contains("60\r\n61\r\n62\r\n63\r\n64\r\n"));
        assert_eq!(editor.buffer.current_index(), 63);

        output.clear();
        editor.page_down_cmd(
            &mut output,
            None,
            None,
            PageBounds { cols: 80, rows: 24 },
        );
        assert!(output.is_empty());
        assert_eq!(editor.buffer.current_index(), 63);
    }

    #[test]
    fn page_up_cmd_to_start() {
        let mut editor = Editor::new(OutputTarget::Other);
        let lines: Vec<String> = (1..=64).map(|n| format!("{n}\r\n")).collect();
        editor.buffer = EditBuffer::from(lines);
        let mut output = String::new();
        editor.page_up_cmd(
            &mut output,
            Some(4),
            None,
            PageBounds { cols: 80, rows: 24 },
        );
        assert!(output.contains("1\r\n2\r\n3\r\n4\r\n5\r\n"));
        assert_eq!(editor.buffer.current_index(), 0);

        output.clear();
        editor.page_up_cmd(
            &mut output,
            None,
            None,
            PageBounds { cols: 80, rows: 24 },
        );
        assert!(output.is_empty());
        assert_eq!(editor.buffer.current_index(), 0);
    }

    #[test]
    fn page_down_cmd_long_lines() {
        let mut editor = Editor::new(OutputTarget::Other);
        let lines: Vec<String> =
            (1..=64).map(|n| format!("{n} {}\r\n", "*".repeat(80))).collect();
        editor.buffer = EditBuffer::from(lines);
        editor.buffer.set_current_index(0);
        let mut output = String::new();
        editor.page_down_cmd(
            &mut output,
            None,
            None,
            PageBounds { cols: 80, rows: 24 },
        );
        assert!(
            output.ends_with("13 ********************************************************************************\r\n"),
            "expected to end with\n\t{:?}got:\n\t{output:?}", editor.buffer[12]
        );
        assert_eq!(editor.buffer.current_index(), 12);
    }

    #[test]
    fn page_up_cmd_long_lines() {
        let mut editor = Editor::new(OutputTarget::Other);
        let lines: Vec<String> =
            (1..=64).map(|n| format!("{n} {}\r\n", "*".repeat(80))).collect();
        editor.buffer = EditBuffer::from(lines);
        editor.buffer.set_current_index(63);
        let mut output = String::new();
        editor.page_up_cmd(
            &mut output,
            None,
            None,
            PageBounds { cols: 80, rows: 24 },
        );
        assert_eq!(editor.buffer.current_index(), 51);
    }

    #[test]
    fn page_down_cmd_saves_windows() {
        let mut editor = Editor::new(OutputTarget::Other);
        let lines: Vec<String> = (1..=64).map(|n| format!("{n}\r\n")).collect();
        editor.buffer = EditBuffer::from(lines);
        let mut output = Vec::new();
        let mut input = b"" as &[u8];
        editor
            .dispatch_cmd(
                Cmd::PageDown(Some(9), Some(3), None),
                &mut output,
                &mut input,
            )
            .expect("scroll 10..12");
        assert_eq!(editor.buffer.current_index(), 11);
        assert_eq!(editor.page_length, NonZero::new(3));
        editor
            .dispatch_cmd(
                Cmd::PageDown(None, None, None),
                &mut output,
                &mut input,
            )
            .expect("scroll 13..15");
        assert_eq!(editor.buffer.current_index(), 14);
        assert_eq!(editor.page_length, NonZero::new(3));
    }

    #[test]
    fn page_up_cmd_saves_windows() {
        let mut editor = Editor::new(OutputTarget::Other);
        let lines: Vec<String> = (1..=64).map(|n| format!("{n}\r\n")).collect();
        editor.buffer = EditBuffer::from(lines);
        let mut output = Vec::new();
        let mut input = b"" as &[u8];
        editor
            .dispatch_cmd(
                Cmd::PageUp(Some(9), Some(3), None),
                &mut output,
                &mut input,
            )
            .expect("page up 8..=10");
        assert_eq!(editor.buffer.current_index(), 7);
        assert_eq!(editor.page_length, NonZero::new(3));
        editor
            .dispatch_cmd(
                Cmd::PageUp(None, None, None),
                &mut output,
                &mut input,
            )
            .expect("page up 5..=7");
        assert_eq!(editor.buffer.current_index(), 4);
        assert_eq!(editor.page_length, NonZero::new(3));
    }

    #[test]
    fn page_down_cmd_resets_window() {
        let mut editor = Editor::new(OutputTarget::Other);
        let lines: Vec<String> = (1..=64).map(|n| format!("{n}\r\n")).collect();
        editor.buffer = EditBuffer::from(lines);
        let mut output = Vec::new();
        let mut input = b"" as &[u8];
        editor
            .dispatch_cmd(
                Cmd::PageDown(Some(0), None, None),
                &mut output,
                &mut input,
            )
            .unwrap();
        let orig_end_line = editor.buffer.current_index();
        editor
            .dispatch_cmd(
                Cmd::PageDown(Some(9), Some(3), None),
                &mut output,
                &mut input,
            )
            .expect("scroll 10..12");
        assert_eq!(editor.buffer.current_index(), 11);
        assert_eq!(editor.page_length, NonZero::new(3));
        editor
            .dispatch_cmd(
                Cmd::PageDown(Some(0), Some(0), None),
                &mut output,
                &mut input,
            )
            .unwrap();
        assert!(editor.page_length.is_none());
        assert_eq!(editor.buffer.current_index(), orig_end_line);
    }

    #[test]
    fn page_up_cmd_resets_window() {
        let mut editor = Editor::new(OutputTarget::Other);
        let lines: Vec<String> = (1..=64).map(|n| format!("{n}\r\n")).collect();
        editor.buffer = EditBuffer::from(lines);
        let mut output = Vec::new();
        let mut input = b"" as &[u8];
        editor
            .dispatch_cmd(
                Cmd::PageDown(Some(0), None, None),
                &mut output,
                &mut input,
            )
            .unwrap();
        assert!(editor.page_length.is_none());
        editor
            .dispatch_cmd(
                Cmd::PageDown(Some(9), Some(3), None),
                &mut output,
                &mut input,
            )
            .expect("scroll 10..12");
        assert_eq!(editor.buffer.current_index(), 11);
        assert_eq!(editor.page_length, NonZero::new(3));
        editor
            .dispatch_cmd(
                Cmd::PageDown(None, Some(0), None),
                &mut output,
                &mut input,
            )
            .unwrap();
        assert!(editor.page_length.is_none());
    }

    #[test]
    fn page_down_cmd_with_print_sfx() {
        let mut editor = Editor::new(OutputTarget::Other);
        let lines: Vec<String> = (1..=64).map(|n| format!("{n}\n")).collect();
        editor.buffer = EditBuffer::from(lines);
        let mut output = Vec::new();
        let mut input = b"" as &[u8];
        editor
            .dispatch_cmd(
                Cmd::PageDown(
                    Some(9),
                    Some(3),
                    Some(PrintSuffix { enumerate: true, ..Default::default() }),
                ),
                &mut output,
                &mut input,
            )
            .expect("scroll 10..12");
        assert_eq!(editor.buffer.current_index(), 11);
        let out_str = str::from_utf8(&output[..]).unwrap();
        assert!(out_str.contains("10  10\n11  11\n12  12\n"));
        assert!(!out_str.contains("13"));
        editor
            .dispatch_cmd(
                Cmd::PageDown(
                    None,
                    None,
                    Some(PrintSuffix {
                        expand_escapes: true,
                        ..Default::default()
                    }),
                ),
                &mut output,
                &mut input,
            )
            .expect("scroll 13..15");
        assert_eq!(editor.buffer.current_index(), 14);
        let out_str = str::from_utf8(&output[..]).unwrap();
        assert!(out_str.contains("13\\n$\n14\\n$\n15\\n$\n"));
        assert!(!out_str.contains("16"));
    }

    #[test]
    fn page_up_cmd_with_print_sfx() {
        let mut editor = Editor::new(OutputTarget::Other);
        let lines: Vec<String> = (1..=64).map(|n| format!("{n}\n")).collect();
        editor.buffer = EditBuffer::from(lines);
        let mut output = Vec::new();
        let mut input = b"" as &[u8];
        editor
            .dispatch_cmd(
                Cmd::PageUp(
                    Some(9),
                    Some(3),
                    Some(PrintSuffix { enumerate: true, ..Default::default() }),
                ),
                &mut output,
                &mut input,
            )
            .expect("page up 8..=10");
        assert_eq!(editor.buffer.current_index(), 7);
        let out_str = str::from_utf8(&output[..]).unwrap();
        assert!(out_str.contains(" 8  8\n 9  9\n10  10\n"));
        assert!(!out_str.contains('7'));
        editor
            .dispatch_cmd(
                Cmd::PageUp(
                    None,
                    None,
                    Some(PrintSuffix {
                        expand_escapes: true,
                        ..Default::default()
                    }),
                ),
                &mut output,
                &mut input,
            )
            .expect("page up 5..=7");
        assert_eq!(editor.buffer.current_index(), 4);
        let out_str = str::from_utf8(&output[..]).unwrap();
        assert!(out_str.contains("5\\n$\n6\\n$\n7\\n$\n"));
        assert!(!out_str.contains('4'));
    }

    #[test]
    fn show_diff_cmd_diffs_current_file() {
        let mut editor = Editor::new(OutputTarget::Other);
        let mut output = FmtWriter(Vec::new());
        let name = Path::new(r"test/assets/text_with_final_eol.txt");
        let _ = editor.edit_cmd(&mut output, name).expect("no error");
        assert_eq!(editor.current_file.as_deref(), Some(name));

        let _ = editor.delete_cmd(Some(5..6)).expect("no error");
        let _ = editor.show_diff_cmd(&mut output.0, None).expect("no error");
        let expected = "10 lines (312 bytes) read [LF]\n--- test/assets/text_with_final_eol.txt\n+++ current buffer\n@@ -3,7 +3,6 @@\n but it will suffice to test commands that\n read\n and\n-edit files. The lines\n are of various lengths, and\n end and begin with \n \"special\" characters (i.e., non-alpha characters).\n";
        let output = str::from_utf8(&output.0[..]).unwrap();
        assert_eq!(output, expected);
    }

    #[test]
    fn show_diff_cmd_with_filename_diffs_filename() {
        let mut editor = Editor::new(OutputTarget::Other);
        let mut output = FmtWriter(Vec::new());
        let name = Path::new(r"test/assets/text_with_final_eol.txt");
        let _ = editor
            .append_cmd(
                &mut String::new().as_bytes(),
                &mut output,
                None,
                InputSource::File(name.to_owned()),
                InputMode::Raw,
            )
            .expect("no error");
        let _ = editor.delete_cmd(Some(5..6)).expect("no error");
        let _ =
            editor.show_diff_cmd(&mut output.0, Some(name)).expect("no error");
        let expected = "10 lines (312 bytes) read\n--- test/assets/text_with_final_eol.txt\n+++ current buffer\n@@ -3,7 +3,6 @@\n but it will suffice to test commands that\n read\n and\n-edit files. The lines\n are of various lengths, and\n end and begin with \n \"special\" characters (i.e., non-alpha characters).\n";
        let output = str::from_utf8(&output.0[..]).unwrap();
        assert_eq!(output, expected);
    }

    #[test]
    fn show_diff_cmd_error_reading_file_fails() {
        let mut editor = Editor::new(OutputTarget::Other);
        editor.buffer = EditBuffer::with_lines(&["1\n", "2", "3"]);
        let mut output = Vec::new();
        let name = Path::new("file_not_found");
        let Err(Error::DiffReadFile { source, filename }) =
            editor.show_diff_cmd(&mut output, Some(name))
        else {
            panic!("error expected");
        };
        assert_eq!(
            source.unwrap().downcast::<std::io::Error>().unwrap().kind(),
            io::ErrorKind::NotFound
        );
        assert_eq!(filename, name);
    }

    #[test]
    fn show_diff_cmd_no_filename_no_current_file_fails() {
        let mut editor = Editor::new(OutputTarget::Other);
        editor.buffer = EditBuffer::with_lines(&["1\n", "2", "3"]);
        let mut output = Vec::new();
        let res =
            editor.show_diff_cmd(&mut output, None).expect_err("no filename");
        assert!(matches!(res, Error::NoFilename));
    }

    #[test]
    fn newline_cmd_same_eol_not_mixed_does_nothing() {
        let mut editor = Editor::new(OutputTarget::Other);
        editor.buffer = EditBuffer::with_lines(&["1\r\n", "2", "3"]);
        let mut output = String::new();
        let res = editor.newline_cmd(&mut output, Some(Eol::Crlf));
        assert!(res.is_none());
    }

    #[test]
    fn newline_cmd_no_arg_prints_eol() {
        let mut editor = Editor::new(OutputTarget::Other);
        editor.buffer = EditBuffer::with_lines(&["1\n", "2", "3"]);
        let mut output = String::new();
        let res = editor.newline_cmd(&mut output, None);
        assert!(res.is_none());
        assert!(output.contains("prevailing newline: LF"));
    }

    #[test]
    fn newline_cmd_invalid_newline_prints_error() {
        let input = b"a\n1\n2\n3\n.\nL HT\nq\nq\n";
        let mut output = Vec::new();
        run(&input[..], &mut output, OutputTarget::Other, &CmdArgs::default())
            .unwrap();
        let output = str::from_utf8(&output[..]).unwrap();
        assert!(output.contains("invalid newline"));
    }

    #[test]
    fn newline_cmd_with_arg_normalizes_and_prints_eol() {
        let mut editor = Editor::new(OutputTarget::Other);
        editor.buffer = EditBuffer::with_lines(&["1\n", "2\r\n", "3\n"]);
        let mut output = String::new();
        let res = editor.newline_cmd(&mut output, None);
        assert!(res.is_none());
        assert!(output.contains("prevailing newline: mostly LF"));
        output.clear();
        let res = editor.newline_cmd(&mut output, Some(Eol::Crlf));
        assert!(res.is_some());
        assert!(output.contains("prevailing newline: CRLF"));
        assert_eq!(editor.buffer.eols().prevailing(), Eol::Crlf);
        output.clear();
        let res = editor.newline_cmd(&mut output, Some(Eol::Lf));
        assert!(res.is_some());
        assert!(output.contains("prevailing newline: LF"));
        assert_eq!(editor.buffer.eols().prevailing(), Eol::Lf);
    }

    #[test]
    fn newline_cmd_undo_redo_restores_eol() {
        let mut editor = Editor::new(OutputTarget::Other);
        editor.buffer = EditBuffer::with_lines(&["1\n", "2\r\n", "3\n"]);
        editor.buffer.set_current_index(1);
        let orig_buffer = editor.buffer.clone();
        let mut output = String::new();
        let mut expected = EditBuffer::with_lines(&["1\r\n", "2", "3"]);
        expected.set_current_index(1);

        let res = editor.newline_cmd(&mut output, Some(Eol::Crlf));
        editor.buffer.push_undo(res.unwrap());
        assert_eq!(editor.buffer[..], expected[..]);
        assert_eq!(editor.buffer.current_index(), expected.current_index());
        assert_eq!(editor.buffer.eols(), expected.eols());

        editor.buffer.undo().unwrap();
        assert_eq!(editor.buffer[..], orig_buffer[..]);
        assert_eq!(editor.buffer.current_index(), orig_buffer.current_index());
        assert_eq!(editor.buffer.eols(), orig_buffer.eols());
        editor.buffer.redo().unwrap();
        assert_eq!(editor.buffer[..], expected[..]);
        assert_eq!(editor.buffer.current_index(), expected.current_index());
        assert_eq!(editor.buffer.eols(), expected.eols());
    }

    #[test]
    fn copy_cmd_with_no_span() {
        let mut editor = Editor::new(OutputTarget::Other);
        editor.buffer =
            EditBuffer::with_lines(&["1\n", "2", "3", "4", "5", "6"]);
        editor.buffer.set_current_index(2);
        assert!(editor.clipboard.is_empty());
        editor.copy_cmd(None);
        assert_eq!(&editor.clipboard, "3\n");
        editor.buffer.set_current_index(5);
        editor.copy_cmd(None);
        assert_eq!(&editor.clipboard, "6\n");
    }

    #[test]
    fn copy_cmd_with_span() {
        let mut editor = Editor::new(OutputTarget::Other);
        editor.buffer =
            EditBuffer::with_lines(&["1\n", "2", "3", "4", "5", "6"]);
        editor.buffer.set_current_index(1);
        assert!(editor.clipboard.is_empty());
        editor.copy_cmd(Some(1..3));
        assert_eq!(&editor.clipboard, "2\n3\n");
        editor.copy_cmd(Some(0..6));
        assert_eq!(&editor.clipboard, "1\n2\n3\n4\n5\n6\n");
    }

    #[test]
    fn cut_cmd_with_no_span() {
        let mut editor = Editor::new(OutputTarget::Other);
        editor.buffer =
            EditBuffer::with_lines(&["1\n", "2", "3", "4", "5", "6"]);
        editor.buffer.set_current_index(2);
        assert!(editor.clipboard.is_empty());
        let changes = editor.cut_cmd(None);
        editor.buffer.push_undo(changes);
        assert_eq!(&editor.clipboard, "3\n");
        assert_eq!(editor.buffer[..], ["1\n", "2\n", "4\n", "5\n", "6\n"]);

        editor.buffer.undo().expect("no error");
        assert_eq!(&editor.clipboard, "3\n");
        assert_eq!(
            editor.buffer[..],
            ["1\n", "2\n", "3\n", "4\n", "5\n", "6\n"]
        );

        editor.buffer.redo().expect("no error");
        assert_eq!(&editor.clipboard, "3\n");
        assert_eq!(editor.buffer[..], ["1\n", "2\n", "4\n", "5\n", "6\n"]);
    }

    #[test]
    fn cut_cmd_with_span() {
        let mut editor = Editor::new(OutputTarget::Other);
        editor.buffer =
            EditBuffer::with_lines(&["1\n", "2", "3", "4", "5", "6"]);
        editor.buffer.set_current_index(2);
        assert!(editor.clipboard.is_empty());
        let changes = editor.cut_cmd(Some(1..4));
        editor.buffer.push_undo(changes);
        assert_eq!(&editor.clipboard, "2\n3\n4\n");
        assert_eq!(editor.buffer[..], ["1\n", "5\n", "6\n"]);

        editor.buffer.undo().expect("no error");
        assert_eq!(&editor.clipboard, "2\n3\n4\n");
        assert_eq!(
            editor.buffer[..],
            ["1\n", "2\n", "3\n", "4\n", "5\n", "6\n"]
        );

        editor.buffer.redo().expect("no error");
        assert_eq!(&editor.clipboard, "2\n3\n4\n");
        assert_eq!(editor.buffer[..], ["1\n", "5\n", "6\n"]);
    }
}
