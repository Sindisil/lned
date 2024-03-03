use std::cmp::Ordering;
use std::fmt;
use std::io::{self, BufRead, Stdout, Write};

use crossterm::cursor::{
    Hide, MoveTo, MoveToNextLine, RestorePosition, SavePosition, Show,
};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::terminal::{self, Clear, ClearType};
use crossterm::{ExecutableCommand, QueueableCommand};
use unicode_width::UnicodeWidthStr;

//// Public structs, enums, and traits
////////

pub trait LineRead {
    fn read_line(&mut self, buffer: &mut String) -> io::Result<usize>;

    fn read_line_or_cancel(
        &mut self,
        buffer: &mut String,
    ) -> io::Result<Option<usize>> {
        self.read_line(buffer).map_or(Ok(None), |bytes| Ok(Some(bytes)))
    }
}

#[derive(Debug)]
pub struct LineInput {
    input: GapBuffer,
}

//// Private structs and enums
////////

#[derive(Debug)]
struct GapBuffer {
    before_gap: String,
    after_gap: String,
    cursor: usize,
}

#[derive(Debug)]
struct RenderContext<'a> {
    prompt: &'a str,
    stdout: &'a mut Stdout,
    prompt_line: u16,
    terminal_size: (u16, u16),
    previous_required_lines: u16,
}

#[derive(Debug)]
enum Response {
    Accept(usize),
    Cancel,
    Continue,
}

//// impls for LineInput
////////

impl Default for LineInput {
    fn default() -> Self {
        Self::new()
    }
}

impl LineInput {
    #[must_use]
    pub fn new() -> LineInput {
        LineInput { input: GapBuffer::new() }
    }

    pub fn read(
        &mut self,
        buffer: &mut String,
        prompt: &str,
    ) -> io::Result<usize> {
        self.read_line(buffer, prompt, false).map(|r| {
            r.expect("shouldn't receive cancel response when not enabled")
        })
    }

    pub fn read_or_cancel(
        &mut self,
        buffer: &mut String,
        prompt: &str,
    ) -> io::Result<Option<usize>> {
        self.read_line(buffer, prompt, true)
    }

    fn read_line(
        &mut self,
        buffer: &mut String,
        prompt: &str,
        cancelable: bool,
    ) -> io::Result<Option<usize>> {
        // clear gap buffer
        self.input.clear();
        // init render_ctx
        let mut stdout = io::stdout();
        let mut render_ctx = RenderContext::new(prompt, &mut stdout);

        // loop handling events until handle_event() returns a Reponse
        loop {
            render_ctx.repaint(&self.input)?;

            // get next event
            let event = event::read()?;

            // handle event
            let response = self.handle_event(buffer, event)?;

            match response {
                Response::Accept(bytes_read) => {
                    render_ctx.stdout.execute(MoveToNextLine(1))?;
                    return Ok(Some(bytes_read));
                }
                Response::Cancel => {
                    if cancelable {
                        render_ctx.stdout.execute(MoveToNextLine(1))?;
                        return Ok(None);
                    }
                }
                Response::Continue => (),
            }
        }
    }

    fn handle_event(
        &mut self,
        buffer: &mut String,
        event: Event,
    ) -> io::Result<Response> {
        match event {
            Event::Key(event) => self.handle_key_event(buffer, event),
            _ => Ok(Response::Continue),
        }
    }

    fn handle_key_event(
        &mut self,
        buffer: &mut String,
        event: KeyEvent,
    ) -> io::Result<Response> {
        match event.code {
            KeyCode::Char('d') if event.modifiers == KeyModifiers::CONTROL => {
                Ok(Response::Cancel)
            }
            KeyCode::Enter => {
                let bytes_read = self.input.len();
                buffer.extend(self.input.before_gap.drain(..));
                buffer.extend(self.input.after_gap.drain(..));
                self.input.cursor = 0;
                Ok(Response::Accept(bytes_read))
            }
            _ => Ok(Response::Continue),
        }
    }
}

//// impls for GapBuffer
////////

impl Default for GapBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for GapBuffer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}", self.before_gap, self.after_gap)
    }
}

impl GapBuffer {
    fn new() -> GapBuffer {
        GapBuffer {
            before_gap: String::new(),
            after_gap: String::new(),
            cursor: 0,
        }
    }

    fn len(&self) -> usize {
        self.before_gap.len() + self.after_gap.len()
    }

    fn clear(&mut self) {
        self.before_gap.clear();
        self.after_gap.clear();
        self.cursor = 0;
    }

    fn gap_to_cursor(&mut self) {
        match self.cursor.cmp(&self.before_gap.len()) {
            Ordering::Less => {
                self.after_gap.insert_str(0, &self.before_gap[self.cursor..]);
                self.before_gap.drain(self.cursor..);
            }
            Ordering::Greater => {
                let to_move = self.cursor - self.before_gap.len();
                self.before_gap.push_str(&self.after_gap[..to_move]);
                self.after_gap.drain(..to_move);
            }
            Ordering::Equal => (),
        }
    }
}

