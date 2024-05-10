use std::io::{self, BufRead, Write};

use crossterm::cursor::{self, Hide, MoveTo, MoveToNextLine, Show};
use crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers,
};
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

    /// # Errors
    ///
    /// Will return `io::Error` if an error is encountered reading a line

    fn read_line_or_cancel(
        &mut self,
        prompt: &str,
        buffer: &mut String,
    ) -> io::Result<Option<usize>> {
        self.read_line(prompt, buffer).map_or(Ok(None), |bytes| Ok(Some(bytes)))
    }
}

#[derive(Debug, Default)]
pub struct LineReader {
    prompt_len: usize,
    prompt_width: usize,
    before_gap: String,
    after_gap: String,
}

// Private structs and enums
////////

#[derive(Debug, Copy, Clone)]
enum DisplayStart {
    /// Prompt's terminal line
    Prompt(u16),

    /// Offset into buffer of beginning of terminal line 0
    /// (index into `before_gap`, lines skipped)
    CharIndex(usize),
}

/// Values needed to render the buffer to the terminal window.
///
/// todo: Some of these may not be necessary (e.g., if they need to be
/// recomputed every time the buffer is rendered) or may only be useful
/// in the future (e.g., if we recompute them now, but perhaps update
/// in response to events in the future as an optimization).
#[derive(Debug)]
struct RenderContext {
    /// Current terminal width
    terminal_columns: u16,

    /// current terminal height
    terminal_lines: u16,

    /// Descriptor of beginning of text in terminal window
    display_start: DisplayStart,

    /// Terminal column of cursor position (0 based)
    cursor_column: u16,

    /// Terminal line of cursor position (0 based)
    cursor_line: u16,
}

#[derive(Debug)]
struct NewRenderContext {
    /// Current terminal width
    terminal_columns: u16,

    /// Current terminal height
    terminal_columns: u16,

    /// First terminal line used
    first_display_line: u16,

    /// Index of first displayed char in buffer
    first_char_idx: usize,
}

#[derive(Debug)]
enum Response {
    Accept,
    Cancel,
    Continue,
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

// impls for LineReader
////////

impl LineReader {
    #[must_use]
    pub fn new() -> LineReader {
        LineReader {
            prompt_len: 0,
            prompt_width: 0,
            before_gap: String::new(),
            after_gap: String::new(),
        }
    }

    #[cfg(not(tarpaulin_include))]
    fn accept_line(
        &mut self,
        prompt: &str,
        cancelable: bool,
        output_buffer: &mut String,
    ) -> io::Result<Option<usize>> {
        let (term_cols, term_lines) = terminal::size()?;
        let (cursor_column, cursor_line) = cursor::position()?;
        let mut render_ctx = RenderContext::new(
            term_cols,
            term_lines,
            cursor_column,
            cursor_line,
        );
        terminal::enable_raw_mode()?;

        // initialize gap buffer
        self.before_gap += prompt;

        self.prompt_width = prompt.width();
        self.prompt_len = prompt.len();

        loop {
            self.repaint(&mut render_ctx)?;
            // get next event
            let event = event::read()?;

            // handle event
            let response = self.handle_event(&event);

            match response {
                Response::Accept => {
                    let bytes_read = self.before_gap.len() - prompt.len()
                        + self.after_gap.len();
                    self.move_to_end(&mut render_ctx)?;
                    *output_buffer += &self.before_gap[prompt.len()..];
                    *output_buffer += &self.after_gap;
                    self.before_gap.clear();
                    self.after_gap.clear();
                    return Ok(Some(bytes_read));
                }
                Response::Cancel => {
                    if cancelable {
                        io::stdout().execute(MoveToNextLine(1))?;
                        self.before_gap.clear();
                        self.after_gap.clear();
                        return Ok(None);
                    }
                }
                Response::Continue => (),
            }
        }
    }

    fn handle_event(&mut self, event: &Event) -> Response {
        match event {
            Event::Key(event) if event.kind == KeyEventKind::Press => {
                self.handle_key_event(event)
            }
            _ => Response::Continue,
        }
    }

