use std::io::{self, BufRead, Write};
use std::ops::{ControlFlow, RangeBounds};

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

#[derive(Debug, Clone, PartialEq)]
pub struct LineReader {
    buffer: Vec<BufferLine>,
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

impl BufferIndex {
    pub(crate) fn new() -> BufferIndex {
        BufferIndex { ..Default::default() }
    }
}

impl From<(usize, usize)> for BufferIndex {
    fn from((line, offset): (usize, usize)) -> BufferIndex {
        BufferIndex { line, offset }
    }
}

impl Cursor {
    pub(crate) fn new() -> Cursor {
        Cursor { ..Default::default() }
    }
}

// impls for LineReader
////////

impl Default for LineReader {
    fn default() -> LineReader {
        LineReader {
            buffer: vec![BufferLine { text: String::new(), width: 0 }],
            input_start: BufferIndex { ..Default::default() },
            display_width: 0,
            display_height: 0,
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
    pub(crate) fn builder() -> LineReaderBuilder {
        LineReaderBuilder::new()
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
        self.cursor = Cursor {
            column: prompt_line.width,
            line: 0,
            index: BufferIndex { line: 0, offset: prompt_line.text.len() },
        };
        self.buffer.splice(.., [prompt_line]);
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
        let mut should_continue = true;
        while should_continue {
            let event = event::read()?;
            should_continue = self.handle_event(&event).is_continue();
            self.repaint()?;
        }

        self.handle_end();
        self.repaint()?;
        let mut stdout = io::stdout().lock();
        stdout.queue(MoveToNextLine(1))?;
        stdout.flush()?;

        let prev_bytes = output_buffer.len();
        output_buffer.extend(
            self.buffer
                .iter()
                .flat_map(|l| l.text.chars())
                .skip(prompt.chars().count()),
        );
        Ok(output_buffer.len() - prev_bytes)
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
                self.buffer.last_mut().unwrap().text.push_str(native_eol());
                ControlFlow::Break(())
            }
            KeyCode::Left => self.handle_left(),
            KeyCode::Right => self.handle_right(),
            KeyCode::Home => self.handle_home(),
            KeyCode::End => self.handle_end(),
            KeyCode::Backspace => self.handle_backspace(),
            KeyCode::Delete => self.handle_delete(),
            KeyCode::Char(c) => self.handle_char_input(c),
            KeyCode::Up => {
                todo!("move to next older entry in history");
            }
            KeyCode::Down => {
                todo!("move to next newer entry in history");
            }
            _ => ControlFlow::Continue(()),
        }
    }

    fn handle_char_input(&mut self, c: char) -> ControlFlow<()> {
        let c_width = c.width().unwrap_or(0);

        if c_width == 0 && self.cursor.index == self.input_start {
            return ControlFlow::Continue(());
        }

        if self.cursor.index.line > 0 {
            let p_line = &mut self.buffer[self.cursor.index.line - 1];
            if self.display_width - p_line.width <= c_width {
                p_line.text.push(c);
                p_line.width += c_width;
                return ControlFlow::Continue(());
            }
        }

        let line = &mut self.buffer[self.cursor.index.line];
        line.text.insert(self.cursor.index.offset, c);
        line.width += c_width;
        self.cursor.index.offset += c.len_utf8();
        self.cursor.column += c_width;
        self.reflow(self.cursor.index.line);

        ControlFlow::Continue(())
    }

    fn handle_backspace(&mut self) -> ControlFlow<()> {
        ControlFlow::Continue(())
    }

    fn handle_left(&mut self) -> ControlFlow<()> {
        ControlFlow::Continue(())
    }

    fn handle_right(&mut self) -> ControlFlow<()> {
        ControlFlow::Continue(())
    }

    fn handle_delete(&mut self) -> ControlFlow<()> {
        ControlFlow::Continue(())
    }

    fn handle_home(&mut self) -> ControlFlow<()> {
        ControlFlow::Continue(())
    }

    fn handle_end(&mut self) -> ControlFlow<()> {
        ControlFlow::Continue(())
    }

    /// Compute last line of viewport
    fn viewport_bottom(&self) -> usize {
        if self.cursor.index.line <= self.buffer.len() {
            self.display_height - 2
        } else {
            self.display_height - 1
        }
    }

    /// Compute first line of viewport
    fn viewport_top(&self) -> usize {
        (self.first_display_line > 0).into()
    }