//// impls for RenderContext
////////

impl<'a> RenderContext<'a> {
    fn new(prompt: &'a str, stdout: &'a mut Stdout) -> RenderContext<'a> {
        RenderContext {
            prompt,
            stdout,
            prompt_line: 0,
            terminal_size: (0, 0),
            previous_required_lines: 0,
        }
    }

    /// Returns terminal width in columns
    fn terminal_width(&self) -> u16 {
        self.terminal_size.0
    }

    /// Returns terminal height in rows
    fn terminal_height(&self) -> u16 {
        self.terminal_size.1
    }

    /// Returns lines from prompt to bottom of terminal
    fn lines_available(&self) -> u16 {
        self.terminal_height().saturating_sub(self.prompt_line)
    }

    fn repaint(&mut self, buffer: &GapBuffer) -> io::Result<()> {
        self.stdout.queue(Hide)?;

        // calculate how many lines we need
        let column_estimate = self.prompt.width()
            + buffer.before_gap.width()
            + buffer.after_gap.width();
        let width = usize::from(self.terminal_width());
        let lines_to_print = ((width + column_estimate) / width) as u16;

        // if necessary, scroll to make room (nb: manual scroll because of bugs)
        let lines_needed =
            lines_to_print.saturating_sub(self.lines_available().into());
        if lines_needed > 0 {
            self.scroll(lines_needed)?;
            self.prompt_line = self.prompt_line.saturating_sub(lines_needed);
        }

        // move cursor to start of prompt & clear to
        // make room for printing the prompt & buffer
        self.stdout
            .queue(MoveTo(0, self.prompt_line))?
            .queue(Clear(ClearType::FromCursorDown))?;

        // print prompt
        write!(self.stdout, "{}", self.prompt)?;

        // print before_gap
        write!(self.stdout, "{}", buffer.before_gap)?;

        self.stdout.queue(SavePosition)?;

        // print after_gap
        write!(self.stdout, "{}", buffer.after_gap)?;

        self.stdout.queue(RestorePosition)?;

        self.stdout.queue(Show)?;

        // Make it so
        self.stdout.flush()
    }

    /// Scroll the terminal up by the specified number of lines.
    /// Using this instead of crossterm scroll command because
    /// of a bug in terminal scrollback.
    /// see https://github.com/nushell/nushell/issues/9166
    fn scroll(&mut self, lines: u16) -> io::Result<()> {
        self.stdout.queue(MoveTo(0, self.terminal_height() - 1))?;
        for _ in 0..lines {
            write!(self.stdout, "\r\n")?;
        }
        Ok(())
    }
}

impl Drop for RenderContext<'_> {
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
        let _ = self.stdout.execute(Show);
    }
}

//// impls for LineRead
////////

impl<T> LineRead for T
where
    T: BufRead,
{
    fn read_line(&mut self, buffer: &mut String) -> io::Result<usize> {
        BufRead::read_line(self, buffer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    //// tests for GapBuffer
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
            cursor,
        };
        assert_eq!(buffer.to_string(), text);
    }

    #[test]
    fn gap_to_cursor_moves_cursor_to_end_of_before_gap() {
        // init buffer as if text was just typed,
        // so cursor is at end of before_gap.
        let mut buf = GapBuffer {
            before_gap: "Text in buffer".to_owned(),
            after_gap: String::new(),
            cursor: 14,
        };

        // simulate moving cursor to space after 'in' (pos 7)
        buf.cursor = 7;
        buf.gap_to_cursor();
        assert_eq!(buf.before_gap, "Text in");
        assert_eq!(buf.after_gap, " buffer");

        // move cursor to first letter in "buffer" (pos: 8)
        buf.cursor = 8;
        buf.gap_to_cursor();
        assert_eq!(buf.before_gap, "Text in ");
        assert_eq!(buf.after_gap, "buffer");
    }

    //// tests for LineInput
    ////////

    #[test]
    fn handle_event_ctrl_d_returns_canceled() {
        let mut input = LineInput::new();
        let mut buffer = String::new();
        let event = Event::Key(KeyEvent::new(
            KeyCode::Char('d'),
            KeyModifiers::CONTROL,
        ));
        let res = input.handle_event(&mut buffer, event).unwrap();
        assert!(matches!(res, Response::Cancel));
        assert!(buffer.is_empty());
    }

    #[test]
    fn handle_event_enter_returns_accept() {
        let expected = "This is some text.".to_owned();
        let mut input = LineInput {
            input: GapBuffer {
                before_gap: expected[..8].to_owned(),
                after_gap: expected[8..].to_owned(),
                cursor: 8,
            },
        };
        let mut buffer = String::new();
        let event =
            Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        let res = input.handle_event(&mut buffer, event).unwrap();
        assert!(
            matches!(res, Response::Accept(bytes) if bytes == expected.len())
        );
        assert_eq!(buffer, expected);
    }
}