    fn handle_key_event(&mut self, event: &KeyEvent) -> Response {
        match event.code {
            KeyCode::Char('d') if event.modifiers == KeyModifiers::CONTROL => {
                Response::Cancel
            }
            KeyCode::Enter => {
                self.after_gap.push_str(native_eol());
                Response::Accept
            }
            KeyCode::Left => {
                if let Some((prev_idx, _)) = self.before_gap[self.prompt_len..]
                    .char_indices()
                    .rfind(|(_, c)| c.width().is_some_and(|w| w > 0))
                {
                    self.after_gap.insert_str(0, &self.before_gap[prev_idx..]);
                    self.before_gap.truncate(prev_idx);
                }
                Response::Continue
            }
            KeyCode::Right => {
                if let Some((next_idx, _)) = self
                    .after_gap
                    .char_indices()
                    .skip(1)
                    .find(|(_, c)| c.width().is_some_and(|w| w > 0))
                {
                    self.before_gap.push_str(&self.after_gap[..next_idx]);
                    self.after_gap.drain(..next_idx);
                } else if !self.after_gap.is_empty() {
                    self.before_gap.push_str(&self.after_gap);
                    self.after_gap.clear();
                }
                Response::Continue
            }
            KeyCode::Home => {
                self.after_gap.insert_str(
                    0,
                    self.before_gap.drain(self.prompt_len..).as_ref(),
                );
                Response::Continue
            }
            KeyCode::End => {
                self.gap_to_end();
                Response::Continue
            }
            KeyCode::Backspace => {
                if let Some((prev_idx, _)) = self.before_gap[self.prompt_len..]
                    .char_indices()
                    .next_back()
                {
                    self.before_gap.truncate(prev_idx);
                }
                Response::Continue
            }
            KeyCode::Delete => {
                if let Some((next_idx, _)) = self
                    .after_gap
                    .char_indices()
                    .skip(1)
                    .find(|(_, c)| c.width().is_some_and(|w| w > 0))
                {
                    self.after_gap.drain(..next_idx);
                } else if !self.after_gap.is_empty() {
                    self.after_gap.clear();
                }
                Response::Continue
            }
            KeyCode::Char(c) => {
                self.before_gap.push(c);
                Response::Continue
            }
            KeyCode::Up => {
                todo!("move to next older entry in history");
            }
            KeyCode::Down => {
                todo!("move to next newer entry in history");
            }
            _ => Response::Continue,
        }
    }

    /// Move gap (insetion point) to end of buffer
    fn gap_to_end(&mut self) {
        if !self.after_gap.is_empty() {
            self.before_gap.push_str(&self.after_gap[..]);
            self.after_gap.clear();
        }
    }

    /// Move gap (insertion point) to beginning of buffer
    #[cfg(not(tarpaulin_include))]
    fn move_to_end(
        &mut self,
        render_ctx: &mut RenderContext,
    ) -> io::Result<()> {
        let (mut cur_col, mut cur_line) = cursor::position()?;
        let after_gap_width = self.after_gap.width();

        let mut stdout = io::stdout().lock();
        let term_height = render_ctx.terminal_lines as usize;
        let last_line = after_gap_width
            / usize::from(render_ctx.terminal_columns)
            + usize::from(cur_line);
        let new_cursor_line = last_line + 1;
        if new_cursor_line >= term_height {
            let scroll_needed = new_cursor_line - term_height + 1;
            if scroll_needed >= term_height {
                cur_line = 0;
                cur_col = 0;
            } else {
                let scroll_needed = u16::try_from(scroll_needed).expect(
                    "scroll_needed < term_height, so should fit in u16",
                );
                stdout.queue(ScrollUp(scroll_needed))?;
                cur_line -= scroll_needed;
            }
            stdout
                .queue(MoveTo(cur_col, cur_line))?
                .queue(Clear(ClearType::FromCursorDown))?;
            let offset = after_gap_width.saturating_sub(
                ((render_ctx.terminal_lines - 1) * render_ctx.terminal_columns)
                    as usize,
            );
            write!(stdout, "{}", &self.after_gap[offset..])?;
        }
        stdout.queue(MoveToNextLine(1))?;
        stdout.flush()
    }