    /// Reflow buffer lines to fit `display_width`, and
    /// snap cursor location to within viewport.
    /// Also might result in setting scroll needed.
    fn reflow(&mut self, start: usize) {
        let mut lines = self.buffer.iter_mut().enumerate();
        let mut this_line = lines.nth(start);
        let mut next_line = lines.next();
        let mut residue = None;
        let mut cols = 0;
        while let Some((tl_idx, ref mut line)) = this_line {
            if line.width == self.display_width {
                cols = line.width;
                this_line = next_line;
                next_line = lines.next();
                continue;
            }
            eprintln!("need to find chars that fit");

            let mut cs = line.text.char_indices().peekable();

            while let Some(&(i, c)) = cs.peek() {
                let c_width = c.width().unwrap_or(0);
                if self.display_width - cols < c_width {
                    residue = Some(i);
                    break;
                }
                cols += c_width;
                cs.next();
            }

            if let Some((nl_idx, ref mut nl)) = next_line {
                if let Some(res_idx) = residue.take() {
                    // this_line too wide
                    // move residue to next_line start
                    nl.width += &line.text[res_idx..].width();
                    nl.text.insert_str(0, line.text.drain(res_idx..).as_str());
                    if tl_idx == self.cursor.index.line
                        && self.cursor.index.offset >= res_idx
                    {
                        // moved chars at cursor to next_line
                        // need to adjust cursor
                        self.cursor.index.line += 1;
                        self.cursor.index.offset -= res_idx;
                        self.cursor.line += 1;
                        self.cursor.column =
                            nl.text[..self.cursor.index.offset].width();
                    }
                } else if cols < self.display_width {
                    // this_line has room
                    // try to fill from next_line start
                    let mut cols_moved = 0;
                    let i = nl
                        .text
                        .chars()
                        .take_while(|c| {
                            let c_width = c.width().unwrap_or(0);
                            if self.display_width - cols < c_width {
                                false
                            } else {
                                cols_moved += c_width;
                                true
                            }
                        })
                        .count();
                    if i > 0 {
                        if nl_idx == self.cursor.index.line {
                            // moved chars from cursor line
                            // need to adjust cursor
                            if self.cursor.index.offset >= i {
                                // chars at cursor not moved
                                self.cursor.index.offset -= i;
                                self.cursor.column -= cols_moved;
                            } else {
                                self.cursor.line -= 1;
                                self.cursor.column = cols;
                                self.cursor.index = (tl_idx, cols).into();
                            }
                        }
                        line.text.extend(nl.text.drain(..i));
                        cols += cols_moved;
                        nl.width -= cols_moved;
                    }
                }

                line.width = cols;

                this_line = next_line;
                next_line = lines.next();
            } else {
                line.width = cols;
                break;
            }
        }

        // handle overflow at end of buffer
        if cols >= self.display_width {
            let (text, width) = if let Some(res_idx) = residue {
                let text: String = self
                    .buffer
                    .last_mut()
                    .unwrap()
                    .text
                    .drain(res_idx..)
                    .collect();
                if self.cursor.index.line == self.buffer.len() - 1
                    && self.cursor.index.offset >= res_idx
                {
                    // chars at cursor moving to new last line
                    self.cursor.index.line += 1;
                    self.cursor.index.offset -= res_idx;
                    self.cursor.line += 1;
                    self.cursor.column =
                        text[..self.cursor.index.offset].width();
                }
                let width = text.width();
                (text, width)
            } else {
                // no residue; add line & move cursor
                self.cursor.line += 1;
                self.cursor.column = 0;
                self.cursor.index = (self.cursor.index.line + 1, 0).into();
                (String::new(), 0)
            };
            self.buffer.push(BufferLine { text, width });
            if width > self.display_width {
                self.reflow(self.buffer.len() - 1);
            }
        }
    }

