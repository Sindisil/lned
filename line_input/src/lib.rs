use std::cmp::Ordering;
use std::fmt;
use std::io::{self, Stdout};

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug)]
pub struct LineInput {
    input: GapBuffer,
}

#[derive(Debug)]
pub enum Response {
    Accept(usize), // contained usize bytes of input copied to buffer
    Cancel,        // Input was canceled, no input written to buffer
}

#[derive(Debug)]
struct GapBuffer {
    before_gap: String,
    after_gap: String,
    position: usize,
}

#[derive(Debug)]
struct RenderContext<'a> {
    prompt: &'a str,
    stdout: &'a mut Stdout,
}

impl Default for LineInput {
    fn default() -> Self {
        Self::new()
    }
}

impl LineInput {
    #[must_use]
    pub fn new() -> LineInput {
        LineInput {
            input: GapBuffer::new(),
        }
    }

    pub fn read_line(&mut self, _buf: &mut String, _prompt: &str) -> io::Result<Response> {
        todo!();
        // clear gap buffer
        // init render_ctx
        // set raw mode
        // display prompt
        // loop handling events until handle_event() returns a Reponse
        // disable raw mode
        // move cursor to next line after input, in column 0
        // copy input buffer into output buffer
        // return Response
    }

    fn handle_event(
        &mut self,
        render_ctx: &mut RenderContext<'_>,
        event: Event,
    ) -> io::Result<Option<Response>> {
        match event {
            Event::Key(event) => self.handle_key_event(render_ctx, event),
            _ => Ok(None),
        }
    }

    fn handle_key_event(
        &mut self,
        _render_ctx: &mut RenderContext<'_>,
        event: KeyEvent,
    ) -> io::Result<Option<Response>> {
        match event.code {
            KeyCode::Char('d') if event.modifiers == KeyModifiers::CONTROL => {
                Ok(Some(Response::Cancel))
            }
            KeyCode::Enter => Ok(Some(Response::Accept(self.input.len()))),
            _ => Ok(None),
        }
    }
}

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
            position: 0,
        }
    }

    fn len(&self) -> usize {
        self.before_gap.len() + self.after_gap.len()
    }

    fn gap_to_position(&mut self) {
        match self.position.cmp(&self.before_gap.len()) {
            Ordering::Less => {
                self.after_gap
                    .insert_str(0, &self.before_gap[self.position..]);
                self.before_gap.drain(self.position..);
            }
            Ordering::Greater => {
                let to_move = self.position - self.before_gap.len();
                self.before_gap.push_str(&self.after_gap[..to_move]);
                self.after_gap.drain(..to_move);
            }
            Ordering::Equal => (),
        }
    }
}

impl<'a> RenderContext<'a> {
    fn new(prompt: &'a str, stdout: &'a mut Stdout) -> RenderContext<'a> {
        RenderContext { prompt, stdout }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gap_buffer_new_creates_empty_buffer() {
        let buf = GapBuffer::new();
        assert_eq!(buf.to_string(), "");
    }

    #[test]
    fn gap_buffer_converts_to_string() {
        let text = "Text before; text after".to_owned();
        let position = 12usize;
        let buffer = GapBuffer {
            before_gap: text[..position].to_owned(),
            after_gap: text[position..].to_owned(),
            position,
        };
        assert_eq!(buffer.to_string(), text);
    }

    #[test]
    fn gap_to_position_moves_position_to_end_of_before_gap() {
        // init buffer as if text was just typed,
        // so position is at end of before_gap.
        let mut buf = GapBuffer {
            before_gap: "Text in buffer".to_owned(),
            after_gap: String::new(),
            position: 14,
        };

        // simulate moving cursor to space after 'in' (pos 7)
        buf.position = 7;
        buf.gap_to_position();
        assert_eq!(buf.before_gap, "Text in");
        assert_eq!(buf.after_gap, " buffer");

        // move cursor to first letter in "buffer" (pos: 8)
        buf.position = 8;
        buf.gap_to_position();
        assert_eq!(buf.before_gap, "Text in ");
        assert_eq!(buf.after_gap, "buffer");
    }

    #[test]
    fn handle_event_ctrl_d_returns_canceled() {
        let mut input = LineInput::new();
        let mut stdout = io::stdout();
        let prompt = "";
        let mut render_ctx = RenderContext::new(&prompt, &mut stdout);
        let event = Event::Key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL));
        let res = input.handle_event(&mut render_ctx, event).unwrap();
        assert!(matches!(res, Some(Response::Cancel)));
    }

    #[test]
    fn handle_event_enter_returns_accept() {
        let expected = "This is some text.".to_owned();
        let mut input = LineInput {
            input: GapBuffer {
                before_gap: expected[..8].to_owned(),
                after_gap: expected[8..].to_owned(),
                position: 8,
            },
        };
        let mut stdout = io::stdout();
        let prompt = "";
        let mut render_ctx = RenderContext::new(&prompt, &mut stdout);
        let event = Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        let res = input.handle_event(&mut render_ctx, event).unwrap();
        assert!(matches!(res, Some(Response::Accept(_))));
        if let Some(Response::Accept(bytes)) = res {
            assert_eq!(bytes, expected.len());
        }
    }
}
