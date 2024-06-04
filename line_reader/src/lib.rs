use std::cmp::{self, Ordering};
use std::io::{self, BufRead, Write};
use std::ops::ControlFlow;

use crossterm::cursor::{self, Hide, MoveTo, MoveToNextLine, Show};
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

#[derive(Debug, Clone)]
pub struct LineReader {
    /// prompt length, in characters
    prompt_len: usize,

    /// prompt display width, in columns
    prompt_width: usize,

    /// characters before the gap (before cursor)
    bg_buf: String,

    /// indicies of characters that start display lines
    bg_line_idx: Vec<usize>,

    /// characters after the gap (at or after cursor)
    ag_buf: String,

    /// current display width, in columns
    display_columns: u16,

    /// current display height, in lines
    display_lines: u16,

    /// Terminal column of cursor position (0 based)
    cursor_column: u16,

    /// Terminal line of cursor position (0 based)
    cursor_line: u16,

    /// First display line used
    first_display_line: u16,

    /// First buffer line displayed
    first_buffer_line: usize,

    /// Number of chars from `ag_buf` that will fit in display
    ag_display_chars: usize,

    /// Lines to scroll up before rendering
    scroll_needed: u16,
}

// Private structs and enums
////////

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

// impls for LineReader
////////

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
        // Get the initial display dimensions
        (self.display_columns, self.display_lines) = terminal::size()?;
        let (_, initial_cursor_line) = cursor::position()?;

        // initialize gap buffer
        self.bg_buf += prompt;
        self.prompt_width = prompt.width();
        self.prompt_len = prompt.len();

        // initialize display line indices
        self.bg_line_idx.splice(.., [0]);
        if self.prompt_len > 0 {
            let mut line = 0;
            let mut col = 0;
            for (i, c) in self.bg_buf.char_indices() {
                let w = u16::try_from(c.width().unwrap_or(0)).unwrap();
                col += w;
                if col > self.display_columns {
                    line += 1;
                    col = w;
                    self.bg_line_idx[line] = i;
                }
            }
        }

        // convert some values to usize for convenience
        let display_lines = usize::from(self.display_lines);
        let cursor_line = usize::from(initial_cursor_line);

        // Initialize rendering related values
        let last_buffer_line_idx =
            self.bg_line_idx.last().copied().unwrap_or(0);
        self.cursor_column =
            u16::try_from(self.bg_buf[last_buffer_line_idx..].width()).unwrap();
        let mut new_cursor_line = cursor_line + self.bg_line_idx.len() - 1;
        if self.cursor_column == self.display_columns {
            new_cursor_line += 1;
            self.cursor_column = 0;
        }
        self.cursor_line = new_cursor_line.try_into().unwrap();

        (self.first_display_line, self.first_buffer_line, self.scroll_needed) =
            if new_cursor_line > display_lines {
                let overrun = new_cursor_line - display_lines + 1;
                let lines_left = display_lines - cursor_line - 1;
                let scroll_needed =
                    u16::try_from(cmp::min(overrun, lines_left)).unwrap();
                let first_display_line = cursor_line.saturating_sub(overrun);
                (first_display_line.try_into().unwrap(), overrun, scroll_needed)
            } else {
                (cursor_line.try_into().unwrap(), 0, 0)
            };

        let _render_ctx = RenderContext::new();
        terminal::enable_raw_mode()?;

        self.repaint()?;
        let mut should_continue = true;
        while should_continue {
            let event = event::read()?;
            should_continue = self.handle_event(&event).is_continue();
            self.repaint()?;
        }

        let bytes_read =
            self.bg_buf.len() - self.prompt_len + self.ag_buf.len();
        self.move_to_end()?;
        *output_buffer += &self.bg_buf[self.prompt_len..];
        *output_buffer += &self.ag_buf;
        self.bg_buf.clear();
        self.ag_buf.clear();
        Ok(bytes_read)
    }

    fn handle_event(&mut self, event: &Event) -> ControlFlow<()> {
        match event {
            Event::Key(event) if event.kind == KeyEventKind::Press => {
                self.handle_key_event(event)
            }
            _ => ControlFlow::Continue(()),
        }
    }

    fn handle_key_event(&mut self, event: &KeyEvent) -> ControlFlow<()> {
        match event.code {
            KeyCode::Enter => {
                self.ag_buf.push_str(native_eol());
                ControlFlow::Break(())
            }
            KeyCode::Left => self.handle_left(),
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
                ControlFlow::Continue(())
            }
            KeyCode::Home => {
                self.ag_buf.insert_str(
                    0,
                    self.bg_buf.drain(self.prompt_len..).as_ref(),
                );
                ControlFlow::Continue(())
            }
            KeyCode::End => {
                self.gap_to_end();
                ControlFlow::Continue(())
            }
            KeyCode::Backspace => self.handle_backspace(),
            KeyCode::Delete => self.handle_delete(),
            KeyCode::Char(c) => self.handle_char_typed(c),
            KeyCode::Up => {
                todo!("move to next older entry in history");
            }
            KeyCode::Down => {
                todo!("move to next newer entry in history");
            }
            _ => ControlFlow::Continue(()),
        }
    }

    fn handle_char_typed(&mut self, c: char) -> ControlFlow<()> {
        let c_width = c.width().unwrap_or(0);

        // handle zero width (combining) characters
        if c_width == 0 {
            // zero width char only valid if not the first character
            if self.bg_buf.len() > self.prompt_len {
                self.bg_buf.push(c);
            }
            return ControlFlow::Continue(());
        }

        let new_char_idx = self.bg_buf.len();
        self.bg_buf.push(c);

        let c_width = u16::try_from(c_width).unwrap();
        self.cursor_column += c_width;

        match self.cursor_column.cmp(&self.display_columns) {
            // typec character overflows cursor line
            Ordering::Greater => {
                self.cursor_column = c_width;
                self.cursor_line += 1;
                self.bg_line_idx.push(new_char_idx);
            }
            // typed character exactly fills cursor line
            Ordering::Equal => {
                self.cursor_column = 0;
                self.cursor_line += 1;
                self.bg_line_idx.push(self.bg_buf.chars().count());
            }
            // typed character fits on cursor line
            Ordering::Less => (),
        }

        // handle cursor moving below viewport bottom
        if self.cursor_line > self.viewport_bottom() {
            self.cursor_line -= 1;
            if self.first_buffer_line == 0 {
                self.scroll_needed = 1;
            }
            if self.first_display_line == 0 {
                self.first_buffer_line += 1;
            } else {
                self.first_display_line =
                    self.first_display_line.saturating_sub(1);
            }
        }

        self.ag_display_chars = self.display_remainder();
        ControlFlow::Continue(())
    }

    fn handle_backspace(&mut self) -> ControlFlow<()> {
        if let Some((i, c)) = self
            .bg_buf
            .char_indices()
            .next_back()
            .filter(|(i, _)| *i >= self.prompt_len)
        {
            self.bg_buf.truncate(i);
            let prev_cursor_line = self.cursor_line;

            if self.cursor_column == 0 {
                // backspacing from column 0, wrap to position of
                // removed char on preceding line.
                let prev_beg = *self.bg_line_idx.iter().nth_back(1).unwrap();
                self.cursor_column =
                    u16::try_from(self.bg_buf[prev_beg..].width()).unwrap();
                self.cursor_line -= 1;
            } else {
                let c_width = u16::try_from(c.width().unwrap_or(0)).unwrap();
                self.cursor_column -= c_width;

                if self.cursor_column == 0 {
                    // backspacing to column 0 - check if room to wrap
                    // to end of preceding line
                    if let Some(&prev_beg) = self.bg_line_idx.iter().nth_back(1)
                    {
                        let prev_w =
                            u16::try_from(self.bg_buf[prev_beg..].width())
                                .unwrap();
                        if prev_w < self.display_columns {
                            self.cursor_column = prev_w;
                            self.cursor_line -= 1;
                        }
                    }
                }
            }

            if prev_cursor_line != self.cursor_line {
                self.bg_line_idx.truncate(self.bg_line_idx.len() - 1);
                if self.cursor_line < self.viewport_top() {
                    self.cursor_line += 1;
                    self.first_buffer_line -= 1;
                }
            }
            self.ag_display_chars = self.display_remainder();
        }
        ControlFlow::Continue(())
    }

    fn handle_left(&mut self) -> ControlFlow<()> {
        if let Some((prev_idx, prev_char)) = self
            .bg_buf
            .char_indices()
            .rfind(|(_, c)| c.width().is_some_and(|w| w > 0))
            .filter(|(i, _)| *i >= self.prompt_len)
        {
            self.ag_buf.insert_str(0, &self.bg_buf[prev_idx..]);
            self.bg_buf.truncate(prev_idx);
            let prev_cursor_line = self.cursor_line;

            if self.cursor_column == 0 {
                let prev_beg = *self.bg_line_idx.iter().nth_back(1).unwrap();
                self.cursor_column =
                    u16::try_from(self.bg_buf[prev_beg..].width()).unwrap();
                self.cursor_line -= 1;
            } else {
                self.cursor_column -=
                    u16::try_from(prev_char.width().unwrap_or(0)).unwrap();
            }
            if prev_cursor_line != self.cursor_line {
                self.bg_line_idx.truncate(self.bg_line_idx.len() - 1);
                if self.cursor_line < self.viewport_top() {
                    self.cursor_line += 1;
                    self.first_buffer_line -= 1;
                }
            }
            self.ag_display_chars = self.display_remainder();
        }

        ControlFlow::Continue(())
    }

    fn handle_delete(&mut self) -> ControlFlow<()> {
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
        self.ag_display_chars = self.display_remainder();
        ControlFlow::Continue(())
    }

    /// Compute last line of viewport
    fn viewport_bottom(&self) -> u16 {
        if self.ag_buf.width()
            <= (self.display_columns.saturating_sub(self.cursor_column)).into()
        {
            self.display_lines - 1
        } else {
            self.display_lines - 2
        }
    }

    /// Compute first line of viewport
    fn viewport_top(&self) -> u16 {
        (self.first_buffer_line > 0).into()
    }

    /// Compute portion of `ag_buf` that fits in display
    fn display_remainder(&self) -> usize {
        let d_width = usize::from(self.display_columns);
        let mut col = usize::from(self.cursor_column);
        let mut line = self.cursor_line;
        let mut n = 0;
        for c in self.ag_buf.chars() {
            let c_width = c.width().unwrap_or(0);
            col += c_width;
            if col > d_width {
                line += 1;
                if line == self.display_lines {
                    break;
                }
                col = c_width;
            }
            n += 1;
        }
        n
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
    fn move_to_end(&mut self) -> io::Result<()> {
        let (mut cur_col, mut cur_line) = cursor::position()?;
        let ag_buf_width = self.ag_buf.width();

        let mut stdout = io::stdout().lock();
        let term_height = usize::from(self.display_lines);
        let last_line = ag_buf_width / usize::from(self.display_columns)
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
                ((self.display_lines - 1) * self.display_columns) as usize,
            );
            write!(stdout, "{}", &self.ag_buf[offset..])?;
        }
        stdout.queue(MoveToNextLine(1))?;
        stdout.flush()
    }

    /// render current buffer to display
    fn repaint(&mut self) -> io::Result<()> {
        let mut stdout = io::stdout().lock();

        stdout.queue(Hide)?;

        if self.scroll_needed > 0 {
            stdout.queue(ScrollUp(self.scroll_needed))?;
            self.scroll_needed = 0;
        }

        stdout
            .queue(MoveTo(0, self.first_display_line))?
            .queue(Clear(ClearType::FromCursorDown))?
            .write_all(
                self.bg_buf[self.bg_line_idx[self.first_buffer_line]..]
                    .as_bytes(),
            )?;
        if !self.ag_buf.is_empty() {
            stdout
                .write_all(self.ag_buf[0..self.ag_display_chars].as_bytes())?;
        }
        stdout
            .queue(MoveTo(self.cursor_column, self.cursor_line))?
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
        self.accept_line(prompt, buffer)
    }
}

