use std::fmt;
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
    buffer: GapBuffer,
}

// Private structs and enums
////////

#[derive(Debug, Default, Clone)]
struct GapBuffer {
    before_gap: String,
    after_gap: String,
}

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
struct Renderer<'a> {
    /// Reference to current prompt string
    prompt: &'a str,

    /// Prompt width in terminal columns
    prompt_width: u16,

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
enum Response {
    Accept(usize),
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

impl<'a> LineReader {
    #[must_use]
    pub fn new() -> LineReader {
        LineReader { buffer: GapBuffer::new() }
    }

    #[cfg(not(tarpaulin_include))]
    fn accept_line(
        &mut self,
        prompt: &'a str,
        cancelable: bool,
        output_buffer: &mut String,
    ) -> io::Result<Option<usize>> {
        let prompt_width =
            u16::try_from(prompt.width()).expect("prompt width < 64k");
        // clear gap buffer
        self.buffer.clear();

        let (term_cols, term_lines) = terminal::size()?;
        let (cursor_column, cursor_line) = cursor::position()?;
        let mut renderer = Renderer::new(
            prompt,
            prompt_width,
            term_cols,
            term_lines,
            cursor_column,
            cursor_line,
        );
        terminal::enable_raw_mode()?;

        loop {
            renderer.repaint(&self.buffer)?;
            // get next event
            let event = event::read()?;

            // handle event
            let response = self.handle_event(&event);

            match response {
                Response::Accept(bytes_read) => {
                    renderer.move_to_end(&mut self.buffer)?;
                    output_buffer.extend(self.buffer.before_gap.drain(..));
                    output_buffer.extend(self.buffer.after_gap.drain(..));
                    return Ok(Some(bytes_read));
                }
                Response::Cancel => {
                    if cancelable {
                        io::stdout().execute(MoveToNextLine(1))?;
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
                self.buffer.after_gap.push_str(native_eol());
                let bytes_read = self.buffer.len();
                Response::Accept(bytes_read)
            }
            KeyCode::Left => {
                if let Some((prev_idx, _)) = self
                    .buffer
                    .before_gap
                    .char_indices()
                    .rfind(|(_, c)| c.width().is_some_and(|w| w > 0))
                {
                    self.buffer
                        .after_gap
                        .insert_str(0, &self.buffer.before_gap[prev_idx..]);
                    self.buffer.before_gap.truncate(prev_idx);
                }
                Response::Continue
            }
            KeyCode::Right => {
                if let Some((next_idx, _)) = self
                    .buffer
                    .after_gap
                    .char_indices()
                    .skip(1)
                    .find(|(_, c)| c.width().is_some_and(|w| w > 0))
                {
                    self.buffer
                        .before_gap
                        .push_str(&self.buffer.after_gap[..next_idx]);
                    self.buffer.after_gap.drain(..next_idx);
                } else if !self.buffer.after_gap.is_empty() {
                    self.buffer.before_gap.push_str(&self.buffer.after_gap);
                    self.buffer.after_gap.clear();
                }
                Response::Continue
            }
            KeyCode::Home => {
                self.buffer.gap_to_beginning();
                Response::Continue
            }
            KeyCode::End => {
                self.buffer.gap_to_end();
                Response::Continue
            }
            KeyCode::Backspace => {
                if let Some((prev_idx, _)) =
                    self.buffer.before_gap.char_indices().next_back()
                {
                    self.buffer.before_gap.truncate(prev_idx);
                }
                Response::Continue
            }
            KeyCode::Delete => {
                if let Some((next_idx, _)) = self
                    .buffer
                    .after_gap
                    .char_indices()
                    .skip(1)
                    .find(|(_, c)| c.width().is_some_and(|w| w > 0))
                {
                    self.buffer.after_gap.drain(..next_idx);
                } else if !self.buffer.after_gap.is_empty() {
                    self.buffer.after_gap.clear();
                }
                Response::Continue
            }
            KeyCode::Char(c) => {
                self.buffer.before_gap.push(c);
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

// impls for GapBuffer
////////

impl fmt::Display for GapBuffer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}", self.before_gap, self.after_gap)
    }
}

impl GapBuffer {
    fn new() -> GapBuffer {
        GapBuffer { before_gap: String::new(), after_gap: String::new() }
    }

    fn len(&self) -> usize {
        self.before_gap.len() + self.after_gap.len()
    }

    fn clear(&mut self) {
        self.before_gap.clear();
        self.after_gap.clear();
    }

    /// Move gap (insetion point) to end of buffer
    fn gap_to_end(&mut self) {
        if !self.after_gap.is_empty() {
            self.before_gap.push_str(&self.after_gap[..]);
            self.after_gap.clear();
        }
    }

    /// Move gap (insertion point) to beginning of buffer
    fn gap_to_beginning(&mut self) {
        if !self.before_gap.is_empty() {
            self.after_gap.insert_str(0, &self.before_gap[..]);
            self.before_gap.clear();
        }
    }
}

// impls for Renderer
////////

impl<'a> Renderer<'a> {
    fn new(
        prompt: &'a str,
        prompt_width: u16,
        terminal_columns: u16,
        terminal_lines: u16,
        cursor_column: u16,
        cursor_line: u16,
    ) -> Renderer<'a> {
        Renderer {
            prompt,
            prompt_width,
            terminal_columns,
            terminal_lines,
            display_start: DisplayStart::Prompt(cursor_line),
            cursor_column,
            cursor_line,
        }
    }

    #[cfg(not(tarpaulin_include))]
    fn move_to_end(&mut self, buffer: &mut GapBuffer) -> io::Result<()> {
        let (mut cur_col, mut cur_line) = cursor::position()?;
        let mut stdout = io::stdout().lock();
        let after_gap_width = buffer.after_gap.width();
        let term_height = self.terminal_lines as usize;
        let last_line = (after_gap_width / self.terminal_columns as usize)
            + cur_line as usize;
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
                ((self.terminal_lines - 1) * self.terminal_columns) as usize,
            );
            write!(stdout, "{}", &buffer.after_gap[offset..])?;
        }
        stdout.queue(MoveToNextLine(1))?;
        stdout.flush()
    }

