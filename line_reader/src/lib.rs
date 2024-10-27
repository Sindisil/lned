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
    buffer: EditBuffer,
    history: HistoryStack,
}

// Non-public structs, enums, and traits
///////////////

#[derive(Debug, Default, Clone, PartialEq)]
struct HistoryStack {
    lines: Vec<String>,
    edited: Vec<Option<String>>,
    index: usize,
}

#[derive(Debug, Clone, PartialEq)]
struct EditBuffer {
    lines: Vec<BufferLine>,
    prompt_char_count: usize,
    input_start: BufferIndex,
    draft: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct RenderContext {
    display_width: usize,
    display_height: usize,
    cursor: Cursor,
    first_display_line: usize,
    first_buffer_line: usize,
    scroll_needed: usize,
}

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

// Non-public functions
////////

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
        LineReader { buffer: EditBuffer::new(), history: HistoryStack::new() }
    }
}

impl LineReader {
    #[must_use]
    pub fn new() -> LineReader {
        LineReader { ..Default::default() }
    }

    #[cfg(not(tarpaulin_include))]
    fn accept_line(
        &mut self,
        prompt: &str,
        output_buffer: &mut String,
    ) -> io::Result<usize> {
        // ensure terminal is reset to cooked w/visible cursor
        let _terminal_session = TerminalSession {};
        // reset for new input
        let (display_width, display_height) = terminal::size()?;
        let (_, first_display_line) = cursor::position()?;

        let mut render_ctx = RenderContext::new(
            display_width.into(),
            display_height.into(),
            first_display_line.into(),
        );
        self.buffer.set_prompt(&mut render_ctx, prompt);
        terminal::enable_raw_mode()?;
        render_ctx.repaint(&self.buffer)?;

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
                    if cursor_line > render_ctx.cursor.line {
                        render_ctx.first_display_line +=
                            cursor_line - render_ctx.cursor.line;
                    } else {
                        render_ctx.first_display_line -=
                            render_ctx.cursor.line - cursor_line;
                    }
                    render_ctx.cursor.line = cursor_line;
                    Event::Resize(x, y)
                }
                event => event,
            };
            res = handle_event(
                &mut self.buffer,
                &mut render_ctx,
                &mut self.history,
                &event,
            );
            if !matches!(event, Event::Resize(..)) {
                render_ctx.repaint(&self.buffer)?;
            }
        }

        handle_end(&mut self.buffer, &mut render_ctx);
        render_ctx.repaint(&self.buffer)?;
        let mut stdout = io::stdout().lock();
        stdout.write_all(b"\r\n")?;
        stdout.flush()?;

        let prev_bytes = output_buffer.len();
        if let Some(true) = res.break_value() {
            output_buffer.extend(
                self.buffer
                    .lines
                    .iter()
                    .flat_map(|l| l.text.chars())
                    .skip(self.buffer.prompt_char_count),
            );
        }
        output_buffer.push_str(native_eol());
        Ok(output_buffer.len() - prev_bytes)
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
    pub fn new(
        display_width: usize,
        display_height: usize,
        first_display_line: usize,
    ) -> RenderContext {
        RenderContext {
            display_width,
            display_height,
            first_display_line,
            ..Default::default()
        }
    }

    /// Compute first line of viewport
    pub fn viewport_top(&self) -> usize {
        (self.first_buffer_line > 0).into()
    }

    /// Compute last line of viewport
    fn viewport_bottom(&self, buffer: &EditBuffer) -> usize {
        if self.cursor.index.line == buffer.lines.len() - 1
            || (buffer.lines.len() - self.first_buffer_line)
                <= (self.display_height - self.first_display_line)
        {
            self.display_height - 1
        } else {
            self.display_height - 2
        }
    }

    #[cfg(not(tarpaulin_include))]
    /// render current buffer to display
    fn repaint(&mut self, buffer: &EditBuffer) -> io::Result<()> {
        let display_lines = self.display_height - self.first_display_line;
        let last_displayed =
            self.first_buffer_line + buffer.lines.len().min(display_lines);

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

        for line in &buffer.lines[self.first_buffer_line..last_displayed] {
            stdout.write_all(line.text.as_bytes())?;
        }

        stdout.queue(MoveTo(cursor_column, cursor_line))?.queue(Show)?.flush()
    }

    fn adjust_viewport(&mut self, buffer: &EditBuffer) {
        if self.cursor.line > self.viewport_bottom(buffer) {
            let diff = self.cursor.line - self.viewport_bottom(buffer);
            self.cursor.line = self.viewport_bottom(buffer);
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
        if buffer.lines.len() <= self.display_height {
            if self.first_buffer_line != 0 {
                // lines above display
                self.cursor.line += self.first_buffer_line;
                self.first_buffer_line = 0;
            } else if self.display_height - self.first_display_line
                < buffer.lines.len()
            {
                // lines below display
                self.scroll_needed = buffer.lines.len()
                    - (self.display_height - self.first_display_line);
                self.cursor.line -= self.scroll_needed;
                self.first_display_line -= self.scroll_needed;
            }
        }
    }
}

struct TerminalSession {}
impl Drop for TerminalSession {
    #[cfg(not(tarpaulin_include))]
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
        let _ = io::stdout().execute(Show);
    }
}

impl Default for EditBuffer {
    fn default() -> EditBuffer {
        EditBuffer {
            lines: vec!["".into()],
            prompt_char_count: 0,
            input_start: (0, 0).into(),
            draft: None,
        }
    }
}

impl EditBuffer {
    #[must_use]
    fn new() -> EditBuffer {
        EditBuffer { ..Default::default() }
    }

    fn set_prompt(&mut self, render_ctx: &mut RenderContext, prompt: &str) {
        let prompt_line =
            BufferLine { text: prompt.to_owned(), width: prompt.width() };
        self.input_start = (0, prompt_line.text.len()).into();
        self.prompt_char_count = prompt.chars().count();
        render_ctx.cursor = Cursor {
            column: prompt_line.width,
            line: render_ctx.first_display_line,
            index: self.input_start,
        };
        self.lines.splice(.., [prompt_line]);
        self.reflow(render_ctx, 0);
    }

    #[must_use]
    pub fn prompt(&self) -> String {
        self.lines
            .iter()
            .flat_map(|l| l.text.chars())
            .take(self.prompt_char_count)
            .collect()
    }

    fn is_empty(&self) -> bool {
        self.input_start == self.buffer_end()
    }

    /// Compute index one past last char in buffer
    fn buffer_end(&self) -> BufferIndex {
        (self.lines.len() - 1, self.lines.last().map(|l| l.text.len()).unwrap())
            .into()
    }

    fn save_draft(&mut self) {
        let draft = self.draft.get_or_insert_with(String::new);
        draft.clear();
        draft.extend(
            self.lines
                .iter()
                .flat_map(|l| l.text.chars())
                .skip(self.prompt_char_count),
        );
    }

