use std::cmp::Ordering;
use std::io::{self, BufRead, Write};
use std::ops::ControlFlow;
use std::time::Duration;

use crossterm::cursor::{self, Hide, MoveTo, Show};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
use crossterm::terminal::{self, Clear, ClearType, ScrollUp};
use crossterm::{ExecutableCommand, QueueableCommand};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

// Public structs, enums, and traits
///////////

pub trait LineRead {
    /// # Errors
    ///
    /// Will return `io::Error` if an error is encountered reading a line
    fn read_line(
        &mut self,
        prompt: &str,
        buffer: &mut String,
    ) -> io::Result<usize>;
}

#[derive(Debug, Clone, PartialEq)]
pub struct LineReader {
    buffer: Vec<BufferLine>,
    history: Vec<String>,
    history_idx: Option<usize>,
    edited_input: Option<String>,
    edited_history: Option<String>,
    prompt_char_count: usize,
    input_start: BufferIndex,
    display_width: usize,
    display_height: usize,
    cursor: Cursor,
    first_display_line: usize,
    first_buffer_line: usize,
    scroll_needed: usize,
}

// Non-public structs, enums, and traits
///////////////

#[derive(Debug, Default, Clone, PartialEq)]
struct BufferLine {
    text: String,
    width: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
struct BufferIndex {
    line: usize,
    offset: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
struct Cursor {
    column: usize,
    line: usize,
    index: BufferIndex,
}

/// Struct used to handle enabling `raw_mode`, and more importantly,
/// who's Drop ensures exiting `raw_mode` and that the cursor doesn't
/// remain hidden in the case of error exit.
#[derive(Debug)]
struct RenderContext;

// public functions
////////

#[must_use]
pub fn native_eol() -> &'static str {
    if std::env::consts::FAMILY == "windows" {
        "\r\n"
    } else {
        "\n"
    }
}

impl BufferLine {
    pub(crate) fn len(&self) -> usize {
        self.text.len()
    }

    pub(crate) fn new() -> BufferLine {
        BufferLine { text: String::new(), width: 0 }
    }
}

impl From<&str> for BufferLine {
    fn from(value: &str) -> BufferLine {
        let width = value.width();
        BufferLine { text: value.to_owned(), width }
    }
}

impl From<(usize, usize)> for BufferIndex {
    fn from((line, offset): (usize, usize)) -> BufferIndex {
        BufferIndex { line, offset }
    }
}

impl From<BufferIndex> for (usize, usize) {
    fn from(i: BufferIndex) -> (usize, usize) {
        (i.line, i.offset)
    }
}

// impls for LineReader
////////

impl Default for LineReader {
    fn default() -> LineReader {
        LineReader {
            buffer: vec![BufferLine { text: String::new(), width: 0 }],
            history: Vec::new(),
            history_idx: None,
            edited_input: None,
            edited_history: None,
            input_start: BufferIndex { ..Default::default() },
            prompt_char_count: 0,
            display_width: 80,
            display_height: 24,
            cursor: Cursor { ..Default::default() },
            first_display_line: 0,
            first_buffer_line: 0,
            scroll_needed: 0,
        }
    }
}

impl LineReader {
    #[must_use]
    pub fn new() -> LineReader {
        LineReader { ..Default::default() }
    }

    #[must_use]
    pub fn prompt(&self) -> String {
        self.buffer
            .iter()
            .flat_map(|l| l.text.chars())
            .take(self.prompt_char_count)
            .collect()
    }

    fn reset(
        &mut self,
        display_width: usize,
        display_height: usize,
        first_display_line: usize,
        prompt: &str,
    ) {
        self.display_width = display_width;
        self.display_height = display_height;
        self.first_display_line = first_display_line;
        let prompt_line =
            BufferLine { text: prompt.to_owned(), width: prompt.width() };
        self.input_start = (0, prompt_line.text.len()).into();
        self.prompt_char_count = prompt.chars().count();
        self.cursor = Cursor {
            column: prompt_line.width,
            line: self.first_display_line,
            index: self.input_start,
        };
        self.buffer.splice(.., [prompt_line]);
        self.reflow(0);
    }

    fn set_buffer(&mut self, line: impl AsRef<str>) {
        let mut text = self.prompt();
        self.input_start = (0, text.len()).into();
        text.push_str(line.as_ref());
        let width = text.width();
        let cursor = Cursor {
            column: width,
            line: self.first_display_line,
            index: (0, text.len()).into(),
        };
        self.buffer.splice(.., [BufferLine { text, width }]);
        self.cursor = cursor;
        self.reflow(0);
    }

    fn set_buffer_from_history(&mut self, line: usize) {
        let mut text = self.prompt();
        text.push_str(&self.history[line]);
        let width = text.width();
        let cursor = Cursor {
            column: width,
            line: self.first_display_line,
            index: (0, text.len()).into(),
        };
        self.buffer.splice(.., [BufferLine { text, width }]);
        self.cursor = cursor;
        self.reflow(0);
    }

    #[cfg(not(tarpaulin_include))]
    fn accept_line(
        &mut self,
        prompt: &str,
        output_buffer: &mut String,
    ) -> io::Result<usize> {
        // reset for new input
        let (display_width, display_height) = terminal::size()?;
        let (_, first_display_line) = cursor::position()?;
        self.reset(
            display_width.into(),
            display_height.into(),
            first_display_line.into(),
            prompt,
        );

        let _render_ctx = RenderContext::new();
        terminal::enable_raw_mode()?;

        self.repaint()?;
        let mut res = ControlFlow::Continue(());
        while res.is_continue() {
            let event = match event::read()? {
                Event::Resize(mut x, mut y) => {
                    let c_pos = cursor::position()?;
                    let mut cursor_line: usize = c_pos.1.into();
                    while let Ok(true) = event::poll(Duration::from_millis(50))
                    {
                        if let Event::Resize(x1, y1) = event::read()? {
                            (x, y) = (x1, y1);
                            let c_pos = cursor::position()?;
                            cursor_line = c_pos.1.into();
                        }
                    }
                    if cursor_line > self.cursor.line {
                        self.first_display_line +=
                            cursor_line - self.cursor.line;
                    } else {
                        self.first_display_line -=
                            self.cursor.line - cursor_line;
                    }
                    self.cursor.line = cursor_line;
                    Event::Resize(x, y)
                }
                event => event,
            };
            res = self.handle_event(&event);
            if !matches!(event, Event::Resize(..)) {
                self.repaint()?;
            }
        }

        self.handle_end();
        self.repaint()?;
        let mut stdout = io::stdout().lock();
        stdout.write_all(b"\r\n")?;
        stdout.flush()?;

        let prev_bytes = output_buffer.len();
        if matches!(res, ControlFlow::Break(true)) {
            self.history.last().inspect(|s| output_buffer.push_str(s));
        }
        output_buffer.push_str(native_eol());
        Ok(output_buffer.len() - prev_bytes)
    }

    fn handle_event(&mut self, event: &Event) -> ControlFlow<bool> {
        match event {
            Event::Key(event) if event.kind == KeyEventKind::Press => {
                self.handle_key_event(event)
            }
            Event::Resize(x, y) => self.handle_resize_event(*x, *y),
            _ => ControlFlow::Continue(()),
        }
    }

    fn handle_resize_event(&mut self, x: u16, y: u16) -> ControlFlow<bool> {
        let old_width = self.display_width;
        let old_height = self.display_height;
        self.display_width = x.into();
        self.display_height = y.into();

        if self.display_width != old_width {
            self.reflow(0);
        } else if self.display_height != old_height {
            self.adjust_viewport();
        }
        if self.display_height < old_height {
            let h_diff = old_height - self.display_height;
            self.scroll_needed = self.scroll_needed.saturating_sub(h_diff);
        }
        ControlFlow::Continue(())
    }

    fn handle_key_event(&mut self, event: &KeyEvent) -> ControlFlow<bool> {
        match event.code {
            KeyCode::Enter => {
                let text_entered = !self.buffer.is_empty();
                if text_entered {
                    let buffer_text: String = self
                        .buffer
                        .iter()
                        .flat_map(|l| l.text.chars())
                        .skip(self.prompt_char_count)
                        .collect();
                    self.history.push(buffer_text);
                }
                ControlFlow::Break(text_entered)
            }
            KeyCode::Left => self.handle_left(),
            KeyCode::Right => self.handle_right(),
            KeyCode::Home => self.handle_home(),
            KeyCode::End => self.handle_end(),
            KeyCode::Backspace => self.handle_backspace(),
            KeyCode::Delete => self.handle_delete(),
            KeyCode::Char(c) => self.handle_char_input(c),
            KeyCode::Up => self.handle_up(),
            KeyCode::Down => self.handle_down(),
            KeyCode::Esc => self.handle_esc(),
            _ => ControlFlow::Continue(()),
        }
    }

    fn handle_esc(&mut self) -> ControlFlow<bool> {
        self.history_idx = None;
        if let Some(edited) =
            self.edited_history.take().or_else(|| self.edited_input.take())
        {
            self.set_buffer(&edited);
        }
        ControlFlow::Continue(())
    }

    fn handle_down(&mut self) -> ControlFlow<bool> {
        let Some(mut i) = self.history_idx else {
            return ControlFlow::Continue(());
        };
        i += 1;
        if i < self.history.len() {
            self.history_idx = Some(i);
            self.set_buffer_from_history(i);
        } else {
            self.history_idx = None;
            let line = if self.edited_history.is_some() {
                self.edited_history.take()
            } else {
                self.edited_input.take()
            };
            if let Some(line) = line {
                self.set_buffer(&line);
            };
        };
        ControlFlow::Continue(())
    }

    fn handle_up(&mut self) -> ControlFlow<bool> {
        if self.history.is_empty() {
            return ControlFlow::Continue(());
        }
        let mut i = *self.history_idx.get_or_insert_with(|| {
            if self.edited_input.is_some() {
                self.edited_history = Some(
                    self.buffer
                        .iter()
                        .flat_map(|l| l.text.chars())
                        .skip(self.prompt_char_count)
                        .collect(),
                );
            } else {
                self.edited_input = Some(
                    self.buffer
                        .iter()
                        .flat_map(|l| l.text.chars())
                        .skip(self.prompt_char_count)
                        .collect(),
                );
            }
            self.history.len()
        });
        if i > 0 {
            i -= 1;
            self.history_idx = Some(i);
            self.set_buffer_from_history(i);
        }
        ControlFlow::Continue(())
    }

