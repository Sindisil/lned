use std::cmp;
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
    bg_buf: String,
    bg_line_idx: Vec<usize>,
    ag_buf: String,
    ag_display_end: usize,
}

// Private structs and enums
////////

#[derive(Debug, Copy, Clone)]
enum DisplayStart {
    /// Prompt's terminal line
    Prompt(u16),

    /// Offset into buffer of beginning of terminal line 0
    /// (index into `bg_buf`, lines skipped)
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

    /// Terminal column of cursor position (0 based)
    cursor_column: u16,

    /// Terminal line of cursor position (0 based)
    cursor_line: u16,

    /// First display line used
    first_display_line: u16,

    /// First buffer line displayed
    first_buffer_line: usize,

    /// Lines to scroll up before rendering
    scroll_needed: u16,
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
            bg_buf: String::new(),
            bg_line_idx: Vec::new(),
            ag_buf: String::new(),
            ag_display_end: 0,
        }
    }

    #[cfg(not(tarpaulin_include))]
    fn accept_line(
        &mut self,
        prompt: &str,
        cancelable: bool,
        output_buffer: &mut String,
    ) -> io::Result<Option<usize>> {
        // Get the initial display dimensions
        let (term_cols, term_lines) = terminal::size()?;

        // initialize gap buffer
        self.bg_buf += prompt;
        self.prompt_width = prompt.width();
        self.prompt_len = prompt.len();

        // Initialize display line indices
        self.ag_display_end = 0;
        if self.prompt_len > 0 {
            self.bg_line_idx.splice(.., [0]);
            let mut line = 0;
            let mut col = 0;
            for (i, c) in self.bg_buf.char_indices() {
                let w = u16::try_from(c.width().unwrap_or(0)).unwrap();
                col += w;
                if col > term_cols {
                    line += 1;
                    col = w;
                    self.bg_line_idx[line] = i;
                }
            }
        } else {
            self.bg_line_idx.clear();
        }

        // Initialize render context
        let (_, initial_cursor_line) = cursor::position()?;
        let mut render_ctx = RenderContext::new(
            self,
            term_cols,
            term_lines,
            initial_cursor_line,
        );
        terminal::enable_raw_mode()?;

        loop {
            self.repaint(&mut render_ctx)?;
            // get next event
            let event = event::read()?;

            // handle event
            let response = self.handle_event(&event);

            match response {
                Response::Accept => {
                    let bytes_read =
                        self.bg_buf.len() - prompt.len() + self.ag_buf.len();
                    self.move_to_end(&mut render_ctx)?;
                    *output_buffer += &self.bg_buf[prompt.len()..];
                    *output_buffer += &self.ag_buf;
                    self.bg_buf.clear();
                    self.ag_buf.clear();
                    return Ok(Some(bytes_read));
                }
                Response::Cancel => {
                    if cancelable {
                        io::stdout().execute(MoveToNextLine(1))?;
                        self.bg_buf.clear();
                        self.ag_buf.clear();
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
                self.ag_buf.push_str(native_eol());
                Response::Accept
            }
            KeyCode::Left => {
                if let Some((prev_idx, _)) = self.bg_buf[self.prompt_len..]
                    .char_indices()
                    .rfind(|(_, c)| c.width().is_some_and(|w| w > 0))
                {
                    self.ag_buf.insert_str(0, &self.bg_buf[prev_idx..]);
                    self.bg_buf.truncate(prev_idx);
                }
                Response::Continue
            }
            KeyCode::Right => {
                if let Some((next_idx, _)) = self
                    .ag_buf
                    .char_indices()
                    .skip(1)
                    .find(|(_, c)| c.width().is_some_and(|w| w > 0))
                {
                    self.bg_buf.push_str(&self.ag_buf[..next_idx]);
                    self.ag_buf.drain(..next_idx);
                } else if !self.ag_buf.is_empty() {
                    self.bg_buf.push_str(&self.ag_buf);
                    self.ag_buf.clear();
                }
                Response::Continue
            }
            KeyCode::Home => {
                self.ag_buf.insert_str(
                    0,
                    self.bg_buf.drain(self.prompt_len..).as_ref(),
                );
                Response::Continue
            }
            KeyCode::End => {
                self.gap_to_end();
                Response::Continue
            }
            KeyCode::Backspace => {
                if let Some((prev_idx, _)) =
                    self.bg_buf[self.prompt_len..].char_indices().next_back()
                {
                    self.bg_buf.truncate(prev_idx);
                }
                Response::Continue
            }
            KeyCode::Delete => {
                if let Some((next_idx, _)) = self
                    .ag_buf
                    .char_indices()
                    .skip(1)
                    .find(|(_, c)| c.width().is_some_and(|w| w > 0))
                {
                    self.ag_buf.drain(..next_idx);
                } else if !self.ag_buf.is_empty() {
                    self.ag_buf.clear();
                }
                Response::Continue
            }
            KeyCode::Char(c) => {
                self.bg_buf.push(c);
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
        if !self.ag_buf.is_empty() {
            self.bg_buf.push_str(&self.ag_buf[..]);
            self.ag_buf.clear();
        }
    }

    /// Move gap (insertion point) to beginning of buffer
    #[cfg(not(tarpaulin_include))]
    fn move_to_end(
        &mut self,
        render_ctx: &mut RenderContext,
    ) -> io::Result<()> {
        let (mut cur_col, mut cur_line) = cursor::position()?;
        let ag_buf_width = self.ag_buf.width();

        let mut stdout = io::stdout().lock();
        let term_height = render_ctx.terminal_lines as usize;
        let last_line = ag_buf_width / usize::from(render_ctx.terminal_columns)
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
            let offset = ag_buf_width.saturating_sub(
                ((render_ctx.terminal_lines - 1) * render_ctx.terminal_columns)
                    as usize,
            );
            write!(stdout, "{}", &self.ag_buf[offset..])?;
        }
        stdout.queue(MoveToNextLine(1))?;
        stdout.flush()
    }

    #[cfg(not(tarpaulin_include))]
    /// render current buffer to display
    fn repaint(&mut self, render_ctx: &mut RenderContext) -> io::Result<()> {
        let mut stdout = io::stdout().lock();

        stdout.queue(Hide)?;

        if render_ctx.scroll_needed > 0 {
            stdout.queue(ScrollUp(render_ctx.scroll_needed))?;
        }

        stdout
            .queue(MoveTo(0, render_ctx.first_display_line))?
            .queue(Clear(ClearType::FromCursorDown))?
            .write_all(
                self.bg_buf[self.bg_line_idx[render_ctx.first_buffer_line]..]
                    .as_bytes(),
            )?;
        if !self.ag_buf.is_empty() {
            stdout.write_all(self.ag_buf[0..self.ag_display_end].as_bytes())?;
        }
        stdout
            .queue(MoveTo(render_ctx.cursor_column, render_ctx.cursor_line))?
            .queue(Show)?
            .flush()
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

impl RenderContext {
    fn new(
        reader: &LineReader,
        terminal_columns: u16,
        terminal_lines: u16,
        cursor_line: u16,
    ) -> RenderContext {
        // convert some values to usize for convenience
        let terminal_lines = usize::from(terminal_lines);
        let cursor_line = usize::from(cursor_line);

        let last_buffer_line_idx =
            reader.bg_line_idx.last().copied().unwrap_or(0);
        let mut new_cursor_column =
            u16::try_from(reader.bg_buf[last_buffer_line_idx..].width())
                .unwrap();
        let mut new_cursor_line = cursor_line + reader.bg_line_idx.len();
        if new_cursor_column == terminal_columns {
            new_cursor_line += 1;
            new_cursor_column = 0;
        }

        let (first_display_line, first_buffer_line, scroll_needed) =
            if new_cursor_line > terminal_lines {
                let overrun = new_cursor_line - terminal_lines + 1;
                let lines_left = terminal_lines - cursor_line - 1;
                let scroll_needed =
                    u16::try_from(cmp::min(overrun, lines_left)).unwrap();
                let first_display_line = cursor_line.saturating_sub(overrun);
                (first_display_line, overrun, scroll_needed)
            } else {
                (cursor_line, 0, 0)
            };

        RenderContext {
            terminal_columns,
            terminal_lines: terminal_lines.try_into().unwrap(),
            cursor_column: new_cursor_column,
            cursor_line: new_cursor_line.try_into().unwrap(),
            first_display_line: first_display_line.try_into().unwrap(),
            first_buffer_line,
            scroll_needed,
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
            bg_buf: "Before|".to_owned(),
            ag_buf: "|After".to_owned(),
            ..Default::default()
        };

        reader.gap_to_end();
        assert_eq!(reader.bg_buf, "Before||After");
        assert!(reader.ag_buf.is_empty());
    }

    #[test]
    fn create_new_reader() {
        let reader = LineReader::new();
        assert_eq!(reader.bg_buf.len(), 0);
    }

    #[test]
    fn create_default_reader() {
        let reader = LineReader { ..Default::default() };
        assert_eq!(reader.bg_buf.len(), 0);
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
            bg_buf: buffer_text[..8].to_owned(),
            ag_buf: buffer_text[8..].to_owned(),
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
        let mut reader =
            LineReader { bg_buf: buffer_text.to_owned(), ..Default::default() };
        let event =
            Event::Key(KeyEvent::new(KeyCode::Char('.'), KeyModifiers::NONE));
        let res = reader.handle_event(&event);
        assert!(matches!(res, Response::Continue));
        reader.handle_event(&Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        assert_eq!(reader.bg_buf.to_string(), expected);
    }

    #[test]
    fn handle_event_backspace_removes_last_code_point() {
        let buffer_text = "This is some text.";
        let mut reader =
            LineReader { bg_buf: buffer_text.to_owned(), ..Default::default() };
        let event =
            Event::Key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        let res = reader.handle_event(&event);
        assert!(matches!(res, Response::Continue));
        assert_eq!(
            reader.bg_buf.to_string(),
            buffer_text[..buffer_text.len() - 1]
        );
    }

    #[test]
    fn handle_event_backspace_removes_only_one_code_point() {
        let buffer_text = "2⁵";
        let expected = "2";
        let mut reader =
            LineReader { bg_buf: buffer_text.to_owned(), ..Default::default() };
        let event =
            Event::Key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        let res = reader.handle_event(&event);
        assert!(matches!(res, Response::Continue));
        assert_eq!(reader.bg_buf, expected);
    }

    #[test]
    fn left_arrow_moves_to_previous_base_char() {
        let buffer_text = "dëf";
        let mut reader =
            LineReader { bg_buf: buffer_text.to_owned(), ..Default::default() };
        let event =
            Event::Key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        let res = reader.handle_event(&event);
        assert!(matches!(res, Response::Continue));
        assert_eq!(reader.bg_buf, "dë");
        assert_eq!(reader.ag_buf, "f");
        let res = reader.handle_event(&event);
        assert!(matches!(res, Response::Continue));
        assert_eq!(reader.bg_buf, "d");
        assert_eq!(reader.ag_buf, "ëf");
    }

    #[test]
    fn left_arrow_at_beginning_does_nothing() {
        let buffer_text = "dëf";
        let mut reader =
            LineReader { ag_buf: buffer_text.to_owned(), ..Default::default() };
        let event =
            Event::Key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        let res = reader.handle_event(&event);
        assert!(matches!(res, Response::Continue));
        assert_eq!(reader.ag_buf, buffer_text);
        assert!(reader.bg_buf.is_empty());
    }

    #[test]
    fn right_arrow_moves_to_next_base_char() {
        let buffer_text = "dëf";
        let mut reader =
            LineReader { ag_buf: buffer_text.to_owned(), ..Default::default() };
        let event =
            Event::Key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        let res = reader.handle_event(&event);
        assert!(matches!(res, Response::Continue));
        assert_eq!(reader.bg_buf, "d");
        assert_eq!(reader.ag_buf, "ëf");
        let res = reader.handle_event(&event);
        assert!(matches!(res, Response::Continue));
        assert_eq!(reader.bg_buf, "dë");
        assert_eq!(reader.ag_buf, "f");
    }

    #[test]
    fn right_arrow_moves_past_final_char() {
        let mut reader = LineReader {
            bg_buf: "lm".to_owned(),
            ag_buf: "ñ".to_owned(),
            ..Default::default()
        };
        let event =
            Event::Key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        let res = reader.handle_event(&event);
        assert!(matches!(res, Response::Continue));
        assert!(reader.ag_buf.is_empty());
        assert_eq!(reader.bg_buf, "lmñ");
    }

    #[test]
    fn right_arrow_at_end_does_nothing() {
        let mut reader =
            LineReader { bg_buf: "lmñ".to_owned(), ..Default::default() };
        let event =
            Event::Key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        let res = reader.handle_event(&event);
        assert!(matches!(res, Response::Continue));
        assert!(reader.ag_buf.is_empty());
        assert_eq!(reader.bg_buf, "lmñ");
    }

    #[test]
    fn home_moves_to_beginning() {
        let mut reader = LineReader {
            bg_buf: "lmn".to_owned(),
            ag_buf: "op".to_owned(),
            ..Default::default()
        };
        let event =
            Event::Key(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE));

        let res = reader.handle_event(&event);
        assert!(matches!(res, Response::Continue));
        assert!(reader.bg_buf.is_empty());
        assert_eq!(reader.ag_buf, "lmnop");
    }

    #[test]
    fn end_moves_to_end() {
        let mut reader = LineReader {
            bg_buf: "lmn".to_owned(),
            ag_buf: "op".to_owned(),
            ..Default::default()
        };
        let event = Event::Key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE));

        let res = reader.handle_event(&event);
        assert!(matches!(res, Response::Continue));
        assert_eq!(reader.bg_buf, "lmnop");
        assert!(reader.ag_buf.is_empty());
    }

    #[test]
    fn delete_removes_char_with_combining_marks() {
        let mut reader = LineReader {
            bg_buf: "d".to_owned(),
            ag_buf: "ëf".to_owned(),
            ..Default::default()
        };
        let event =
            Event::Key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE));
        let res = reader.handle_event(&event);
        assert!(matches!(res, Response::Continue));
        assert_eq!(reader.ag_buf, "f");
        assert_eq!(reader.bg_buf, "d");
    }

    #[test]
    fn delete_removes_last_char() {
        let mut reader = LineReader {
            bg_buf: "d".to_owned(),
            ag_buf: "ë".to_owned(),
            ..Default::default()
        };
        let event =
            Event::Key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE));
        let res = reader.handle_event(&event);
        assert!(matches!(res, Response::Continue));
        assert!(reader.ag_buf.is_empty());
        assert_eq!(reader.bg_buf, "d");
    }
}