    fn input_chars(&self) -> impl Iterator<Item = char> + use<'_> {
        self.lines
            .iter()
            .flat_map(|l| l.text.chars())
            .skip(self.prompt_char_count)
    }
    /// Reflow buffer lines to fit `display_width`, and
    /// snap cursor location to within viewport.
    /// Also might result in setting scroll needed.
    fn reflow(&mut self, render_ctx: &mut RenderContext, start: usize) {
        let mut tl_idx = start;
        while tl_idx < self.lines.len() {
            match self.lines[tl_idx].width.cmp(&render_ctx.display_width) {
                Ordering::Less => {
                    if self.try_fill_from_next(render_ctx, tl_idx).is_none()
                        || self.lines[tl_idx].width == render_ctx.display_width
                    {
                        tl_idx += 1;
                    }
                }
                Ordering::Greater => {
                    self.move_overflow_to_next(render_ctx, tl_idx);
                    tl_idx += 1;
                }
                Ordering::Equal => {
                    if tl_idx == render_ctx.cursor.index.line
                        && render_ctx.cursor.column >= render_ctx.display_width
                    {
                        render_ctx.cursor.line += 1;
                        render_ctx.cursor.column = 0;
                        render_ctx.cursor.index.line += 1;
                        render_ctx.cursor.index.offset = 0;
                        if render_ctx.cursor.index.line == self.lines.len() {
                            self.lines.push(BufferLine::new());
                        }
                    }
                    tl_idx += 1;
                }
            }
        }

        if self.lines.last().unwrap().width == render_ctx.display_width {
            self.lines.push(BufferLine::new());
        }

        render_ctx.adjust_viewport(self);
    }

    // attempt to fill this line, up to but not beyond,
    // display_width.
    // returns Some(prev_line_len) (i.e., idx of first
    // moved char), or None if no chars moved
    fn try_fill_from_next(
        &mut self,
        render_ctx: &mut RenderContext,
        tl_idx: usize,
    ) -> Option<(usize, usize)> {
        if tl_idx == self.lines.len() - 1 {
            return None;
        }

        let tl_width = self.lines[tl_idx].width;
        let nl_idx = tl_idx + 1;
        let moved = self.lines[nl_idx].text.char_indices().try_fold(
            (0, 0),
            |(res_idx, cols_moved), (i, c)| {
                let c_width = c.width().unwrap_or(0);
                if render_ctx.display_width >= (tl_width + cols_moved + c_width)
                {
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
            if render_ctx.cursor.index.line == nl_idx {
                // if cursor was on next line, adjust cursor
                if render_ctx.cursor.index.offset < res_idx
                    || res_idx == self.lines[nl_idx].text.len()
                {
                    // char at cursor moved to this line
                    render_ctx.cursor.line -= 1;
                    render_ctx.cursor.column += tl_width;
                    render_ctx.cursor.index.line -= 1;
                    render_ctx.cursor.index.offset += self.lines[tl_idx].len();
                } else {
                    // cursor still on next line
                    render_ctx.cursor.index.offset -= res_idx;
                    render_ctx.cursor.column -= cols_moved;
                }
            }

            if self.input_start.line == nl_idx {
                // if input_start was on next line, adjust it
                if self.input_start.offset < res_idx
                    || res_idx == self.lines[nl_idx].text.len()
                {
                    // input_start moved to this line
                    self.input_start.line -= 1;
                    self.input_start.offset += self.lines[tl_idx].len();
                } else {
                    // input_start still on next line
                    self.input_start.offset -= res_idx;
                }
            }

            let (this_part, next_part) = self.lines.split_at_mut(nl_idx);
            let this_line = &mut this_part[tl_idx];
            let next_line = &mut next_part[0];
            this_line.text.extend(next_line.text.drain(..res_idx));
            this_line.width += cols_moved;
            next_line.width -= cols_moved;
        }

        if self.lines[nl_idx].text.is_empty()
            && self.lines[tl_idx].width < render_ctx.display_width
        {
            self.lines.remove(nl_idx);
            if render_ctx.cursor.index.line > tl_idx {
                render_ctx.cursor.index.line -= 1;
                render_ctx.cursor.line -= 1;
            }
        }

        match res_idx {
            0 => None,
            _ => Some((res_idx, cols_moved)),
        }
    }

    fn move_overflow_to_next(
        &mut self,
        render_ctx: &mut RenderContext,
        tl_idx: usize,
    ) {
        assert!(self.lines[tl_idx].width > render_ctx.display_width);
        // check to see if there's a next_line & push one if not
        if tl_idx == self.lines.len() - 1 {
            self.lines.push(BufferLine::new());
        }

        // move this_line's residue to beginning of next line
        let mut cols = 0;
        let (this, next) = self.lines.split_at_mut(tl_idx + 1);
        let (this, next) = (&mut this[tl_idx], &mut next[0]);
        let (res_idx, _) = this
            .text
            .char_indices()
            .find(|(_, c)| {
                let c_width = c.width().unwrap_or(0);
                if render_ctx.display_width - cols < c_width {
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

        if tl_idx == render_ctx.cursor.index.line
            && res_idx <= render_ctx.cursor.index.offset
        {
            // if this was the cursor line & char at cursor moved,
            // adjust cursor
            render_ctx.cursor.line += 1;
            render_ctx.cursor.column -= this.width;
            render_ctx.cursor.index.line += 1;
            render_ctx.cursor.index.offset -= res_idx;
        } else if render_ctx.cursor.index.line == tl_idx + 1 {
            // if next line was cursor line, adjust cursor column
            render_ctx.cursor.column += cols_moved;
            render_ctx.cursor.index.offset += bytes_moved;
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

    fn set(&mut self, render_ctx: &mut RenderContext, line: impl AsRef<str>) {
        let mut text = self.prompt();
        self.input_start = (0, text.len()).into();
        text.push_str(line.as_ref());
        let width = text.width();
        let cursor = Cursor {
            column: width,
            line: render_ctx.first_display_line,
            index: (0, text.len()).into(),
        };
        self.lines.splice(.., [BufferLine { text, width }]);
        render_ctx.cursor = cursor;
        self.reflow(render_ctx, 0);
    }

    //    fn set_from_draft(&mut self, render_ctx: &mut RenderContext) {
    //        let mut text = self.prompt();
    //        self.input_start = (0, text.len()).into();
    //        text.push_str(self.draft.take());
    //        let width = text.width();
    //        let cursor = Cursor {
    //            column: width,
    //            line: render_ctx.first_display_line,
    //            index: (0, text.len()).into(),
    //        };
    //        self.lines.splice(.., [BufferLine { text, width }]);
    //        render_ctx.cursor = cursor;
    //        self.reflow(render_ctx, 0);
    //    }
}

impl HistoryStack {
    #[must_use]
    fn new() -> HistoryStack {
        HistoryStack { ..Default::default() }
    }

    fn is_at_top(&self) -> bool {
        self.index == self.lines.len()
    }

    fn is_at_bottom(&self) -> bool {
        self.index == 0
    }

    fn push(&mut self, line: String) {
        self.lines.push(line);
        self.edited.push(None);
        self.index = self.lines.len();
    }

    fn current(&mut self) -> Option<(&str, &mut Option<String>)> {
        if self.index == self.lines.len() {
            return None;
        }
        Some((self.lines[self.index].as_str(), &mut self.edited[self.index]))
    }

    fn next_newer(&mut self) -> Option<(&str, &mut Option<String>)> {
        self.index = self.lines.len().min(self.index + 1);
        if self.index == self.lines.len() {
            return None;
        }
        Some((self.lines[self.index].as_str(), &mut self.edited[self.index]))
    }

    fn next_older(&mut self) -> Option<(&str, &mut Option<String>)> {
        if self.index == 0 {
            return None;
        }
        self.index -= 1;
        Some((self.lines[self.index].as_str(), &mut self.edited[self.index]))
    }

    fn last(&mut self) -> Option<(&str, &mut Option<String>)> {
        if self.lines.is_empty() {
            None
        } else {
            let last = self.lines.len() - 1;
            Some((self.lines[last].as_ref(), &mut self.edited[last]))
        }
    }

    fn rewind(&mut self) {
        for e in &mut self.edited {
            e.take();
        }
        self.index = self.lines.len();
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

fn handle_event(
    buffer: &mut EditBuffer,
    render_ctx: &mut RenderContext,
    history: &mut HistoryStack,
    event: &Event,
) -> ControlFlow<bool> {
    match event {
        Event::Key(event) if event.kind == KeyEventKind::Press => {
            handle_key_event(buffer, render_ctx, history, event)
        }
        Event::Resize(x, y) => handle_resize_event(buffer, render_ctx, *x, *y),
        _ => ControlFlow::Continue(()),
    }
}

fn handle_resize_event(
    buffer: &mut EditBuffer,
    render_ctx: &mut RenderContext,
    x: u16,
    y: u16,
) -> ControlFlow<bool> {
    let old_width = render_ctx.display_width;
    let old_height = render_ctx.display_height;
    render_ctx.display_width = x.into();
    render_ctx.display_height = y.into();

    if render_ctx.display_width != old_width {
        buffer.reflow(render_ctx, 0);
    } else if render_ctx.display_height != old_height {
        render_ctx.adjust_viewport(buffer);
    }
    if render_ctx.display_height < old_height {
        let h_diff = old_height - render_ctx.display_height;
        render_ctx.scroll_needed =
            render_ctx.scroll_needed.saturating_sub(h_diff);
    }
    ControlFlow::Continue(())
}

fn handle_key_event(
    buffer: &mut EditBuffer,
    render_ctx: &mut RenderContext,
    history: &mut HistoryStack,
    event: &KeyEvent,
) -> ControlFlow<bool> {
    match event.code {
        KeyCode::Enter => {
            if !buffer.is_empty()
                && history.last().is_none_or(|(last, _)| {
                    last.chars().ne(buffer.input_chars())
                })
            {
                history.push(buffer.input_chars().collect());
            }
            ControlFlow::Break(true)
        }
        KeyCode::Left => handle_left(buffer, render_ctx),
        KeyCode::Right => handle_right(buffer, render_ctx),
        KeyCode::Home => handle_home(buffer, render_ctx),
        KeyCode::End => handle_end(buffer, render_ctx),
        KeyCode::Backspace => handle_backspace(buffer, render_ctx),
        KeyCode::Delete => handle_delete(buffer, render_ctx),
        KeyCode::Char(c) => handle_char_input(buffer, render_ctx, c),
        KeyCode::Up => handle_up(buffer, render_ctx, history),
        KeyCode::Down => handle_down(buffer, render_ctx, history),
        KeyCode::Esc => handle_esc(buffer, render_ctx, history),
        _ => ControlFlow::Continue(()),
    }
}

fn handle_esc(
    buffer: &mut EditBuffer,
    render_ctx: &mut RenderContext,
    history: &mut HistoryStack,
) -> ControlFlow<bool> {
    if let Some(draft) = buffer.draft.take() {
        buffer.set(render_ctx, draft);
    }
    history.rewind();
    ControlFlow::Continue(())
}

fn handle_down(
    buffer: &mut EditBuffer,
    render_ctx: &mut RenderContext,
    history: &mut HistoryStack,
) -> ControlFlow<bool> {
    if let Some((cur_a, &mut ref mut cur_e)) = history.current() {
        // If buffer differs from current edited (if any) or else
        // current accepted history, copy buffer to edited.
        if buffer
            .input_chars()
            .ne(cur_e.as_ref().map_or_else(|| cur_a.chars(), |e| e.chars()))
        {
            let edited = cur_e.get_or_insert_with(String::new);
            edited.clear();
            edited.extend(buffer.input_chars());
        }
    } else {
        // Not viewing history, nothing to do
        return ControlFlow::Continue(());
    }

    // Advance to next newer history.
    // If there is none, take draft to load buffer
    // Otherwise load buffer edited, if any, or accepted.
    if let Some((ah, eh)) = history.next_newer() {
        buffer.set(render_ctx, eh.as_ref().map_or(ah, |eh| eh.as_str()));
    } else {
        let draft =
            buffer.draft.take().expect("viewing history, so should be a draft");
        buffer.set(render_ctx, draft);
    };

    ControlFlow::Continue(())
}

fn handle_up(
    buffer: &mut EditBuffer,
    render_ctx: &mut RenderContext,
    history: &mut HistoryStack,
) -> ControlFlow<bool> {
    // If no older history to view, nothing to do
    if !history.is_at_bottom() {
        if history.is_at_top() {
            // If not viewing history, save buffer to draft
            buffer.save_draft();
        } else {
            // Otherwise, if buffer differs from current
            // edited (if any) or else current accepted
            // history, save buffer to current edited
            // history.
            let (cur_a, cur_e) = history
                .current()
                .expect("should be neither at_top or at_bottom");
            if buffer
                .input_chars()
                .ne(cur_e.as_ref().map_or_else(|| cur_a.chars(), |e| e.chars()))
            {
                let edited = cur_e.get_or_insert_with(String::new);
                edited.clear();
                edited.extend(buffer.input_chars());
            }
        }
        // Advance to next older history and load
        // buffer from edited, if any, or else accepted.
        let (accepted, edited) = history
            .next_older()
            .expect("shouldn't be either at_top or at_bottom");
        eprintln!("line: {accepted:?}  edited: {edited:?}");
        buffer
            .set(render_ctx, edited.as_ref().map_or(accepted, |e| e.as_str()));
    }
    ControlFlow::Continue(())
}

fn handle_char_input(
    buffer: &mut EditBuffer,
    render_ctx: &mut RenderContext,
    c: char,
) -> ControlFlow<bool> {
    let c_width = c.width().unwrap_or(0);
    // if char is zero width, but no previous chars exist to
    //  which it can  be combined, do nothing (i.e., don't accept
    // the input)
    if c_width == 0 && render_ctx.cursor.index == buffer.input_start {
        return ControlFlow::Continue(());
    }

    // insert new char at curser and let reflow sort it out
    let line = &mut buffer.lines[render_ctx.cursor.index.line];
    line.text.insert(render_ctx.cursor.index.offset, c);
    line.width += c_width;
    render_ctx.cursor.index.offset += c.len_utf8();
    render_ctx.cursor.column += c_width;

    // reflow from line before cursor, if it exists,
    // catching case where new char fits on previous line
    buffer.reflow(render_ctx, render_ctx.cursor.index.line.saturating_sub(1));

    ControlFlow::Continue(())
}

fn handle_backspace(
    buffer: &mut EditBuffer,
    render_ctx: &mut RenderContext,
) -> ControlFlow<bool> {
    if render_ctx.cursor.index == buffer.input_start {
        return ControlFlow::Continue(());
    }

    if render_ctx.cursor.index.offset == 0 {
        render_ctx.cursor.index.line -= 1;
        render_ctx.cursor.index.offset =
            buffer.lines[render_ctx.cursor.index.line].len();
        render_ctx.cursor.line -= 1;
        render_ctx.cursor.column =
            buffer.lines[render_ctx.cursor.index.line].width;
    }
    if let Some((i, c)) = buffer.lines[render_ctx.cursor.index.line].text
        [..render_ctx.cursor.index.offset]
        .char_indices()
        .next_back()
    {
        buffer.lines[render_ctx.cursor.index.line].text.remove(i);
        let removed_width = c.width().unwrap_or(0);

        buffer.lines[render_ctx.cursor.index.line].width -= removed_width;
        render_ctx.cursor.index.offset = i;
        render_ctx.cursor.column -= removed_width;
    }
    buffer.reflow(render_ctx, render_ctx.cursor.index.line.saturating_sub(1));
    ControlFlow::Continue(())
}

fn handle_left(
    buffer: &mut EditBuffer,
    render_ctx: &mut RenderContext,
) -> ControlFlow<bool> {
    if render_ctx.cursor.index == buffer.input_start {
        return ControlFlow::Continue(());
    }

    if render_ctx.cursor.index.offset == 0 {
        render_ctx.cursor.index.line -= 1;
        render_ctx.cursor.index.offset =
            buffer.lines[render_ctx.cursor.index.line].len();
        render_ctx.cursor.column =
            buffer.lines[render_ctx.cursor.index.line].width;
        render_ctx.cursor.line -= 1;
    }

    if let Some((prev_idx, prev_width)) = buffer.lines
        [render_ctx.cursor.index.line]
        .text[..render_ctx.cursor.index.offset]
        .char_indices()
        .map(|(i, c)| (i, c.width().unwrap_or(0)))
        .rfind(|(_, w)| *w > 0)
    {
        render_ctx.cursor.index.offset = prev_idx;
        render_ctx.cursor.column -= prev_width;
    }

    render_ctx.adjust_viewport(buffer);
    ControlFlow::Continue(())
}

fn handle_right(
    buffer: &mut EditBuffer,
    render_ctx: &mut RenderContext,
) -> ControlFlow<bool> {
    if render_ctx.cursor.index
        == (buffer.lines.len() - 1, buffer.lines.last().unwrap().text.len())
            .into()
    {
        return ControlFlow::Continue(());
    }

    if let Some((i, _)) = buffer.lines[render_ctx.cursor.index.line].text
        [render_ctx.cursor.index.offset..]
        .char_indices()
        .skip(1)
        .find(|(_, c)| c.width().unwrap_or(0) > 0)
    {
        let cur_char_width = buffer.lines[render_ctx.cursor.index.line].text
            [render_ctx.cursor.index.offset..]
            .chars()
            .next()
            .and_then(UnicodeWidthChar::width)
            .unwrap();
        render_ctx.cursor.column += cur_char_width;
        render_ctx.cursor.index.offset += i;
    } else {
        render_ctx.cursor.line += 1;
        render_ctx.cursor.column = 0;
        render_ctx.cursor.index.line += 1;
        render_ctx.cursor.index.offset = 0;
    }
    render_ctx.adjust_viewport(buffer);
    ControlFlow::Continue(())
}

fn handle_delete(
    buffer: &mut EditBuffer,
    render_ctx: &mut RenderContext,
) -> ControlFlow<bool> {
    // if at end of buffer, nothing to do
    if render_ctx.cursor.index != buffer.buffer_end() {
        let (cur_line, cur_offset) = render_ctx.cursor.index.into();
        let mut c_idx =
            buffer.lines[cur_line].text[cur_offset..].char_indices();
        let c_width =
            c_idx.next().map(|(_, c)| c.width().unwrap_or(0)).unwrap();
        let next_c_offset =
            c_idx.find(|(_, c)| c.width().unwrap_or(0) > 0).map_or_else(
                || buffer.lines[cur_line].len(),
                |(i, _)| i + cur_offset,
            );
        buffer.lines[cur_line]
            .text
            .replace_range(cur_offset..next_c_offset, "");
        buffer.lines[cur_line].width -= c_width;
        buffer.reflow(render_ctx, cur_line.saturating_sub(1));
    }
    ControlFlow::Continue(())
}

fn handle_home(
    buffer: &mut EditBuffer,
    render_ctx: &mut RenderContext,
) -> ControlFlow<bool> {
    if render_ctx.cursor.index != buffer.input_start {
        render_ctx.first_buffer_line = 0;
        render_ctx.cursor.index = buffer.input_start;
        render_ctx.cursor.line =
            render_ctx.first_display_line + render_ctx.cursor.index.line;
        render_ctx.cursor.column = buffer.lines[render_ctx.cursor.index.line]
            .text[..render_ctx.cursor.index.offset]
            .width();
        render_ctx.adjust_viewport(buffer);
    }
    ControlFlow::Continue(())
}

fn handle_end(
    buffer: &mut EditBuffer,
    render_ctx: &mut RenderContext,
) -> ControlFlow<bool> {
    let buffer_end = buffer.buffer_end();
    if render_ctx.cursor.index != buffer_end {
        render_ctx.cursor.line +=
            buffer_end.line - render_ctx.cursor.index.line;
        render_ctx.cursor.column = buffer.lines[buffer_end.line].width;
        render_ctx.cursor.index = buffer_end;
        render_ctx.adjust_viewport(buffer);
    }
    ControlFlow::Continue(())
}

#[cfg(test)]
#[allow(clippy::unicode_not_nfc)]
mod tests {
    use super::*;

    use crossterm::event::KeyModifiers;

    use similar_asserts::assert_eq;

    fn make_buf(lines: &[&str], prompt: char) -> EditBuffer {
        let mut buf = EditBuffer { lines: Vec::new(), ..Default::default() };
        for &l in lines {
            buf.lines.push(l.into());
        }
        buf.prompt_char_count = 1;
        buf.input_start = (0, prompt.len_utf8()).into();
        if let Some(l) = buf.lines.get_mut(0) {
            l.text.insert(0, prompt);
            l.width = l.text.width();
        }
        buf
    }

    #[test]
    fn viewport_all_within_display() {
        let buffer = EditBuffer {
            lines: vec![
                ":123456789".into(),
                "0123456789".into(),
                "012345".into(),
            ],
            prompt_char_count: 1,
            input_start: (0, 1).into(),
            draft: None,
        };
        let render_ctx = RenderContext {
            display_width: 10,
            display_height: 5,
            cursor: Cursor { column: 6, line: 2, index: (2, 6).into() },
            ..Default::default()
        };
        assert_eq!(
            render_ctx.viewport_bottom(&buffer),
            render_ctx.display_height - 1
        );
        assert_eq!(render_ctx.viewport_top(), 0);
    }

    #[test]
    fn viewport_buffer_beyond_top() {
        let buffer = EditBuffer {
            lines: vec![
                ":123456789".into(),
                "0123456789".into(),
                "0123456789".into(),
                "0123456789".into(),
                "0123456789".into(),
                "0123456789".into(),
                "012345".into(),
            ],
            prompt_char_count: 1,
            input_start: (0, 1).into(),
            draft: None,
        };
        let render_ctx = RenderContext {
            display_width: 10,
            display_height: 5,
            cursor: Cursor { column: 6, line: 4, index: (6, 6).into() },
            first_buffer_line: 2,
            ..Default::default()
        };
        let vp_bottom = render_ctx.viewport_bottom(&buffer);
        let vp_top = render_ctx.viewport_top();
        assert_eq!(vp_bottom, render_ctx.display_height - 1);
        assert_eq!(vp_top, 1);
    }

    #[test]
    fn viewport_buffer_beyond_bottom() {
        let buffer = EditBuffer {
            lines: vec![
                ":123456789".into(),
                "0123456789".into(),
                "0123456789".into(),
                "0123456789".into(),
                "0123456789".into(),
                "0123456789".into(),
                "012345".into(),
            ],
            prompt_char_count: 1,
            input_start: (0, 1).into(),
            draft: None,
        };
        let render_ctx = RenderContext {
            display_width: 10,
            display_height: 5,
            cursor: Cursor { column: 1, line: 0, index: (0, 1).into() },
            ..Default::default()
        };
        assert_eq!(
            render_ctx.viewport_bottom(&buffer),
            render_ctx.display_height - 2
        );
        assert_eq!(render_ctx.viewport_top(), 0);
    }

    #[test]
    fn viewport_buffer_beyond_both() {
        let buffer = EditBuffer {
            lines: vec![
                ":123456789".into(),
                "0123456789".into(),
                "0123456789".into(),
                "0123456789".into(),
                "0123456789".into(),
                "0123456789".into(),
                "012345".into(),
            ],
            input_start: (0, 1).into(),
            prompt_char_count: 1,
            draft: None,
        };
        let render_ctx = RenderContext {
            display_width: 10,
            display_height: 5,
            cursor: Cursor { column: 5, line: 2, index: (3, 5).into() },
            first_buffer_line: 1,
            ..Default::default()
        };
        assert_eq!(
            render_ctx.viewport_bottom(&buffer),
            render_ctx.display_height - 2
        );
        assert_eq!(render_ctx.viewport_top(), 1);
    }

    #[test]
    fn unimplemented_event_ignored() {
        let mut buf = EditBuffer::new();
        let mut ctx = RenderContext::new(10, 5, 0);
        let mut hs = HistoryStack::new();
        let event = Event::FocusLost;
        let res = handle_event(&mut buf, &mut ctx, &mut hs, &event);
        assert!(res.is_continue());
    }

    #[test]
    fn unimplemented_key_event_ignored() {
        let event =
            Event::Key(KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE));
        let res = handle_event(
            &mut EditBuffer::new(),
            &mut RenderContext::new(10, 5, 0),
            &mut HistoryStack::new(),
            &event,
        );
        assert!(res.is_continue());
    }

    #[test]
    fn enter_breaks_input_loop() {
        let event =
            Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        let res = handle_event(
            &mut EditBuffer::new(),
            &mut RenderContext::new(10, 5, 0),
            &mut HistoryStack::new(),
            &event,
        );
        assert!(matches!(res, ControlFlow::Break(true)));
    }

    #[test]
    fn char_input_non_0w_inserts() {
        let mut buf = EditBuffer::new();
        let mut ctx = RenderContext::new(10, 5, 0);
        let expected_buf =
            EditBuffer { lines: vec!["🎸".into()], ..Default::default() };
        let expected_ctx = RenderContext {
            display_width: 10,
            display_height: 5,
            cursor: Cursor { column: 2, line: 0, index: (0, 4).into() },
            ..Default::default()
        };
        let event =
            Event::Key(KeyEvent::new(KeyCode::Char('🎸'), KeyModifiers::NONE));
        let res =
            handle_event(&mut buf, &mut ctx, &mut HistoryStack::new(), &event);
        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert_eq!(ctx, expected_ctx);
    }

    #[test]
    fn char_input_0w_requires_base_char() {
        let mut buf = EditBuffer {
            lines: vec![":".into()],
            input_start: (0, 1).into(),
            prompt_char_count: 1,
            ..Default::default()
        };
        let mut ctx = RenderContext {
            display_width: 10,
            display_height: 5,
            cursor: Cursor { line: 0, column: 1, index: (0, 1).into() },
            ..Default::default()
        };
        let expected_buf = buf.clone();
        let expected_ctx = ctx;

        let event = Event::Key(KeyEvent::new(
            KeyCode::Char('\u{0308}'),
            KeyModifiers::NONE,
        ));

        let res =
            handle_event(&mut buf, &mut ctx, &mut HistoryStack::new(), &event);
        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert_eq!(ctx, expected_ctx);

        let mut buf = EditBuffer {
            lines: vec![":a".into()],
            input_start: (0, 1).into(),
            prompt_char_count: 1,
            ..Default::default()
        };
        let mut ctx = RenderContext {
            display_width: 10,
            display_height: 5,
            cursor: Cursor { line: 0, column: 2, index: (0, 2).into() },
            ..Default::default()
        };

        let expected_buf = EditBuffer {
            lines: vec![":ä".into()],
            input_start: (0, 1).into(),
            prompt_char_count: 1,
            ..Default::default()
        };
        let expected_ctx = RenderContext {
            display_width: 10,
            display_height: 5,
            cursor: Cursor { line: 0, column: 2, index: (0, 4).into() },
            ..Default::default()
        };

        let event = Event::Key(KeyEvent::new(
            KeyCode::Char('\u{0308}'),
            KeyModifiers::NONE,
        ));

        let res =
            handle_event(&mut buf, &mut ctx, &mut HistoryStack::new(), &event);
        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert_eq!(ctx, expected_ctx);
    }

    #[test]
    fn char_input_before_eol_moves_cursor_char_width() {
        let mut buf = make_buf(&["e"], ':');
        let expected_buf = make_buf(&["ë"], ':');
        let mut ctx = RenderContext {
            display_width: 10,
            display_height: 5,
            cursor: Cursor { line: 0, column: 2, index: (0, 2).into() },
            ..Default::default()
        };
        let expected_ctx = RenderContext {
            cursor: Cursor { line: 0, column: 2, index: (0, 4).into() },
            ..ctx
        };

        let event = Event::Key(KeyEvent::new(
            KeyCode::Char('\u{0308}'),
            KeyModifiers::NONE,
        ));
        let res =
            handle_event(&mut buf, &mut ctx, &mut HistoryStack::new(), &event);
        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert_eq!(ctx, expected_ctx);

        let event =
            Event::Key(KeyEvent::new(KeyCode::Char('🎸'), KeyModifiers::NONE));
        let expected_buf = make_buf(&["ë🎸"], ':');
        let expected_ctx = RenderContext {
            cursor: Cursor { line: 0, column: 4, index: (0, 8).into() },
            ..ctx
        };
        let res =
            handle_event(&mut buf, &mut ctx, &mut HistoryStack::new(), &event);
        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert_eq!(ctx, expected_ctx);

        let event =
            Event::Key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE));
        let expected_buf =
            EditBuffer { lines: vec![":ë🎸o".into()], ..buf.clone() };
        let expected_ctx = RenderContext {
            cursor: Cursor { line: 0, column: 5, index: (0, 9).into() },
            ..ctx
        };
        let res =
            handle_event(&mut buf, &mut ctx, &mut HistoryStack::new(), &event);
        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert_eq!(ctx, expected_ctx);
    }

    #[test]
    fn char_input_to_eol_wraps_cursor_to_next_line_start() {
        let mut buf = EditBuffer {
            lines: vec![":1234567".into()],
            input_start: (0, 1).into(),
            prompt_char_count: 1,
            ..Default::default()
        };
        let mut ctx = RenderContext {
            display_width: 10,
            display_height: 5,
            cursor: Cursor { column: 8, line: 0, index: (0, 8).into() },
            ..Default::default()
        };

        let event =
            Event::Key(KeyEvent::new(KeyCode::Char('🎸'), KeyModifiers::NONE));
        let expected_buf = EditBuffer {
            lines: vec![":1234567🎸".into(), "".into()],
            ..buf.clone()
        };
        let expected_ctx = RenderContext {
            cursor: Cursor { column: 0, line: 1, index: (1, 0).into() },
            ..ctx
        };
        let res =
            handle_event(&mut buf, &mut ctx, &mut HistoryStack::new(), &event);
        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert_eq!(ctx, expected_ctx);
    }

    #[test]
    fn char_input_append_to_previous_line_if_fits() {
        let mut buf = EditBuffer {
            lines: vec![":12345678".into(), "🎸abc".into()],
            input_start: (0, 1).into(),
            prompt_char_count: 1,
            ..Default::default()
        };
        let mut ctx = RenderContext {
            display_width: 10,
            display_height: 5,
            cursor: Cursor { column: 0, line: 4, index: (1, 0).into() },
            first_display_line: 3,
            ..Default::default()
        };
        let expected_buf = EditBuffer {
            lines: vec![":123456789".into(), "🎸abc".into()],
            ..buf.clone()
        };
        let expected_ctx = ctx;

        let event =
            Event::Key(KeyEvent::new(KeyCode::Char('9'), KeyModifiers::NONE));
        let res =
            handle_event(&mut buf, &mut ctx, &mut HistoryStack::new(), &event);
        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert_eq!(ctx, expected_ctx);
    }

    #[test]
    fn char_input_char_too_wide_at_end_wraps_to_next_line() {
        let mut buf = EditBuffer {
            lines: vec![":12345678".into()],
            input_start: (0, 1).into(),
            prompt_char_count: 1,
            ..Default::default()
        };
        let mut ctx = RenderContext {
            display_width: 10,
            display_height: 5,
            cursor: Cursor { column: 9, line: 0, index: (0, 9).into() },
            ..Default::default()
        };

        let expected_buf = EditBuffer {
            lines: vec![":12345678".into(), "🎸".into()],
            ..buf.clone()
        };
        let expected_ctx = RenderContext {
            cursor: Cursor { column: 2, line: 1, index: (1, 4).into() },
            ..ctx
        };

        let event =
            Event::Key(KeyEvent::new(KeyCode::Char('🎸'), KeyModifiers::NONE));
        let res =
            handle_event(&mut buf, &mut ctx, &mut HistoryStack::new(), &event);

        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert_eq!(ctx, expected_ctx);
    }

    #[test]
    fn char_input_past_eol_wraps_input_to_next_line_start() {
        let mut buf = EditBuffer {
            lines: vec![":123456789".into(), "abc".into()],
            input_start: (0, 1).into(),
            prompt_char_count: 1,
            ..Default::default()
        };
        let mut ctx = RenderContext {
            display_width: 10,
            display_height: 5,
            cursor: Cursor { column: 8, line: 0, index: (0, 8).into() },
            ..Default::default()
        };

        let expected_buf = EditBuffer {
            lines: vec![":1234567🎸".into(), "89abc".into()],
            ..buf.clone()
        };
        let expected_ctx = RenderContext {
            cursor: Cursor { column: 0, line: 1, index: (1, 0).into() },
            ..ctx
        };

        let event =
            Event::Key(KeyEvent::new(KeyCode::Char('🎸'), KeyModifiers::NONE));
        let res =
            handle_event(&mut buf, &mut ctx, &mut HistoryStack::new(), &event);

        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert_eq!(ctx, expected_ctx);
    }

    #[test]
    fn char_input_at_end_of_small_buffer_moving_cursor_beyond_bottom() {
        let mut buf = EditBuffer {
            lines: vec![":12345678".into(), "🎸2345678".into()],
            input_start: (0, 1).into(),
            prompt_char_count: 1,
            ..Default::default()
        };
        let mut ctx = RenderContext {
            display_width: 10,
            display_height: 5,
            first_display_line: 3,
            cursor: Cursor { column: 9, line: 4, index: (1, 11).into() },
            ..Default::default()
        };
        let expected_buf = EditBuffer {
            lines: vec![":12345678".into(), "🎸2345678a".into(), "".into()],
            ..buf.clone()
        };
        let expected_ctx = RenderContext {
            first_display_line: 2,
            cursor: Cursor { column: 0, line: 4, index: (2, 0).into() },
            scroll_needed: 1,
            ..ctx
        };
        let event =
            Event::Key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
        let res =
            handle_event(&mut buf, &mut ctx, &mut HistoryStack::new(), &event);
        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert_eq!(ctx, expected_ctx);
    }

    #[test]
    fn char_input_at_end_of_large_buffer_moving_cursor_beyond_bottom() {
        let mut buf = EditBuffer {
            lines: vec![
                ":123456789".into(),
                "0123456789".into(),
                "0123456789".into(),
                "0123456789".into(),
                "012345678".into(),
                "🎸2345678".into(),
            ],
            input_start: (0, 1).into(),
            prompt_char_count: 1,
            ..Default::default()
        };
        let mut ctx = RenderContext {
            display_width: 10,
            display_height: 5,
            first_buffer_line: 1,
            cursor: Cursor { column: 9, line: 4, index: (5, 11).into() },
            ..Default::default()
        };

        let expected_buf = EditBuffer {
            lines: vec![
                ":123456789".into(),
                "0123456789".into(),
                "0123456789".into(),
                "0123456789".into(),
                "012345678".into(),
                "🎸2345678a".into(),
                "".into(),
            ],
            ..buf.clone()
        };
        let expected_ctx = RenderContext {
            first_buffer_line: 2,
            cursor: Cursor { column: 0, line: 4, index: (6, 0).into() },
            ..ctx
        };

        let event =
            Event::Key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
        let res =
            handle_event(&mut buf, &mut ctx, &mut HistoryStack::new(), &event);
        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert_eq!(ctx, expected_ctx);
    }

    #[test]
    fn char_input_within_small_buffer_extending_below_display() {
        let mut buf = EditBuffer {
            lines: vec![
                ":123456789".into(),
                "012345678".into(),
                "🎸2345678".into(),
            ],
            input_start: (0, 1).into(),
            prompt_char_count: 1,
            ..Default::default()
        };
        let mut ctx = RenderContext {
            display_width: 10,
            display_height: 5,
            first_display_line: 3,
            cursor: Cursor { column: 9, line: 3, index: (0, 9).into() },
            ..Default::default()
        };

        let expected_buf = EditBuffer {
            lines: vec![
                ":12345678a".into(),
                "9012345678".into(),
                "🎸2345678".into(),
            ],
            ..buf.clone()
        };
        let expected_ctx = RenderContext {
            first_display_line: 2,
            cursor: Cursor { column: 0, line: 3, index: (1, 0).into() },
            scroll_needed: 1,
            ..ctx
        };
        let event =
            Event::Key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
        let res =
            handle_event(&mut buf, &mut ctx, &mut HistoryStack::new(), &event);
        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert_eq!(ctx, expected_ctx);
    }

    #[test]
    fn char_input_within_large_buffer_extending_beyond_display() {
        let mut buf = EditBuffer {
            lines: vec![
                ":123456789".into(),
                "0123456789".into(),
                "0123456789".into(),
                "0123456789".into(),
                "0123456789".into(),
                "0123456789".into(),
                "012345678".into(),
            ],
            input_start: (0, 1).into(),
            prompt_char_count: 1,
            ..Default::default()
        };
        let mut ctx = RenderContext {
            display_width: 10,
            display_height: 5,
            first_buffer_line: 1,
            cursor: Cursor { column: 9, line: 3, index: (4, 9).into() },
            ..Default::default()
        };

        let expected_buf = EditBuffer {
            lines: vec![
                ":123456789".into(),
                "0123456789".into(),
                "0123456789".into(),
                "0123456789".into(),
                "012345678a".into(),
                "9012345678".into(),
                "9012345678".into(),
                "".into(),
            ],
            ..buf.clone()
        };
        let expected_ctx = RenderContext {
            first_buffer_line: 2,
            cursor: Cursor { column: 0, line: 3, index: (5, 0).into() },
            ..ctx
        };
        let event =
            Event::Key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
        let res =
            handle_event(&mut buf, &mut ctx, &mut HistoryStack::new(), &event);
        assert!(res.is_continue());
        assert_eq!((buf, ctx), (expected_buf, expected_ctx));
    }

    #[test]
    fn backspace_0w() {
        let mut buf = EditBuffer {
            lines: vec![":ë".into()],
            input_start: (0, 1).into(),
            prompt_char_count: 1,
            ..Default::default()
        };
        let mut ctx = RenderContext {
            display_width: 10,
            display_height: 5,
            cursor: Cursor { column: 2, line: 0, index: (0, 4).into() },
            ..Default::default()
        };

        let expected_buf =
            EditBuffer { lines: vec![":e".into()], ..buf.clone() };
        let expected_ctx = RenderContext {
            cursor: Cursor { column: 2, line: 0, index: (0, 2).into() },
            ..ctx
        };

        let event =
            Event::Key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        let res =
            handle_event(&mut buf, &mut ctx, &mut HistoryStack::new(), &event);
        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert_eq!(ctx, expected_ctx);
    }

    #[test]
    fn backspace_1w() {
        let mut buf = make_buf(&["e"], ':');
        let expected_buf = make_buf(&[""], ':');
        let mut ctx = RenderContext {
            display_width: 10,
            display_height: 5,
            cursor: Cursor { column: 2, line: 0, index: (0, 2).into() },
            ..Default::default()
        };
        let expected_ctx = RenderContext {
            cursor: Cursor { column: 1, line: 0, index: (0, 1).into() },
            ..ctx
        };

        let event =
            Event::Key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        let res =
            handle_event(&mut buf, &mut ctx, &mut HistoryStack::new(), &event);
        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert_eq!(ctx, expected_ctx);
    }

    #[test]
    fn backspace_2w() {
        let mut buf = make_buf(&["🎸"], ':');
        let mut ctx = RenderContext {
            display_width: 10,
            display_height: 5,
            cursor: Cursor { column: 3, line: 0, index: (0, 5).into() },
            ..Default::default()
        };
        let expected_buf = make_buf(&[""], ':');
        let expected_ctx = RenderContext {
            cursor: Cursor { column: 1, line: 0, index: (0, 1).into() },
            ..ctx
        };

        let event =
            Event::Key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        let res =
            handle_event(&mut buf, &mut ctx, &mut HistoryStack::new(), &event);
        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert_eq!(ctx, expected_ctx);
    }

    #[test]
    fn backspace_input_start() {
        let mut buf = make_buf(&[""], ':');
        let mut ctx = RenderContext {
            display_width: 10,
            display_height: 5,
            cursor: Cursor { column: 1, line: 0, index: (0, 1).into() },
            ..Default::default()
        };
        let expected_buf = buf.clone();
        let expected_ctx = ctx;

        let event =
            Event::Key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));

        let res =
            handle_event(&mut buf, &mut ctx, &mut HistoryStack::new(), &event);
        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert_eq!(ctx, expected_ctx);
    }

    #[test]
    fn backspace_to_column_0_wraps_if_room_on_preceding_line() {
        let mut buf = make_buf(&["12345678", "🎸9"], ':');
        let mut ctx = RenderContext {
            display_width: 10,
            display_height: 5,
            cursor: Cursor { column: 2, line: 1, index: (1, 4).into() },
            ..Default::default()
        };
        let expected_buf = make_buf(&["123456789", ""], ':');
        let expected_ctx = RenderContext {
            cursor: Cursor { column: 9, line: 0, index: (0, 9).into() },
            ..ctx
        };
        let event =
            Event::Key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        let res =
            handle_event(&mut buf, &mut ctx, &mut HistoryStack::new(), &event);
        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert_eq!(ctx, expected_ctx);
    }

    #[test]
    fn backspace_from_column_0_wraps_if_room_on_preceding_line() {
        let event =
            Event::Key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));

        // base case
        let mut buf = make_buf(&["123456789", ""], ':');
        let mut ctx = RenderContext {
            display_width: 10,
            display_height: 10,
            cursor: Cursor { column: 0, line: 1, index: (1, 0).into() },
            ..Default::default()
        };

        let expected_buf = make_buf(&["12345678"], ':');
        let expected_ctx = RenderContext {
            cursor: Cursor { column: 9, line: 0, index: (0, 9).into() },
            ..ctx
        };
        let res =
            handle_event(&mut buf, &mut ctx, &mut HistoryStack::new(), &event);
        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert_eq!(ctx, expected_ctx);

        // zero len char at preceding line end
        let mut buf = make_buf(&["12345678ä", "eiou"], ':');
        let mut ctx = RenderContext {
            display_width: 10,
            display_height: 5,
            cursor: Cursor { column: 0, line: 1, index: (1, 0).into() },
            ..Default::default()
        };
        let expected_buf = make_buf(&["12345678a", "eiou"], ':');
        let expected_ctx = ctx;
        let res =
            handle_event(&mut buf, &mut ctx, &mut HistoryStack::new(), &event);
        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert_eq!(ctx, expected_ctx);
    }

    #[test]
    fn backspace_moving_cursor_above_top_pans_buffer() {
        let mut buf = make_buf(
            &[
                "123456789",
                "0123456789",
                "0123456789",
                "0123456789",
                "0123456789",
                "0123",
            ],
            ':',
        );
        let mut ctx = RenderContext {
            display_width: 10,
            display_height: 5,
            first_buffer_line: 1,
            cursor: Cursor { line: 1, column: 0, index: (2, 0).into() },
            ..Default::default()
        };
        let expected_buf = make_buf(
            &[
                "123456789",
                "0123456780",
                "1234567890",
                "1234567890",
                "1234567890",
                "123",
            ],
            ':',
        );
        let expected_ctx = RenderContext {
            first_buffer_line: 0,
            cursor: Cursor { line: 1, column: 9, index: (1, 9).into() },
            ..ctx
        };

        let event =
            Event::Key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        let res =
            handle_event(&mut buf, &mut ctx, &mut HistoryStack::new(), &event);
        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert_eq!(ctx, expected_ctx);
    }

    #[test]
    fn left_from_input_start_does_nothing() {
        let mut buf = make_buf(&["12345"], ':');
        let mut ctx = RenderContext {
            display_width: 10,
            display_height: 5,
            cursor: Cursor { column: 1, line: 0, index: (0, 1).into() },
            ..Default::default()
        };
        let expected_buf = buf.clone();
        let expected_ctx = ctx;
        let event =
            Event::Key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        let res =
            handle_event(&mut buf, &mut ctx, &mut HistoryStack::new(), &event);
        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert_eq!(ctx, expected_ctx);
    }

    #[test]
    fn left_moves_cursor_to_preceding_base_char() {
        let event =
            Event::Key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        let mut buf = make_buf(&["aë🎸iou"], ':');
        let mut ctx = RenderContext {
            display_width: 10,
            display_height: 5,
            cursor: Cursor { column: 6, line: 0, index: (0, 10).into() },
            ..Default::default()
        };
        let expected_buf = buf.clone();
        let expected_ctx = RenderContext {
            cursor: Cursor { column: 5, line: 0, index: (0, 9).into() },
            ..ctx
        };
        let res =
            handle_event(&mut buf, &mut ctx, &mut HistoryStack::new(), &event);
        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert_eq!(ctx, expected_ctx);

        let expected_ctx = RenderContext {
            cursor: Cursor { column: 3, line: 0, index: (0, 5).into() },
            ..ctx
        };
        let res =
            handle_event(&mut buf, &mut ctx, &mut HistoryStack::new(), &event);
        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert_eq!(ctx, expected_ctx);

        let expected_ctx = RenderContext {
            cursor: Cursor { column: 2, line: 0, index: (0, 2).into() },
            ..ctx
        };
        let res =
            handle_event(&mut buf, &mut ctx, &mut HistoryStack::new(), &event);
        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert_eq!(ctx, expected_ctx);
    }

    #[test]
    fn left_from_column_0_moves_cursor_to_last_base_char_on_preceding_line() {
        let event =
            Event::Key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        let mut buf = make_buf(&["12345678", "🎸abc"], ':');
        let mut ctx = RenderContext {
            display_width: 10,
            display_height: 5,
            cursor: Cursor { column: 0, line: 1, index: (1, 0).into() },
            ..Default::default()
        };
        let expected_buf = buf.clone();
        let expected_ctx = RenderContext {
            display_width: 10,
            display_height: 5,
            cursor: Cursor { column: 8, line: 0, index: (0, 8).into() },
            ..ctx
        };

        let res =
            handle_event(&mut buf, &mut ctx, &mut HistoryStack::new(), &event);
        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert_eq!(ctx, expected_ctx);
    }

    #[test]
    fn left_moving_cursor_above_top_pans_buffer_down_one_line() {
        let event =
            Event::Key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        let mut buf = make_buf(
            &[
                "123456789",
                "012345678",
                "🎸23456789",
                "0123456789",
                "012345678",
                "🎸abc",
            ],
            ':',
        );
        let mut ctx = RenderContext {
            display_width: 10,
            display_height: 5,
            first_buffer_line: 1,
            cursor: Cursor { column: 0, line: 1, index: (2, 0).into() },
            ..Default::default()
        };
        let expected_buf = buf.clone();
        let expected_ctx = RenderContext {
            first_buffer_line: 0,
            cursor: Cursor { column: 8, line: 1, index: (1, 8).into() },
            ..ctx
        };

        let res =
            handle_event(&mut buf, &mut ctx, &mut HistoryStack::new(), &event);
        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert_eq!(ctx, expected_ctx);
    }

    #[test]
    fn home_from_input_start_does_nothing() {
        let event =
            Event::Key(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE));
        let mut buf =
            make_buf(&["123456789", "0123456789", "012345678", "🎸abcd"], ':');
        let mut ctx = RenderContext {
            display_width: 10,
            display_height: 5,
            cursor: Cursor { column: 1, line: 0, index: (0, 1).into() },
            ..Default::default()
        };
        let expected_buf = buf.clone();
        let expected_ctx = ctx;
        let res =
            handle_event(&mut buf, &mut ctx, &mut HistoryStack::new(), &event);
        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert_eq!(ctx, expected_ctx);
    }

    #[test]
    fn home_moves_cursor_to_input_start() {
        let event =
            Event::Key(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE));
        let mut buf =
            make_buf(&["123456789", "0123456789", "012345678", "🎸abcd"], ':');
        let mut ctx = RenderContext {
            display_width: 10,
            display_height: 5,
            cursor: Cursor { column: 0, line: 3, index: (3, 0).into() },
            ..Default::default()
        };
        let expected_buf = buf.clone();
        let expected_ctx = RenderContext {
            cursor: Cursor { column: 1, line: 0, index: (0, 1).into() },
            ..ctx
        };
        let res =
            handle_event(&mut buf, &mut ctx, &mut HistoryStack::new(), &event);
        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert_eq!(ctx, expected_ctx);
    }

    #[test]
    fn home_moving_cursor_above_top_pans_buffer() {
        let event =
            Event::Key(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE));
        let mut buf =
            make_buf(&["123456789", "0123456789", "012345678", "🎸abcd"], ':');
        let mut ctx = RenderContext {
            display_width: 10,
            display_height: 5,
            first_buffer_line: 2,
            cursor: Cursor { column: 0, line: 1, index: (3, 0).into() },
            ..Default::default()
        };
        let expected_buf = buf.clone();
        let expected_ctx = RenderContext {
            display_width: 10,
            display_height: 5,
            first_buffer_line: 0,
            cursor: Cursor { column: 1, line: 0, index: (0, 1).into() },
            ..Default::default()
        };
        let res =
            handle_event(&mut buf, &mut ctx, &mut HistoryStack::new(), &event);
        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert_eq!(ctx, expected_ctx);
    }
    #[test]
    fn right_at_buffer_end_does_nothing() {
        let event =
            Event::Key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        let mut buf = make_buf(&["123456"], ':');
        let mut ctx = RenderContext {
            display_width: 10,
            display_height: 5,
            cursor: Cursor { column: 7, line: 0, index: (0, 7).into() },
            ..Default::default()
        };
        let expected_buf = buf.clone();
        let expected_ctx = ctx;
        let res =
            handle_event(&mut buf, &mut ctx, &mut HistoryStack::new(), &event);
        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert_eq!(ctx, expected_ctx);
    }

    #[test]
    fn right_moves_cursor_to_next_base_char() {
        let event =
            Event::Key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        let mut buf = make_buf(&["aë🎸ou"], ':');
        let mut ctx = RenderContext {
            display_width: 10,
            display_height: 5,
            cursor: Cursor { column: 1, line: 0, index: (0, 1).into() },
            ..Default::default()
        };
        let expected_buf = buf.clone();
        let expected_ctx = RenderContext {
            cursor: Cursor { column: 2, line: 0, index: (0, 2).into() },
            ..ctx
        };
        let res =
            handle_event(&mut buf, &mut ctx, &mut HistoryStack::new(), &event);
        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert_eq!(ctx, expected_ctx);

        let expected_ctx = RenderContext {
            cursor: Cursor { column: 3, line: 0, index: (0, 5).into() },
            ..ctx
        };
        let res =
            handle_event(&mut buf, &mut ctx, &mut HistoryStack::new(), &event);
        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert_eq!(ctx, expected_ctx);

        let expected_ctx = RenderContext {
            cursor: Cursor { column: 5, line: 0, index: (0, 9).into() },
            ..ctx
        };
        let res =
            handle_event(&mut buf, &mut ctx, &mut HistoryStack::new(), &event);
        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert_eq!(ctx, expected_ctx);
    }

    #[test]
    fn right_from_last_base_char_moves_to_next_column_0() {
        let event =
            Event::Key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        let mut buf = make_buf(&["12345678", "🎸23456789", ""], ':');
        let mut ctx = RenderContext {
            display_width: 10,
            display_height: 5,
            cursor: Cursor { line: 0, column: 8, index: (0, 8).into() },
            ..Default::default()
        };
        let expected_buf = buf.clone();
        let expected_ctx = RenderContext {
            cursor: Cursor { line: 1, column: 0, index: (1, 0).into() },
            ..ctx
        };
        let res =
            handle_event(&mut buf, &mut ctx, &mut HistoryStack::new(), &event);
        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert_eq!(ctx, expected_ctx);

        let mut ctx = RenderContext {
            display_width: 10,
            display_height: 5,
            cursor: Cursor { line: 1, column: 9, index: (1, 11).into() },
            ..Default::default()
        };
        let expected_ctx = RenderContext {
            cursor: Cursor { line: 2, column: 0, index: (2, 0).into() },
            ..ctx
        };
        let res =
            handle_event(&mut buf, &mut ctx, &mut HistoryStack::new(), &event);
        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert_eq!(ctx, expected_ctx);
    }

    #[test]
    fn right_past_bottom_of_large_buffer_pans_buffer_up() {
        let event =
            Event::Key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        let mut buf = make_buf(
            &[
                "1234567ö",
                "🎸23456789",
                "0123456789",
                "0123456789",
                "0123456789",
                "0123456789",
                "abc",
            ],
            ':',
        );
        let mut ctx = RenderContext {
            display_width: 10,
            display_height: 5,
            cursor: Cursor { line: 3, column: 9, index: (3, 9).into() },
            ..Default::default()
        };
        let expected_buf = buf.clone();
        let expected_ctx = RenderContext {
            first_buffer_line: 1,
            cursor: Cursor { line: 3, column: 0, index: (4, 0).into() },
            ..ctx
        };

        let res =
            handle_event(&mut buf, &mut ctx, &mut HistoryStack::new(), &event);
        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert_eq!(ctx, expected_ctx);
    }

    #[test]
    fn end_at_buffer_end_does_nothing() {
        let event = Event::Key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE));
        let mut buf = make_buf(
            &[
                "123456789",
                "0123456789",
                "0123456789",
                "0123456789",
                "0123456789",
                "0123456789",
                "0123456789",
                "0123456789",
                "0123456789",
                "",
            ],
            ':',
        );
        let mut ctx = RenderContext {
            display_width: 10,
            display_height: 5,
            first_buffer_line: 5,
            cursor: Cursor { column: 0, line: 4, index: (9, 0).into() },
            ..Default::default()
        };
        let expected_buf = buf.clone();
        let expected_ctx = ctx;
        let ret =
            handle_event(&mut buf, &mut ctx, &mut HistoryStack::new(), &event);
        assert!(ret.is_continue());
        assert_eq!(buf, expected_buf);
        assert_eq!(ctx, expected_ctx);
    }

    #[test]
    fn end_moves_cursor_to_buffer_end() {
        let event = Event::Key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE));
        let mut buf = make_buf(
            &[
                "123456789",
                "0123456789",
                "0123456789",
                "0123456789",
                "0123456789",
                "0123456789",
                "0123456789",
                "0123456789",
                "0123456789",
                "",
            ],
            ':',
        );
        let mut ctx = RenderContext {
            display_width: 10,
            display_height: 5,
            first_buffer_line: 5,
            cursor: Cursor { column: 5, line: 3, index: (8, 5).into() },
            ..Default::default()
        };
        let expected_buf = buf.clone();
        let expected_ctx = RenderContext {
            cursor: Cursor { column: 0, line: 4, index: (9, 0).into() },
            ..ctx
        };
        let ret =
            handle_event(&mut buf, &mut ctx, &mut HistoryStack::new(), &event);
        assert!(ret.is_continue());
        assert_eq!(buf, expected_buf);
        assert_eq!(ctx, expected_ctx);
    }

    #[test]
    fn end_past_display_bottom_in_small_buffer_scrolls_up() {
        let event = Event::Key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE));
        let mut buf = make_buf(
            &["123456789", "0123456789", "0123456789", "0123456789", ""],
            ':',
        );
        let mut ctx = RenderContext {
            display_width: 10,
            display_height: 5,
            first_buffer_line: 0,
            first_display_line: 3,
            cursor: Cursor { column: 1, line: 3, index: buf.input_start },
            ..Default::default()
        };

        let expected_buf = buf.clone();
        let expected_ctx = RenderContext {
            cursor: Cursor { column: 0, line: 4, index: (4, 0).into() },
            first_display_line: 0,
            scroll_needed: 3,
            ..ctx
        };
        let ret =
            handle_event(&mut buf, &mut ctx, &mut HistoryStack::new(), &event);
        assert!(ret.is_continue());
        assert_eq!(buf, expected_buf);
        assert_eq!(ctx, expected_ctx);
    }

    #[test]
    fn end_past_display_bottom_in_large_buffer_pans_up() {
        let event = Event::Key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE));
        let mut buf = make_buf(
            &[
                "123456789",
                "0123456789",
                "0123456789",
                "0123456789",
                "0123456789",
                "0123456789",
                "0123456789",
                "0123456789",
                "0123456789",
                "",
            ],
            ':',
        );
        let mut ctx = RenderContext {
            display_width: 10,
            display_height: 5,
            first_buffer_line: 0,
            cursor: Cursor { column: 1, line: 0, index: buf.input_start },
            ..Default::default()
        };
        let expected_buf = buf.clone();
        let expected_ctx = RenderContext {
            cursor: Cursor { column: 0, line: 4, index: (9, 0).into() },
            first_buffer_line: 5,
            ..ctx
        };
        let ret =
            handle_event(&mut buf, &mut ctx, &mut HistoryStack::new(), &event);
        assert!(ret.is_continue());
        assert_eq!(buf, expected_buf);
        assert_eq!(ctx, expected_ctx);
    }

    #[test]
    fn delete_at_buffer_end_does_nothing() {
        let event =
            Event::Key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE));
        let mut buf = make_buf(&["aë🎸io"], ':');
        let mut ctx = RenderContext {
            display_width: 10,
            display_height: 5,
            cursor: Cursor { column: 7, line: 0, index: (0, 11).into() },
            ..Default::default()
        };
        let expected_buf = buf.clone();
        let expected_ctx = ctx;
        let res =
            handle_event(&mut buf, &mut ctx, &mut HistoryStack::new(), &event);
        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert_eq!(ctx, expected_ctx);
    }

    #[test]
    fn delete_removes_chars_from_cursor_to_next_base_char() {
        let event =
            Event::Key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE));
        let mut buf = make_buf(&["aë🎸io"], ':');
        let mut ctx = RenderContext {
            display_width: 10,
            display_height: 5,
            cursor: Cursor { column: 2, line: 0, index: (0, 2).into() },
            ..Default::default()
        };

        let expected_buf = make_buf(&["a🎸io"], ':');
        let expected_ctx = ctx;
        let res =
            handle_event(&mut buf, &mut ctx, &mut HistoryStack::new(), &event);
        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert_eq!(ctx, expected_ctx);

        let expected_buf = make_buf(&["aio"], ':');
        let res =
            handle_event(&mut buf, &mut ctx, &mut HistoryStack::new(), &event);
        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert_eq!(ctx, expected_ctx);

        let expected_buf = make_buf(&["ao"], ':');
        let res =
            handle_event(&mut buf, &mut ctx, &mut HistoryStack::new(), &event);
        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert_eq!(ctx, expected_ctx);
    }

    #[test]
    fn delete_at_line_start_wraps_to_previous_if_new_first_char_fits() {
        let event =
            Event::Key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE));
        let mut buf = make_buf(&["12345678", "🎸abc"], ':');
        let mut ctx = RenderContext {
            display_width: 10,
            display_height: 5,
            cursor: Cursor { column: 0, line: 1, index: (1, 0).into() },
            ..Default::default()
        };

        let expected_buf = make_buf(&["12345678a", "bc"], ':');
        let expected_ctx = RenderContext {
            cursor: Cursor { column: 9, line: 0, index: (0, 9).into() },
            ..ctx
        };
        let res =
            handle_event(&mut buf, &mut ctx, &mut HistoryStack::new(), &event);
        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert_eq!(ctx, expected_ctx);
    }

    #[test]
    fn delete_reflows_buffer_from_new_cursor_line() {
        let event =
            Event::Key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE));
        let mut buf = make_buf(
            &["123456789", "0123456789", "0123456789", "0123456789"],
            ':',
        );
        let mut ctx = RenderContext {
            display_width: 10,
            display_height: 5,
            cursor: Cursor { column: 9, line: 0, index: (0, 9).into() },
            ..Default::default()
        };

        let expected_buf = make_buf(
            &["123456780", "1234567890", "1234567890", "123456789"],
            ':',
        );
        let expected_ctx = RenderContext {
            cursor: Cursor { column: 9, line: 0, index: (0, 9).into() },
            ..ctx
        };
        let res =
            handle_event(&mut buf, &mut ctx, &mut HistoryStack::new(), &event);
        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert_eq!(ctx, expected_ctx);
    }

    #[test]
    fn resize_height_smaller_cursor_is_at_end() {
        let mut buf = make_buf(
            &[
                "123456789",
                "012345678",
                "🎸23456789",
                "0123456789",
                "0123456789",
                "0123456789",
                "äbc",
            ],
            ':',
        );
        let mut ctx = RenderContext {
            display_width: 10,
            display_height: 10,
            first_display_line: 3,
            cursor: Cursor { column: 3, line: 9, index: (6, 5).into() },
            ..Default::default()
        };

        let expected_ctx = RenderContext {
            display_height: 8,
            first_display_line: 1,
            cursor: Cursor { column: 3, line: 7, index: (6, 5).into() },
            ..ctx
        };
        let expected_buf = buf.clone();
        let res = handle_event(
            &mut buf,
            &mut ctx,
            &mut HistoryStack::new(),
            &Event::Resize(10, 8),
        );
        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert_eq!(ctx, expected_ctx);

        let expected_ctx = RenderContext {
            display_height: 7,
            first_display_line: 0,
            cursor: Cursor { column: 3, line: 6, index: (6, 5).into() },
            ..ctx
        };
        let res = handle_event(
            &mut buf,
            &mut ctx,
            &mut HistoryStack::new(),
            &Event::Resize(10, 7),
        );
        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert_eq!(ctx, expected_ctx);

        let expected_ctx = RenderContext {
            display_height: 5,
            first_buffer_line: 2,
            cursor: Cursor { column: 3, line: 4, index: (6, 5).into() },
            ..ctx
        };
        let res = handle_event(
            &mut buf,
            &mut ctx,
            &mut HistoryStack::new(),
            &Event::Resize(10, 5),
        );
        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert_eq!(ctx, expected_ctx);
    }

    #[test]
    fn resize_height_smaller_cursor_at_start() {
        let mut buf = make_buf(
            &[
                "123456789",
                "012345678",
                "🎸23456789",
                "0123456789",
                "0123456789",
                "0123456789",
                "äbc",
            ],
            ':',
        );
        let mut ctx = RenderContext {
            display_width: 10,
            display_height: 10,
            first_display_line: 3,
            cursor: Cursor { column: 1, line: 3, index: (0, 1).into() },
            ..Default::default()
        };

        let expected_buf = buf.clone();
        let expected_ctx = RenderContext {
            display_height: 8,
            first_display_line: 1,
            cursor: Cursor { column: 1, line: 1, index: (0, 1).into() },
            ..ctx
        };
        let res = handle_event(
            &mut buf,
            &mut ctx,
            &mut HistoryStack::new(),
            &Event::Resize(10, 8),
        );
        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert_eq!(ctx, expected_ctx);

        let expected_ctx = RenderContext {
            display_height: 7,
            first_display_line: 0,
            cursor: Cursor { column: 1, line: 0, index: (0, 1).into() },
            ..ctx
        };
        let res = handle_event(
            &mut buf,
            &mut ctx,
            &mut HistoryStack::new(),
            &Event::Resize(10, 7),
        );
        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert_eq!(ctx, expected_ctx);

        let expected_ctx = RenderContext {
            display_height: 5,
            cursor: Cursor { column: 1, line: 0, index: (0, 1).into() },
            ..ctx
        };
        let res = handle_event(
            &mut buf,
            &mut ctx,
            &mut HistoryStack::new(),
            &Event::Resize(10, 5),
        );
        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert_eq!(ctx, expected_ctx);
    }

    #[test]
    fn resize_width_smaller_cursor_at_start() {
        let mut buf = make_buf(
            &[
                "123456789",
                "012345678",
                "🎸23456789",
                "0123456789",
                "0123456789",
                "0123456789",
                "äbcdefgh",
            ],
            ':',
        );
        let mut ctx = RenderContext {
            display_width: 10,
            display_height: 10,
            first_display_line: 3,
            cursor: Cursor { column: 1, line: 3, index: (0, 1).into() },
            ..Default::default()
        };

        let expected_buf = make_buf(
            &[
                "12345", "678901", "234567", "8🎸234", "567890", "123456",
                "789012", "345678", "901234", "56789ä", "bcdefg", "h",
            ],
            ':',
        );
        let expected_ctx = RenderContext { display_width: 6, ..ctx };

        let res = handle_event(
            &mut buf,
            &mut ctx,
            &mut HistoryStack::new(),
            &Event::Resize(6, 10),
        );
        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert_eq!(ctx, expected_ctx);
    }

    #[test]
    fn resize_width_smaller_cursor_at_start_lg_prompt() {
        let mut buf = EditBuffer {
            lines: vec![
                "lgprompt:9".into(),
                "012345678".into(),
                "🎸23456789".into(),
                "0123456789".into(),
                "0123456789".into(),
                "0123456789".into(),
                "äbcdefgh".into(),
            ],
            prompt_char_count: 9,
            input_start: (0, 9).into(),
            ..Default::default()
        };
        let mut ctx = RenderContext {
            display_width: 10,
            display_height: 10,
            first_display_line: 3,
            cursor: Cursor { column: 9, line: 3, index: (0, 9).into() },
            ..Default::default()
        };

        let expected_buf = EditBuffer {
            lines: vec![
                "lgprom".into(),
                "pt:901".into(),
                "234567".into(),
                "8🎸234".into(),
                "567890".into(),
                "123456".into(),
                "789012".into(),
                "345678".into(),
                "901234".into(),
                "56789ä".into(),
                "bcdefg".into(),
                "h".into(),
            ],
            input_start: (1, 3).into(),
            ..buf.clone()
        };
        let expected_ctx = RenderContext {
            display_width: 6,
            cursor: Cursor { column: 3, line: 4, index: (1, 3).into() },
            ..ctx
        };

        let res = handle_event(
            &mut buf,
            &mut ctx,
            &mut HistoryStack::new(),
            &Event::Resize(6, 10),
        );
        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert_eq!(ctx, expected_ctx);
    }

    #[test]
    fn resize_width_smaller_cursor_is_at_end() {
        let mut buf = make_buf(
            &[
                "123456789",
                "012345678",
                "🎸23456789",
                "0123456789",
                "0123456789",
                "0123456789",
                "äbcdefgh",
            ],
            ':',
        );
        let mut ctx = RenderContext {
            display_width: 10,
            display_height: 10,
            first_display_line: 3,
            cursor: Cursor { column: 8, line: 9, index: (6, 10).into() },
            ..Default::default()
        };

        let expected_buf = make_buf(
            &[
                "12345", "678901", "234567", "8🎸234", "567890", "123456",
                "789012", "345678", "901234", "56789ä", "bcdefg", "h",
            ],
            ':',
        );
        let expected_ctx = RenderContext {
            cursor: Cursor { column: 1, line: 9, index: (11, 1).into() },
            first_display_line: 0,
            scroll_needed: 3,
            first_buffer_line: 2,
            display_width: 6,
            ..ctx
        };

        let res = handle_event(
            &mut buf,
            &mut ctx,
            &mut HistoryStack::new(),
            &Event::Resize(6, 10),
        );
        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert_eq!(ctx, expected_ctx);
    }

    #[test]
    fn resize_height_larger_cursor_is_at_end() {
        let mut buf = make_buf(
            &[
                "123456789",
                "012345678",
                "🎸23456789",
                "0123456789",
                "0123456789",
                "0123456789",
                "0123456789",
                "0123456789",
                "äbcdefgh",
            ],
            ':',
        );
        let mut ctx = RenderContext {
            display_width: 10,
            display_height: 6,
            first_buffer_line: 3,
            cursor: Cursor { column: 8, line: 5, index: (8, 10).into() },
            ..Default::default()
        };

        let event = Event::Resize(10, 10);
        let expected_buf = buf.clone();
        let expected_ctx = RenderContext {
            first_display_line: 0,
            first_buffer_line: 0,
            display_height: 10,
            cursor: Cursor { column: 8, line: 8, index: (8, 10).into() },
            ..ctx
        };
        let res =
            handle_event(&mut buf, &mut ctx, &mut HistoryStack::new(), &event);
        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert_eq!(ctx, expected_ctx);
    }

    #[test]
    fn resize_height_larger_cursor_at_start() {
        let mut buf = make_buf(
            &[
                "123456789",
                "012345678",
                "🎸23456789",
                "0123456789",
                "0123456789",
                "0123456789",
                "0123456789",
                "0123456789",
                "äbcdefgh",
            ],
            ':',
        );
        let mut ctx = RenderContext {
            display_width: 10,
            display_height: 6,
            cursor: Cursor { column: 1, line: 0, index: (0, 1).into() },
            ..Default::default()
        };

        let event = Event::Resize(10, 10);
        let expected_buf = buf.clone();
        let expected_ctx = RenderContext { display_height: 10, ..ctx };
        let res =
            handle_event(&mut buf, &mut ctx, &mut HistoryStack::new(), &event);
        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert_eq!(ctx, expected_ctx);
    }

    #[test]
    fn resize_width_larger_cursor_at_start() {
        let mut buf = make_buf(
            &[
                "12345", "678901", "234567", "8🎸234", "567890", "123456",
                "789012", "345678", "901234", "56789ä", "bcdefg", "h",
            ],
            ':',
        );
        let mut ctx = RenderContext {
            display_width: 6,
            display_height: 10,
            first_display_line: 0,
            cursor: Cursor { column: 1, line: 0, index: (0, 1).into() },
            ..Default::default()
        };

        let expected_buf = make_buf(
            &[
                "123456789",
                "012345678",
                "🎸23456789",
                "0123456789",
                "0123456789",
                "0123456789",
                "äbcdefgh",
            ],
            ':',
        );
        let expected_ctx = RenderContext { display_width: 10, ..ctx };
        let res = handle_event(
            &mut buf,
            &mut ctx,
            &mut HistoryStack::new(),
            &Event::Resize(10, 10),
        );
        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert_eq!(ctx, expected_ctx);
    }

    #[test]
    fn resize_width_larger_cursor_at_start_lg_prompt() {
        let mut buf = EditBuffer {
            lines: vec![
                "lgprom".into(),
                "pt:901".into(),
                "234567".into(),
                "8🎸234".into(),
                "567890".into(),
                "123456".into(),
                "789012".into(),
                "345678".into(),
                "901234".into(),
                "56789ä".into(),
                "bcdefg".into(),
                "h".into(),
            ],
            input_start: (1, 3).into(),
            prompt_char_count: 9,
            ..Default::default()
        };
        let mut ctx = RenderContext {
            display_width: 6,
            display_height: 10,
            first_display_line: 0,
            cursor: Cursor { column: 3, line: 1, index: (1, 3).into() },
            ..Default::default()
        };

        let expected_buf = EditBuffer {
            lines: vec![
                "lgprompt:9".into(),
                "012345678".into(),
                "🎸23456789".into(),
                "0123456789".into(),
                "0123456789".into(),
                "0123456789".into(),
                "äbcdefgh".into(),
            ],
            input_start: (0, 9).into(),
            ..buf.clone()
        };
        let expected_ctx = RenderContext {
            display_width: 10,
            cursor: Cursor { column: 9, line: 0, index: (0, 9).into() },
            ..ctx
        };
        let res = handle_event(
            &mut buf,
            &mut ctx,
            &mut HistoryStack::new(),
            &Event::Resize(10, 10),
        );
        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert_eq!(ctx, expected_ctx);
    }

    #[test]
    fn resize_width_larger_cursor_is_at_end() {
        let mut buf = make_buf(
            &[
                "12345", "678901", "234567", "8🎸234", "567890", "123456",
                "789012", "345678", "901234", "56789ä", "bcdefg", "hi",
            ],
            ':',
        );
        let mut ctx = RenderContext {
            display_width: 6,
            display_height: 10,
            first_buffer_line: 2,
            cursor: Cursor { column: 2, line: 9, index: (11, 2).into() },
            ..Default::default()
        };

        let expected_buf = make_buf(
            &[
                "123456789",
                "012345678",
                "🎸23456789",
                "0123456789",
                "0123456789",
                "0123456789",
                "äbcdefghi",
            ],
            ':',
        );
        let expected_ctx = RenderContext {
            display_width: 10,
            first_buffer_line: 0,
            cursor: Cursor { column: 9, line: 6, index: (6, 11).into() },
            ..ctx
        };
        let res = handle_event(
            &mut buf,
            &mut ctx,
            &mut HistoryStack::new(),
            &Event::Resize(10, 10),
        );
        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert_eq!(ctx, expected_ctx);
    }

    #[test]
    fn up_nop_if_empty_history() {
        let mut buf = make_buf(&["abcdëf🎸"], ':');
        let mut ctx = RenderContext {
            display_width: 10,
            display_height: 5,
            cursor: Cursor { column: 9, line: 0, index: (0, 13).into() },
            ..Default::default()
        };
        let expected_buf = buf.clone();
        let expected_ctx = ctx;
        let res = handle_event(
            &mut buf,
            &mut ctx,
            &mut HistoryStack::new(),
            &Event::Key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)),
        );
        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert_eq!(ctx, expected_ctx);
    }

    #[test]
    fn down_nop_if_empty_history() {
        let mut buf = make_buf(&["abcdëf🎸"], ':');
        let mut ctx = RenderContext {
            display_width: 10,
            display_height: 5,
            cursor: Cursor { column: 9, line: 0, index: (0, 13).into() },
            ..Default::default()
        };
        let expected_buf = buf.clone();
        let expected_ctx = ctx;
        let res = handle_event(
            &mut buf,
            &mut ctx,
            &mut HistoryStack::new(),
            &Event::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)),
        );
        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert_eq!(ctx, expected_ctx);
    }

    #[test]
    fn esc_nop_if_empty_history() {
        let mut buf = make_buf(&["abcdëf🎸"], ':');
        let mut ctx = RenderContext {
            display_width: 10,
            display_height: 10,
            cursor: Cursor { column: 9, line: 0, index: (0, 13).into() },
            ..Default::default()
        };
        let expected_buf = buf.clone();
        let expected_ctx = ctx;
        let res = handle_event(
            &mut buf,
            &mut ctx,
            &mut HistoryStack::new(),
            &Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        );
        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert_eq!(ctx, expected_ctx);
    }

    #[test]
    fn down_nop_when_not_viewing_history() {
        let mut buf = make_buf(&["abcdëf🎸"], ':');
        buf.draft = Some("abcdë🎸".to_owned());
        let mut ctx = RenderContext {
            display_width: 10,
            display_height: 5,
            cursor: Cursor { column: 9, line: 0, index: (0, 13).into() },
            ..Default::default()
        };
        let mut hs = HistoryStack::new();
        let expected_buf = buf.clone();
        let expected_ctx = ctx;
        let expected_hs = hs.clone();
        let res = handle_event(
            &mut buf,
            &mut ctx,
            &mut hs,
            &Event::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)),
        );
        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert_eq!(ctx, expected_ctx);
        assert_eq!(hs, expected_hs);
    }

    #[test]
    fn enter_adds_non_empty_input_to_history() {
        let mut buf = make_buf(&["123456789", "abc"], ':');
        buf.draft = Some("abc".to_owned());
        let mut ctx = RenderContext {
            display_width: 10,
            display_height: 5,
            cursor: Cursor { column: 3, line: 1, index: (1, 3).into() },
            ..Default::default()
        };
        let mut hs = HistoryStack::new();
        let expected_hs = HistoryStack {
            lines: vec!["123456789abc".to_owned()],
            edited: vec![None],
            index: 1,
        };
        let res = handle_event(
            &mut buf,
            &mut ctx,
            &mut hs,
            &Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        );
        assert!(res.is_break());
        assert_eq!(hs, expected_hs);
    }

    #[test]
    fn up_editing_input_saves_input_and_views_most_recent_history() {
        let mut buf = make_buf(&["123456789", "abc"], ':');
        let mut ctx = RenderContext {
            display_width: 10,
            display_height: 5,
            cursor: Cursor { column: 3, line: 1, index: (1, 3).into() },
            ..Default::default()
        };
        let mut hs = HistoryStack {
            lines: vec!["foo".to_owned(), "bar".to_owned(), "baz".to_owned()],
            edited: vec![None, None, None],
            index: 3,
        };
        let expected_buf = EditBuffer {
            lines: vec![":baz".into()],
            input_start: (0, 1).into(),
            prompt_char_count: 1,
            draft: Some("123456789abc".to_owned()),
        };
        let mut expected_hs = hs.clone();
        expected_hs.index = 2;
        let expected_ctx = RenderContext {
            cursor: Cursor { column: 4, line: 0, index: (0, 4).into() },
            ..ctx
        };
        let event = Event::Key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        let res = handle_event(&mut buf, &mut ctx, &mut hs, &event);
        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert_eq!(ctx, expected_ctx);
        assert_eq!(hs, expected_hs);
    }

    #[test]
    fn up_editing_history_saves_edited_and_views_next_older_history() {
        let mut ctx = RenderContext {
            display_width: 10,
            display_height: 5,
            cursor: Cursor { column: 3, line: 0, index: (0, 3).into() },
            ..Default::default()
        };
        let mut buf = EditBuffer {
            lines: vec![":ba".into()],
            input_start: (0, 1).into(),
            prompt_char_count: 1,
            draft: Some("123456789abc".to_owned()),
        };
        let mut hs = HistoryStack {
            lines: vec!["foo".to_owned(), "bar".to_owned(), "baz".to_owned()],
            edited: vec![None, None, None],
            index: 1,
        };

        let expected_ctx = RenderContext {
            cursor: Cursor { column: 4, line: 0, index: (0, 4).into() },
            ..ctx
        };
        let expected_buf =
            EditBuffer { lines: vec![":foo".into()], ..buf.clone() };
        let expected_hs = HistoryStack {
            index: 0,
            edited: vec![None, Some("ba".to_owned()), None],
            ..hs.clone()
        };
        let event = Event::Key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        let res = handle_event(&mut buf, &mut ctx, &mut hs, &event);
        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert_eq!(ctx, expected_ctx);
        assert_eq!(hs, expected_hs);
    }

    #[test]
    fn up_viewing_history_views_next_oldest_history() {
        let mut buf = make_buf(&["baz"], ':');
        buf.draft = Some("123456789abc".to_owned());
        let mut ctx = RenderContext {
            display_width: 10,
            display_height: 5,
            cursor: Cursor { column: 4, line: 0, index: (0, 4).into() },
            ..Default::default()
        };
        let mut hs = HistoryStack {
            lines: vec!["foo".to_owned(), "bar".to_owned(), "baz".to_owned()],
            edited: vec![None, None, None],
            index: 2,
        };
        let expected_buf =
            EditBuffer { lines: vec![":bar".into()], ..buf.clone() };
        let expected_ctx = ctx;
        let expected_hs = HistoryStack { index: 1, ..hs.clone() };
        let event = Event::Key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        let res = handle_event(&mut buf, &mut ctx, &mut hs, &event);
        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert_eq!(ctx, expected_ctx);
        assert_eq!(hs, expected_hs);
    }

    #[test]
    fn up_viewing_history_nop_after_oldest_history() {
        let mut buf = make_buf(&["foo"], ':');
        buf.draft = Some("123456789abc".to_owned());
        let mut ctx = RenderContext {
            display_width: 10,
            display_height: 5,
            cursor: Cursor { column: 4, line: 0, index: (0, 4).into() },
            ..Default::default()
        };
        let mut hs = HistoryStack {
            lines: vec!["foo".to_owned(), "bar".to_owned(), "baz".to_owned()],
            edited: vec![None, None, None],
            index: 0,
        };
        let expected_buf = buf.clone();
        let expected_ctx = ctx;
        let expected_hs = hs.clone();
        let event = Event::Key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        let res = handle_event(&mut buf, &mut ctx, &mut hs, &event);
        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert_eq!(ctx, expected_ctx);
        assert_eq!(hs, expected_hs);
    }

    #[test]
    fn down_viewing_history_views_next_newest_history() {
        let mut buf = make_buf(&["foo"], ':');
        buf.draft = Some("123456789abc".to_owned());
        let mut ctx = RenderContext {
            display_width: 10,
            display_height: 5,
            cursor: Cursor { column: 4, line: 0, index: (0, 4).into() },
            ..Default::default()
        };
        let mut hs = HistoryStack {
            lines: vec!["foo".to_owned(), "bar".to_owned(), "baz".to_owned()],
            edited: vec![None, None, None],
            index: 0,
        };
        let expected_buf =
            EditBuffer { lines: vec![":bar".into()], ..buf.clone() };
        let expected_ctx = ctx;
        let expected_hs = HistoryStack { index: 1, ..hs.clone() };
        let event =
            Event::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        let res = handle_event(&mut buf, &mut ctx, &mut hs, &event);
        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert_eq!(ctx, expected_ctx);
        assert_eq!(hs, expected_hs);
    }

    #[test]
    fn down_from_newest_history_returns_to_editing_draft() {
        let mut ctx = RenderContext {
            display_width: 10,
            display_height: 5,
            cursor: Cursor { column: 4, line: 0, index: (0, 4).into() },
            ..Default::default()
        };
        let mut buf = EditBuffer {
            lines: vec![":baz".into()],
            prompt_char_count: 1,
            input_start: (0, 1).into(),
            draft: Some("123456789abc".to_owned()),
        };
        let mut hs = HistoryStack {
            lines: vec!["foo".to_owned(), "bar".to_owned(), "baz".to_owned()],
            edited: vec![None, None, None],
            index: 2,
        };
        let expected_ctx = RenderContext {
            cursor: Cursor { column: 3, line: 1, index: (1, 3).into() },
            ..ctx
        };
        let expected_buf = EditBuffer {
            lines: vec![":123456789".into(), "abc".into()],
            draft: None,
            ..buf.clone()
        };
        let expected_hs = HistoryStack { index: 3, ..hs.clone() };
        let event =
            Event::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        let res = handle_event(&mut buf, &mut ctx, &mut hs, &event);
        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert_eq!(ctx, expected_ctx);
        assert_eq!(hs, expected_hs);
    }

    #[test]
    fn esc_editing_history_edits_draft() {
        let mut hs = HistoryStack {
            lines: vec!["foo".to_owned(), "bar".to_owned(), "baz".to_owned()],
            edited: vec![None, None, None],
            index: 0,
        };
        let mut buf = make_buf(&["fo"], ':');
        buf.draft = Some("123456789abc".to_owned());
        let mut ctx = RenderContext {
            display_width: 10,
            display_height: 5,
            cursor: Cursor { column: 3, line: 0, index: (0, 3).into() },
            ..Default::default()
        };
        let expected_buf = make_buf(&["123456789", "abc"], ':');
        let expected_ctx = RenderContext {
            cursor: Cursor { column: 3, line: 1, index: (1, 3).into() },
            ..ctx
        };
        let expected_hs = HistoryStack {
            lines: vec!["foo".to_owned(), "bar".to_owned(), "baz".to_owned()],
            edited: vec![None, None, None],
            index: 3,
        };
        let res = handle_event(
            &mut buf,
            &mut ctx,
            &mut hs,
            &Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        );
        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert_eq!(ctx, expected_ctx);
        assert_eq!(hs, expected_hs);
    }

    #[test]
    fn esc_nop_when_editing_input() {
        let mut buf = make_buf(&["some text"], ':');
        let mut ctx = RenderContext {
            display_width: 10,
            display_height: 5,
            cursor: Cursor { line: 0, column: 10, index: (0, 10).into() },
            ..Default::default()
        };
        let expected_buf = buf.clone();
        let expected_ctx = ctx;
        let res = handle_event(
            &mut buf,
            &mut ctx,
            &mut HistoryStack::new(),
            &Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        );
        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert_eq!(ctx, expected_ctx);
    }

    #[test]
    fn esc_viewing_history_after_editing_input_edits_input() {
        let mut buf = EditBuffer {
            lines: vec![":foo".into()],
            input_start: (0, 1).into(),
            prompt_char_count: 1,
            draft: Some("123456789abc".to_owned()),
        };
        let mut ctx = RenderContext {
            display_width: 10,
            display_height: 5,
            cursor: Cursor { column: 4, line: 0, index: (0, 4).into() },
            ..Default::default()
        };
        let mut hs = HistoryStack {
            lines: vec!["foo".to_owned(), "bar".to_owned(), "baz".to_owned()],
            edited: vec![None, None, None],
            index: 0,
        };

        let expected_buf = EditBuffer {
            lines: vec![":123456789".into(), "abc".into()],
            draft: None,
            ..buf.clone()
        };
        let expected_ctx = RenderContext {
            cursor: Cursor { column: 3, line: 1, index: (1, 3).into() },
            ..ctx
        };
        let expected_hs = HistoryStack { index: 3, ..hs.clone() };
        let res = handle_event(
            &mut buf,
            &mut ctx,
            &mut hs,
            &Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        );
        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert_eq!(ctx, expected_ctx);
        assert_eq!(hs, expected_hs);
    }
}