    #[cfg(not(tarpaulin_include))]
    /// repaint current buffer
    fn repaint(&mut self, buffer: &GapBuffer) -> io::Result<()> {
        // update terminal size
        (self.terminal_columns, self.terminal_lines) = terminal::size()?;

        // Compute new cursor location
        (self.cursor_column, self.cursor_line) = match self.display_start {
            DisplayStart::Prompt(l) => {
                let mut col = self.prompt_width;
                let mut line = l;
                for c in buffer.before_gap.chars() {
                    let w = u16::try_from(c.width().unwrap_or(0)).unwrap();
                    col += w;
                    if col >= self.terminal_columns {
                        line += 1;

                        col = w - 1;
                    }
                }
                (col, line)
            }
            DisplayStart::CharIndex(i) => {
                let mut col = 0;
                let mut line = 0;
                for c in buffer.before_gap[i..].chars() {
                    let w = u16::try_from(c.width().unwrap_or(0)).unwrap();
                    col += w;
                    if col >= self.terminal_columns {
                        line += 1;

                        col = w - 1;
                    }
                }
                (col, line)
            }
        };

        // Compute viewport bounds
        let last_vp_line = self.terminal_lines
            - 1
            - u16::from(
                u16::try_from(buffer.after_gap.width()).unwrap()
                    + self.cursor_column
                    > self.terminal_columns,
            );

        // Compute new display_start if cursor outside viewport
        let prev_first_line =
            if let DisplayStart::Prompt(l) = self.display_start { l } else { 0 };
        let (first_line, char_idx, prompt) = match self.display_start {
            DisplayStart::Prompt(l) => {
                if self.cursor_line > last_vp_line {
                    let d_cur = self.cursor_line - last_vp_line;
                    if d_cur <= l {
                        let d_start = l - d_cur;
                        self.display_start = DisplayStart::Prompt(d_start);
                        self.cursor_line -= d_cur;
                        (d_start, 0, self.prompt)
                    } else {
                        let i = self.skip_lines(buffer, (d_cur - l).into());
                        self.display_start = DisplayStart::CharIndex(i);
                        self.cursor_line = last_vp_line;
                        (0, i, "")
                    }
                } else {
                    (l, 0, self.prompt)
                }
            }
            DisplayStart::CharIndex(i) => (0, i, ""),
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
            .write_all(prompt.as_bytes())?;
        // Output from display_start to cursor
        stdout.write_all(buffer.before_gap[char_idx..].as_bytes())?;

        // Output from cursor to last char that fits terminal, if necessary
        if !buffer.after_gap.is_empty() {
            stdout.write_all(
                buffer.after_gap[..self.display_end(buffer)].as_bytes(),
            )?;
        }

        // Move cursor to new cursor location
        stdout.queue(MoveTo(self.cursor_column, self.cursor_line))?;

        // Show the cursor
        stdout.queue(Show)?.flush()
    }

    fn skip_lines(&self, buffer: &GapBuffer, mut n: usize) -> usize {
        let mut cols = self.prompt_width;
        for (i, c) in buffer.before_gap.chars().enumerate() {
            let w = u16::try_from(c.width().unwrap_or(0)).unwrap();
            cols += w;
            if cols > self.terminal_columns {
                if n == 1 {
                    return i;
                }
                n -= 1;
                cols = w;
            }
        }
        0
    }

    fn display_end(&self, buffer: &GapBuffer) -> usize {
        let mut cols = self.cursor_column;
        let mut lines_left = self.terminal_lines - 1 - self.cursor_line;
        for (i, c) in buffer.after_gap.chars().enumerate() {
            let w = u16::try_from(c.width().unwrap_or(0)).unwrap();
            cols += w;
            if cols + u16::from(buffer.after_gap.is_empty())
                > self.terminal_columns
            {
                if lines_left == 0 {
                    return i;
                }
                lines_left -= 1;
                cols = w;
            }
        }
        buffer.after_gap.len()
    }
}

impl Drop for Renderer<'_> {
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