    #[cfg(not(tarpaulin_include))]
    /// repaint current buffer
    fn new_repaint(
        &mut self,
        render_ctx: &mut NewRenderContext,
    ) -> io::Result<()> {
        // update display size
        (render_ctx.terminal_columns, render_ctx.terminal_lines) =
            terminal::size()?;

        // compute buffer extents
        let mut lines_before = 0;
        let mut remainder_before = 0;
        let mut first_input_idx: Option<usize> = None;
        let mut lines_before_display: Option<u16> = None;
        let mut lines_after = 0;
        let mut remainder_after = 0;
        let mut first_line_idx_after: Option<usize> = None;

        for (i, c) in self.before_gap.char_indices() {
            if i == self.prompt_len && first_input_idx.is_none() {
                first_input_idx = Some(i);
            }
            let w = u16::try_from(c.width().unwrap_or(0)).unwrap();
            remainder_before += w;
            if remainder_before > render_ctx.terminal_columns {
                lines_before += 1;
                remainder_before = w;
            }
        }

        for (i, c) in self.after_gap.char_indices() {
            let w = u16::try_from(c.width().unwrap_or(0)).unwrap();
            remainder_after += w;
            if remainder_before + remainder_after > render_ctx.terminal_columns
            {
                first_line_idx_after.get_or_insert(i);
                lines_after += 1;
                remainder_after = w;
            }
        }

        let last_vp_line = if render_ctx.terminal_lines > 2
            && (first_line_idx_after.is_some()
                || remainder_before + remainder_after
                    == render_ctx.terminal_columns)
        {
            render_ctx.terminal_lines - 2
        } else {
            render_ctx.terminal_lines - 1
        };

        // update display model
        let mut scroll_needed: Option<u16> = None;
        match render_ctx.display_start {
            DisplayStart::Prompt(l) => {
                if lines_before > last_vp_line {
                    if l > 0 {
                        scroll_needed = Some(l);
                    }
                    render_ctx.display_start =
                        DisplayStart::CharIndex(self.skip_display_lines(
                            render_ctx,
                            lines_before - last_vp_line,
                        ));
                } else {
                    let new_l = last_vp_line - lines_before;
                    if new_l < l {
                        scroll_needed = new_l - l;
                        render_ctx.display_start = DisplayStart::Prompt(new_l);
                    }
                }
            }
            DisplayStart::CharIndex(i) => {
                todo!();
            }
        }
        // render buffer to display
        if i >= self.before_gap.len() {
            // cursor would be above display
        }
    }

    fn repaint(&mut self, render_ctx: &mut RenderContext) -> io::Result<()> {
        // update terminal size
        (render_ctx.terminal_columns, render_ctx.terminal_lines) =
            terminal::size()?;

        // Compute new cursor location
        (render_ctx.cursor_column, render_ctx.cursor_line) =
            match render_ctx.display_start {
                DisplayStart::Prompt(l) => {
                    let (bg_lines, bg_rem) =
                        render_ctx.display_space(&self.before_gap);
                    (bg_rem - 1, bg_lines)
                    //                    let mut col = 0;
                    //                    let mut line = l;
                    //                    for c in self.before_gap.chars() {
                    //                        let w = u16::try_from(c.width().unwrap_or(0)).unwrap();
                    //                        col += w;
                    //                        if col >= render_ctx.terminal_columns {
                    //                            line += 1;
                    //
                    //                            col = w - 1;
                    //                        }
                    //                    }
                    //                    (col, line)
                }
                DisplayStart::CharIndex(i) => {
                    let mut col = 0;
                    let mut line = 0;
                    for c in self.before_gap.chars().skip(i) {
                        let w = u16::try_from(c.width().unwrap_or(0)).unwrap();
                        col += w;
                        if col >= render_ctx.terminal_columns {
                            line += 1;

                            col = w - 1;
                        }
                    }
                    (col, line)
                }
            };

        // Compute viewport bounds
        let last_vp_line = render_ctx.terminal_lines
            - 1
            - u16::from(
                u16::try_from(self.after_gap.width()).unwrap()
                    + render_ctx.cursor_column
                    > render_ctx.terminal_columns,
            );

        // Compute new display_start if cursor outside viewport
        let prev_first_line =
            if let DisplayStart::Prompt(l) = render_ctx.display_start {
                l
            } else {
                0
            };
        eprintln!(
            "repaint: {:?} cursor ({}, {})",
            render_ctx.display_start,
            render_ctx.cursor_column,
            render_ctx.cursor_line
        );
        let (first_line, char_idx) = match render_ctx.display_start {
            DisplayStart::Prompt(l) => {
                if render_ctx.cursor_line > last_vp_line {
                    let d_cur = render_ctx.cursor_line - last_vp_line;
                    if d_cur <= l {
                        let d_start = l - d_cur;
                        render_ctx.display_start =
                            DisplayStart::Prompt(d_start);
                        render_ctx.cursor_line -= d_cur;
                        (d_start, 0)
                    } else {
                        let i = self.skip_display_lines(
                            render_ctx,
                            d_cur - l,
                            None,
                        );
                        render_ctx.display_start = DisplayStart::CharIndex(i);
                        render_ctx.cursor_line = last_vp_line;
                        (0, i)
                    }
                } else {
                    (l, 0)
                }
            }
            DisplayStart::CharIndex(i) => {
                if render_ctx.cursor_line > last_vp_line {
                    // new cursor past end of vp, skip lines to adjust display_start
                    let d_cur = render_ctx.cursor_line - last_vp_line;
                    let new_i =
                        self.skip_display_lines(render_ctx, d_cur, Some(i));
                    render_ctx.display_start = DisplayStart::CharIndex(new_i);
                    render_ctx.cursor_line = last_vp_line;
                    (0, new_i)
                } else if i > 0 && render_ctx.cursor_line == 0 {
                    // new cursor before start of vp
                    let new_i =
                        self.skip_display_lines_rev(render_ctx, 1u16, Some(i));
                    render_ctx.display_start = DisplayStart::CharIndex(new_i);
                    (0, new_i)
                } else {
                    (0, i)
                }
            }
        };

        // Prepare to send commands & output to terminal
        let mut stdout = io::stdout().lock();

        // Hide the cursor
        // Move cursor to new display_start
        // Clear to end of terminal
        stdout.queue(Hide)?;
        if prev_first_line > first_line {
            stdout.queue(ScrollUp(prev_first_line - first_line))?;
        }
        stdout
            .queue(MoveTo(0, first_line))?
            .queue(Clear(ClearType::FromCursorDown))?
            .write_all(self.before_gap[char_idx..].as_bytes())?;

        // Output from cursor to last char that fits terminal, if necessary
        if !self.after_gap.is_empty() {
            stdout.write_all(
                self.after_gap[..self.display_end(render_ctx)].as_bytes(),
            )?;
        }

        // Move cursor to new cursor location
        stdout
            .queue(MoveTo(render_ctx.cursor_column, render_ctx.cursor_line))?;

        // Show the cursor
        stdout.queue(Show)?.flush()
    }

