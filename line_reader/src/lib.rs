use std::fmt;
use std::io::{self, BufRead, Stdout, Write};

use crossterm::cursor::{
    self, Hide, MoveTo, MoveToNextLine, RestorePosition, SavePosition, Show,
};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::terminal::{self, Clear, ClearType, ScrollUp};
use crossterm::{ExecutableCommand, QueueableCommand};
use unicode_width::UnicodeWidthStr;

// Public structs, enums, and traits
///////////

pub trait LineRead {
    /// # Errors
    ///
    /// Will return `io::Error` if an error is encountered reading a line
    fn read_line(
        &mut self,
        buffer: &mut String,
        prompt: &str,
    ) -> io::Result<usize>;

    /// # Errors
    ///
    /// Will return `io::Error` if an error is encountered reading a line

    fn read_line_or_cancel(
        &mut self,
        buffer: &mut String,
        prompt: &str,
    ) -> io::Result<Option<usize>> {
        self.read_line(buffer, prompt).map_or(Ok(None), |bytes| Ok(Some(bytes)))
    }
}

#[derive(Debug, Default)]
pub struct LineReader {
    buffer: GapBuffer,
}

// Private structs and enums
////////

#[derive(Debug, Default)]
struct GapBuffer {
    before_gap: String,
    after_gap: String,
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
        LineReader { buffer: GapBuffer::new() }
    }

    #[cfg(not(tarpaulin_include))]
    fn accept_line(
        &mut self,
        output_buffer: &mut String,
        prompt: &str,
        cancelable: bool,
    ) -> io::Result<Option<usize>> {
        // clear gap buffer
        self.buffer.clear();

        // init render_ctx
        let mut stdout = io::stdout();
        let mut render_ctx = RenderContext::new(prompt, &mut stdout);
        render_ctx.initialize()?;

        loop {
            render_ctx.repaint(&self.buffer)?;

            // get next event
            let event = event::read()?;

            // handle event
            let response = self.handle_event(&event);

            match response {
                Response::Accept(bytes_read) => {
                    render_ctx.move_to_end(&mut self.buffer)?;
                    output_buffer.extend(self.buffer.before_gap.drain(..));
                    output_buffer.extend(self.buffer.after_gap.drain(..));
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

    fn handle_event(&mut self, event: &Event) -> Response {
        match event {
            Event::Key(event) => self.handle_key_event(event),
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
                todo!("move cursor left until previous base char");
            }
            KeyCode::Right => {
                todo!("move cursor right until next base char");
            }
            KeyCode::Backspace => {
                todo!("remove code point to left of cursor, but cursor doesn't move unless iit has width > 0");
            }
            KeyCode::Delete => {
                todo!("remove base char at cursor, along with any zero width code points to its right up until next base char");
            }
            KeyCode::Char(c) => {
                self.buffer.before_gap.push(c);
                Response::Continue
            }
            _ => Response::Continue,
        }
    }
}

impl LineRead for LineReader {
    #[cfg(not(tarpaulin_include))]
    fn read_line(
        &mut self,
        buffer: &mut String,
        prompt: &str,
    ) -> io::Result<usize> {
        Ok(self.accept_line(buffer, prompt, false)?.unwrap_or(0))
    }

    fn read_line_or_cancel(
        &mut self,
        buffer: &mut String,
        prompt: &str,
    ) -> io::Result<Option<usize>> {
        self.accept_line(buffer, prompt, true)
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
}

// impls for RenderContext
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

    #[cfg(not(tarpaulin_include))]
    fn initialize(&mut self) -> io::Result<()> {
        self.terminal_size = terminal::size()?;
        let cursor_pos = cursor::position()?;
        self.prompt_line = cursor_pos.1;
        terminal::enable_raw_mode()
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

    #[cfg(not(tarpaulin_include))]
    fn move_to_end(&mut self, buffer: &mut GapBuffer) -> io::Result<()> {
        let (mut cur_col, mut cur_line) = cursor::position()?;
        let after_gap_width = buffer.after_gap.width();
        let term_height = self.terminal_height() as usize;
        let last_line = (after_gap_width / self.terminal_width() as usize)
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
                self.stdout.queue(ScrollUp(scroll_needed))?;
                cur_line -= scroll_needed;
            }
            self.stdout
                .queue(MoveTo(cur_col, cur_line))?
                .queue(Clear(ClearType::FromCursorDown))?;
            let offset = after_gap_width.saturating_sub(
                ((self.terminal_height() - 1) * self.terminal_width()) as usize,
            );
            write!(self.stdout, "{}", &buffer.after_gap[offset..])?;
        }
        self.stdout.queue(MoveToNextLine(1))?;
        self.stdout.flush()
    }

    #[cfg(not(tarpaulin_include))]
    fn repaint(&mut self, buffer: &GapBuffer) -> io::Result<()> {
        self.stdout.queue(Hide)?;

        // calculate how many lines we need
        let column_estimate = self.prompt.width()
            + buffer.before_gap.width()
            + buffer.after_gap.width();
        let width = usize::from(self.terminal_width());
        let lines_to_print = (width + column_estimate) / width;

        // if necessary, scroll to make room (nb: manual scroll because of bugs)
        let required_lines = u16::try_from(
            lines_to_print.saturating_sub(self.lines_available().into()),
        )
        .unwrap_or(self.terminal_height());
        if required_lines > 0 {
            self.scroll(required_lines)?;
            self.prompt_line = self.prompt_line.saturating_sub(required_lines);
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

        self.previous_required_lines = required_lines;

        self.stdout.queue(RestorePosition)?;

        self.stdout.queue(Show)?;

        // Make it so
        self.stdout.flush()
    }

    /// Scroll the terminal up by the specified number of lines.
    /// Using this instead of crossterm scroll command because
    /// of a bug in terminal scrollback.
    /// see <https://github.com/nushell/nushell/issues/9166>
    #[cfg(not(tarpaulin_include))]
    fn scroll(&mut self, lines: u16) -> io::Result<()> {
        self.stdout.queue(MoveTo(0, self.terminal_height() - 1))?;
        for _ in 0..lines {
            write!(self.stdout, "\r\n")?;
        }
        Ok(())
    }
}

impl Drop for RenderContext<'_> {
    #[cfg(not(tarpaulin_include))]
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
        let _ = self.stdout.execute(Show);
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
        buffer: &mut String,
        _prompt: &str,
    ) -> io::Result<usize> {
        BufRead::read_line(self, buffer)
    }
}

#[cfg(test)]
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
}