    // tests for GapBuffer
    ////////

    #[test]
    fn gap_buffer_new_creates_empty_buffer() {
        let buf = GapBuffer::new();
        assert_eq!(buf.to_string(), "");
    }

    #[test]
    fn gap_buffer_converts_to_string() {
        let text = "Text before; text after".to_owned();
        let cursor = 12usize;
        let buffer = GapBuffer {
            before_gap: text[..cursor].to_owned(),
            after_gap: text[cursor..].to_owned(),
        };
        assert_eq!(buffer.to_string(), text);
    }

    #[test]
    fn gap_buffer_clears() {
        let text = "Text before; text after".to_owned();
        let cursor = 12usize;
        let mut buffer = GapBuffer {
            before_gap: text[..cursor].to_owned(),
            after_gap: text[cursor..].to_owned(),
        };
        buffer.clear();
        assert_eq!(buffer.len(), 0);
    }

    #[test]
    fn move_gap_to_end() {
        let mut buffer = GapBuffer {
            before_gap: "Before|".to_owned(),
            after_gap: "|After".to_owned(),
        };

        buffer.gap_to_end();
        assert_eq!(buffer.before_gap, "Before||After");
        assert!(buffer.after_gap.is_empty());
    }

    #[test]
    fn move_gap_to_beginning() {
        let mut buffer = GapBuffer {
            before_gap: "Before|".to_owned(),
            after_gap: "|After".to_owned(),
        };

        buffer.gap_to_beginning();
        assert_eq!(buffer.after_gap, "Before||After");
        assert!(buffer.before_gap.is_empty());
    }

    // tests for LineReader
    ////////

    #[test]
    fn create_new_reader() {
        let reader = LineReader::new();
        assert_eq!(reader.buffer.len(), 0);
    }

    #[test]
    fn create_default_reader() {
        let reader = LineReader { ..Default::default() };
        assert_eq!(reader.buffer.len(), 0);
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
        let expected = format!("{buffer_text}{}", native_eol());
        let mut reader = LineReader {
            buffer: GapBuffer {
                before_gap: buffer_text[..8].to_owned(),
                after_gap: buffer_text[8..].to_owned(),
            },
        };
        let event =
            Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        let res = reader.handle_event(&event);
        assert!(
            matches!(res, Response::Accept(bytes) if bytes == expected.len())
        );
    }