    fn handle_char_input(&mut self, c: char) -> ControlFlow<bool> {
        let c_width = c.width().unwrap_or(0);
        // if char is zero width, but no previous chars exist to
        //  which it can  be combined, do nothing (i.e., don't accept
        // the input)
        if c_width == 0 && self.cursor.index == self.input_start {
            return ControlFlow::Continue(());
        }

        // insert new char at curser and let reflow sort it out
        let line = &mut self.buffer[self.cursor.index.line];
        line.text.insert(self.cursor.index.offset, c);
        line.width += c_width;
        self.cursor.index.offset += c.len_utf8();
        self.cursor.column += c_width;

        // reflow from line before cursor, if it exists,
        // catching case where new char fits on previous line
        self.reflow(self.cursor.index.line.saturating_sub(1));

        ControlFlow::Continue(())
    }

    fn handle_backspace(&mut self) -> ControlFlow<bool> {
        if self.cursor.index == self.input_start {
            return ControlFlow::Continue(());
        }

        if self.cursor.index.offset == 0 {
            self.cursor.index.line -= 1;
            self.cursor.index.offset =
                self.buffer[self.cursor.index.line].len();
            self.cursor.line -= 1;
            self.cursor.column = self.buffer[self.cursor.index.line].width;
        }
        if let Some((i, c)) = self.buffer[self.cursor.index.line].text
            [..self.cursor.index.offset]
            .char_indices()
            .next_back()
        {
            self.buffer[self.cursor.index.line].text.remove(i);
            let removed_width = c.width().unwrap_or(0);

            self.buffer[self.cursor.index.line].width -= removed_width;
            self.cursor.index.offset = i;
            self.cursor.column -= removed_width;
        }
        self.reflow(self.cursor.index.line.saturating_sub(1));
        ControlFlow::Continue(())
    }

    fn handle_left(&mut self) -> ControlFlow<bool> {
        if self.cursor.index == self.input_start {
            return ControlFlow::Continue(());
        }

        if self.cursor.index.offset == 0 {
            self.cursor.index.line -= 1;
            self.cursor.index.offset =
                self.buffer[self.cursor.index.line].len();
            self.cursor.column = self.buffer[self.cursor.index.line].width;
            self.cursor.line -= 1;
        }

        if let Some((prev_idx, prev_width)) =
            self.buffer[self.cursor.index.line].text[..self.cursor.index.offset]
                .char_indices()
                .map(|(i, c)| (i, c.width().unwrap_or(0)))
                .rfind(|(_, w)| *w > 0)
        {
            self.cursor.index.offset = prev_idx;
            self.cursor.column -= prev_width;
        }

        self.adjust_viewport();
        ControlFlow::Continue(())
    }

    fn handle_right(&mut self) -> ControlFlow<bool> {
        if self.cursor.index
            == (self.buffer.len() - 1, self.buffer.last().unwrap().text.len())
                .into()
        {
            return ControlFlow::Continue(());
        }

        if let Some((i, _)) = self.buffer[self.cursor.index.line].text
            [self.cursor.index.offset..]
            .char_indices()
            .skip(1)
            .find(|(_, c)| c.width().unwrap_or(0) > 0)
        {
            let cur_char_width = self.buffer[self.cursor.index.line].text
                [self.cursor.index.offset..]
                .chars()
                .next()
                .and_then(UnicodeWidthChar::width)
                .unwrap();
            self.cursor.column += cur_char_width;
            self.cursor.index.offset += i;
        } else {
            self.cursor.line += 1;
            self.cursor.column = 0;
            self.cursor.index.line += 1;
            self.cursor.index.offset = 0;
        }
        self.adjust_viewport();
        ControlFlow::Continue(())
    }

    fn handle_delete(&mut self) -> ControlFlow<bool> {
        // if at end of buffer, nothing to do
        if self.cursor.index != self.buffer_end() {
            let (cur_line, cur_offset) = self.cursor.index.into();
            let mut c_idx =
                self.buffer[cur_line].text[cur_offset..].char_indices();
            let c_width =
                c_idx.next().map(|(_, c)| c.width().unwrap_or(0)).unwrap();
            let next_c_offset =
                c_idx.find(|(_, c)| c.width().unwrap_or(0) > 0).map_or_else(
                    || self.buffer[cur_line].len(),
                    |(i, _)| i + cur_offset,
                );
            self.buffer[cur_line]
                .text
                .replace_range(cur_offset..next_c_offset, "");
            self.buffer[cur_line].width -= c_width;
            self.reflow(cur_line.saturating_sub(1));
        }
        ControlFlow::Continue(())
    }

    fn handle_home(&mut self) -> ControlFlow<bool> {
        if self.cursor.index != self.input_start {
            self.first_buffer_line = 0;
            self.cursor.index = self.input_start;
            self.cursor.line = self.first_display_line + self.cursor.index.line;
            self.cursor.column = self.buffer[self.cursor.index.line].text
                [..self.cursor.index.offset]
                .width();
            self.adjust_viewport();
        }
        ControlFlow::Continue(())
    }

    fn handle_end(&mut self) -> ControlFlow<bool> {
        let buffer_end = self.buffer_end();
        if self.cursor.index != buffer_end {
            self.cursor.line += buffer_end.line - self.cursor.index.line;
            self.cursor.column = self.buffer[buffer_end.line].width;
            self.cursor.index = buffer_end;
            self.adjust_viewport();
        }
        ControlFlow::Continue(())
    }

    /// Compute index one past last char in buffer
    fn buffer_end(&self) -> BufferIndex {
        (
            self.buffer.len() - 1,
            self.buffer.last().map(|l| l.text.len()).unwrap(),
        )
            .into()
    }

    /// Compute last line of viewport
    pub(crate) fn viewport_bottom(&self) -> usize {
        if self.cursor.index.line == self.buffer.len() - 1
            || (self.buffer.len() - self.first_buffer_line)
                <= (self.display_height - self.first_display_line)
        {
            self.display_height - 1
        } else {
            self.display_height - 2
        }
    }

    /// Compute first line of viewport
    pub(crate) fn viewport_top(&self) -> usize {
        (self.first_buffer_line > 0).into()
    }

    fn adjust_viewport(&mut self) {
        if self.cursor.line > self.viewport_bottom() {
            let diff = self.cursor.line - self.viewport_bottom();
            self.cursor.line = self.viewport_bottom();
            if self.first_display_line == 0 {
                self.first_buffer_line += diff;
            } else {
                self.scroll_needed = self.first_display_line.min(diff);
                self.first_display_line =
                    self.first_display_line.saturating_sub(diff);
                self.first_buffer_line += diff - self.scroll_needed;
            }
        } else if self.cursor.line < self.viewport_top() {
            let diff = self.viewport_top() - self.cursor.line;
            self.cursor.line = self.viewport_top();
            self.first_buffer_line =
                self.first_buffer_line.saturating_sub(diff);
        }
        if self.buffer.len() <= self.display_height {
            if self.first_buffer_line != 0 {
                // lines above display
                self.cursor.line += self.first_buffer_line;
                self.first_buffer_line = 0;
            } else if self.display_height - self.first_display_line
                < self.buffer.len()
            {
                // lines below display
                self.scroll_needed = self.buffer.len()
                    - (self.display_height - self.first_display_line);
                self.cursor.line -= self.scroll_needed;
                self.first_display_line -= self.scroll_needed;
            }
        }
    }

    /// Reflow buffer lines to fit `display_width`, and
    /// snap cursor location to within viewport.
    /// Also might result in setting scroll needed.
    fn reflow(&mut self, start: usize) {
        let mut tl_idx = start;
        while tl_idx < self.buffer.len() {
            match self.buffer[tl_idx].width.cmp(&self.display_width) {
                Ordering::Less => {
                    if self.try_fill_from_next(tl_idx).is_none()
                        || self.buffer[tl_idx].width == self.display_width
                    {
                        tl_idx += 1;
                    }
                }
                Ordering::Greater => {
                    self.move_overflow_to_next(tl_idx);
                    tl_idx += 1;
                }
                Ordering::Equal => {
                    if tl_idx == self.cursor.index.line
                        && self.cursor.column >= self.display_width
                    {
                        self.cursor.line += 1;
                        self.cursor.column = 0;
                        self.cursor.index.line += 1;
                        self.cursor.index.offset = 0;
                        if self.cursor.index.line == self.buffer.len() {
                            self.buffer.push(BufferLine::new());
                        }
                    }
                    tl_idx += 1;
                }
            }
        }

        if self.buffer.last().unwrap().width == self.display_width {
            self.buffer.push(BufferLine::new());
        }

        self.adjust_viewport();
    }

    // attempt to fill this line, up to but not beyond,
    // display_width.
    // returns Some(prev_line_len) (i.e., idx of first
    // moved char), or None if no chars moved
    fn try_fill_from_next(&mut self, tl_idx: usize) -> Option<(usize, usize)> {
        if tl_idx == self.buffer.len() - 1 {
            return None;
        }

        let tl_width = self.buffer[tl_idx].width;
        let nl_idx = tl_idx + 1;
        let moved = self.buffer[nl_idx].text.char_indices().try_fold(
            (0, 0),
            |(res_idx, cols_moved), (i, c)| {
                let c_width = c.width().unwrap_or(0);
                if self.display_width >= (tl_width + cols_moved + c_width) {
                    ControlFlow::Continue((i + 1, cols_moved + c_width))
                } else {
                    ControlFlow::Break((res_idx, cols_moved))
                }
            },
        );
        let (res_idx, cols_moved) = match moved {
            ControlFlow::Continue(result) | ControlFlow::Break(result) => {
                result
            }
        };
        if res_idx > 0 {
            if self.cursor.index.line == nl_idx {
                // if cursor was on next line, adjust cursor
                if self.cursor.index.offset < res_idx
                    || res_idx == self.buffer[nl_idx].text.len()
                {
                    // char at cursor moved to this line
                    self.cursor.line -= 1;
                    self.cursor.column += tl_width;
                    self.cursor.index.line -= 1;
                    self.cursor.index.offset += self.buffer[tl_idx].len();
                } else {
                    // cursor still on next line
                    self.cursor.index.offset -= res_idx;
                    self.cursor.column -= cols_moved;
                }
            }

            if self.input_start.line == nl_idx {
                // if input_start was on next line, adjust it
                if self.input_start.offset < res_idx
                    || res_idx == self.buffer[nl_idx].text.len()
                {
                    // input_start moved to this line
                    self.input_start.line -= 1;
                    self.input_start.offset += self.buffer[tl_idx].len();
                } else {
                    // input_start still on next line
                    self.input_start.offset -= res_idx;
                }
            }

            let (this_part, next_part) = self.buffer.split_at_mut(nl_idx);
            let this_line = &mut this_part[tl_idx];
            let next_line = &mut next_part[0];
            this_line.text.extend(next_line.text.drain(..res_idx));
            this_line.width += cols_moved;
            next_line.width -= cols_moved;
        }

        if self.buffer[nl_idx].text.is_empty()
            && self.buffer[tl_idx].width < self.display_width
        {
            self.buffer.remove(nl_idx);
            if self.cursor.index.line > tl_idx {
                self.cursor.index.line -= 1;
                self.cursor.line -= 1;
            }
        }

        match res_idx {
            0 => None,
            _ => Some((res_idx, cols_moved)),
        }
    }