    fn skip_display_lines(
        &self,
        render_ctx: &mut RenderContext,
        n: impl Into<usize>,
        char_offset: Option<usize>,
    ) -> usize {
        let mut n: usize = n.into();
        let mut cols = 0;
        let char_offset = char_offset.unwrap_or(0);
        for (i, c) in self.before_gap.char_indices().skip(char_offset) {
            let w = u16::try_from(c.width().unwrap_or(0)).unwrap();
            cols += w;
            if cols > render_ctx.terminal_columns {
                if n == 1 {
                    return i;
                }
                n -= 1;
                cols = w;
            }
        }
        0
    }

    fn skip_display_lines_rev(
        &self,
        render_ctx: &mut RenderContext,
        n: impl Into<usize>,
        char_offset: Option<usize>,
    ) -> usize {
        let char_offset = char_offset.unwrap_or(0);
        if char_offset == 0 {
            return 0;
        }
        let mut n: usize = n.into();
        let mut cols = 0;
        for (i, c) in self.before_gap[..char_offset].char_indices().rev() {
            let w = u16::try_from(c.width().unwrap_or(0)).unwrap();
            cols += w;
            if cols > render_ctx.terminal_columns {
                if n == 1 {
                    return i;
                }
                n -= 1;
                cols = w;
            }
        }
        0
    }

    fn display_end(&self, render_ctx: &RenderContext) -> usize {
        let mut cols = render_ctx.cursor_column;
        eprintln!(
            "display_end: lines_left = {} - 1 - {}",
            render_ctx.terminal_lines, render_ctx.cursor_line
        );
        let mut lines_left =
            render_ctx.terminal_lines - 1 - render_ctx.cursor_line;
        for (i, c) in self.after_gap.chars().enumerate() {
            let w = u16::try_from(c.width().unwrap_or(0)).unwrap();
            cols += w;
            if cols + u16::from(self.after_gap.is_empty())
                > render_ctx.terminal_columns
            {
                if lines_left == 0 {
                    return i;
                }
                lines_left -= 1;
                cols = w;
            }
        }
        self.after_gap.len()
    }
}

impl LineRead for LineReader {
    #[cfg(not(tarpaulin_include))]
    fn read_line(
        &mut self,
        prompt: &str,
        buffer: &mut String,
    ) -> io::Result<usize> {
        Ok(self.accept_line(prompt, false, buffer)?.unwrap_or(0))
    }