    #[cfg(not(tarpaulin_include))]
    /// render current buffer to display
    fn repaint(&mut self) -> io::Result<()> {
        let display_lines = self.display_height - self.first_display_line;
        let buffer_limit = self.buffer.len().min(display_lines);

        let mut stdout = io::stdout().lock();

        stdout.queue(Hide)?;

        if self.scroll_needed > 0 {
            let scroll_needed = u16::try_from(self.scroll_needed)
                .expect("scroll needed fits in u16");
            stdout.queue(ScrollUp(scroll_needed))?;
            self.scroll_needed = 0;
        }
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

        for line in &self.buffer[self.first_buffer_line..buffer_limit] {
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

#[derive(Debug)]
pub struct LineReaderBuilder {
    text: Option<Vec<String>>,
    cursor: Cursor,
    display_width: usize,
    display_height: usize,
    first_display_line: usize,
    first_buffer_line: usize,
    input_start: BufferIndex,
}

impl LineReaderBuilder {
    fn new() -> Self {
        LineReaderBuilder {
            text: None,
            display_width: 10,
            display_height: 5,
            first_display_line: 0,
            first_buffer_line: 0,
            cursor: Cursor { ..Default::default() },
            input_start: BufferIndex { line: 0, offset: 0 },
        }
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

    fn display_height(&mut self, h: usize) -> &mut Self {
        self.display_height = h;
        self
    }

    fn display_width(&mut self, w: usize) -> &mut Self {
        self.display_width = w;
        self
    }

    fn cursor(&mut self, c: Cursor) -> &mut Self {
        self.cursor = c;
        self
    }

    fn text<S>(&mut self, t: &[S]) -> &mut Self
    where
        S: AsRef<str>,
    {
        self.text =
            Some(t.as_ref().iter().map(|s| s.as_ref().to_owned()).collect());
        self
    }

    fn build(&self) -> LineReader {
        let buffer = self.text.as_ref().map_or_else(
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

        for l in &buffer {
            assert!(l.width <= self.display_width,);
        }
        if buffer.is_empty() {
            assert_eq!(self.input_start, BufferIndex { line: 0, offset: 0 });
            assert_eq!(self.cursor.index, BufferIndex { line: 0, offset: 0 });
        } else {
            assert!(
                (self.cursor.index.line == buffer.len()
                    && self.cursor.index.offset == 0)
                    || (self.cursor.index.line < buffer.len()
                        && self.cursor.index.offset
                            <= buffer[self.cursor.index.line].len())
            );
            assert!(
                (self.input_start.line == buffer.len()
                    && self.input_start.offset == 0)
                    || (self.input_start.line < buffer.len()
                        && self.input_start.offset
                            <= buffer[self.input_start.line].len())
            );
        }
        assert!(self.cursor.column < self.display_width);
        assert!(self.cursor.line < self.display_height);
        assert!(self.first_buffer_line <= buffer.len());

        LineReader {
            buffer,
            input_start: self.input_start,
            display_width: self.display_width,
            display_height: self.display_height,
            cursor: self.cursor,
            first_display_line: self.first_display_line,
            first_buffer_line: self.first_buffer_line,
            scroll_needed: 0,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unicode_not_nfc)]
mod tests {
    use super::*;

    use crossterm::event::KeyModifiers;

    #[test]
    fn builder_base_case() {
        let b = LineReaderBuilder::new();
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
        let mut b = LineReaderBuilder::new();
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

        let mut b = LineReaderBuilder::new();
        b.text(&[
            ":123456789abcde",
            "🎸23456789abcdef",
            "🎸23456789abcdef",
            "🎸23456789abcdef",
            "🎸23456789abcdef",
            "🎸23456789abcdef",
            "012345",
        ]);
        b.display_width(16).display_height(6);
        b.input_start(BufferIndex { line: 2, offset: 6 }).cursor(Cursor {
            line: 5,
            column: 6,
            index: BufferIndex { line: 6, offset: 6 },
        });
        b.first_display_line(0).first_buffer_line(1);
        let r = b.build();
        assert_eq!(r, expected);
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
        assert!(matches!(res, ControlFlow::Break(())));
    }

    #[test]
    fn char_input_non_0w_inserts() {
        let mut b = LineReaderBuilder::new();
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
        let mut b = LineReaderBuilder::new();
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
        let mut b = LineReader::builder();
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
        let mut b = LineReader::builder();
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
        let mut b = LineReader::builder();
        b.text(&[":12345678", "🎸abc"])
            .input_start((0, 1).into())
            .cursor(Cursor { column: 0, line: 4, index: (1, 0).into() })
            .display_width(10)
            .display_height(5)
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
    fn char_input_cursor_follows_character_if_wrapped() {
        todo!();
    }

    #[test]
    fn char_input_past_eol_wraps_input_to_next_line_start() {
        todo!();
    }

    #[test]
    fn char_input_cursor_bound_to_viewport() {
        todo!();
    }
}