    fn move_overflow_to_next(&mut self, tl_idx: usize) {
        assert!(self.buffer[tl_idx].width > self.display_width);
        // check to see if there's a next_line & push one if not
        if tl_idx == self.buffer.len() - 1 {
            self.buffer.push(BufferLine::new());
        }

        // move this_line's residue to beginning of next line
        let mut cols = 0;
        let (this, next) = self.buffer.split_at_mut(tl_idx + 1);
        let (this, next) = (&mut this[tl_idx], &mut next[0]);
        let (res_idx, _) = this
            .text
            .char_indices()
            .find(|(_, c)| {
                let c_width = c.width().unwrap_or(0);
                if self.display_width - cols < c_width {
                    true
                } else {
                    cols += c_width;
                    false
                }
            })
            .unwrap();
        let cols_moved = this.width - cols;
        let bytes_moved = this.len() - res_idx;
        this.width = cols;
        next.width += cols_moved;
        next.text.insert_str(0, this.text.drain(res_idx..).as_str());

        if tl_idx == self.cursor.index.line
            && res_idx <= self.cursor.index.offset
        {
            // if this was the cursor line & char at cursor moved,
            // adjust cursor
            self.cursor.line += 1;
            self.cursor.column -= this.width;
            self.cursor.index.line += 1;
            self.cursor.index.offset -= res_idx;
        } else if self.cursor.index.line == tl_idx + 1 {
            // if next line was cursor line, adjust cursor column
            self.cursor.column += cols_moved;
            self.cursor.index.offset += bytes_moved;
        }

        if tl_idx == self.input_start.line && res_idx <= self.input_start.offset
        {
            // if the line where input_start is located, and chars at or
            // before input start moved, adjust input_start
            self.input_start.line += 1;
            self.input_start.offset -= res_idx;
        } else if self.input_start.line == tl_idx + 1 {
            // if next line was input_start.line, adjust input_start column
            self.input_start.offset += bytes_moved;
        }
    }

    #[cfg(not(tarpaulin_include))]
    /// render current buffer to display
    fn repaint(&mut self) -> io::Result<()> {
        let display_lines = self.display_height - self.first_display_line;
        let last_displayed =
            self.first_buffer_line + self.buffer.len().min(display_lines);

        let mut stdout = io::stdout().lock();

        stdout.queue(Hide)?;

        // convert values to u16 for crossterm
        let first_display_line = u16::try_from(self.first_display_line)
            .expect("first_display_line fits u16");
        let cursor_column =
            u16::try_from(self.cursor.column).expect("cursor column fits u16");
        let cursor_line =
            u16::try_from(self.cursor.line).expect("cursor line fits u16");

        stdout
            .queue(MoveTo(0, first_display_line))?
            .queue(Clear(ClearType::FromCursorDown))?;

        if self.scroll_needed > 0 {
            let scroll_needed = u16::try_from(self.scroll_needed)
                .expect("scroll needed fits in u16");
            stdout.queue(ScrollUp(scroll_needed - 1))?;
            self.scroll_needed = 0;
        }

        for line in &self.buffer[self.first_buffer_line..last_displayed] {
            stdout.write_all(line.text.as_bytes())?;
        }

        stdout.queue(MoveTo(cursor_column, cursor_line))?.queue(Show)?.flush()
    }
}

impl LineRead for LineReader {
    #[cfg(not(tarpaulin_include))]
    fn read_line(
        &mut self,
        prompt: &str,
        buffer: &mut String,
    ) -> io::Result<usize> {
        self.accept_line(prompt, buffer)
    }
}

// impls for RenderContext
////////

impl RenderContext {
    fn new() -> RenderContext {
        RenderContext {}
    }
}

impl Drop for RenderContext {
    #[cfg(not(tarpaulin_include))]
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
        let _ = io::stdout().execute(Show);
    }
}

// impls of LineRead
////////

impl<T> LineRead for T
where
    T: BufRead,
{
    fn read_line(
        &mut self,
        _prompt: &str,
        buffer: &mut String,
    ) -> io::Result<usize> {
        BufRead::read_line(self, buffer)
    }
}

#[cfg(test)]
#[allow(clippy::unicode_not_nfc)]
mod tests {
    use super::*;

    use crossterm::event::KeyModifiers;

    use similar_asserts::assert_eq;

    #[derive(Debug)]
    pub struct LineReaderBuilder {
        display_width: usize,
        display_height: usize,
        prompt_char_count: usize,
        text: Option<Vec<String>>,
        history: Vec<String>,
        history_idx: Option<usize>,
        edited_input: Option<String>,
        edited_history: Option<String>,
        input_start: BufferIndex,
        first_display_line: usize,
        first_buffer_line: usize,
        cursor: Cursor,
        scroll_needed: usize,
    }

    impl LineReaderBuilder {
        fn new(display_width: usize, display_height: usize) -> Self {
            LineReaderBuilder {
                display_width,
                display_height,
                text: None,
                prompt_char_count: 0,
                history: Vec::new(),
                history_idx: None,
                edited_input: None,
                edited_history: None,
                input_start: BufferIndex { line: 0, offset: 0 },
                first_display_line: 0,
                first_buffer_line: 0,
                scroll_needed: 0,
                cursor: Cursor { ..Default::default() },
            }
        }

        fn display_width(&mut self, w: usize) -> &mut Self {
            self.display_width = w;
            self
        }

        fn display_height(&mut self, h: usize) -> &mut Self {
            self.display_height = h;
            self
        }

        fn text<S>(&mut self, t: &[S]) -> &mut Self
        where
            S: AsRef<str>,
        {
            self.text = Some(
                t.as_ref().iter().map(|s| s.as_ref().to_owned()).collect(),
            );
            self
        }

        fn prompt_char_count(&mut self, n: usize) -> &mut Self {
            self.prompt_char_count = n;
            self
        }

        fn history<S>(&mut self, h: &[S]) -> &mut Self
        where
            S: AsRef<str>,
        {
            self.history =
                h.as_ref().iter().map(|s| s.as_ref().to_owned()).collect();
            self
        }

        fn history_idx(&mut self, i: Option<usize>) -> &mut Self {
            self.history_idx = i;
            self
        }

        fn edited_input<S>(&mut self, ei: Option<S>) -> &mut Self
        where
            S: AsRef<str>,
        {
            self.edited_input = ei.map(|s| s.as_ref().to_owned());
            self
        }

        fn edited_history<S>(&mut self, eh: Option<S>) -> &mut Self
        where
            S: AsRef<str>,
        {
            self.edited_history = eh.map(|s| s.as_ref().to_owned());
            self
        }

        fn input_start(&mut self, i: BufferIndex) -> &mut Self {
            self.input_start = i;
            self
        }

        fn first_buffer_line(&mut self, l: usize) -> &mut Self {
            self.first_buffer_line = l;
            self
        }

        fn first_display_line(&mut self, l: usize) -> &mut Self {
            self.first_display_line = l;
            self
        }

        fn cursor(&mut self, c: Cursor) -> &mut Self {
            self.cursor = c;
            self
        }

        fn scroll_needed(&mut self, n: usize) -> &mut Self {
            self.scroll_needed = n;
            self
        }

        fn build(&self) -> LineReader {
            let mut buffer = self.text.as_ref().map_or_else(
                || vec![BufferLine { text: String::new(), width: 0 }],
                |t| {
                    t.iter()
                        .cloned()
                        .map(|text| {
                            let width = text.width();
                            BufferLine { text, width }
                        })
                        .collect::<Vec<BufferLine>>()
                },
            );
            let last_line = buffer.last();
            if last_line.is_none()
                || last_line.is_some_and(|l| l.width == self.display_width)
            {
                buffer.push(BufferLine::new());
            }

            LineReader {
                buffer,
                prompt_char_count: self.prompt_char_count,
                history: self.history.clone(),
                history_idx: self.history_idx,
                edited_input: self.edited_input.clone(),
                edited_history: self.edited_history.clone(),
                input_start: self.input_start,
                display_width: self.display_width,
                display_height: self.display_height,
                cursor: self.cursor,
                first_display_line: self.first_display_line,
                first_buffer_line: self.first_buffer_line,
                scroll_needed: self.scroll_needed,
            }
        }
    }

    #[test]
    fn builder_base_case() {
        let b = LineReaderBuilder::new(10, 5);
        let r = b.build();
        assert_eq!(
            r,
            LineReader {
                display_width: 10,
                display_height: 5,
                ..Default::default()
            }
        );
    }

    #[test]
    fn builder_simple_case() {
        let mut b = LineReaderBuilder::new(10, 5);
        let r = b
            .text(&[":ë🎸o"])
            .cursor(Cursor {
                line: 0,
                column: 5,
                index: BufferIndex { line: 0, offset: 9 },
            })
            .build();
        assert_eq!(
            r,
            LineReader {
                buffer: vec![BufferLine {
                    text: ":ë🎸o".to_owned(), width: 5
                },],
                cursor: Cursor {
                    line: 0,
                    column: 5,
                    index: BufferIndex { line: 0, offset: 9 },
                },
                display_width: 10,
                display_height: 5,
                ..Default::default()
            }
        );
    }

    #[test]
    fn builder_full_case() {
        let expected = LineReader {
            buffer: vec![
                BufferLine { text: ":123456789abcde".to_owned(), width: 15 },
                BufferLine { text: "🎸23456789abcdef".to_owned(), width: 16 },
                BufferLine { text: "🎸23456789abcdef".to_owned(), width: 16 },
                BufferLine { text: "🎸23456789abcdef".to_owned(), width: 16 },
                BufferLine { text: "🎸23456789abcdef".to_owned(), width: 16 },
                BufferLine { text: "🎸23456789abcdef".to_owned(), width: 16 },
                BufferLine { text: "012345".to_owned(), width: 6 },
            ],
            prompt_char_count: 1,
            history: Vec::new(),
            history_idx: None,
            edited_input: None,
            edited_history: None,
            input_start: BufferIndex { line: 2, offset: 6 },
            display_width: 16,
            display_height: 6,
            cursor: Cursor {
                line: 5,
                column: 6,
                index: BufferIndex { line: 6, offset: 6 },
            },
            first_display_line: 0,
            first_buffer_line: 1,
            scroll_needed: 0,
        };
        let mut b = LineReaderBuilder::new(16, 6);
        b.text(&[
            ":123456789abcde",
            "🎸23456789abcdef",
            "🎸23456789abcdef",
            "🎸23456789abcdef",
            "🎸23456789abcdef",
            "🎸23456789abcdef",
            "012345",
        ]);
        b.input_start(BufferIndex { line: 2, offset: 6 }).cursor(Cursor {
            line: 5,
            column: 6,
            index: BufferIndex { line: 6, offset: 6 },
        });
        b.first_display_line(0).first_buffer_line(1).prompt_char_count(1);
        let r = b.build();
        assert_eq!(r, expected);
    }
    #[test]
    fn viewport_all_within_display() {
        let mut b = LineReaderBuilder::new(10, 5);
        b.text(&[":123456789", "0123456789", "012345"])
            .input_start((0, 1).into());
        b.cursor(Cursor { column: 6, line: 2, index: (2, 6).into() });
        let reader = b.build();
        assert_eq!(reader.viewport_bottom(), reader.display_height - 1);
        assert_eq!(reader.viewport_top(), 0);
    }