    fn read_line_or_cancel(
        &mut self,
        prompt: &str,
        buffer: &mut String,
    ) -> io::Result<Option<usize>> {
        self.accept_line(prompt, true, buffer)
    }
}

// impls for RenderContext
////////

impl NewRenderContext {
    fn new(
        terminal_columns: u16,
        terminal_lines: u16,
        first_display_line: u16,
        first_char_index: usize,
    ) -> NewRenderContext {
        NewRenderContext {
            terminal_columns,
            terminal_lines,
            first_display_line,
            first_char_index,
        }
    }
}

impl Drop for NewRenderContext {
    #[cfg(not(tarpaulin_include))]
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
        let _ = io::stdout().execute(Show);
    }
}

impl RenderContext {
    fn new(
        terminal_columns: u16,
        terminal_lines: u16,
        cursor_column: u16,
        cursor_line: u16,
    ) -> RenderContext {
        RenderContext {
            terminal_columns,
            terminal_lines,
            display_start: DisplayStart::Prompt(cursor_line),
            cursor_column,
            cursor_line,
        }
    }

    fn display_space(&self, s: &str) -> (u16, u16) {
        let mut cols = 0;
        let mut lines = 0;
        for c in s.chars() {
            let w = u16::try_from(c.width().unwrap_or(0)).unwrap();
            cols += w;
            if cols >= self.terminal_columns {
                lines += 1;
                cols = w;
            }
        }
        (lines, cols)
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

    // tests for LineReader
    ////////

    #[test]
    fn move_gap_to_end() {
        let mut reader = LineReader {
            before_gap: "Before|".to_owned(),
            after_gap: "|After".to_owned(),
            ..Default::default()
        };

        reader.gap_to_end();
        assert_eq!(reader.before_gap, "Before||After");
        assert!(reader.after_gap.is_empty());
    }

    #[test]
    fn create_new_reader() {
        let reader = LineReader::new();
        assert_eq!(reader.before_gap.len(), 0);
    }

    #[test]
    fn create_default_reader() {
        let reader = LineReader { ..Default::default() };
        assert_eq!(reader.before_gap.len(), 0);
    }

    #[test]
    fn unimplemented_event_ignored() {
        let mut reader = LineReader::new();
        let event = Event::FocusLost;
        let res = reader.handle_event(&event);
        assert!(matches!(res, Response::Continue));
    }

    #[test]
    fn unimplemented_key_event_ignored() {
        let mut reader = LineReader::new();
        let event =
            Event::Key(KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE));
        let res = reader.handle_event(&event);
        assert!(matches!(res, Response::Continue));
    }

    #[test]
    fn handle_event_ctrl_d_returns_canceled() {
        let mut reader = LineReader::new();
        let event = Event::Key(KeyEvent::new(
            KeyCode::Char('d'),
            KeyModifiers::CONTROL,
        ));
        let res = reader.handle_event(&event);
        assert!(matches!(res, Response::Cancel));
    }

    #[test]
    fn handle_event_enter_returns_accept() {
        let buffer_text = "This is some text.";
        let mut reader = LineReader {
            before_gap: buffer_text[..8].to_owned(),
            after_gap: buffer_text[8..].to_owned(),
            ..Default::default()
        };
        let event =
            Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        let res = reader.handle_event(&event);
        assert!(matches!(res, Response::Accept));
    }