impl Default for LineReader {
    fn default() -> Self {
        LineReader {
            prompt_len: 0,
            prompt_width: 0,
            bg_buf: String::new(),
            bg_line_idx: vec![0],
            ag_buf: String::new(),
            display_columns: 80,
            display_lines: 24,
            cursor_column: 0,
            cursor_line: 23,
            first_display_line: 23,
            first_buffer_line: 0,
            ag_display_chars: 0,
            scroll_needed: 0,
        }
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
        let buffer_text = "This is some text.";
        let mut reader = LineReader {
            bg_buf: buffer_text[..8].to_owned(),
            ag_buf: buffer_text[8..].to_owned(),
            ..Default::default()
        };
        let event =
            Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        let res = reader.handle_event(&event);
        assert!(matches!(res, ControlFlow::Break(())));
    }

    #[test]
    fn backspace_removes_only_last_input_char_before_gap() {
        let mut reader = LineReader::new();
        reader.bg_buf.push_str(":ë̱🎸");
        reader.prompt_len = 1;
        reader.prompt_width = 1;
        reader.cursor_column = 4;

        let event =
            Event::Key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));

        // 2w
        let res = reader.handle_event(&event);
        assert!(res.is_continue());
        assert_eq!(&reader.bg_buf, ":ë̱");

        // 1st 0w
        let res = reader.handle_event(&event);
        assert!(res.is_continue());
        assert_eq!(&reader.bg_buf, ":ë");

        // 2nd 0w
        let res = reader.handle_event(&event);
        assert!(res.is_continue());
        assert_eq!(&reader.bg_buf, ":e");

        // 1w
        let res = reader.handle_event(&event);
        assert!(res.is_continue());
        assert_eq!(&reader.bg_buf, ":");

        // prompt
        let res = reader.handle_event(&event);
        assert!(res.is_continue());
        assert_eq!(&reader.bg_buf, ":");
    }

    #[test]
    fn backspace_moves_cursor_back_removed_char_width() {
        let mut reader = LineReader::new();
        reader.bg_buf.push_str(":ë̱🎸");
        reader.prompt_len = 1;
        reader.prompt_width = 1;
        reader.cursor_column = 4;

        let event =
            Event::Key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));

        // 2w
        let res = reader.handle_event(&event);
        assert!(res.is_continue());
        assert_eq!(reader.cursor_column, 2);

        // 1st 0w
        let res = reader.handle_event(&event);
        assert!(res.is_continue());
        assert_eq!(reader.cursor_column, 2);

        // 2nd 0w
        let res = reader.handle_event(&event);
        assert!(res.is_continue());
        assert_eq!(reader.cursor_column, 2);

        // 1w
        let res = reader.handle_event(&event);
        assert!(res.is_continue());
        assert_eq!(reader.cursor_column, 1);

        // prompt
        let res = reader.handle_event(&event);
        assert!(res.is_continue());
        assert_eq!(reader.cursor_column, 1);

        // on second line when first line doesn't fill full width
        let mut reader = LineReader::new();
        reader.prompt_len = 1;
        reader.prompt_width = 1;
        reader.display_columns = 20;
        reader.display_lines = 10;
        reader.bg_buf.push_str(":123456789012345678🎸🎸");
        reader.bg_line_idx = vec![0, 20];
        reader.first_display_line = 8;
        reader.cursor_line = 9;
        reader.cursor_column = 4;
        let event =
            Event::Key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        let res = reader.handle_event(&event);
        assert!(res.is_continue());
        assert_eq!(&reader.bg_buf, ":123456789012345678🎸");
        assert_eq!(reader.cursor_column, 2);
        assert_eq!(reader.cursor_line, 9);
    }

    #[test]
    fn backspace_at_column_0_wraps_cursor_to_preceding_line() {
        let mut reader = LineReader::new();
        reader.prompt_len = 1;
        reader.prompt_width = 1;
        reader.display_columns = 20;
        reader.display_lines = 10;
        reader.bg_buf.push_str(":1234567890123456789");
        reader.bg_line_idx = vec![0, 20];
        reader.first_display_line = 8;
        reader.cursor_line = 9;
        reader.cursor_column = 0;
        let event =
            Event::Key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        let res = reader.handle_event(&event);
        assert!(res.is_continue());
        assert_eq!(&reader.bg_buf, ":123456789012345678");
        assert_eq!(reader.cursor_column, 19);
        assert_eq!(reader.cursor_line, 8);
        assert_eq!(reader.first_display_line, 8);
        assert_eq!(reader.bg_line_idx, vec![0]);
    }

    #[test]
    fn backspace_to_column_0_wraps_cursor_if_room() {
        // room on preceding line
        let mut reader = LineReader::new();
        reader.prompt_len = 1;
        reader.prompt_width = 1;
        reader.display_columns = 20;
        reader.display_lines = 10;
        reader.bg_buf.push_str(":123456789012345678🎸");
        reader.bg_line_idx = vec![0, 20];
        reader.first_display_line = 8;
        reader.cursor_line = 9;
        reader.cursor_column = 2;
        let event =
            Event::Key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        let res = reader.handle_event(&event);
        assert!(res.is_continue());
        assert_eq!(&reader.bg_buf, ":123456789012345678");
        assert_eq!(reader.cursor_column, 19);
        assert_eq!(reader.cursor_line, 8);
        assert_eq!(reader.first_display_line, 8);
        assert_eq!(reader.bg_line_idx, vec![0]);

        // no room on preceding line
        let mut reader = LineReader::new();
        reader.prompt_len = 1;
        reader.prompt_width = 1;
        reader.display_columns = 20;
        reader.display_lines = 10;
        reader.bg_buf.push_str(":1234567890123456789🎸");
        reader.bg_line_idx = vec![0, 20];
        reader.first_display_line = 8;
        reader.cursor_line = 9;
        reader.cursor_column = 2;
        let event =
            Event::Key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        let res = reader.handle_event(&event);
        assert!(res.is_continue());
        assert_eq!(&reader.bg_buf, ":1234567890123456789");
        assert_eq!(reader.cursor_column, 0);
        assert_eq!(reader.cursor_line, 9);
        assert_eq!(reader.first_display_line, 8);
        assert_eq!(reader.bg_line_idx, vec![0, 20]);
    }

    #[test]
    fn backspace_moving_cursor_past_top_pans_buffer() {
        let lines = [
            ":1234567890123456789",
            "a1234567890123456789",
            "b1234567890123456789",
            "c1234567890123456789",
            "d1234567890123456789",
        ];

        let mut reader = LineReader {
            display_columns: 20,
            display_lines: 5,
            bg_buf: lines.join(""),
            bg_line_idx: vec![0, 20, 40, 60, 80, 100],
            prompt_len: 1,
            prompt_width: 1,
            cursor_column: 0,
            cursor_line: 1,
            first_display_line: 0,
            first_buffer_line: 4,
            ..Default::default()
        };

        let event =
            Event::Key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        let res = reader.handle_event(&event);
        assert!(res.is_continue());
        assert_eq!(reader.cursor_column, 19);
        assert_eq!(reader.cursor_line, 1);
        assert_eq!(reader.first_display_line, 0);
        assert_eq!(reader.first_buffer_line, 3);
        assert_eq!(reader.bg_line_idx, vec![0, 20, 40, 60, 80,]);
    }

    #[test]
    fn left_moves_to_previous_base_char() {
        let buffer_text = "dëf";
        let mut reader = LineReader {
            bg_buf: buffer_text.to_owned(),
            cursor_column: 3,
            ..Default::default()
        };
        let event =
            Event::Key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        let res = reader.handle_event(&event);
        assert!(res.is_continue());
        assert_eq!(reader.bg_buf, "dë");
        assert_eq!(reader.ag_buf, "f");
        let res = reader.handle_event(&event);
        assert!(res.is_continue());
        assert_eq!(reader.bg_buf, "d");
        assert_eq!(reader.ag_buf, "ëf");
    }

    #[test]
    fn left_at_beginning_does_nothing() {
        let buffer_text = "dëf";
        let mut reader =
            LineReader { ag_buf: buffer_text.to_owned(), ..Default::default() };
        let event =
            Event::Key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        let res = reader.handle_event(&event);
        assert!(res.is_continue());
        assert_eq!(reader.ag_buf, buffer_text);
        assert!(reader.bg_buf.is_empty());
    }

    #[test]
    fn left_moves_cursor_back_by_preceding_char_width() {
        let mut reader = LineReader {
            bg_buf: ":dë🎸f".to_owned(),
            prompt_width: 1,
            prompt_len: 1,
            cursor_column: 6,
            ..Default::default()
        };

        let event =
            Event::Key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));

        // 1w
        let res = reader.handle_event(&event);
        assert!(res.is_continue());
        assert_eq!(reader.cursor_column, 5);

        // 2w
        let res = reader.handle_event(&event);
        assert!(res.is_continue());
        assert_eq!(reader.cursor_column, 3);

        // 1w with combining character
        let res = reader.handle_event(&event);
        assert!(res.is_continue());
        assert_eq!(reader.cursor_column, 2);

        // 1 to prompt
        let res = reader.handle_event(&event);
        assert!(res.is_continue());
        assert_eq!(reader.cursor_column, 1);

        // at prompt
        let res = reader.handle_event(&event);
        assert!(res.is_continue());
        assert_eq!(reader.cursor_column, 1);
    }

    #[test]
    fn left_at_column_0_wraps_cursor_to_preceding_line() {
        let mut reader = LineReader {
            bg_buf: ":01234567🎸".to_owned(),
            bg_line_idx: vec![0, 10],
            prompt_width: 1,
            prompt_len: 1,
            display_columns: 10,
            display_lines: 5,
            first_display_line: 3,
            cursor_line: 4,
            cursor_column: 0,
            ..Default::default()
        };
        let event =
            Event::Key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        let res = reader.handle_event(&event);
        assert!(res.is_continue());
        assert_eq!((reader.cursor_column, reader.cursor_line), (9, 3));
    }

    #[test]
    fn left_wrapping_cursor_above_top_pans_buffer() {
        let lines = [
            ":1234567890123456789",
            "01234567890123456789",
            "01234567890123456789",
            "01234567890123456789",
            "01234567890123456789",
        ];

        let mut reader = LineReader {
            display_columns: 20,
            display_lines: 5,
            bg_buf: lines.join(""),
            bg_line_idx: vec![0, 20, 40, 60, 80, 100],
            prompt_len: 1,
            prompt_width: 1,
            cursor_column: 0,
            cursor_line: 1,
            first_display_line: 0,
            first_buffer_line: 4,
            ..Default::default()
        };

        let event =
            Event::Key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        let res = reader.handle_event(&event);
        assert!(res.is_continue());
        assert_eq!(reader.cursor_column, 19);
        assert_eq!(reader.cursor_line, 1);
        assert_eq!(reader.first_display_line, 0);
        assert_eq!(reader.first_buffer_line, 3);
        assert_eq!(reader.bg_line_idx, vec![0, 20, 40, 60, 80,]);
    }

    #[test]
    fn right_arrow_moves_to_next_base_char() {
        let buffer_text = "dëf";
        let mut reader =
            LineReader { ag_buf: buffer_text.to_owned(), ..Default::default() };
        let event =
            Event::Key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        let res = reader.handle_event(&event);
        assert!(res.is_continue());
        assert_eq!(reader.bg_buf, "d");
        assert_eq!(reader.ag_buf, "ëf");
        let res = reader.handle_event(&event);
        assert!(res.is_continue());
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
        assert!(res.is_continue());
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
        assert!(res.is_continue());
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
        assert!(res.is_continue());
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
        assert!(res.is_continue());
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
        assert!(res.is_continue());
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
        assert!(res.is_continue());
        assert!(reader.ag_buf.is_empty());
        assert_eq!(reader.bg_buf, "d");
    }

    #[test]
    fn delete_adjusts_display_end() {
        let mut reader = LineReader {
            ag_buf: "123456789012345678901".to_owned(),
            display_columns: 10,
            display_lines: 5,
            first_display_line: 3,
            cursor_line: 3,
            ag_display_chars: 20,
            ..Default::default()
        };
        let event =
            Event::Key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE));

        let res = reader.handle_event(&event);
        assert!(res.is_continue());
        assert_eq!(reader.ag_display_chars, 20);

        let res = reader.handle_event(&event);
        assert!(res.is_continue());
        assert_eq!(reader.ag_display_chars, 19);
    }

    #[test]
    fn viewport_bottom() {
        let mut reader = LineReader::new();
        reader.display_columns = 20;
        reader.display_lines = 24;
        reader.prompt_len = 1;
        reader.prompt_width = 1;
        reader.bg_buf.push_str(":1234567890");
        reader.cursor_column = 11;

        // ag_buf.is_empty()
        assert_eq!(reader.viewport_bottom(), 23);

        // ag_buf.width() < cursor line remainder
        reader.ag_buf.push_str("12345");
        assert_eq!(reader.viewport_bottom(), 23);

        // ag_buf.width() == cursor line remainder
        reader.ag_buf.push_str("67890");
        assert_eq!(reader.viewport_bottom(), 22);

        // ag_buf.width() > cursor line remainder
        reader.ag_buf.push_str("1234567890123456789012345");
        assert_eq!(reader.viewport_bottom(), 22);
    }

    #[test]
    fn display_remainder() {
        let mut reader = LineReader::new();
        reader.display_columns = 20;
        reader.display_lines = 24;
        reader.prompt_len = 1;
        reader.prompt_width = 1;
        reader.bg_buf.push_str(":1234567890");
        reader.cursor_column = 11;
        reader.first_display_line = 22;
        reader.cursor_line = 22;

        // ag_buf.is_empty()
        assert_eq!(reader.display_remainder(), 0);

        // ag_buf.width() < cursor line remainder
        reader.ag_buf.push_str("123🎸");
        assert_eq!(reader.display_remainder(), 4);

        // ag_buf.width() == cursor line remainder
        reader.ag_buf.push_str("678🎸");
        assert_eq!(reader.display_remainder(), 8);

        // ag_buf.width() > cursor line remainder
        reader.ag_buf.push_str("1234567890123456789🎸12345");
        assert_eq!(reader.display_remainder(), 26);
    }

    // Char insertion tests

    #[test]
    fn char_typed_non_0w_inserts() {
        let mut reader = LineReader::new();
        reader.bg_buf.push(':');
        reader.prompt_len = 1;
        reader.prompt_width = 1;
        reader.cursor_column = 1;
        let event =
            Event::Key(KeyEvent::new(KeyCode::Char('🎸'), KeyModifiers::NONE));
        let res = reader.handle_event(&event);
        assert!(res.is_continue());
        assert_eq!(&reader.bg_buf, ":🎸");
    }

    #[test]
    fn char_typed_0w_requires_base_char() {
        let mut reader = LineReader::new();
        reader.bg_buf.push(':');
        reader.prompt_len = 1;
        reader.prompt_width = 1;
        reader.cursor_column = 1;
        let event = Event::Key(KeyEvent::new(
            KeyCode::Char('\u{0308}'),
            KeyModifiers::NONE,
        ));
        let res = reader.handle_event(&event);
        assert!(res.is_continue());
        assert_eq!(&reader.bg_buf, ":");

        let mut reader = LineReader::new();
        reader.bg_buf.push_str(":e");
        reader.prompt_len = ".".len();
        reader.prompt_width = 1;
        reader.cursor_column = 2;
        let event = Event::Key(KeyEvent::new(
            KeyCode::Char('\u{0308}'),
            KeyModifiers::NONE,
        ));
        let res = reader.handle_event(&event);
        assert!(res.is_continue());
        assert_eq!(&reader.bg_buf, ":e\u{0308}");
    }

    #[test]
    fn char_typed_before_eol_moves_cursor_char_width() {
        let mut reader = LineReader::new();
        reader.bg_buf.push_str(":e");
        reader.prompt_len = ".".len();
        reader.prompt_width = 1;
        reader.cursor_column = 2;
        let event = Event::Key(KeyEvent::new(
            KeyCode::Char('\u{0308}'),
            KeyModifiers::NONE,
        ));
        let res = reader.handle_event(&event);
        assert!(res.is_continue());
        assert_eq!((reader.cursor_column, reader.cursor_line), (2, 23));

        let event =
            Event::Key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
        let res = reader.handle_event(&event);
        assert!(res.is_continue());
        assert_eq!(reader.cursor_column, 3);

        let event =
            Event::Key(KeyEvent::new(KeyCode::Char('🎸'), KeyModifiers::NONE));
        let res = reader.handle_event(&event);
        assert!(res.is_continue());
        assert_eq!(reader.cursor_column, 5);
    }

    #[test]
    fn char_typed_to_eol_before_bottom_wraps_cursor_to_column_0() {
        let mut reader = LineReader::new();
        reader.display_columns = 10;
        reader.prompt_len = 1;
        reader.prompt_width = 1;
        reader.bg_buf.push_str(":1234567");
        reader.cursor_column = 8;
        reader.cursor_line = 0;
        let event =
            Event::Key(KeyEvent::new(KeyCode::Char('🎸'), KeyModifiers::NONE));
        let res = reader.handle_event(&event);
        assert!(res.is_continue());
        assert_eq!((reader.cursor_column, reader.cursor_line), (0, 1));
        assert_eq!(reader.bg_line_idx, vec![0, 9]);
    }

    #[test]
    fn char_typed_past_eol_before_bottom_wraps_cursor_to_after_char() {
        let mut reader = LineReader::new();
        reader.display_columns = 10;
        reader.bg_buf.push_str(":12345678");
        reader.prompt_len = 1;
        reader.prompt_width = 1;
        reader.cursor_column = 9;
        reader.cursor_line = 0;
        let event =
            Event::Key(KeyEvent::new(KeyCode::Char('🎸'), KeyModifiers::NONE));
        let res = reader.handle_event(&event);
        assert!(res.is_continue());
        assert_eq!((reader.cursor_column, reader.cursor_line), (2, 1));
        assert_eq!(reader.bg_line_idx, vec![0, 9]);
    }

    #[test]
    fn char_typed_to_bottom_when_bg_fits_pans_display() {
        // ag_buf.is_empty(), so viewport == display
        let mut reader = LineReader::new();
        reader.display_columns = 20;
        reader.display_lines = 5;
        reader.bg_buf.push_str(":123456789012345678");
        reader.prompt_len = 1;
        reader.prompt_width = 1;
        reader.cursor_column = 19;
        reader.cursor_line = 4;
        reader.first_display_line = 4;
        let event =
            Event::Key(KeyEvent::new(KeyCode::Char('🎸'), KeyModifiers::NONE));
        let res = reader.handle_event(&event);
        assert!(res.is_continue());
        assert_eq!(reader.bg_line_idx, vec![0, 19]);
        assert_eq!((reader.cursor_column, reader.cursor_line), (2, 4));
        assert_eq!(reader.first_display_line, 3);
        assert_eq!(reader.scroll_needed, 1);

        // ag_buf.width() past bottom of display, so viewport == display - 1
        let mut reader = LineReader::new();
        reader.display_columns = 20;
        reader.display_lines = 5;
        reader.bg_buf.push_str(":123456789012345678");
        reader.prompt_len = 1;
        reader.prompt_width = 1;
        reader.cursor_column = 19;
        reader.cursor_line = 3;
        reader.first_display_line = 3;
        reader.ag_buf.push_str("123456789012345678901234567890123456789");
        let event =
            Event::Key(KeyEvent::new(KeyCode::Char('🎸'), KeyModifiers::NONE));
        let res = reader.handle_event(&event);
        assert!(res.is_continue());
        assert_eq!((reader.cursor_column, reader.cursor_line), (2, 3));
        assert_eq!(reader.first_display_line, 2);
        assert_eq!(reader.scroll_needed, 1);
    }

    #[test]
    fn char_typed_to_bottom_when_bg_overflows_pans_buffer() {
        // ag_buf.is_empty(), so viewport == display
        let lines = [
            ":1234567890123456789",
            "01234567890123456789",
            "01234567890123456789",
            "01234567890123456789",
            "01234567890123456789",
            "012345678901234567",
        ];

        let mut reader = LineReader::new();
        reader.display_columns = 20;
        reader.display_lines = 5;
        reader.bg_buf.push_str(&lines.join(""));
        reader.bg_line_idx = vec![0, 20, 40, 60, 80, 100];
        reader.prompt_len = 1;
        reader.prompt_width = 1;
        reader.cursor_column = 18;
        reader.cursor_line = 4;
        reader.first_display_line = 0;
        reader.first_buffer_line = 1;
        let event =
            Event::Key(KeyEvent::new(KeyCode::Char('🎸'), KeyModifiers::NONE));
        let res = reader.handle_event(&event);
        assert!(res.is_continue());
        assert_eq!(reader.bg_line_idx, vec![0, 20, 40, 60, 80, 100, 119]);
        assert_eq!((reader.cursor_column, reader.cursor_line), (0, 4));
        assert_eq!(reader.first_display_line, 0);
        assert_eq!(reader.first_buffer_line, 2);
        assert_eq!(reader.scroll_needed, 0);

        // ag_buf.width() past bottom of display, so viewport == display - 1
        let mut reader = LineReader::new();
        reader.display_columns = 20;
        reader.display_lines = 5;
        reader.bg_buf.push_str(&lines.join(""));
        reader.bg_line_idx = vec![0, 20, 40, 60, 80, 100];
        reader.prompt_len = 1;
        reader.prompt_width = 1;
        reader.cursor_column = 18;
        reader.cursor_line = 3;
        reader.first_display_line = 0;
        reader.first_buffer_line = 2;
        reader.ag_buf.push_str("123456789012345678901234567890123456789");
        let event =
            Event::Key(KeyEvent::new(KeyCode::Char('🎸'), KeyModifiers::NONE));
        let res = reader.handle_event(&event);
        assert!(res.is_continue());
        assert_eq!(reader.bg_line_idx, vec![0, 20, 40, 60, 80, 100, 119]);
        assert_eq!((reader.cursor_column, reader.cursor_line), (0, 3));
        assert_eq!(reader.first_display_line, 0);
        assert_eq!(reader.first_buffer_line, 3);
        assert_eq!(reader.scroll_needed, 0);
    }

    #[test]
    fn char_typed_when_ag_overflows_stops_at_display_remainder() {
        let lines = [
            ":1234567890123456789",
            "01234567890123456789",
            "01234567890123456789",
            "01234567890123456789",
            "01234567890123456789",
            "012345678901234568",
        ];

        let mut reader = LineReader::new();
        reader.display_columns = 20;
        reader.display_lines = 5;
        reader.bg_buf.push_str(&lines.join(""));
        reader.bg_line_idx = vec![0, 20, 40, 60, 80, 100];
        reader.prompt_len = 1;
        reader.prompt_width = 1;
        reader.cursor_column = 19;
        reader.cursor_line = 3;
        reader.first_display_line = 0;
        reader.first_buffer_line = 1;
        reader.ag_buf.push_str("123456789012345678901234567890123456789");
        let event =
            Event::Key(KeyEvent::new(KeyCode::Char('🎸'), KeyModifiers::NONE));
        let res = reader.handle_event(&event);
        assert!(res.is_continue());
        assert_eq!(reader.ag_display_chars, 38);
    }
}