    #[test]
    fn viewport_buffer_beyond_top() {
        let mut b = LineReaderBuilder::new(10, 5);
        b.text(&[
            ":123456789",
            "0123456789",
            "0123456789",
            "0123456789",
            "0123456789",
            "0123456789",
            "012345",
        ])
        .input_start((0, 1).into())
        .cursor(Cursor { column: 6, line: 4, index: (6, 6).into() })
        .first_buffer_line(2);
        let reader = b.build();
        let vp_bottom = reader.viewport_bottom();
        let vp_top = reader.viewport_top();
        assert_eq!(vp_bottom, reader.display_height - 1);
        assert_eq!(vp_top, 1);
    }

    #[test]
    fn viewport_buffer_beyond_bottom() {
        let mut b = LineReaderBuilder::new(10, 5);
        b.text(&[
            ":123456789",
            "0123456789",
            "0123456789",
            "0123456789",
            "0123456789",
            "0123456789",
            "012345",
        ])
        .input_start((0, 1).into())
        .cursor(Cursor { column: 1, line: 0, index: (0, 1).into() });
        let reader = b.build();
        assert_eq!(reader.viewport_bottom(), reader.display_height - 2);
        assert_eq!(reader.viewport_top(), 0);
    }

    #[test]
    fn viewport_buffer_beyond_both() {
        let mut b = LineReaderBuilder::new(10, 5);
        b.text(&[
            ":123456789",
            "0123456789",
            "0123456789",
            "0123456789",
            "0123456789",
            "0123456789",
            "012345",
        ])
        .input_start((0, 1).into())
        .cursor(Cursor { column: 5, line: 2, index: (3, 5).into() })
        .first_buffer_line(1);
        let reader = b.build();
        assert_eq!(reader.viewport_bottom(), reader.display_height - 2);
        assert_eq!(reader.viewport_top(), 1);
    }

    #[test]
    fn unimplemented_event_ignored() {
        let mut reader = LineReader::new();
        let event = Event::FocusLost;
        let res = reader.handle_event(&event);
        assert!(res.is_continue());
    }