    #[test]
    fn handle_event_char_adds_char_to_buffer() {
        let buffer_text = "This is some text";
        let expected = format!("{buffer_text}.");
        let mut reader = LineReader {
            before_gap: buffer_text.to_owned(),
            ..Default::default()
        };
        let event =
            Event::Key(KeyEvent::new(KeyCode::Char('.'), KeyModifiers::NONE));
        let res = reader.handle_event(&event);
        assert!(matches!(res, Response::Continue));
        reader.handle_event(&Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        assert_eq!(reader.before_gap.to_string(), expected);
    }

    #[test]
    fn handle_event_backspace_removes_last_code_point() {
        let buffer_text = "This is some text.";
        let mut reader = LineReader {
            before_gap: buffer_text.to_owned(),
            ..Default::default()
        };
        let event =
            Event::Key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        let res = reader.handle_event(&event);
        assert!(matches!(res, Response::Continue));
        assert_eq!(
            reader.before_gap.to_string(),
            buffer_text[..buffer_text.len() - 1]
        );
    }

    #[test]
    fn handle_event_backspace_removes_only_one_code_point() {
        let buffer_text = "2⁵";
        let expected = "2";
        let mut reader = LineReader {
            before_gap: buffer_text.to_owned(),
            ..Default::default()
        };
        let event =
            Event::Key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        let res = reader.handle_event(&event);
        assert!(matches!(res, Response::Continue));
        assert_eq!(reader.before_gap, expected);
    }

    #[test]
    fn left_arrow_moves_to_previous_base_char() {
        let buffer_text = "dëf";
        let mut reader = LineReader {
            before_gap: buffer_text.to_owned(),
            ..Default::default()
        };
        let event =
            Event::Key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        let res = reader.handle_event(&event);
        assert!(matches!(res, Response::Continue));
        assert_eq!(reader.before_gap, "dë");
        assert_eq!(reader.after_gap, "f");
        let res = reader.handle_event(&event);
        assert!(matches!(res, Response::Continue));
        assert_eq!(reader.before_gap, "d");
        assert_eq!(reader.after_gap, "ëf");
    }

    #[test]
    fn left_arrow_at_beginning_does_nothing() {
        let buffer_text = "dëf";
        let mut reader = LineReader {
            after_gap: buffer_text.to_owned(),
            ..Default::default()
        };
        let event =
            Event::Key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        let res = reader.handle_event(&event);
        assert!(matches!(res, Response::Continue));
        assert_eq!(reader.after_gap, buffer_text);
        assert!(reader.before_gap.is_empty());
    }

    #[test]
    fn right_arrow_moves_to_next_base_char() {
        let buffer_text = "dëf";
        let mut reader = LineReader {
            after_gap: buffer_text.to_owned(),
            ..Default::default()
        };
        let event =
            Event::Key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        let res = reader.handle_event(&event);
        assert!(matches!(res, Response::Continue));
        assert_eq!(reader.before_gap, "d");
        assert_eq!(reader.after_gap, "ëf");
        let res = reader.handle_event(&event);
        assert!(matches!(res, Response::Continue));
        assert_eq!(reader.before_gap, "dë");
        assert_eq!(reader.after_gap, "f");
    }

    #[test]
    fn right_arrow_moves_past_final_char() {
        let mut reader = LineReader {
            before_gap: "lm".to_owned(),
            after_gap: "ñ".to_owned(),
            ..Default::default()
        };
        let event =
            Event::Key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        let res = reader.handle_event(&event);
        assert!(matches!(res, Response::Continue));
        assert!(reader.after_gap.is_empty());
        assert_eq!(reader.before_gap, "lmñ");
    }

    #[test]
    fn right_arrow_at_end_does_nothing() {
        let mut reader =
            LineReader { before_gap: "lmñ".to_owned(), ..Default::default() };
        let event =
            Event::Key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        let res = reader.handle_event(&event);
        assert!(matches!(res, Response::Continue));
        assert!(reader.after_gap.is_empty());
        assert_eq!(reader.before_gap, "lmñ");
    }

    #[test]
    fn home_moves_to_beginning() {
        let mut reader = LineReader {
            before_gap: "lmn".to_owned(),
            after_gap: "op".to_owned(),
            ..Default::default()
        };
        let event =
            Event::Key(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE));

        let res = reader.handle_event(&event);
        assert!(matches!(res, Response::Continue));
        assert!(reader.before_gap.is_empty());
        assert_eq!(reader.after_gap, "lmnop");
    }

    #[test]
    fn end_moves_to_end() {
        let mut reader = LineReader {
            before_gap: "lmn".to_owned(),
            after_gap: "op".to_owned(),
            ..Default::default()
        };
        let event = Event::Key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE));

        let res = reader.handle_event(&event);
        assert!(matches!(res, Response::Continue));
        assert_eq!(reader.before_gap, "lmnop");
        assert!(reader.after_gap.is_empty());
    }

    #[test]
    fn delete_removes_char_with_combining_marks() {
        let mut reader = LineReader {
            before_gap: "d".to_owned(),
            after_gap: "ëf".to_owned(),
            ..Default::default()
        };
        let event =
            Event::Key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE));
        let res = reader.handle_event(&event);
        assert!(matches!(res, Response::Continue));
        assert_eq!(reader.after_gap, "f");
        assert_eq!(reader.before_gap, "d");
    }

    #[test]
    fn delete_removes_last_char() {
        let mut reader = LineReader {
            before_gap: "d".to_owned(),
            after_gap: "ë".to_owned(),
            ..Default::default()
        };
        let event =
            Event::Key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE));
        let res = reader.handle_event(&event);
        assert!(matches!(res, Response::Continue));
        assert!(reader.after_gap.is_empty());
        assert_eq!(reader.before_gap, "d");
    }
}