    #[test]
    fn handle_event_char_adds_char_to_buffer() {
        let buffer_text = "This is some text";
        let expected = format!("{buffer_text}.{}", native_eol());
        let mut reader = LineReader {
            buffer: GapBuffer {
                before_gap: buffer_text.to_owned(),
                ..Default::default()
            },
        };
        let event =
            Event::Key(KeyEvent::new(KeyCode::Char('.'), KeyModifiers::NONE));
        let res = reader.handle_event(&event);
        assert!(matches!(res, Response::Continue));
        let res = reader.handle_event(&Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        if let Response::Accept(bytes) = res {
            assert_eq!(bytes, expected.len());
        } else {
            panic!("response was not Accept");
        }
        assert_eq!(reader.buffer.to_string(), expected);
    }

    #[test]
    fn handle_event_backspace_removes_last_code_point() {
        let buffer_text = "This is some text.";
        let mut reader = LineReader {
            buffer: GapBuffer {
                before_gap: buffer_text.to_owned(),
                ..Default::default()
            },
        };
        let event =
            Event::Key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        let res = reader.handle_event(&event);
        assert!(matches!(res, Response::Continue));
        assert_eq!(
            reader.buffer.to_string(),
            buffer_text[..buffer_text.len() - 1]
        );
    }

    #[test]
    fn handle_event_backspace_removes_only_one_code_point() {
        let buffer_text = "2⁵";
        let expected = "2";
        let mut reader = LineReader {
            buffer: GapBuffer {
                before_gap: buffer_text.to_owned(),
                ..Default::default()
            },
        };
        let event =
            Event::Key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        let res = reader.handle_event(&event);
        assert!(matches!(res, Response::Continue));
        assert_eq!(reader.buffer.to_string(), expected);
    }

    #[test]
    fn left_arrow_moves_to_previous_base_char() {
        let buffer_text = "dëf";
        let mut reader = LineReader {
            buffer: GapBuffer {
                before_gap: buffer_text.to_owned(),
                ..Default::default()
            },
        };
        let event =
            Event::Key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        let res = reader.handle_event(&event);
        assert!(matches!(res, Response::Continue));
        assert_eq!(reader.buffer.before_gap, "dë");
        assert_eq!(reader.buffer.after_gap, "f");
        let res = reader.handle_event(&event);
        assert!(matches!(res, Response::Continue));
        assert_eq!(reader.buffer.before_gap, "d");
        assert_eq!(reader.buffer.after_gap, "ëf");
    }

    #[test]
    fn left_arrow_at_beginning_does_nothing() {
        let buffer_text = "dëf";
        let mut reader = LineReader {
            buffer: GapBuffer {
                after_gap: buffer_text.to_owned(),
                ..Default::default()
            },
        };
        let event =
            Event::Key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        let res = reader.handle_event(&event);
        assert!(matches!(res, Response::Continue));
        assert_eq!(reader.buffer.after_gap, buffer_text);
        assert!(reader.buffer.before_gap.is_empty());
    }

    #[test]
    fn right_arrow_moves_to_next_base_char() {
        let buffer_text = "dëf";
        let mut reader = LineReader {
            buffer: GapBuffer {
                after_gap: buffer_text.to_owned(),
                ..Default::default()
            },
        };
        let event =
            Event::Key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        let res = reader.handle_event(&event);
        assert!(matches!(res, Response::Continue));
        assert_eq!(reader.buffer.before_gap, "d");
        assert_eq!(reader.buffer.after_gap, "ëf");
        let res = reader.handle_event(&event);
        assert!(matches!(res, Response::Continue));
        assert_eq!(reader.buffer.before_gap, "dë");
        assert_eq!(reader.buffer.after_gap, "f");
    }

    #[test]
    fn right_arrow_moves_past_final_char() {
        let mut reader = LineReader {
            buffer: GapBuffer {
                before_gap: "lm".to_owned(),
                after_gap: "ñ".to_owned(),
            },
        };
        let event =
            Event::Key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        let res = reader.handle_event(&event);
        assert!(matches!(res, Response::Continue));
        assert!(reader.buffer.after_gap.is_empty());
        assert_eq!(reader.buffer.before_gap, "lmñ");
    }

    #[test]
    fn right_arrow_at_end_does_nothing() {
        let mut reader = LineReader {
            buffer: GapBuffer {
                before_gap: "lmñ".to_owned(),
                ..Default::default()
            },
        };
        let event =
            Event::Key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        let res = reader.handle_event(&event);
        assert!(matches!(res, Response::Continue));
        assert!(reader.buffer.after_gap.is_empty());
        assert_eq!(reader.buffer.before_gap, "lmñ");
    }

    #[test]
    fn home_moves_to_beginning() {
        let mut reader = LineReader {
            buffer: GapBuffer {
                before_gap: "lmn".to_owned(),
                after_gap: "op".to_owned(),
            },
        };
        let event =
            Event::Key(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE));

        let res = reader.handle_event(&event);
        assert!(matches!(res, Response::Continue));
        assert!(reader.buffer.before_gap.is_empty());
        assert_eq!(reader.buffer.after_gap, "lmnop");
    }

    #[test]
    fn end_moves_to_end() {
        let mut reader = LineReader {
            buffer: GapBuffer {
                before_gap: "lmn".to_owned(),
                after_gap: "op".to_owned(),
            },
        };
        let event = Event::Key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE));

        let res = reader.handle_event(&event);
        assert!(matches!(res, Response::Continue));
        assert_eq!(reader.buffer.before_gap, "lmnop");
        assert!(reader.buffer.after_gap.is_empty());
    }

    #[test]
    fn delete_removes_char_with_combining_marks() {
        let mut reader = LineReader {
            buffer: GapBuffer {
                before_gap: "d".to_owned(),
                after_gap: "ëf".to_owned(),
            },
        };
        let event =
            Event::Key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE));
        let res = reader.handle_event(&event);
        assert!(matches!(res, Response::Continue));
        assert_eq!(reader.buffer.after_gap, "f");
        assert_eq!(reader.buffer.before_gap, "d");
    }

    #[test]
    fn delete_removes_last_char() {
        let mut reader = LineReader {
            buffer: GapBuffer {
                before_gap: "d".to_owned(),
                after_gap: "ë".to_owned(),
            },
        };
        let event =
            Event::Key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE));
        let res = reader.handle_event(&event);
        assert!(matches!(res, Response::Continue));
        assert!(reader.buffer.after_gap.is_empty());
        assert_eq!(reader.buffer.before_gap, "d");
    }
}