    #[test]
    fn unimplemented_key_event_ignored() {
        let mut reader = LineReader::new();
        let event =
            Event::Key(KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE));
        let res = reader.handle_event(&event);
        assert!(res.is_continue());
    }

    #[test]
    fn enter_breaks_input_loop() {
        let text = "This is some text.".to_owned();
        let width = text.width();
        let mut reader = LineReader {
            buffer: vec![BufferLine { text, width }],
            ..Default::default()
        };
        let event =
            Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        let res = reader.handle_event(&event);
        assert!(matches!(res, ControlFlow::Break(true)));
    }

    #[test]
    fn char_input_non_0w_inserts() {
        let mut b = LineReaderBuilder::new(10, 5);
        let mut reader = b.build();

        b.text(&["🎸"]).cursor(Cursor {
            column: 2,
            line: 0,
            index: BufferIndex { line: 0, offset: 4 },
        });
        let expected = b.build();

        let event =
            Event::Key(KeyEvent::new(KeyCode::Char('🎸'), KeyModifiers::NONE));
        let res = reader.handle_event(&event);
        assert!(res.is_continue());
        assert_eq!(reader, expected);
    }

    #[test]
    fn char_input_0w_requires_base_char() {
        let mut b = LineReaderBuilder::new(10, 5);
        b.text(&[":"]).input_start(BufferIndex { line: 0, offset: 1 }).cursor(
            Cursor {
                line: 0,
                column: 1,
                index: BufferIndex { line: 0, offset: 1 },
            },
        );

        let mut reader = b.build();

        let expected = b.build();

        let event = Event::Key(KeyEvent::new(
            KeyCode::Char('\u{0308}'),
            KeyModifiers::NONE,
        ));

        let res = reader.handle_event(&event);
        assert!(res.is_continue());
        assert_eq!(reader, expected);

        b.text(&[":a"]).input_start(BufferIndex { line: 0, offset: 1 }).cursor(
            Cursor {
                line: 0,
                column: 2,
                index: BufferIndex { line: 0, offset: 2 },
            },
        );
        let mut reader = b.build();

        b.text(&[":ä"]).input_start(BufferIndex { line: 0, offset: 1 }).cursor(
            Cursor {
                line: 0,
                column: 2,
                index: BufferIndex { line: 0, offset: 4 },
            },
        );
        let expected = b.build();

        let event = Event::Key(KeyEvent::new(
            KeyCode::Char('\u{0308}'),
            KeyModifiers::NONE,
        ));

        let res = reader.handle_event(&event);
        assert!(res.is_continue());
        assert_eq!(reader, expected);
    }

    #[test]
    fn char_input_before_eol_moves_cursor_char_width() {
        let mut b = LineReaderBuilder::new(10, 5);
        b.text(&[":e"])
            .input_start(BufferIndex { line: 0, offset: 1 })
            .cursor(Cursor { line: 0, column: 2, index: (0, 2).into() });
        let mut reader = b.build();

        let event = Event::Key(KeyEvent::new(
            KeyCode::Char('\u{0308}'),
            KeyModifiers::NONE,
        ));
        b.text(&[":ë"]).cursor(Cursor {
            line: 0,
            column: 2,
            index: (0, 4).into(),
        });
        let expected = b.build();
        let res = reader.handle_event(&event);
        assert!(res.is_continue());
        assert_eq!(reader, expected);

        let event =
            Event::Key(KeyEvent::new(KeyCode::Char('🎸'), KeyModifiers::NONE));
        b.text(&[":ë🎸"]).cursor(Cursor {
            line: 0,
            column: 4,
            index: (0, 8).into(),
        });
        let expected = b.build();
        let res = reader.handle_event(&event);
        assert!(res.is_continue());
        assert_eq!(reader, expected);

        let event =
            Event::Key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE));
        b.text(&[":ë🎸o"]).cursor(Cursor {
            line: 0,
            column: 5,
            index: (0, 9).into(),
        });
        let expected = b.build();
        let res = reader.handle_event(&event);
        assert!(res.is_continue());
        assert_eq!(reader, expected);
    }

    #[test]
    fn char_input_to_eol_wraps_cursor_to_next_line_start() {
        let mut b = LineReaderBuilder::new(10, 5);
        b.text(&[":1234567"]).input_start((0, 1).into()).cursor(Cursor {
            column: 8,
            line: 0,
            index: (0, 8).into(),
        });

        let mut reader = b.build();
        let event =
            Event::Key(KeyEvent::new(KeyCode::Char('🎸'), KeyModifiers::NONE));
        b.text(&[":1234567🎸", ""]).cursor(Cursor {
            column: 0,
            line: 1,
            index: (1, 0).into(),
        });
        let expected = b.build();
        let res = reader.handle_event(&event);
        assert!(res.is_continue());
        assert_eq!(reader, expected);
    }

    #[test]
    fn char_input_append_to_previous_line_if_fits() {
        let mut b = LineReaderBuilder::new(10, 5);
        b.text(&[":12345678", "🎸abc"])
            .input_start((0, 1).into())
            .cursor(Cursor { column: 0, line: 4, index: (1, 0).into() })
            .first_display_line(3);
        let mut reader = b.build();

        b.text(&[":123456789", "🎸abc"]);
        let expected = b.build();

        let event =
            Event::Key(KeyEvent::new(KeyCode::Char('9'), KeyModifiers::NONE));
        let res = reader.handle_event(&event);
        assert!(res.is_continue());
        assert_eq!(reader, expected);
    }

    #[test]
    fn char_input_char_too_wide_at_end_wraps_to_next_line() {
        let mut b = LineReaderBuilder::new(10, 5);
        b.text(&[":12345678"]).input_start((0, 1).into()).cursor(Cursor {
            column: 9,
            line: 0,
            index: (0, 9).into(),
        });
        let mut reader = b.build();

        b.text(&[":12345678", "🎸"]).cursor(Cursor {
            column: 2,
            line: 1,
            index: (1, 4).into(),
        });
        let expected = b.build();

        let event =
            Event::Key(KeyEvent::new(KeyCode::Char('🎸'), KeyModifiers::NONE));
        let res = reader.handle_event(&event);

        assert!(res.is_continue());
        assert_eq!(reader, expected);
    }

    #[test]
    fn char_input_past_eol_wraps_input_to_next_line_start() {
        let mut b = LineReaderBuilder::new(10, 5);
        b.text(&[":123456789", "abc"])
            .input_start((0, 1).into())
            .cursor(Cursor { column: 8, line: 0, index: (0, 8).into() });
        let mut reader = b.build();

        b.text(&[":1234567🎸", "89abc"]).cursor(Cursor {
            column: 0,
            line: 1,
            index: (1, 0).into(),
        });
        let expected = b.build();

        let event =
            Event::Key(KeyEvent::new(KeyCode::Char('🎸'), KeyModifiers::NONE));
        let res = reader.handle_event(&event);

        assert!(res.is_continue());
        assert_eq!(reader, expected);
    }

    #[test]
    fn char_input_at_end_of_small_buffer_moving_cursor_beyond_bottom() {
        let mut b = LineReaderBuilder::new(10, 5);
        b.text(&[":12345678", "🎸2345678"])
            .input_start((0, 1).into())
            .first_display_line(3)
            .cursor(Cursor { column: 9, line: 4, index: (1, 11).into() });

        let mut reader = b.build();

        b.text(&[":12345678", "🎸2345678a", ""])
            .first_display_line(2)
            .cursor(Cursor { column: 0, line: 4, index: (2, 0).into() })
            .scroll_needed(1);
        let expected = b.build();
        let event =
            Event::Key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
        let res = reader.handle_event(&event);
        assert!(res.is_continue());
        assert_eq!(reader, expected);
    }

    #[test]
    fn char_input_at_end_of_large_buffer_moving_cursor_beyond_bottom() {
        let mut b = LineReaderBuilder::new(10, 5);
        b.text(&[
            ":123456789",
            "0123456789",
            "0123456789",
            "0123456789",
            "012345678",
            "🎸2345678",
        ])
        .input_start((0, 1).into())
        .first_buffer_line(1)
        .cursor(Cursor { column: 9, line: 4, index: (5, 11).into() });

        let mut reader = b.build();

        b.text(&[
            ":123456789",
            "0123456789",
            "0123456789",
            "0123456789",
            "012345678",
            "🎸2345678a",
            "",
        ])
        .first_buffer_line(2)
        .cursor(Cursor { column: 0, line: 4, index: (6, 0).into() });
        let expected = b.build();
        let event =
            Event::Key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
        let res = reader.handle_event(&event);
        assert!(res.is_continue());
        assert_eq!(
            reader, expected,
            "\nleft: {reader:#?}\nright: {expected:#?}"
        );
    }

    #[test]
    fn char_input_within_small_buffer_extending_below_display() {
        let mut b = LineReaderBuilder::new(10, 5);
        b.text(&[":123456789", "012345678", "🎸2345678"])
            .input_start((0, 1).into())
            .first_display_line(3)
            .cursor(Cursor { column: 9, line: 3, index: (0, 9).into() });

        let mut reader = b.build();

        b.text(&[":12345678a", "9012345678", "🎸2345678"])
            .first_display_line(2)
            .cursor(Cursor { column: 0, line: 3, index: (1, 0).into() })
            .scroll_needed(1);
        let expected = b.build();
        let event =
            Event::Key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
        let res = reader.handle_event(&event);
        assert!(res.is_continue());
        assert_eq!(reader, expected);
    }

    #[test]
    fn char_input_within_large_buffer_extending_beyond_display() {
        let mut b = LineReaderBuilder::new(10, 5);
        b.text(&[
            ":123456789",
            "0123456789",
            "0123456789",
            "0123456789",
            "0123456789",
            "0123456789",
            "012345678",
        ])
        .input_start((0, 1).into())
        .first_buffer_line(1)
        .cursor(Cursor { column: 9, line: 3, index: (4, 9).into() });

        let mut reader = b.build();

        b.text(&[
            ":123456789",
            "0123456789",
            "0123456789",
            "0123456789",
            "012345678a",
            "9012345678",
            "9012345678",
            "",
        ])
        .first_buffer_line(2)
        .cursor(Cursor { column: 0, line: 3, index: (5, 0).into() });
        let expected = b.build();
        let event =
            Event::Key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
        let res = reader.handle_event(&event);
        assert!(res.is_continue());
        assert_eq!(
            reader, expected,
            "\nleft: {reader:#?}\nright: {expected:#?}"
        );
    }

    #[test]
    fn backspace_0w() {
        let mut b = LineReaderBuilder::new(10, 5);
        b.text(&[":ë"]).input_start((0, 1).into()).cursor(Cursor {
            column: 2,
            line: 0,
            index: (0, 4).into(),
        });
        let mut reader = b.build();

        b.text(&[":e"]).cursor(Cursor {
            column: 2,
            line: 0,
            index: (0, 2).into(),
        });
        let expected = b.build();

        let event =
            Event::Key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        let res = reader.handle_event(&event);
        assert!(res.is_continue());
        assert_eq!(reader, expected);
    }

    #[test]
    fn backspace_1w() {
        let mut b = LineReaderBuilder::new(10, 5);
        b.text(&[":e"]).input_start((0, 1).into()).cursor(Cursor {
            column: 2,
            line: 0,
            index: (0, 2).into(),
        });
        let mut reader = b.build();

        b.text(&[":"]).cursor(Cursor {
            column: 1,
            line: 0,
            index: (0, 1).into(),
        });
        let expected = b.build();

        let event =
            Event::Key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        let res = reader.handle_event(&event);
        assert!(res.is_continue());
        assert_eq!(reader, expected);
    }

    #[test]
    fn backspace_2w() {
        let mut b = LineReaderBuilder::new(10, 5);
        b.text(&[":🎸"]).input_start((0, 1).into()).cursor(Cursor {
            column: 3,
            line: 0,
            index: (0, 5).into(),
        });
        let mut reader = b.build();

        b.text(&[":"]).cursor(Cursor {
            column: 1,
            line: 0,
            index: (0, 1).into(),
        });
        let expected = b.build();

        let event =
            Event::Key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        let res = reader.handle_event(&event);
        assert!(res.is_continue());
        assert_eq!(reader, expected);
    }

    #[test]
    fn backspace_input_start() {
        let mut b = LineReaderBuilder::new(10, 5);
        b.text(&[":"]).input_start((0, 1).into()).cursor(Cursor {
            column: 1,
            line: 0,
            index: (0, 1).into(),
        });
        let mut reader = b.build();
        let expected = b.build();

        let event =
            Event::Key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));

        let res = reader.handle_event(&event);
        assert!(res.is_continue());
        assert_eq!(reader, expected);
    }

    #[test]
    fn backspace_to_column_0_wraps_if_room_on_preceding_line() {
        let mut b = LineReaderBuilder::new(10, 5);
        b.text(&[":12345678", "🎸9"]).input_start((0, 1).into());
        b.cursor(Cursor { column: 2, line: 1, index: (1, 4).into() });
        let mut reader = b.build();

        b.text(&[":123456789", ""]).cursor(Cursor {
            column: 9,
            line: 0,
            index: (0, 9).into(),
        });
        let expected = b.build();
        let event =
            Event::Key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        let res = reader.handle_event(&event);
        assert!(res.is_continue());
        assert_eq!(reader, expected);
    }

    #[test]
    fn backspace_from_column_0_wraps_if_room_on_preceding_line() {
        let mut b = LineReaderBuilder::new(10, 5);
        let event =
            Event::Key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));

        // base case
        b.text(&[":123456789", ""]).input_start((0, 1).into());
        b.cursor(Cursor { column: 0, line: 1, index: (1, 0).into() });
        let mut reader = b.build();

        b.text(&[":12345678"]).cursor(Cursor {
            column: 9,
            line: 0,
            index: (0, 9).into(),
        });
        let expected = b.build();
        let res = reader.handle_event(&event);
        assert!(res.is_continue());
        assert_eq!(reader, expected);

        // zero len char at preceding line end
        b.text(&[":12345678ä", "eiou"]);
        b.cursor(Cursor { column: 0, line: 1, index: (1, 0).into() });
        let mut reader = b.build();

        b.text(&[":12345678a", "eiou"]);
        let expected = b.build();
        let res = reader.handle_event(&event);
        assert!(res.is_continue());
        assert_eq!(reader, expected);
    }

    #[test]
    fn backspace_moving_cursor_above_top_pans_buffer() {
        let mut b = LineReaderBuilder::new(10, 5);
        b.text(&[
            ":123456789",
            "0123456789",
            "0123456789",
            "0123456789",
            "0123456789",
            "0123",
        ])
        .input_start((0, 1).into());
        b.first_buffer_line(1).cursor(Cursor {
            line: 1,
            column: 0,
            index: (2, 0).into(),
        });
        let mut reader = b.build();

        b.text(&[
            ":123456789",
            "0123456780",
            "1234567890",
            "1234567890",
            "1234567890",
            "123",
        ]);
        b.first_buffer_line(0).cursor(Cursor {
            line: 1,
            column: 9,
            index: (1, 9).into(),
        });
        let expected = b.build();

        let event =
            Event::Key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        let res = reader.handle_event(&event);
        assert!(res.is_continue());
        assert_eq!(reader, expected);
    }
    #[test]
    fn left_from_input_start_does_nothing() {
        let mut b = LineReaderBuilder::new(10, 5);
        b.text(&[":12345"]).input_start((0, 1).into());
        b.cursor(Cursor { column: 1, line: 0, index: (0, 1).into() });
        let mut reader = b.build();
        let expected = b.build();
        let event =
            Event::Key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        let res = reader.handle_event(&event);
        assert!(res.is_continue());
        assert_eq!(reader, expected);
    }

    #[test]
    fn left_moves_cursor_to_preceding_base_char() {
        let event =
            Event::Key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        let mut b = LineReaderBuilder::new(10, 5);
        b.text(&[":aë🎸iou"]).input_start((0, 1).into());
        b.cursor(Cursor { column: 6, line: 0, index: (0, 10).into() });
        let mut reader = b.build();

        b.cursor(Cursor { column: 5, line: 0, index: (0, 9).into() });
        let expected = b.build();
        let res = reader.handle_event(&event);
        assert!(res.is_continue());
        assert_eq!(
            reader, expected,
            "\nleft: {reader:#?}\nright: {expected:#?}"
        );

        b.cursor(Cursor { column: 3, line: 0, index: (0, 5).into() });
        let expected = b.build();
        let res = reader.handle_event(&event);
        assert!(res.is_continue());
        assert_eq!(reader, expected);

        b.cursor(Cursor { column: 2, line: 0, index: (0, 2).into() });
        let expected = b.build();
        let res = reader.handle_event(&event);
        assert!(res.is_continue());
        assert_eq!(reader, expected);
    }

    #[test]
    fn left_from_column_0_moves_cursor_to_last_base_char_on_preceding_line() {
        let event =
            Event::Key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        let mut b = LineReaderBuilder::new(10, 5);
        b.text(&[":12345678", "🎸abc"]).input_start((0, 1).into());
        b.cursor(Cursor { column: 0, line: 1, index: (1, 0).into() });
        let mut reader = b.build();

        b.cursor(Cursor { column: 8, line: 0, index: (0, 8).into() });
        let expected = b.build();

        let res = reader.handle_event(&event);
        assert!(res.is_continue());
        assert_eq!(reader, expected);
    }

    #[test]
    fn left_moving_cursor_above_top_pans_buffer_down_one_line() {
        let event =
            Event::Key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        let mut b = LineReaderBuilder::new(10, 5);
        b.text(&[
            ":123456789",
            "012345678",
            "🎸23456789",
            "0123456789",
            "012345678",
            "🎸abc",
        ])
        .input_start((0, 1).into());
        b.first_buffer_line(1);
        b.cursor(Cursor { column: 0, line: 1, index: (2, 0).into() });
        let mut reader = b.build();

        b.first_buffer_line(0);
        b.cursor(Cursor { column: 8, line: 1, index: (1, 8).into() });
        let expected = b.build();

        let res = reader.handle_event(&event);
        assert!(res.is_continue());
        assert_eq!(reader, expected);
    }

    #[test]
    fn home_from_input_start_does_nothing() {
        let event =
            Event::Key(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE));
        let mut b = LineReaderBuilder::new(10, 5);
        b.text(&[":123456789", "0123456789", "012345678", "🎸abcd"])
            .input_start((0, 1).into());
        b.cursor(Cursor { column: 1, line: 0, index: (0, 1).into() });
        let mut reader = b.build();
        let expected = b.build();
        let res = reader.handle_event(&event);
        assert!(res.is_continue());
        assert_eq!(reader, expected);
    }

    #[test]
    fn home_moves_cursor_to_input_start() {
        let event =
            Event::Key(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE));
        let mut b = LineReaderBuilder::new(10, 5);
        b.text(&[":123456789", "0123456789", "012345678", "🎸abcd"])
            .input_start((0, 1).into());
        b.cursor(Cursor { column: 0, line: 3, index: (3, 0).into() });
        let mut reader = b.build();
        b.cursor(Cursor { column: 1, line: 0, index: (0, 1).into() });
        let expected = b.build();
        let res = reader.handle_event(&event);
        assert!(res.is_continue());
        assert_eq!(reader, expected);
    }

    #[test]
    fn home_moving_cursor_above_top_pans_buffer() {
        let event =
            Event::Key(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE));
        let mut b = LineReaderBuilder::new(10, 5);
        b.text(&[":123456789", "0123456789", "012345678", "🎸abcd"])
            .input_start((0, 1).into());
        b.first_buffer_line = 2;
        b.cursor(Cursor { column: 0, line: 1, index: (3, 0).into() });
        let mut reader = b.build();
        b.first_buffer_line = 0;
        b.cursor(Cursor { column: 1, line: 0, index: (0, 1).into() });
        let expected = b.build();
        let res = reader.handle_event(&event);
        assert!(res.is_continue());
        assert_eq!(reader, expected);
    }
    #[test]
    fn right_at_buffer_end_does_nothing() {
        let event =
            Event::Key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        let mut b = LineReaderBuilder::new(10, 5);
        b.text(&[":123456"]).input_start((0, 1).into());
        b.cursor(Cursor { column: 7, line: 0, index: (0, 7).into() });
        let mut reader = b.build();
        let expected = b.build();
        let res = reader.handle_event(&event);
        assert!(res.is_continue());
        assert_eq!(reader, expected);
    }

    #[test]
    fn right_moves_cursor_to_next_base_char() {
        let event =
            Event::Key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        let mut b = LineReaderBuilder::new(10, 5);
        b.text(&[":aë🎸ou"]).input_start((0, 1).into());
        b.cursor(Cursor { column: 1, line: 0, index: (0, 1).into() });
        let mut reader = b.build();

        b.cursor(Cursor { column: 2, line: 0, index: (0, 2).into() });
        let expected = b.build();
        let res = reader.handle_event(&event);
        assert!(res.is_continue());
        assert_eq!(reader, expected);

        b.cursor(Cursor { column: 3, line: 0, index: (0, 5).into() });
        let expected = b.build();
        let res = reader.handle_event(&event);
        assert!(res.is_continue());
        assert_eq!(reader, expected);

        b.cursor(Cursor { column: 5, line: 0, index: (0, 9).into() });
        let expected = b.build();
        let res = reader.handle_event(&event);
        assert!(res.is_continue());
        assert_eq!(reader, expected);
    }

    #[test]
    fn right_from_last_base_char_moves_to_next_column_0() {
        let event =
            Event::Key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        let mut b = LineReaderBuilder::new(10, 5);
        b.text(&[":12345678", "🎸23456789", ""]).input_start((0, 1).into());
        b.cursor(Cursor { line: 0, column: 8, index: (0, 8).into() });
        let mut reader = b.build();
        b.cursor(Cursor { line: 1, column: 0, index: (1, 0).into() });
        let expected = b.build();
        let res = reader.handle_event(&event);
        assert!(res.is_continue());
        assert_eq!(reader, expected);

        b.cursor(Cursor { line: 1, column: 9, index: (1, 11).into() });
        let mut reader = b.build();
        b.cursor(Cursor { line: 2, column: 0, index: (2, 0).into() });
        let expected = b.build();
        let res = reader.handle_event(&event);
        assert!(res.is_continue());
        assert_eq!(reader, expected);
    }

    #[test]
    fn right_past_bottom_of_large_buffer_pans_buffer_up() {
        let event =
            Event::Key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        let mut b = LineReaderBuilder::new(10, 5);
        b.text(&[
            ":1234567ö",
            "🎸23456789",
            "0123456789",
            "0123456789",
            "0123456789",
            "0123456789",
            "abc",
        ])
        .input_start((0, 1).into());
        b.cursor(Cursor { line: 3, column: 9, index: (3, 9).into() });
        let mut reader = b.build();

        b.first_buffer_line(1);
        b.cursor(Cursor { line: 3, column: 0, index: (4, 0).into() });
        let expected = b.build();

        let res = reader.handle_event(&event);
        assert!(res.is_continue());
        assert_eq!(reader, expected);
    }

    #[test]
    fn end_at_buffer_end_does_nothing() {
        let event = Event::Key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE));
        let mut b = LineReaderBuilder::new(10, 5);
        b.text(&[
            ":123456789",
            "0123456789",
            "0123456789",
            "0123456789",
            "0123456789",
            "0123456789",
            "0123456789",
            "0123456789",
            "0123456789",
        ])
        .input_start((0, 1).into());
        b.first_buffer_line(5);
        b.cursor(Cursor { column: 0, line: 4, index: (9, 0).into() });
        let mut reader = b.build();
        let expected = b.build();
        let ret = reader.handle_event(&event);
        assert!(ret.is_continue());
        assert_eq!(reader, expected);
    }

    #[test]
    fn end_moves_cursor_to_buffer_end() {
        let event = Event::Key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE));
        let mut b = LineReaderBuilder::new(10, 5);
        b.text(&[
            ":123456789",
            "0123456789",
            "0123456789",
            "0123456789",
            "0123456789",
            "0123456789",
            "0123456789",
            "0123456789",
            "0123456789",
        ])
        .input_start((0, 1).into());
        b.first_buffer_line(5);
        b.cursor(Cursor { column: 5, line: 3, index: (8, 5).into() });
        let mut reader = b.build();

        b.cursor(Cursor { column: 0, line: 4, index: (9, 0).into() });
        let expected = b.build();
        let ret = reader.handle_event(&event);
        assert!(ret.is_continue());
        assert_eq!(reader, expected);
    }

    #[test]
    fn end_past_display_bottom_in_small_buffer_scrolls_up() {
        let event = Event::Key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE));
        let mut b = LineReaderBuilder::new(10, 5);
        b.text(&[":123456789", "0123456789", "0123456789", "0123456789"])
            .input_start((0, 1).into());
        b.first_buffer_line(0);
        b.first_display_line(3);
        b.cursor(Cursor { column: 1, line: 3, index: b.input_start });
        let mut reader = b.build();

        b.cursor(Cursor { column: 0, line: 4, index: (4, 0).into() });
        b.first_display_line(0);
        b.scroll_needed(3);
        let expected = b.build();
        let ret = reader.handle_event(&event);
        assert!(ret.is_continue());
        assert_eq!(reader, expected);
    }

    #[test]
    fn end_past_display_bottom_in_large_buffer_pans_up() {
        let event = Event::Key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE));
        let mut b = LineReaderBuilder::new(10, 5);
        b.text(&[
            ":123456789",
            "0123456789",
            "0123456789",
            "0123456789",
            "0123456789",
            "0123456789",
            "0123456789",
            "0123456789",
            "0123456789",
        ])
        .input_start((0, 1).into());
        b.first_buffer_line(0);
        b.cursor(Cursor { column: 1, line: 0, index: b.input_start });
        let mut reader = b.build();

        b.cursor(Cursor { column: 0, line: 4, index: (9, 0).into() });
        b.first_buffer_line(5);
        let expected = b.build();
        let ret = reader.handle_event(&event);
        assert!(ret.is_continue());
        assert_eq!(reader, expected);
    }

    #[test]
    fn delete_at_buffer_end_does_nothing() {
        let event =
            Event::Key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE));
        let mut b = LineReaderBuilder::new(10, 5);
        b.text(&[":aë🎸io"]).input_start((0, 1).into());
        b.cursor(Cursor { column: 7, line: 0, index: (0, 11).into() });
        let mut reader = b.build();
        let expected = b.build();
        let res = reader.handle_event(&event);
        assert!(res.is_continue());
        assert_eq!(reader, expected);
    }

    #[test]
    fn delete_removes_chars_from_cursor_to_next_base_char() {
        let event =
            Event::Key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE));
        let mut b = LineReaderBuilder::new(10, 5);
        b.text(&[":aë🎸io"]).input_start((0, 1).into());
        b.cursor(Cursor { column: 2, line: 0, index: (0, 2).into() });
        let mut reader = b.build();

        b.text(&[":a🎸io"]);
        let expected = b.build();
        let res = reader.handle_event(&event);
        assert!(res.is_continue());
        assert_eq!(reader, expected);

        b.text(&[":aio"]);
        let expected = b.build();
        let res = reader.handle_event(&event);
        assert!(res.is_continue());
        assert_eq!(reader, expected);

        b.text(&[":ao"]);
        let expected = b.build();
        let res = reader.handle_event(&event);
        assert!(res.is_continue());
        assert_eq!(reader, expected);
    }

    #[test]
    fn delete_at_line_start_wraps_to_previous_if_new_first_char_fits() {
        let event =
            Event::Key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE));
        let mut b = LineReaderBuilder::new(10, 5);
        b.text(&[":12345678", "🎸abc"]).input_start((0, 1).into());
        b.cursor(Cursor { column: 0, line: 1, index: (1, 0).into() });
        let mut reader = b.build();

        b.text(&[":12345678a", "bc"]);
        b.cursor(Cursor { column: 9, line: 0, index: (0, 9).into() });
        let expected = b.build();
        let res = reader.handle_event(&event);
        assert!(res.is_continue());
        assert_eq!(reader, expected);
    }

    #[test]
    fn delete_reflows_buffer_from_new_cursor_line() {
        let event =
            Event::Key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE));
        let mut b = LineReaderBuilder::new(10, 5);
        b.text(&[":123456789", "0123456789", "0123456789", "0123456789"])
            .input_start((0, 1).into());
        b.cursor(Cursor { column: 9, line: 0, index: (0, 9).into() });
        let mut reader = b.build();

        b.text(&[":123456780", "1234567890", "1234567890", "123456789"]);
        b.cursor(Cursor { column: 9, line: 0, index: (0, 9).into() });
        let expected = b.build();
        let res = reader.handle_event(&event);
        assert!(res.is_continue());
        assert_eq!(reader, expected);
    }

    #[test]
    fn resize_height_smaller_cursor_at_end() {
        let mut b = LineReaderBuilder::new(10, 10);
        b.text(&[
            ":123456789",
            "012345678",
            "🎸23456789",
            "0123456789",
            "0123456789",
            "0123456789",
            "äbc",
        ])
        .input_start((0, 1).into())
        .first_display_line(3)
        .cursor(Cursor { column: 3, line: 9, index: (6, 5).into() });
        let mut reader = b.build();

        b.display_height(8).first_display_line(1).cursor(Cursor {
            column: 3,
            line: 7,
            index: (6, 5).into(),
        });
        let expected = b.build();
        let res = reader.handle_event(&Event::Resize(10, 8));
        assert!(res.is_continue());
        assert_eq!(reader, expected);

        b.display_height(7).first_display_line(0).cursor(Cursor {
            column: 3,
            line: 6,
            index: (6, 5).into(),
        });
        let expected = b.build();
        let res = reader.handle_event(&Event::Resize(10, 7));
        assert!(res.is_continue());
        assert_eq!(reader, expected);

        b.display_height(5).first_buffer_line(2).cursor(Cursor {
            column: 3,
            line: 4,
            index: (6, 5).into(),
        });
        let expected = b.build();
        let res = reader.handle_event(&Event::Resize(10, 5));
        assert!(res.is_continue());
        assert_eq!(reader, expected);
    }

    #[test]
    fn resize_height_smaller_cursor_at_start() {
        let mut b = LineReaderBuilder::new(10, 10);
        b.text(&[
            ":123456789",
            "012345678",
            "🎸23456789",
            "0123456789",
            "0123456789",
            "0123456789",
            "äbc",
        ])
        .input_start((0, 1).into())
        .first_display_line(3)
        .cursor(Cursor { column: 1, line: 3, index: (0, 1).into() });
        let mut reader = b.build();

        b.display_height(8).first_display_line(1).cursor(Cursor {
            column: 1,
            line: 1,
            index: (0, 1).into(),
        });
        let expected = b.build();
        let res = reader.handle_event(&Event::Resize(10, 8));
        assert!(res.is_continue());
        assert_eq!(reader, expected);

        b.display_height(7).first_display_line(0).cursor(Cursor {
            column: 1,
            line: 0,
            index: (0, 1).into(),
        });
        let expected = b.build();
        let res = reader.handle_event(&Event::Resize(10, 7));
        assert!(res.is_continue());
        assert_eq!(reader, expected);

        b.display_height(5).cursor(Cursor {
            column: 1,
            line: 0,
            index: (0, 1).into(),
        });
        let expected = b.build();
        let res = reader.handle_event(&Event::Resize(10, 5));
        assert!(res.is_continue());
        assert_eq!(reader, expected);
    }

    #[test]
    fn resize_width_smaller_cursor_at_start() {
        let mut b = LineReaderBuilder::new(10, 10);
        b.text(&[
            ":123456789",
            "012345678",
            "🎸23456789",
            "0123456789",
            "0123456789",
            "0123456789",
            "äbcdefgh",
        ])
        .input_start((0, 1).into())
        .first_display_line(3)
        .cursor(Cursor { column: 1, line: 3, index: (0, 1).into() });
        let mut reader = b.build();

        b.text(&[
            ":12345", "678901", "234567", "8🎸234", "567890", "123456",
            "789012", "345678", "901234", "56789ä", "bcdefg", "h",
        ])
        .display_width(6);
        let expected = b.build();

        let res = reader.handle_event(&Event::Resize(6, 10));
        assert!(res.is_continue());
        assert_eq!(reader, expected);
    }

    #[test]
    fn resize_width_smaller_cursor_at_start_lg_prompt() {
        let mut b = LineReaderBuilder::new(10, 10);
        b.text(&[
            "lgprompt:9",
            "012345678",
            "🎸23456789",
            "0123456789",
            "0123456789",
            "0123456789",
            "äbcdefgh",
        ])
        .input_start((0, 9).into())
        .first_display_line(3)
        .cursor(Cursor { column: 9, line: 3, index: (0, 9).into() });
        let mut reader = b.build();

        b.text(&[
            "lgprom", "pt:901", "234567", "8🎸234", "567890", "123456",
            "789012", "345678", "901234", "56789ä", "bcdefg", "h",
        ])
        .input_start((1, 3).into())
        .cursor(Cursor { column: 3, line: 4, index: (1, 3).into() })
        .display_width(6);
        let expected = b.build();

        let res = reader.handle_event(&Event::Resize(6, 10));
        assert!(res.is_continue());
        assert_eq!(reader, expected);
    }

    #[test]
    fn resize_width_smaller_cursor_at_end() {
        let mut b = LineReaderBuilder::new(10, 10);
        b.text(&[
            ":123456789",
            "012345678",
            "🎸23456789",
            "0123456789",
            "0123456789",
            "0123456789",
            "äbcdefgh",
        ])
        .input_start((0, 1).into())
        .first_display_line(3)
        .cursor(Cursor { column: 8, line: 9, index: (6, 10).into() });
        let mut reader = b.build();

        b.text(&[
            ":12345", "678901", "234567", "8🎸234", "567890", "123456",
            "789012", "345678", "901234", "56789ä", "bcdefg", "h",
        ])
        .cursor(Cursor { column: 1, line: 9, index: (11, 1).into() })
        .first_display_line(0)
        .scroll_needed(3)
        .first_buffer_line(2)
        .display_width(6);
        let expected = b.build();

        let res = reader.handle_event(&Event::Resize(6, 10));
        assert!(res.is_continue());
        assert_eq!(reader, expected);
    }

    #[test]
    fn resize_height_larger_cursor_at_end() {
        let mut b = LineReaderBuilder::new(10, 6);
        b.text(&[
            ":123456789",
            "012345678",
            "🎸23456789",
            "0123456789",
            "0123456789",
            "0123456789",
            "0123456789",
            "0123456789",
            "äbcdefgh",
        ])
        .input_start((0, 1).into())
        .first_buffer_line(3)
        .cursor(Cursor { column: 8, line: 5, index: (8, 10).into() });
        let mut reader = b.build();

        let event = Event::Resize(10, 10);
        b.first_display_line(0).first_buffer_line(0).display_height(10);
        b.cursor(Cursor { column: 8, line: 8, index: (8, 10).into() });
        let expected = b.build();
        let res = reader.handle_event(&event);
        assert!(res.is_continue());
        assert_eq!(reader, expected);
    }

    #[test]
    fn resize_height_larger_cursor_at_start() {
        let mut b = LineReaderBuilder::new(10, 6);
        b.text(&[
            ":123456789",
            "012345678",
            "🎸23456789",
            "0123456789",
            "0123456789",
            "0123456789",
            "0123456789",
            "0123456789",
            "äbcdefgh",
        ])
        .input_start((0, 1).into())
        .cursor(Cursor { column: 1, line: 0, index: (0, 1).into() });
        let mut reader = b.build();

        let event = Event::Resize(10, 10);
        b.display_height(10);
        let expected = b.build();
        let res = reader.handle_event(&event);
        assert!(res.is_continue());
        assert_eq!(reader, expected);
    }

    #[test]
    fn resize_width_larger_cursor_at_start() {
        let mut b = LineReaderBuilder::new(6, 10);
        b.text(&[
            ":12345", "678901", "234567", "8🎸234", "567890", "123456",
            "789012", "345678", "901234", "56789ä", "bcdefg", "h",
        ])
        .input_start((0, 1).into());
        b.first_display_line(0).cursor(Cursor {
            column: 1,
            line: 0,
            index: (0, 1).into(),
        });
        let mut reader = b.build();

        b.text(&[
            ":123456789",
            "012345678",
            "🎸23456789",
            "0123456789",
            "0123456789",
            "0123456789",
            "äbcdefgh",
        ])
        .display_width(10);
        let expected = b.build();
        let res = reader.handle_event(&Event::Resize(10, 10));
        assert!(res.is_continue());
        assert_eq!(reader, expected);
    }

    #[test]
    fn resize_width_larger_cursor_at_start_lg_prompt() {
        let mut b = LineReaderBuilder::new(6, 10);
        b.text(&[
            "lgprom", "pt:901", "234567", "8🎸234", "567890", "123456",
            "789012", "345678", "901234", "56789ä", "bcdefg", "h",
        ])
        .input_start((1, 3).into());
        b.first_display_line(0).cursor(Cursor {
            column: 3,
            line: 1,
            index: (1, 3).into(),
        });
        let mut reader = b.build();

        b.text(&[
            "lgprompt:9",
            "012345678",
            "🎸23456789",
            "0123456789",
            "0123456789",
            "0123456789",
            "äbcdefgh",
        ])
        .display_width(10)
        .input_start((0, 9).into())
        .cursor(Cursor { column: 9, line: 0, index: (0, 9).into() });
        let expected = b.build();
        let res = reader.handle_event(&Event::Resize(10, 10));
        assert!(res.is_continue());
        assert_eq!(reader, expected);
    }

    #[test]
    fn resize_width_larger_cursor_at_end() {
        let mut b = LineReaderBuilder::new(6, 10);
        b.text(&[
            ":12345", "678901", "234567", "8🎸234", "567890", "123456",
            "789012", "345678", "901234", "56789ä", "bcdefg", "hi",
        ])
        .input_start((0, 1).into());
        b.first_buffer_line(2).cursor(Cursor {
            column: 2,
            line: 9,
            index: (11, 2).into(),
        });
        let mut reader = b.build();

        b.text(&[
            ":123456789",
            "012345678",
            "🎸23456789",
            "0123456789",
            "0123456789",
            "0123456789",
            "äbcdefghi",
        ])
        .display_width(10);
        b.first_buffer_line(0).cursor(Cursor {
            column: 9,
            line: 6,
            index: (6, 11).into(),
        });
        let expected = b.build();
        let res = reader.handle_event(&Event::Resize(10, 10));
        assert!(res.is_continue());
        assert_eq!(reader, expected);
    }

    #[test]
    fn up_nop_if_empty_history() {
        let mut b = LineReaderBuilder::new(10, 5);
        b.text(&[":abcdëf🎸"]).input_start((0, 1).into());
        b.cursor(Cursor { column: 9, line: 0, index: (0, 13).into() });
        let mut reader = b.build();
        let expected = b.build();
        let res = reader.handle_event(&Event::Key(KeyEvent::new(
            KeyCode::Up,
            KeyModifiers::NONE,
        )));
        assert!(res.is_continue());
        assert_eq!(reader, expected);
    }

    #[test]
    fn down_nop_if_empty_history() {
        let mut b = LineReaderBuilder::new(10, 5);
        b.text(&[":abcdëf🎸"]).input_start((0, 1).into());
        b.cursor(Cursor { column: 9, line: 0, index: (0, 13).into() });
        let mut reader = b.build();
        let expected = b.build();
        let res = reader.handle_event(&Event::Key(KeyEvent::new(
            KeyCode::Down,
            KeyModifiers::NONE,
        )));
        assert!(res.is_continue());
        assert_eq!(reader, expected);
    }

    #[test]
    fn esc_nop_if_empty_history() {
        let mut b = LineReaderBuilder::new(10, 5);
        b.text(&[":abcdëf🎸"]).input_start((0, 1).into());
        b.cursor(Cursor { column: 9, line: 0, index: (0, 13).into() });
        let mut reader = b.build();
        let expected = b.build();
        let res = reader.handle_event(&Event::Key(KeyEvent::new(
            KeyCode::Esc,
            KeyModifiers::NONE,
        )));
        assert!(res.is_continue());
        assert_eq!(reader, expected);
    }

    #[test]
    fn down_nop_when_not_viewing_history() {
        let mut b = LineReaderBuilder::new(10, 5);
        b.text(&[":abcdëf🎸"]).input_start((0, 1).into());
        b.cursor(Cursor { column: 9, line: 0, index: (0, 13).into() });
        b.edited_input(Some("abcdë🎸".to_owned()));
        b.edited_history(Some("abcdë🎸".to_owned()));
        let mut reader = b.build();
        let expected = b.build();
        let res = reader.handle_event(&Event::Key(KeyEvent::new(
            KeyCode::Down,
            KeyModifiers::NONE,
        )));
        assert!(res.is_continue());
        assert_eq!(reader, expected);
    }

    #[test]
    fn enter_adds_non_empty_input_to_history() {
        let mut b = LineReaderBuilder::new(10, 5);
        b.text(&[":123456789", "abc"])
            .input_start((0, 1).into())
            .prompt_char_count(1)
            .cursor(Cursor { column: 3, line: 1, index: (1, 3).into() })
            .edited_input(Some("abc".to_owned()));
        let mut reader = b.build();
        let res = reader.handle_event(&Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        assert!(res.is_break());
        assert_eq!(reader.history, &["123456789abc".to_owned()]);
    }

    #[test]
    fn up_editing_input_saves_input_and_views_most_recent_history() {
        let mut b = LineReaderBuilder::new(10, 5);
        b.text(&[":123456789", "abc"])
            .input_start((0, 1).into())
            .prompt_char_count(1)
            .cursor(Cursor { column: 3, line: 1, index: (1, 3).into() })
            .history(&["foo", "bar", "baz"]);
        let mut reader = b.build();
        b.history_idx(Some(2))
            .text(&[":baz"])
            .cursor(Cursor { column: 4, line: 0, index: (0, 4).into() })
            .edited_input(Some("123456789abc"));
        let expected = b.build();
        let event = Event::Key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        let res = reader.handle_event(&event);
        assert!(res.is_continue());
        assert_eq!(reader, expected);
    }

    #[test]
    fn up_editing_history_saves_edited_and_views_most_recent_history() {
        let mut b = LineReaderBuilder::new(10, 5);
        b.text(&[":fo"])
            .input_start((0, 1).into())
            .prompt_char_count(1)
            .cursor(Cursor { column: 3, line: 0, index: (0, 3).into() })
            .history(&["foo", "bar", "baz"])
            .edited_input(Some("123456789abc"));
        let mut reader = b.build();
        b.history_idx(Some(2))
            .text(&[":baz"])
            .cursor(Cursor { column: 4, line: 0, index: (0, 4).into() })
            .edited_history(Some("fo"));
        let expected = b.build();
        let event = Event::Key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        let res = reader.handle_event(&event);
        assert!(res.is_continue());
        assert_eq!(reader, expected);
    }

    #[test]
    fn up_viewing_history_views_next_oldest_history() {
        let mut b = LineReaderBuilder::new(10, 5);
        b.input_start((0, 1).into())
            .text(&[":baz"])
            .prompt_char_count(1)
            .cursor(Cursor { column: 4, line: 0, index: (0, 4).into() })
            .history(&["foo", "bar", "baz"])
            .history_idx(Some(2))
            .edited_input(Some("123456789abc"));
        let mut reader = b.build();
        b.history_idx(Some(1)).text(&[":bar"]);
        let expected = b.build();
        let event = Event::Key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        let res = reader.handle_event(&event);
        assert!(res.is_continue());
        assert_eq!(reader, expected);
    }

    #[test]
    fn up_viewing_history_nop_after_oldest_history() {
        let mut b = LineReaderBuilder::new(10, 5);
        b.input_start((0, 1).into())
            .text(&[":foo"])
            .prompt_char_count(1)
            .cursor(Cursor { column: 4, line: 0, index: (0, 4).into() })
            .history(&["foo", "bar", "baz"])
            .history_idx(Some(0))
            .edited_input(Some("123456789abc"));
        let mut reader = b.build();
        let expected = b.build();
        let event = Event::Key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        let res = reader.handle_event(&event);
        assert!(res.is_continue());
        assert_eq!(reader, expected);
    }

    #[test]
    fn down_viewing_history_views_next_newest_history() {
        let mut b = LineReaderBuilder::new(10, 5);
        b.input_start((0, 1).into())
            .text(&[":foo"])
            .prompt_char_count(1)
            .cursor(Cursor { column: 4, line: 0, index: (0, 4).into() })
            .history(&["foo", "bar", "baz"])
            .history_idx(Some(0))
            .edited_input(Some("123456789abc"));
        let mut reader = b.build();
        b.text(&[":bar"]).history_idx(Some(1));
        let expected = b.build();
        let event =
            Event::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        let res = reader.handle_event(&event);
        assert!(res.is_continue());
        assert_eq!(reader, expected);
    }

    #[test]
    fn down_from_newest_history_returns_to_editing_history() {
        let mut b = LineReaderBuilder::new(10, 5);
        b.input_start((0, 1).into())
            .text(&[":baz"])
            .prompt_char_count(1)
            .cursor(Cursor { column: 4, line: 0, index: (0, 4).into() })
            .history(&["foo", "bar", "baz"])
            .history_idx(Some(2))
            .edited_history(Some("foobar"))
            .edited_input(Some("123456789abc"));
        let mut reader = b.build();
        b.text(&[":foobar"])
            .cursor(Cursor { column: 7, line: 0, index: (0, 7).into() })
            .history_idx(None)
            .edited_history::<&str>(None);
        let expected = b.build();
        let event =
            Event::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        let res = reader.handle_event(&event);
        assert!(res.is_continue());
        assert_eq!(reader, expected);
    }

    #[test]
    fn down_from_newest_history_returns_to_edting_input() {
        let mut b = LineReaderBuilder::new(10, 5);
        b.input_start((0, 1).into())
            .text(&[":baz"])
            .prompt_char_count(1)
            .cursor(Cursor { column: 4, line: 0, index: (0, 4).into() })
            .history(&["foo", "bar", "baz"])
            .history_idx(Some(2))
            .edited_input(Some("123456789abc"));
        let mut reader = b.build();
        b.text(&[":123456789", "abc"])
            .cursor(Cursor { column: 3, line: 1, index: (1, 3).into() })
            .history_idx(None)
            .edited_input::<&str>(None);
        let expected = b.build();
        let event =
            Event::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        let res = reader.handle_event(&event);
        assert!(res.is_continue());
        assert_eq!(reader, expected);
    }

    #[test]
    fn esc_editing_history_edits_input() {
        let mut b = LineReaderBuilder::new(10, 5);
        b.history(&["foo", "bar", "baz"])
            .edited_input(Some("123456789abc"))
            .text(&[":fo"])
            .input_start((0, 1).into())
            .prompt_char_count(1)
            .cursor(Cursor { column: 3, line: 0, index: (0, 3).into() });
        let mut reader = b.build();
        b.edited_input::<&str>(None)
            .text(&[":123456789", "abc"])
            .cursor(Cursor { column: 3, line: 1, index: (1, 3).into() });
        let expected = b.build();
        let res = reader.handle_event(&Event::Key(KeyEvent::new(
            KeyCode::Esc,
            KeyModifiers::NONE,
        )));
        assert!(res.is_continue());
        assert_eq!(reader, expected);
    }

    #[test]
    fn esc_nop_when_editing_input() {
        let mut b = LineReaderBuilder::new(10, 5);
        b.text(&[":some text"]).input_start((0, 1).into()).cursor(Cursor {
            line: 0,
            column: 10,
            index: (0, 10).into(),
        });
        let mut reader = b.build();
        let expected = b.build();
        let res = reader.handle_event(&Event::Key(KeyEvent::new(
            KeyCode::Esc,
            KeyModifiers::NONE,
        )));
        assert!(res.is_continue());
        assert_eq!(reader, expected);
    }

    #[test]
    fn esc_viewing_history_after_editing_history_edits_history() {
        let mut b = LineReaderBuilder::new(10, 5);
        b.history(&["foo", "bar", "baz"])
            .edited_input(Some("123456789abc"))
            .edited_history(Some("bat"))
            .text(&[":foo"])
            .history_idx(Some(0))
            .input_start((0, 1).into())
            .prompt_char_count(1)
            .cursor(Cursor { column: 4, line: 0, index: (0, 4).into() });
        let mut reader = b.build();
        b.edited_history::<&str>(None).text(&[":bat"]).history_idx(None);
        let expected = b.build();
        let res = reader.handle_event(&Event::Key(KeyEvent::new(
            KeyCode::Esc,
            KeyModifiers::NONE,
        )));
        assert!(res.is_continue());
        assert_eq!(reader, expected);
    }

    #[test]
    fn esc_viewing_history_after_editing_input_edits_input() {
        let mut b = LineReaderBuilder::new(10, 5);
        b.history(&["foo", "bar", "baz"])
            .edited_input(Some("123456789abc"))
            .text(&[":foo"])
            .history_idx(Some(0))
            .input_start((0, 1).into())
            .prompt_char_count(1)
            .cursor(Cursor { column: 4, line: 0, index: (0, 4).into() });
        let mut reader = b.build();
        b.edited_input::<&str>(None)
            .text(&[":123456789", "abc"])
            .history_idx(None)
            .cursor(Cursor { column: 3, line: 1, index: (1, 3).into() });
        let expected = b.build();
        let res = reader.handle_event(&Event::Key(KeyEvent::new(
            KeyCode::Esc,
            KeyModifiers::NONE,
        )));
        assert!(res.is_continue());
        assert_eq!(reader, expected);
    }
}
