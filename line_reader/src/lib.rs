mod edit_buffer;
mod history_stack;
mod renderer;

use std::io::{self, BufRead, Write};
use std::time::Duration;

use crossterm::ExecutableCommand;
use crossterm::cursor::{self, Show};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
use crossterm::terminal;

use crate::edit_buffer::BufferLine;
use crate::edit_buffer::EditBuffer;
use crate::history_stack::HistoryStack;
use crate::renderer::Renderer;

pub trait LineRead {
    /// # Errors
    ///
    /// Will return `io::Error` if an error is encountered reading a line
    fn read_line(
        &mut self,
        prompt: Option<char>,
        buffer: &mut String,
    ) -> io::Result<usize>;

    /// # Errors
    ///
    /// Will return `io::Error` if an error is encountered reading a line
    fn read(
        &mut self,
        buffer: &mut String,
        options: &LineReaderOptions,
    ) -> io::Result<usize>;
}

#[derive(Debug, Clone, PartialEq)]
pub struct LineReader {
    buffer: EditBuffer,
    history: Option<HistoryStack>,
}

#[derive(Debug, Clone)]
pub struct LineReaderOptions {
    pub prompt: Option<char>,
    pub history: bool,
}

#[derive(Debug)]
enum EventResult {
    Accept,   // Accept line and return
    Continue, // Continue without repainting display
    Repaint,  // Continue after repainting display
}

#[must_use]
pub fn native_eol() -> &'static str {
    if std::env::consts::FAMILY == "windows" { "\r\n" } else { "\n" }
}

impl Default for LineReader {
    fn default() -> LineReader {
        LineReader { buffer: EditBuffer::new(), history: None }
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
        output_buffer: &mut String,
        options: &LineReaderOptions,
    ) -> io::Result<usize> {
        // ensure terminal is reset to cooked w/visible cursor
        let _terminal_session = TerminalSession {};
        // reset for new input
        let (display_width, display_height) = terminal::size()?;
        let (_, first_display_line) = cursor::position()?;

        let mut renderer = Renderer::new(
            display_width.into(),
            display_height.into(),
            first_display_line.into(),
        );
        self.buffer.reset(&mut renderer, options.prompt);
        terminal::enable_raw_mode()?;
        renderer.repaint(&self.buffer)?;

        // instantiate and/or get history stack, if necessary
        let history = if options.history {
            self.history.get_or_insert_with(HistoryStack::new);
            &mut self.history
        } else {
            &mut None
        };

        let mut res = EventResult::Continue;
        while !(matches!(res, EventResult::Accept)) {
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
                    if cursor_line > renderer.cursor.line {
                        renderer.first_display_line +=
                            cursor_line - renderer.cursor.line;
                    } else {
                        renderer.first_display_line -=
                            renderer.cursor.line - cursor_line;
                    }
                    renderer.cursor.line = cursor_line;
                    Event::Resize(x, y)
                }
                event => event,
            };
            res = handle_event(
                &mut self.buffer,
                &mut renderer,
                history.as_mut(),
                &event,
            );
            if !matches!(event, Event::Resize(..)) {
                renderer.repaint(&self.buffer)?;
            }
        }

        let _ = handle_end(&mut self.buffer, &mut renderer);
        renderer.repaint(&self.buffer)?;
        let mut stdout = io::stdout().lock();
        stdout.write_all(b"\r\n")?;
        stdout.flush()?;

        let prev_bytes = output_buffer.len();
        output_buffer.extend(self.buffer.input_chars());
        output_buffer.push_str(native_eol());
        Ok(output_buffer.len() - prev_bytes)
    }
}

#[cfg(not(tarpaulin_include))]
impl LineRead for LineReader {
    fn read_line(
        &mut self,
        prompt: Option<char>,
        buffer: &mut String,
    ) -> io::Result<usize> {
        self.accept_line(
            buffer,
            &LineReaderOptions { prompt, ..Default::default() },
        )
    }

    fn read(
        &mut self,
        buffer: &mut String,
        options: &LineReaderOptions,
    ) -> io::Result<usize> {
        self.accept_line(buffer, options)
    }
}

// impls for LineReaderOptions
impl LineReaderOptions {
    #[must_use]
    pub fn new() -> Self {
        LineReaderOptions { ..Default::default() }
    }
}

impl Default for LineReaderOptions {
    fn default() -> Self {
        LineReaderOptions { prompt: None, history: true }
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

impl<T> LineRead for T
where
    T: BufRead,
{
    fn read_line(
        &mut self,
        _prompt: Option<char>,
        buffer: &mut String,
    ) -> io::Result<usize> {
        BufRead::read_line(self, buffer)
    }

    fn read(
        &mut self,
        buffer: &mut String,
        _options: &LineReaderOptions,
    ) -> io::Result<usize> {
        BufRead::read_line(self, buffer)
    }
}

fn handle_event(
    buffer: &mut EditBuffer,
    renderer: &mut Renderer,
    history: Option<&mut HistoryStack>,
    event: &Event,
) -> EventResult {
    match event {
        Event::Key(event) if event.kind == KeyEventKind::Press => {
            handle_key_event(buffer, renderer, history, event)
        }
        Event::Resize(x, y) => {
            handle_resize_event(buffer, renderer, *x, *y)
        }
        _ => EventResult::Continue,
    }
}

fn handle_resize_event(
    buffer: &mut EditBuffer,
    renderer: &mut Renderer,
    x: u16,
    y: u16,
) -> EventResult {
    let old_width = renderer.display_width;
    let old_height = renderer.display_height;
    renderer.display_width = x.into();
    renderer.display_height = y.into();

    if renderer.display_width != old_width {
        buffer.reflow(renderer, 0);
    } else if renderer.display_height != old_height {
        renderer.adjust_viewport(buffer);
    }
    if renderer.display_height < old_height {
        let h_diff = old_height - renderer.display_height;
        renderer.scroll_needed =
            renderer.scroll_needed.saturating_sub(h_diff);
    }
    EventResult::Repaint
}

fn handle_key_event(
    buffer: &mut EditBuffer,
    renderer: &mut Renderer,
    history: Option<&mut HistoryStack>,
    event: &KeyEvent,
) -> EventResult {
    match event.code {
        KeyCode::Enter => {
            if let Some(history) = history {
                history.rewind();
                if !buffer.is_empty()
                    && history.last().is_none_or(|(last, _)| {
                        last.chars().ne(buffer.input_chars())
                    })
                {
                    history.push(buffer.input_chars().collect());
                }
            }
            EventResult::Accept
        }
        KeyCode::Left => handle_left(buffer, renderer),
        KeyCode::Right => handle_right(buffer, renderer),
        KeyCode::Home => handle_home(buffer, renderer),
        KeyCode::End => handle_end(buffer, renderer),
        KeyCode::Backspace => handle_backspace(buffer, renderer),
        KeyCode::Delete => handle_delete(buffer, renderer),
        KeyCode::Char(c) => handle_char_input(buffer, renderer, c),
        KeyCode::Up => handle_up(buffer, renderer, history),
        KeyCode::Down => handle_down(buffer, renderer, history),
        KeyCode::Esc => handle_esc(buffer, renderer, history),
        KeyCode::Tab => handle_char_input(buffer, renderer, '\t'),
        _ => EventResult::Continue,
    }
}

fn handle_esc(
    buffer: &mut EditBuffer,
    renderer: &mut Renderer,
    history: Option<&mut HistoryStack>,
) -> EventResult {
    buffer.set_from_draft(renderer);
    if let Some(history) = history {
        history.rewind();
    }
    EventResult::Repaint
}

fn handle_down(
    buffer: &mut EditBuffer,
    renderer: &mut Renderer,
    history: Option<&mut HistoryStack>,
) -> EventResult {
    let Some(history) = history else {
        return EventResult::Continue;
    };
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
        return EventResult::Continue;
    }

    // Advance to next newer history.
    // If there is none, take draft to load buffer
    // Otherwise load buffer edited, if any, or accepted.
    if let Some((ah, eh)) = history.next_newer() {
        buffer.set_input_text(
            renderer,
            eh.as_ref().map_or(ah, |eh| eh.as_str()),
        );
    } else {
        buffer.set_from_draft(renderer);
    }

    EventResult::Repaint
}

fn handle_up(
    buffer: &mut EditBuffer,
    renderer: &mut Renderer,
    history: Option<&mut HistoryStack>,
) -> EventResult {
    let Some(history) = history else {
        return EventResult::Continue;
    };
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
        buffer.set_input_text(
            renderer,
            edited.as_ref().map_or(accepted, |e| e.as_str()),
        );
    }
    EventResult::Repaint
}

fn handle_char_input(
    buffer: &mut EditBuffer,
    renderer: &mut Renderer,
    c: char,
) -> EventResult {
    // if char is zero width, but no previous chars exist to
    //  which it can  be combined, do nothing (i.e., don't accept
    // the input)
    if c != '\t' && edit_buffer::char_width(c, 0) == 0 {
        let check_line = if renderer.cursor.offset > 0 {
            renderer.cursor.line
        } else {
            renderer.cursor.line - 1
        };
        let check_start_offset = if check_line == buffer.input_start.line {
            buffer.input_start.offset
        } else {
            0
        };
        let check_end_offset = if renderer.cursor.offset == 0 {
            buffer.lines[check_line].len()
        } else {
            renderer.cursor.offset
        };
        if !buffer.lines[check_line][check_start_offset..check_end_offset]
            .chars()
            .rev()
            .take_while(|c| *c != '\t')
            .any(|c| edit_buffer::char_width(c, 0) > 0)
        {
            return EventResult::Continue;
        }
    }

    // insert new char at curser and let reflow sort it out
    assert!(renderer.cursor.line <= buffer.len());
    if renderer.cursor.line == buffer.len() {
        buffer.lines.push(BufferLine::new());
    }
    buffer.lines[renderer.cursor.line]
        .insert(renderer.cursor.offset, c);
    renderer.cursor.offset += c.len_utf8();

    // reflow from line before cursor, if it exists,
    // catching case where new char fits on previous line
    buffer
        .reflow(renderer, renderer.cursor.line.saturating_sub(1));

    EventResult::Repaint
}

fn handle_backspace(
    buffer: &mut EditBuffer,
    renderer: &mut Renderer,
) -> EventResult {
    if renderer.cursor == buffer.input_start {
        return EventResult::Continue;
    }

    if renderer.cursor.offset == 0 {
        renderer.cursor.line -= 1;
        renderer.cursor.offset =
            buffer.lines[renderer.cursor.line].len();
    }

    if let Some((i, _)) = buffer.lines[renderer.cursor.line]
        [..renderer.cursor.offset]
        .char_indices()
        .next_back()
    {
        buffer.lines[renderer.cursor.line].remove(i);
        renderer.cursor.offset = i;
    }
    buffer
        .reflow(renderer, renderer.cursor.line.saturating_sub(1));
    EventResult::Repaint
}

fn handle_left(
    buffer: &mut EditBuffer,
    renderer: &mut Renderer,
) -> EventResult {
    use unicode_width::UnicodeWidthChar;

    if renderer.cursor == buffer.input_start {
        return EventResult::Continue;
    }

    if renderer.cursor.offset == 0 {
        renderer.cursor.line -= 1;
        renderer.cursor.offset =
            buffer.lines[renderer.cursor.line].len();
    }

    if let Some((prev_idx, _)) = buffer.lines[renderer.cursor.line]
        [..renderer.cursor.offset]
        .char_indices()
        .rfind(|(_, c)| *c == '\t' || c.width().unwrap_or(0) > 0)
    {
        renderer.cursor.offset = prev_idx;
    }

    renderer.adjust_viewport(buffer);
    EventResult::Repaint
}

fn handle_right(
    buffer: &mut EditBuffer,
    renderer: &mut Renderer,
) -> EventResult {
    // If aleady at end, nothing to do
    if renderer.cursor
        == (buffer.lines.len() - 1, buffer.lines.last().unwrap().len()).into()
    {
        return EventResult::Continue;
    }

    let width_before_next_cursor = edit_buffer::str_width(
        &buffer.lines[renderer.cursor.line]
            [..renderer.cursor.offset],
        0,
    );

    if let Some((i, _)) = buffer.lines[renderer.cursor.line]
        [renderer.cursor.offset..]
        .char_indices()
        .skip(1)
        .find(|(_, c)| {
            edit_buffer::char_width(*c, width_before_next_cursor) > 0
        })
    {
        // next cursor pos on this line
        renderer.cursor.offset += i;
    } else if renderer.cursor.line == buffer.len() - 1
        && renderer.display_width - width_before_next_cursor > 0
    {
        // next cusor pos is at end of buffer
        renderer.cursor.offset =
            buffer.lines[renderer.cursor.line].len();
    } else {
        // next cursor pos is on next line
        renderer.cursor.line += 1;
        renderer.cursor.offset = 0;
    }

    renderer.adjust_viewport(buffer);
    EventResult::Repaint
}

fn handle_delete(
    buffer: &mut EditBuffer,
    renderer: &mut Renderer,
) -> EventResult {
    use unicode_width::UnicodeWidthChar;

    // if at end of buffer, nothing to do
    if renderer.cursor == buffer.buffer_end() {
        return EventResult::Continue;
    }

    let (cur_line, cur_offset) = renderer.cursor.into();
    let next_c_offset = buffer.lines[cur_line][cur_offset..]
        .char_indices()
        .skip(1)
        .find(|(_, c)| *c == '\t' || c.width().unwrap_or(0) > 0)
        .map_or_else(|| buffer.lines[cur_line].len(), |(i, _)| i + cur_offset);
    buffer.lines[cur_line].replace_range(cur_offset..next_c_offset, "");
    buffer.reflow(renderer, cur_line.saturating_sub(1));

    EventResult::Repaint
}

fn handle_home(
    buffer: &mut EditBuffer,
    renderer: &mut Renderer,
) -> EventResult {
    if renderer.cursor == buffer.input_start {
        return EventResult::Continue;
    }

    renderer.first_buffer_line = 0;
    renderer.cursor = buffer.input_start;
    renderer.adjust_viewport(buffer);

    EventResult::Repaint
}

fn handle_end(
    buffer: &mut EditBuffer,
    renderer: &mut Renderer,
) -> EventResult {
    let buffer_end = buffer.buffer_end();
    if renderer.cursor == buffer_end {
        return EventResult::Continue;
    }

    renderer.cursor = buffer_end;
    buffer.reflow(renderer, buffer_end.line);

    EventResult::Repaint
}

#[cfg(test)]
#[allow(clippy::unicode_not_nfc)]
mod tests {
    use super::*;

    use crossterm::event::KeyModifiers;
    use similar_asserts::assert_eq;

    fn make_buf(lines: &[&str], prompt: char) -> EditBuffer {
        let mut buf = EditBuffer {
            lines: Vec::new(),
            prompt: Some(prompt),
            ..Default::default()
        };
        for &l in lines {
            buf.lines.push(l.into());
        }
        buf.input_start = (0, prompt.len_utf8()).into();
        if let Some(l) = buf.lines.get_mut(0) {
            l.insert(0, prompt);
        }
        buf
    }

    #[test]
    fn unimplemented_event_ignored() {
        let mut buf = EditBuffer::new();
        let mut renderer = Renderer::new(10, 5, 0);
        let event = Event::FocusLost;
        let res = handle_event(&mut buf, &mut renderer, None, &event);
        assert!(
            matches!(res, EventResult::Continue),
            "expected {:?}, got {:?}",
            EventResult::Continue,
            res
        );
    }

    #[test]
    fn unimplemented_key_event_ignored() {
        let event =
            Event::Key(KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE));
        let res = handle_event(
            &mut EditBuffer::new(),
            &mut Renderer::new(10, 5, 0),
            None,
            &event,
        );
        assert!(
            matches!(res, EventResult::Continue),
            "expected {:?}, got {:?}",
            EventResult::Continue,
            res
        );
    }

    #[test]
    fn enter_breaks_input_loop() {
        let event =
            Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        let res = handle_event(
            &mut EditBuffer::new(),
            &mut Renderer::new(10, 5, 0),
            None,
            &event,
        );
        assert!(
            matches!(res, EventResult::Accept),
            "expected {:?}, got {:?}",
            EventResult::Accept,
            res
        );
    }

    #[test]
    fn char_input_non_0w_inserts() {
        let mut buf = EditBuffer::new();
        let mut renderer = Renderer::new(10, 5, 0);
        let expected_buf =
            EditBuffer { lines: vec!["🎸".into()], ..Default::default() };
        let expected_renderer = Renderer {
            display_width: 10,
            display_height: 5,
            cursor: (0, 4).into(),
            ..Default::default()
        };
        let event =
            Event::Key(KeyEvent::new(KeyCode::Char('🎸'), KeyModifiers::NONE));
        let res = handle_event(&mut buf, &mut renderer, None, &event);
        assert!(
            matches!(res, EventResult::Repaint),
            "expected {:?}, got {:?}",
            EventResult::Repaint,
            res
        );
        assert_eq!(buf, expected_buf);
        assert_eq!(renderer, expected_renderer);
    }

    #[test]
    fn char_input_0w_requires_base_char() {
        let mut buf = EditBuffer {
            lines: vec![":".into()],
            input_start: (0, 1).into(),
            ..Default::default()
        };
        let mut renderer = Renderer {
            display_width: 10,
            display_height: 5,
            cursor: (0, 1).into(),
            ..Default::default()
        };
        let expected_buf = buf.clone();
        let expected_renderer = renderer;

        let event = Event::Key(KeyEvent::new(
            KeyCode::Char('\u{0308}'),
            KeyModifiers::NONE,
        ));

        let res = handle_event(&mut buf, &mut renderer, None, &event);
        assert!(
            matches!(res, EventResult::Continue),
            "expected {:?}, got {:?}",
            EventResult::Continue,
            res
        );
        assert_eq!(buf, expected_buf);
        assert_eq!(renderer, expected_renderer);

        let mut buf = EditBuffer {
            lines: vec![":a".into()],
            input_start: (0, 1).into(),
            ..Default::default()
        };
        let mut renderer = Renderer {
            display_width: 10,
            display_height: 5,
            cursor: (0, 2).into(),
            ..Default::default()
        };

        let expected_buf = EditBuffer {
            lines: vec![":ä".into()],
            input_start: (0, 1).into(),
            ..Default::default()
        };
        let expected_renderer = Renderer {
            display_width: 10,
            display_height: 5,
            cursor: (0, 4).into(),
            ..Default::default()
        };

        let event = Event::Key(KeyEvent::new(
            KeyCode::Char('\u{0308}'),
            KeyModifiers::NONE,
        ));

        let res = handle_event(&mut buf, &mut renderer, None, &event);
        assert!(
            matches!(res, EventResult::Repaint),
            "expected {:?}, got {:?}",
            EventResult::Repaint,
            res
        );
        assert_eq!(buf, expected_buf);
        assert_eq!(renderer, expected_renderer);
    }

    #[test]
    fn char_input_before_eol_moves_cursor_char_width() {
        let mut buf = make_buf(&["e"], ':');
        let expected_buf = make_buf(&["ë"], ':');
        let mut renderer = Renderer {
            display_width: 10,
            display_height: 5,
            cursor: (0, 2).into(),
            ..Default::default()
        };
        let expected_renderer = Renderer { cursor: (0, 4).into(), ..renderer };

        let event = Event::Key(KeyEvent::new(
            KeyCode::Char('\u{0308}'),
            KeyModifiers::NONE,
        ));
        let res = handle_event(&mut buf, &mut renderer, None, &event);
        assert!(
            matches!(res, EventResult::Repaint),
            "expected {:?}, got {:?}",
            EventResult::Repaint,
            res
        );
        assert_eq!(buf, expected_buf);
        assert_eq!(renderer, expected_renderer);

        let event =
            Event::Key(KeyEvent::new(KeyCode::Char('🎸'), KeyModifiers::NONE));
        let expected_buf = make_buf(&["ë🎸"], ':');
        let expected_renderer = Renderer { cursor: (0, 8).into(), ..renderer };
        let res = handle_event(&mut buf, &mut renderer, None, &event);
        assert!(
            matches!(res, EventResult::Repaint),
            "expected {:?}, got {:?}",
            EventResult::Repaint,
            res
        );
        assert_eq!(buf, expected_buf);
        assert_eq!(renderer, expected_renderer);

        let event =
            Event::Key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE));
        let expected_buf =
            EditBuffer { lines: vec![":ë🎸o".into()], ..buf.clone() };
        let expected_renderer = Renderer { cursor: (0, 9).into(), ..renderer };
        let res = handle_event(&mut buf, &mut renderer, None, &event);
        assert!(
            matches!(res, EventResult::Repaint),
            "expected {:?}, got {:?}",
            EventResult::Repaint,
            res
        );
        assert_eq!(buf, expected_buf);
        assert_eq!(renderer, expected_renderer);
    }

    #[test]
    fn char_input_with_tab() {
        let mut buf = make_buf(&["a2345z"], ':');
        let mut renderer = Renderer {
            display_width: 80,
            display_height: 24,
            cursor: (0, 6).into(),
            ..Default::default()
        };

        let res = handle_event(
            &mut buf,
            &mut renderer,
            None,
            &Event::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)),
        );
        assert!(
            matches!(res, EventResult::Repaint),
            "expected {:?}, got {:?}",
            EventResult::Repaint,
            res
        );
        assert_eq!(*buf.lines[0], *":a2345\tz");
        assert_eq!(buf.lines[0].width(), 9);
        assert_eq!(renderer.cursor, (0, 7).into());

        renderer.cursor = (0, 6).into();
        let res = handle_event(
            &mut buf,
            &mut renderer,
            None,
            &Event::Key(KeyEvent::new(KeyCode::Char('6'), KeyModifiers::NONE)),
        );
        assert!(
            matches!(res, EventResult::Repaint),
            "expected {:?}, got {:?}",
            EventResult::Repaint,
            res
        );
        assert_eq!(*buf.lines[0], *":a23456\tz");
        assert_eq!(buf.lines[0].width(), 9);
        assert_eq!(renderer.cursor, (0, 7).into());

        let res = handle_event(
            &mut buf,
            &mut renderer,
            None,
            &Event::Key(KeyEvent::new(KeyCode::Char('7'), KeyModifiers::NONE)),
        );
        assert!(
            matches!(res, EventResult::Repaint),
            "expected {:?}, got {:?}",
            EventResult::Repaint,
            res
        );
        assert_eq!(*buf.lines[0], *":a234567\tz");
        assert_eq!(buf.lines[0].width(), 17);
        assert_eq!(renderer.cursor, (0, 8).into());
    }

    #[test]
    fn char_input_to_eol_wraps_cursor_to_next_line_start() {
        let mut buf = EditBuffer {
            lines: vec![":1234567".into()],
            input_start: (0, 1).into(),
            ..Default::default()
        };
        let mut renderer = Renderer {
            display_width: 10,
            display_height: 5,
            cursor: (0, 8).into(),
            ..Default::default()
        };

        let event =
            Event::Key(KeyEvent::new(KeyCode::Char('🎸'), KeyModifiers::NONE));
        let expected_buf =
            EditBuffer { lines: vec![":1234567🎸".into()], ..buf.clone() };
        let expected_renderer = Renderer { cursor: (1, 0).into(), ..renderer };
        let res = handle_event(&mut buf, &mut renderer, None, &event);
        assert!(
            matches!(res, EventResult::Repaint),
            "expected {:?}, got {:?}",
            EventResult::Repaint,
            res
        );
        assert_eq!(buf, expected_buf);
        assert_eq!(renderer, expected_renderer);
    }

    #[test]
    fn char_input_append_to_previous_line_if_fits() {
        let mut buf = EditBuffer {
            lines: vec![":12345678".into(), "🎸abc".into()],
            input_start: (0, 1).into(),
            ..Default::default()
        };
        let mut renderer = Renderer {
            display_width: 10,
            display_height: 5,
            cursor: (1, 0).into(),
            first_display_line: 3,
            ..Default::default()
        };
        let expected_buf = EditBuffer {
            lines: vec![":123456789".into(), "🎸abc".into()],
            ..buf.clone()
        };
        let expected_renderer = renderer;

        let event =
            Event::Key(KeyEvent::new(KeyCode::Char('9'), KeyModifiers::NONE));
        let res = handle_event(&mut buf, &mut renderer, None, &event);
        assert!(
            matches!(res, EventResult::Repaint),
            "expected {:?}, got {:?}",
            EventResult::Repaint,
            res
        );
        assert_eq!(buf, expected_buf);
        assert_eq!(renderer, expected_renderer);
    }

    #[test]
    fn char_input_char_too_wide_at_end_wraps_to_next_line() {
        let mut buf = EditBuffer {
            lines: vec![":12345678".into()],
            input_start: (0, 1).into(),
            ..Default::default()
        };
        let mut renderer = Renderer {
            display_width: 10,
            display_height: 5,
            cursor: (0, 9).into(),
            ..Default::default()
        };

        let expected_buf = EditBuffer {
            lines: vec![":12345678".into(), "🎸".into()],
            ..buf.clone()
        };
        let expected_renderer = Renderer { cursor: (1, 4).into(), ..renderer };

        let event =
            Event::Key(KeyEvent::new(KeyCode::Char('🎸'), KeyModifiers::NONE));
        let res = handle_event(&mut buf, &mut renderer, None, &event);

        assert!(
            matches!(res, EventResult::Repaint),
            "expected {:?}, got {:?}",
            EventResult::Repaint,
            res
        );
        assert_eq!(buf, expected_buf);
        assert_eq!(renderer, expected_renderer);
    }

    #[test]
    fn char_input_past_eol_wraps_input_to_next_line_start() {
        let mut buf = EditBuffer {
            lines: vec![":123456789".into(), "abc".into()],
            input_start: (0, 1).into(),
            ..Default::default()
        };
        let mut renderer = Renderer {
            display_width: 10,
            display_height: 5,
            cursor: (0, 8).into(),
            ..Default::default()
        };

        let expected_buf = EditBuffer {
            lines: vec![":1234567🎸".into(), "89abc".into()],
            ..buf.clone()
        };
        let expected_renderer = Renderer { cursor: (1, 0).into(), ..renderer };

        let event =
            Event::Key(KeyEvent::new(KeyCode::Char('🎸'), KeyModifiers::NONE));
        let res = handle_event(&mut buf, &mut renderer, None, &event);

        assert!(
            matches!(res, EventResult::Repaint),
            "expected {:?}, got {:?}",
            EventResult::Repaint,
            res
        );
        assert_eq!(buf, expected_buf);
        assert_eq!(renderer, expected_renderer);
    }

    #[test]
    fn char_input_at_end_of_small_buffer_moving_cursor_beyond_bottom() {
        let mut buf = EditBuffer {
            lines: vec![":12345678".into(), "🎸2345678".into()],
            input_start: (0, 1).into(),
            ..Default::default()
        };
        let mut renderer = Renderer {
            display_width: 10,
            display_height: 5,
            first_display_line: 3,
            cursor: (1, 11).into(),
            ..Default::default()
        };
        let expected_buf = EditBuffer {
            lines: vec![":12345678".into(), "🎸2345678a".into()],
            ..buf.clone()
        };
        let expected_renderer = Renderer {
            first_display_line: 2,
            cursor: (2, 0).into(),
            scroll_needed: 1,
            ..renderer
        };
        let event =
            Event::Key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
        let res = handle_event(&mut buf, &mut renderer, None, &event);
        assert!(
            matches!(res, EventResult::Repaint),
            "expected {:?}, got {:?}",
            EventResult::Repaint,
            res
        );
        assert_eq!(buf, expected_buf);
        assert_eq!(renderer, expected_renderer);
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
            ..Default::default()
        };
        let mut renderer = Renderer {
            display_width: 10,
            display_height: 5,
            first_buffer_line: 1,
            cursor: (5, 11).into(),
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
            ],
            ..buf.clone()
        };
        let expected_renderer = Renderer {
            first_buffer_line: 2,
            cursor: (6, 0).into(),
            ..renderer
        };

        let event =
            Event::Key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
        let res = handle_event(&mut buf, &mut renderer, None, &event);
        assert!(
            matches!(res, EventResult::Repaint),
            "expected {:?}, got {:?}",
            EventResult::Repaint,
            res
        );
        assert_eq!(buf, expected_buf);
        assert_eq!(renderer, expected_renderer);
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
            ..Default::default()
        };
        let mut renderer = Renderer {
            display_width: 10,
            display_height: 5,
            first_display_line: 3,
            cursor: (0, 9).into(),
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
        let expected_renderer = Renderer {
            first_display_line: 2,
            cursor: (1, 0).into(),
            scroll_needed: 1,
            ..renderer
        };
        let event =
            Event::Key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
        let res = handle_event(&mut buf, &mut renderer, None, &event);
        assert!(
            matches!(res, EventResult::Repaint),
            "expected {:?}, got {:?}",
            EventResult::Repaint,
            res
        );
        assert_eq!(buf, expected_buf);
        assert_eq!(renderer, expected_renderer);
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
            ..Default::default()
        };
        let mut renderer = Renderer {
            display_width: 10,
            display_height: 5,
            first_buffer_line: 1,
            cursor: (4, 9).into(),
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
            ],
            ..buf.clone()
        };
        let expected_renderer = Renderer {
            first_buffer_line: 2,
            cursor: (5, 0).into(),
            ..renderer
        };
        let event =
            Event::Key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
        let res = handle_event(&mut buf, &mut renderer, None, &event);
        assert!(
            matches!(res, EventResult::Repaint),
            "expected {:?}, got {:?}",
            EventResult::Repaint,
            res
        );
        assert_eq!((buf, renderer), (expected_buf, expected_renderer));
    }

    #[test]
    fn backspace_0w() {
        let mut buf = EditBuffer {
            lines: vec![":ë".into()],
            input_start: (0, 1).into(),
            ..Default::default()
        };
        let mut renderer = Renderer {
            display_width: 10,
            display_height: 5,
            cursor: (0, 4).into(),
            ..Default::default()
        };

        let expected_buf =
            EditBuffer { lines: vec![":e".into()], ..buf.clone() };
        let expected_renderer = Renderer { cursor: (0, 2).into(), ..renderer };

        let event =
            Event::Key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        let res = handle_event(&mut buf, &mut renderer, None, &event);
        assert!(
            matches!(res, EventResult::Repaint),
            "expected {:?}, got {:?}",
            EventResult::Repaint,
            res
        );
        assert_eq!(buf, expected_buf);
        assert_eq!(renderer, expected_renderer);
    }

    #[test]
    fn backspace_1w() {
        let mut buf = make_buf(&["e"], ':');
        let expected_buf = make_buf(&[""], ':');
        let mut renderer = Renderer {
            display_width: 10,
            display_height: 5,
            cursor: (0, 2).into(),
            ..Default::default()
        };
        let expected_renderer = Renderer { cursor: (0, 1).into(), ..renderer };

        let event =
            Event::Key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        let res = handle_event(&mut buf, &mut renderer, None, &event);
        assert!(
            matches!(res, EventResult::Repaint),
            "expected {:?}, got {:?}",
            EventResult::Repaint,
            res
        );
        assert_eq!(buf, expected_buf);
        assert_eq!(renderer, expected_renderer);
    }

    #[test]
    fn backspace_2w() {
        let mut buf = make_buf(&["🎸"], ':');
        let mut renderer = Renderer {
            display_width: 10,
            display_height: 5,
            cursor: (0, 5).into(),
            ..Default::default()
        };
        let expected_buf = make_buf(&[""], ':');
        let expected_renderer = Renderer { cursor: (0, 1).into(), ..renderer };

        let event =
            Event::Key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        let res = handle_event(&mut buf, &mut renderer, None, &event);
        assert!(
            matches!(res, EventResult::Repaint),
            "expected {:?}, got {:?}",
            EventResult::Repaint,
            res
        );
        assert_eq!(buf, expected_buf);
        assert_eq!(renderer, expected_renderer);
    }

    #[test]
    fn backspace_input_start() {
        let mut buf = make_buf(&[""], ':');
        let mut renderer = Renderer {
            display_width: 10,
            display_height: 5,
            cursor: (0, 1).into(),
            ..Default::default()
        };
        let expected_buf = buf.clone();
        let expected_renderer = renderer;

        let event =
            Event::Key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));

        let res = handle_event(&mut buf, &mut renderer, None, &event);
        assert!(
            matches!(res, EventResult::Continue),
            "expected {:?}, got {:?}",
            EventResult::Continue,
            res
        );
        assert_eq!(buf, expected_buf);
        assert_eq!(renderer, expected_renderer);
    }

    #[test]
    fn backspace_to_column_0_wraps_if_room_on_preceding_line() {
        let mut buf = make_buf(&["12345678", "🎸9"], ':');
        let mut renderer = Renderer {
            display_width: 10,
            display_height: 5,
            cursor: (1, 4).into(),
            ..Default::default()
        };
        let expected_buf = make_buf(&["123456789", ""], ':');
        let expected_renderer = Renderer { cursor: (0, 9).into(), ..renderer };
        let event =
            Event::Key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        let res = handle_event(&mut buf, &mut renderer, None, &event);
        assert!(
            matches!(res, EventResult::Repaint),
            "expected {:?}, got {:?}",
            EventResult::Repaint,
            res
        );
        assert_eq!(buf, expected_buf);
        assert_eq!(renderer, expected_renderer);
    }

    #[test]
    fn backspace_from_column_0_wraps_if_room_on_preceding_line() {
        let event =
            Event::Key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));

        // base case
        let mut buf = make_buf(&["123456789", ""], ':');
        let mut renderer = Renderer {
            display_width: 10,
            display_height: 10,
            cursor: (1, 0).into(),
            ..Default::default()
        };

        let expected_buf = make_buf(&["12345678"], ':');
        let expected_renderer = Renderer { cursor: (0, 9).into(), ..renderer };
        let res = handle_event(&mut buf, &mut renderer, None, &event);
        assert!(
            matches!(res, EventResult::Repaint),
            "expected {:?}, got {:?}",
            EventResult::Repaint,
            res
        );
        assert_eq!(buf, expected_buf);
        assert_eq!(renderer, expected_renderer);

        // zero len char at preceding line end
        let mut buf = make_buf(&["12345678ä", "eiou"], ':');
        let mut renderer = Renderer {
            display_width: 10,
            display_height: 5,
            cursor: (1, 0).into(),
            ..Default::default()
        };
        let expected_buf = make_buf(&["12345678a", "eiou"], ':');
        let expected_renderer = renderer;
        let res = handle_event(&mut buf, &mut renderer, None, &event);
        assert!(
            matches!(res, EventResult::Repaint),
            "expected {:?}, got {:?}",
            EventResult::Repaint,
            res
        );
        assert_eq!(buf, expected_buf);
        assert_eq!(renderer, expected_renderer);
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
        let mut renderer = Renderer {
            display_width: 10,
            display_height: 5,
            first_buffer_line: 1,
            cursor: (2, 0).into(),
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
        let expected_renderer = Renderer {
            first_buffer_line: 0,
            cursor: (1, 9).into(),
            ..renderer
        };

        let event =
            Event::Key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        let res = handle_event(&mut buf, &mut renderer, None, &event);
        assert!(
            matches!(res, EventResult::Repaint),
            "expected {:?}, got {:?}",
            EventResult::Repaint,
            res
        );
        assert_eq!(buf, expected_buf);
        assert_eq!(renderer, expected_renderer);
    }

    #[test]
    fn left_from_input_start_does_nothing() {
        let mut buf = make_buf(&["12345"], ':');
        let mut renderer = Renderer {
            display_width: 10,
            display_height: 5,
            cursor: (0, 1).into(),
            ..Default::default()
        };
        let expected_buf = buf.clone();
        let expected_renderer = renderer;
        let event =
            Event::Key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        let res = handle_event(&mut buf, &mut renderer, None, &event);
        assert!(
            matches!(res, EventResult::Continue),
            "expected {:?}, got {:?}",
            EventResult::Continue,
            res
        );
        assert_eq!(buf, expected_buf);
        assert_eq!(renderer, expected_renderer);
    }

    #[test]
    fn left_moves_cursor_to_preceding_base_char() {
        let event =
            Event::Key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        let mut buf = make_buf(&["aë🎸iou"], ':');
        let mut renderer = Renderer {
            display_width: 10,
            display_height: 5,
            cursor: (0, 10).into(),
            ..Default::default()
        };
        let expected_buf = buf.clone();
        let expected_renderer = Renderer { cursor: (0, 9).into(), ..renderer };
        let res = handle_event(&mut buf, &mut renderer, None, &event);
        assert!(
            matches!(res, EventResult::Repaint),
            "expected {:?}, got {:?}",
            EventResult::Repaint,
            res
        );
        assert_eq!(buf, expected_buf);
        assert_eq!(renderer, expected_renderer);

        let expected_renderer = Renderer { cursor: (0, 5).into(), ..renderer };
        let res = handle_event(&mut buf, &mut renderer, None, &event);
        assert!(
            matches!(res, EventResult::Repaint),
            "expected {:?}, got {:?}",
            EventResult::Repaint,
            res
        );
        assert_eq!(buf, expected_buf);
        assert_eq!(renderer, expected_renderer);

        let expected_renderer = Renderer { cursor: (0, 2).into(), ..renderer };
        let res = handle_event(&mut buf, &mut renderer, None, &event);
        assert!(
            matches!(res, EventResult::Repaint),
            "expected {:?}, got {:?}",
            EventResult::Repaint,
            res
        );
        assert_eq!(buf, expected_buf);
        assert_eq!(renderer, expected_renderer);
    }

    #[test]
    fn left_from_column_0_moves_cursor_to_last_base_char_on_preceding_line() {
        let event =
            Event::Key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        let mut buf = make_buf(&["12345678", "🎸abc"], ':');
        let mut renderer = Renderer {
            display_width: 10,
            display_height: 5,
            cursor: (1, 0).into(),
            ..Default::default()
        };
        let expected_buf = buf.clone();
        let expected_renderer = Renderer {
            display_width: 10,
            display_height: 5,
            cursor: (0, 8).into(),
            ..renderer
        };

        let res = handle_event(&mut buf, &mut renderer, None, &event);
        assert!(
            matches!(res, EventResult::Repaint),
            "expected {:?}, got {:?}",
            EventResult::Repaint,
            res
        );
        assert_eq!(buf, expected_buf);
        assert_eq!(renderer, expected_renderer);
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
        let mut renderer = Renderer {
            display_width: 10,
            display_height: 5,
            first_buffer_line: 1,
            cursor: (2, 0).into(),
            ..Default::default()
        };
        let expected_buf = buf.clone();
        let expected_renderer = Renderer {
            first_buffer_line: 0,
            cursor: (1, 8).into(),
            ..renderer
        };

        let res = handle_event(&mut buf, &mut renderer, None, &event);
        assert!(
            matches!(res, EventResult::Repaint),
            "expected {:?}, got {:?}",
            EventResult::Repaint,
            res
        );
        assert_eq!(buf, expected_buf);
        assert_eq!(renderer, expected_renderer);
    }

    #[test]
    fn home_from_input_start_does_nothing() {
        let event =
            Event::Key(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE));
        let mut buf =
            make_buf(&["123456789", "0123456789", "012345678", "🎸abcd"], ':');
        let mut renderer = Renderer {
            display_width: 10,
            display_height: 5,
            cursor: (0, 1).into(),
            ..Default::default()
        };
        let expected_buf = buf.clone();
        let expected_renderer = renderer;
        let res = handle_event(&mut buf, &mut renderer, None, &event);
        assert!(
            matches!(res, EventResult::Continue),
            "expected {:?}, got {:?}",
            EventResult::Continue,
            res
        );
        assert_eq!(buf, expected_buf);
        assert_eq!(renderer, expected_renderer);
    }

    #[test]
    fn home_moves_cursor_to_input_start() {
        let event =
            Event::Key(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE));
        let mut buf =
            make_buf(&["123456789", "0123456789", "012345678", "🎸abcd"], ':');
        let mut renderer = Renderer {
            display_width: 10,
            display_height: 5,
            cursor: (3, 0).into(),
            ..Default::default()
        };
        let expected_buf = buf.clone();
        let expected_renderer = Renderer { cursor: (0, 1).into(), ..renderer };
        let res = handle_event(&mut buf, &mut renderer, None, &event);
        assert!(
            matches!(res, EventResult::Repaint),
            "expected {:?}, got {:?}",
            EventResult::Repaint,
            res
        );
        assert_eq!(buf, expected_buf);
        assert_eq!(renderer, expected_renderer);
    }

    #[test]
    fn home_moving_cursor_above_top_pans_buffer() {
        let event =
            Event::Key(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE));
        let mut buf =
            make_buf(&["123456789", "0123456789", "012345678", "🎸abcd"], ':');
        let mut renderer = Renderer {
            display_width: 10,
            display_height: 5,
            first_buffer_line: 2,
            cursor: (3, 0).into(),
            ..Default::default()
        };
        let expected_buf = buf.clone();
        let expected_renderer = Renderer {
            display_width: 10,
            display_height: 5,
            first_buffer_line: 0,
            cursor: (0, 1).into(),
            ..Default::default()
        };
        let res = handle_event(&mut buf, &mut renderer, None, &event);
        assert!(
            matches!(res, EventResult::Repaint),
            "expected {:?}, got {:?}",
            EventResult::Repaint,
            res
        );
        assert_eq!(buf, expected_buf);
        assert_eq!(renderer, expected_renderer);
    }
    #[test]
    fn right_at_buffer_end_does_nothing() {
        let event =
            Event::Key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        let mut buf = make_buf(&["123456"], ':');
        let mut renderer = Renderer {
            display_width: 10,
            display_height: 5,
            cursor: (0, 7).into(),
            ..Default::default()
        };
        let expected_buf = buf.clone();
        let expected_renderer = renderer;
        let res = handle_event(&mut buf, &mut renderer, None, &event);
        assert!(
            matches!(res, EventResult::Continue),
            "expected {:?}, got {:?}",
            EventResult::Continue,
            res
        );
        assert_eq!(buf, expected_buf);
        assert_eq!(renderer, expected_renderer);
    }

    #[test]
    fn right_moves_cursor_to_next_base_char_until_end() {
        let event =
            Event::Key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        let mut buf = make_buf(&["aë🎸o"], ':');
        let mut renderer = Renderer {
            display_width: 10,
            display_height: 5,
            cursor: (0, 1).into(),
            ..Default::default()
        };
        let expected_buf = buf.clone();
        let expected_renderer = Renderer { cursor: (0, 2).into(), ..renderer };
        let res = handle_event(&mut buf, &mut renderer, None, &event);
        assert!(
            matches!(res, EventResult::Repaint),
            "expected {:?}, got {:?}",
            EventResult::Repaint,
            res
        );
        assert_eq!(buf, expected_buf);
        assert_eq!(renderer, expected_renderer);

        let expected_renderer = Renderer { cursor: (0, 5).into(), ..renderer };
        let res = handle_event(&mut buf, &mut renderer, None, &event);
        assert!(
            matches!(res, EventResult::Repaint),
            "expected {:?}, got {:?}",
            EventResult::Repaint,
            res
        );
        assert_eq!(buf, expected_buf);
        assert_eq!(renderer, expected_renderer);

        let expected_renderer = Renderer { cursor: (0, 9).into(), ..renderer };
        let res = handle_event(&mut buf, &mut renderer, None, &event);
        assert!(
            matches!(res, EventResult::Repaint),
            "expected {:?}, got {:?}",
            EventResult::Repaint,
            res
        );
        assert_eq!(buf, expected_buf);
        assert_eq!(renderer, expected_renderer);

        let expected_renderer = Renderer { cursor: (0, 10).into(), ..renderer };
        let res = handle_event(&mut buf, &mut renderer, None, &event);
        assert!(
            matches!(res, EventResult::Repaint),
            "expected {:?}, got {:?}",
            EventResult::Repaint,
            res
        );
        assert_eq!(buf, expected_buf);
        assert_eq!(renderer, expected_renderer);
    }

    #[test]
    fn right_from_last_base_char_moves_to_next_column_0() {
        let event =
            Event::Key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        let mut buf = make_buf(&["12345678", "🎸23456789", ""], ':');
        let mut renderer = Renderer {
            display_width: 10,
            display_height: 5,
            cursor: (0, 8).into(),
            ..Default::default()
        };
        let expected_buf = buf.clone();
        let expected_renderer = Renderer { cursor: (1, 0).into(), ..renderer };
        let res = handle_event(&mut buf, &mut renderer, None, &event);
        assert!(
            matches!(res, EventResult::Repaint),
            "expected {:?}, got {:?}",
            EventResult::Repaint,
            res
        );
        assert_eq!(buf, expected_buf);
        assert_eq!(renderer, expected_renderer);

        let mut renderer = Renderer {
            display_width: 10,
            display_height: 5,
            cursor: (1, 11).into(),
            ..Default::default()
        };
        let expected_renderer = Renderer { cursor: (2, 0).into(), ..renderer };
        let res = handle_event(&mut buf, &mut renderer, None, &event);
        assert!(
            matches!(res, EventResult::Repaint),
            "expected {:?}, got {:?}",
            EventResult::Repaint,
            res
        );
        assert_eq!(buf, expected_buf);
        assert_eq!(renderer, expected_renderer);
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
        let mut renderer = Renderer {
            display_width: 10,
            display_height: 5,
            cursor: (3, 9).into(),
            ..Default::default()
        };
        let expected_buf = buf.clone();
        let expected_renderer = Renderer {
            first_buffer_line: 1,
            cursor: (4, 0).into(),
            ..renderer
        };

        let res = handle_event(&mut buf, &mut renderer, None, &event);
        assert!(
            matches!(res, EventResult::Repaint),
            "expected {:?}, got {:?}",
            EventResult::Repaint,
            res
        );
        assert_eq!(buf, expected_buf);
        assert_eq!(renderer, expected_renderer);
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
        let mut renderer = Renderer {
            display_width: 10,
            display_height: 5,
            first_buffer_line: 5,
            cursor: (9, 0).into(),
            ..Default::default()
        };
        let expected_buf = buf.clone();
        let expected_renderer = renderer;
        let res = handle_event(&mut buf, &mut renderer, None, &event);
        assert!(
            matches!(res, EventResult::Continue),
            "expected {:?}, got {:?}",
            EventResult::Continue,
            res
        );
        assert_eq!(buf, expected_buf);
        assert_eq!(renderer, expected_renderer);
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
        let mut renderer = Renderer {
            display_width: 10,
            display_height: 5,
            first_buffer_line: 5,
            cursor: (8, 5).into(),
            ..Default::default()
        };
        let expected_buf = buf.clone();
        let expected_renderer = Renderer { cursor: (9, 0).into(), ..renderer };
        let res = handle_event(&mut buf, &mut renderer, None, &event);
        assert!(
            matches!(res, EventResult::Repaint),
            "expected {:?}, got {:?}",
            EventResult::Repaint,
            res
        );
        assert_eq!(buf, expected_buf);
        assert_eq!(renderer, expected_renderer);
    }

    #[test]
    fn end_past_display_bottom_in_small_buffer_scrolls_up() {
        let event = Event::Key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE));
        let mut buf = make_buf(
            &["123456789", "0123456789", "0123456789", "0123456789", ""],
            ':',
        );
        let mut renderer = Renderer {
            display_width: 10,
            display_height: 5,
            first_buffer_line: 0,
            first_display_line: 3,
            cursor: buf.input_start,
            ..Default::default()
        };

        let expected_buf = buf.clone();
        let expected_renderer = Renderer {
            cursor: (4, 0).into(),
            first_display_line: 0,
            scroll_needed: 3,
            ..renderer
        };
        let res = handle_event(&mut buf, &mut renderer, None, &event);
        assert!(
            matches!(res, EventResult::Repaint),
            "expected {:?}, got {:?}",
            EventResult::Repaint,
            res
        );
        assert_eq!(buf, expected_buf);
        assert_eq!(renderer, expected_renderer);
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
        let mut renderer = Renderer {
            display_width: 10,
            display_height: 5,
            first_buffer_line: 0,
            cursor: buf.input_start,
            ..Default::default()
        };
        let expected_buf = buf.clone();
        let expected_renderer = Renderer {
            cursor: (9, 0).into(),
            first_buffer_line: 5,
            ..renderer
        };
        let res = handle_event(&mut buf, &mut renderer, None, &event);
        assert!(
            matches!(res, EventResult::Repaint),
            "expected {:?}, got {:?}",
            EventResult::Repaint,
            res
        );
        assert_eq!(buf, expected_buf);
        assert_eq!(renderer, expected_renderer);
    }

    #[test]
    fn delete_at_buffer_end_does_nothing() {
        let event =
            Event::Key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE));
        let mut buf = make_buf(&["aë🎸io"], ':');
        let mut renderer = Renderer {
            display_width: 10,
            display_height: 5,
            cursor: (0, 11).into(),
            ..Default::default()
        };
        let expected_buf = buf.clone();
        let expected_renderer = renderer;
        let res = handle_event(&mut buf, &mut renderer, None, &event);
        assert!(
            matches!(res, EventResult::Continue),
            "expected {:?}, got {:?}",
            EventResult::Continue,
            res
        );
        assert_eq!(buf, expected_buf);
        assert_eq!(renderer, expected_renderer);
    }

    #[test]
    fn delete_removes_chars_from_cursor_to_next_base_char() {
        let event =
            Event::Key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE));
        let mut buf = make_buf(&["aë🎸io"], ':');
        let mut renderer = Renderer {
            display_width: 10,
            display_height: 5,
            cursor: (0, 2).into(),
            ..Default::default()
        };

        let expected_buf = make_buf(&["a🎸io"], ':');
        let expected_renderer = renderer;
        let res = handle_event(&mut buf, &mut renderer, None, &event);
        assert!(
            matches!(res, EventResult::Repaint),
            "expected {:?}, got {:?}",
            EventResult::Repaint,
            res
        );
        assert_eq!(buf, expected_buf);
        assert_eq!(renderer, expected_renderer);

        let expected_buf = make_buf(&["aio"], ':');
        let res = handle_event(&mut buf, &mut renderer, None, &event);
        assert!(
            matches!(res, EventResult::Repaint),
            "expected {:?}, got {:?}",
            EventResult::Repaint,
            res
        );
        assert_eq!(buf, expected_buf);
        assert_eq!(renderer, expected_renderer);

        let expected_buf = make_buf(&["ao"], ':');
        let res = handle_event(&mut buf, &mut renderer, None, &event);
        assert!(
            matches!(res, EventResult::Repaint),
            "expected {:?}, got {:?}",
            EventResult::Repaint,
            res
        );
        assert_eq!(buf, expected_buf);
        assert_eq!(renderer, expected_renderer);
    }

    #[test]
    fn delete_at_line_start_wraps_to_previous_if_new_first_char_fits() {
        let event =
            Event::Key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE));
        let mut buf = make_buf(&["12345678", "🎸abc"], ':');
        let mut renderer = Renderer {
            display_width: 10,
            display_height: 5,
            cursor: (1, 0).into(),
            ..Default::default()
        };

        let expected_buf = make_buf(&["12345678a", "bc"], ':');
        let expected_renderer = Renderer { cursor: (0, 9).into(), ..renderer };
        let res = handle_event(&mut buf, &mut renderer, None, &event);
        assert!(
            matches!(res, EventResult::Repaint),
            "expected {:?}, got {:?}",
            EventResult::Repaint,
            res
        );
        assert_eq!(buf, expected_buf);
        assert_eq!(renderer, expected_renderer);
    }

    #[test]
    fn delete_reflows_buffer_from_new_cursor_line() {
        let event =
            Event::Key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE));
        let mut buf = make_buf(
            &["123456789", "0123456789", "0123456789", "0123456789"],
            ':',
        );
        let mut renderer = Renderer {
            display_width: 10,
            display_height: 5,
            cursor: (0, 9).into(),
            ..Default::default()
        };

        let expected_buf = make_buf(
            &["123456780", "1234567890", "1234567890", "123456789"],
            ':',
        );
        let expected_renderer = Renderer { cursor: (0, 9).into(), ..renderer };
        let res = handle_event(&mut buf, &mut renderer, None, &event);
        assert!(
            matches!(res, EventResult::Repaint),
            "expected {:?}, got {:?}",
            EventResult::Repaint,
            res
        );
        assert_eq!(buf, expected_buf);
        assert_eq!(renderer, expected_renderer);
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
        let mut renderer = Renderer {
            display_width: 10,
            display_height: 10,
            first_display_line: 3,
            cursor: (6, 5).into(),
            ..Default::default()
        };

        let expected_renderer = Renderer {
            display_height: 8,
            first_display_line: 1,
            cursor: (6, 5).into(),
            ..renderer
        };
        let expected_buf = buf.clone();
        let res =
            handle_event(&mut buf, &mut renderer, None, &Event::Resize(10, 8));
        assert!(
            matches!(res, EventResult::Repaint),
            "expected {:?}, got {:?}",
            EventResult::Repaint,
            res
        );
        assert_eq!(buf, expected_buf);
        assert_eq!(renderer, expected_renderer);

        let expected_renderer = Renderer {
            display_height: 7,
            first_display_line: 0,
            cursor: (6, 5).into(),
            ..renderer
        };
        let res =
            handle_event(&mut buf, &mut renderer, None, &Event::Resize(10, 7));
        assert!(
            matches!(res, EventResult::Repaint),
            "expected {:?}, got {:?}",
            EventResult::Repaint,
            res
        );
        assert_eq!(buf, expected_buf);
        assert_eq!(renderer, expected_renderer);

        let expected_renderer = Renderer {
            display_height: 5,
            first_buffer_line: 2,
            cursor: (6, 5).into(),
            ..renderer
        };
        let res =
            handle_event(&mut buf, &mut renderer, None, &Event::Resize(10, 5));
        assert!(
            matches!(res, EventResult::Repaint),
            "expected {:?}, got {:?}",
            EventResult::Repaint,
            res
        );
        assert_eq!(buf, expected_buf);
        assert_eq!(renderer, expected_renderer);
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
        let mut renderer = Renderer {
            display_width: 10,
            display_height: 10,
            first_display_line: 3,
            cursor: (0, 1).into(),
            ..Default::default()
        };

        let expected_buf = buf.clone();
        let expected_renderer = Renderer {
            display_height: 8,
            first_display_line: 1,
            cursor: (0, 1).into(),
            ..renderer
        };
        let res =
            handle_event(&mut buf, &mut renderer, None, &Event::Resize(10, 8));
        assert!(
            matches!(res, EventResult::Repaint),
            "expected {:?}, got {:?}",
            EventResult::Repaint,
            res
        );
        assert_eq!(buf, expected_buf);
        assert_eq!(renderer, expected_renderer);

        let expected_renderer = Renderer {
            display_height: 7,
            first_display_line: 0,
            cursor: (0, 1).into(),
            ..renderer
        };
        let res =
            handle_event(&mut buf, &mut renderer, None, &Event::Resize(10, 7));
        assert!(
            matches!(res, EventResult::Repaint),
            "expected {:?}, got {:?}",
            EventResult::Repaint,
            res
        );
        assert_eq!(buf, expected_buf);
        assert_eq!(renderer, expected_renderer);

        let expected_renderer =
            Renderer { display_height: 5, cursor: (0, 1).into(), ..renderer };
        let res =
            handle_event(&mut buf, &mut renderer, None, &Event::Resize(10, 5));
        assert!(
            matches!(res, EventResult::Repaint),
            "expected {:?}, got {:?}",
            EventResult::Repaint,
            res
        );
        assert_eq!(buf, expected_buf);
        assert_eq!(renderer, expected_renderer);
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
        let mut renderer = Renderer {
            display_width: 10,
            display_height: 10,
            first_display_line: 3,
            cursor: (0, 1).into(),
            ..Default::default()
        };

        let expected_buf = make_buf(
            &[
                "12345", "678901", "234567", "8🎸234", "567890", "123456",
                "789012", "345678", "901234", "56789ä", "bcdefg", "h",
            ],
            ':',
        );
        let expected_renderer = Renderer { display_width: 6, ..renderer };

        let res =
            handle_event(&mut buf, &mut renderer, None, &Event::Resize(6, 10));
        assert!(
            matches!(res, EventResult::Repaint),
            "expected {:?}, got {:?}",
            EventResult::Repaint,
            res
        );
        assert_eq!(buf, expected_buf);
        assert_eq!(renderer, expected_renderer);
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
            input_start: (0, 9).into(),
            ..Default::default()
        };
        let mut renderer = Renderer {
            display_width: 10,
            display_height: 10,
            first_display_line: 3,
            cursor: (0, 9).into(),
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
        let expected_renderer =
            Renderer { display_width: 6, cursor: (1, 3).into(), ..renderer };

        let res =
            handle_event(&mut buf, &mut renderer, None, &Event::Resize(6, 10));
        assert!(
            matches!(res, EventResult::Repaint),
            "expected {:?}, got {:?}",
            EventResult::Repaint,
            res
        );
        assert_eq!(buf, expected_buf);
        assert_eq!(renderer, expected_renderer);
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
        let mut renderer = Renderer {
            display_width: 10,
            display_height: 10,
            first_display_line: 3,
            cursor: (6, 10).into(),
            ..Default::default()
        };

        let expected_buf = make_buf(
            &[
                "12345", "678901", "234567", "8🎸234", "567890", "123456",
                "789012", "345678", "901234", "56789ä", "bcdefg", "h",
            ],
            ':',
        );
        let expected_renderer = Renderer {
            cursor: (11, 1).into(),
            first_display_line: 0,
            scroll_needed: 3,
            first_buffer_line: 2,
            display_width: 6,
            ..renderer
        };

        let res =
            handle_event(&mut buf, &mut renderer, None, &Event::Resize(6, 10));
        assert!(
            matches!(res, EventResult::Repaint),
            "expected {:?}, got {:?}",
            EventResult::Repaint,
            res
        );
        assert_eq!(buf, expected_buf);
        assert_eq!(renderer, expected_renderer);
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
        let mut renderer = Renderer {
            display_width: 10,
            display_height: 6,
            first_buffer_line: 3,
            cursor: (8, 10).into(),
            ..Default::default()
        };

        let event = Event::Resize(10, 10);
        let expected_buf = buf.clone();
        let expected_renderer = Renderer {
            first_display_line: 0,
            first_buffer_line: 0,
            display_height: 10,
            cursor: (8, 10).into(),
            ..renderer
        };
        let res = handle_event(&mut buf, &mut renderer, None, &event);
        assert!(
            matches!(res, EventResult::Repaint),
            "expected {:?}, got {:?}",
            EventResult::Repaint,
            res
        );
        assert_eq!(buf, expected_buf);
        assert_eq!(renderer, expected_renderer);
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
        let mut renderer = Renderer {
            display_width: 10,
            display_height: 6,
            cursor: (0, 1).into(),
            ..Default::default()
        };

        let event = Event::Resize(10, 10);
        let expected_buf = buf.clone();
        let expected_renderer = Renderer { display_height: 10, ..renderer };
        let res = handle_event(&mut buf, &mut renderer, None, &event);
        assert!(
            matches!(res, EventResult::Repaint),
            "expected {:?}, got {:?}",
            EventResult::Repaint,
            res
        );
        assert_eq!(buf, expected_buf);
        assert_eq!(renderer, expected_renderer);
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
        let mut renderer = Renderer {
            display_width: 6,
            display_height: 10,
            first_display_line: 0,
            cursor: (0, 1).into(),
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
        let expected_renderer = Renderer { display_width: 10, ..renderer };
        let res =
            handle_event(&mut buf, &mut renderer, None, &Event::Resize(10, 10));
        assert!(
            matches!(res, EventResult::Repaint),
            "expected {:?}, got {:?}",
            EventResult::Repaint,
            res
        );
        assert_eq!(buf, expected_buf);
        assert_eq!(renderer, expected_renderer);
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
            ..Default::default()
        };
        let mut renderer = Renderer {
            display_width: 6,
            display_height: 10,
            first_display_line: 0,
            cursor: (1, 3).into(),
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
        let expected_renderer =
            Renderer { display_width: 10, cursor: (0, 9).into(), ..renderer };
        let res =
            handle_event(&mut buf, &mut renderer, None, &Event::Resize(10, 10));
        assert!(
            matches!(res, EventResult::Repaint),
            "expected {:?}, got {:?}",
            EventResult::Repaint,
            res
        );
        assert_eq!(buf, expected_buf);
        assert_eq!(renderer, expected_renderer);
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
        let mut renderer = Renderer {
            display_width: 6,
            display_height: 10,
            first_buffer_line: 2,
            cursor: (11, 2).into(),
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
        let expected_renderer = Renderer {
            display_width: 10,
            first_buffer_line: 0,
            cursor: (6, 11).into(),
            ..renderer
        };
        let res =
            handle_event(&mut buf, &mut renderer, None, &Event::Resize(10, 10));
        assert!(
            matches!(res, EventResult::Repaint),
            "expected {:?}, got {:?}",
            EventResult::Repaint,
            res
        );
        assert_eq!(buf, expected_buf);
        assert_eq!(renderer, expected_renderer);
    }

    #[test]
    fn up_nop_if_empty_history() {
        let mut buf = make_buf(&["abcdëf🎸"], ':');
        let mut renderer = Renderer {
            display_width: 10,
            display_height: 5,
            cursor: (0, 13).into(),
            ..Default::default()
        };
        let expected_buf = buf.clone();
        let expected_renderer = renderer;
        let res = handle_event(
            &mut buf,
            &mut renderer,
            None,
            &Event::Key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)),
        );
        assert!(
            matches!(res, EventResult::Continue),
            "expected {:?}, got {:?}",
            EventResult::Continue,
            res
        );
        assert_eq!(buf, expected_buf);
        assert_eq!(renderer, expected_renderer);
    }

    #[test]
    fn down_nop_if_empty_history() {
        let mut buf = make_buf(&["abcdëf🎸"], ':');
        let mut renderer = Renderer {
            display_width: 10,
            display_height: 5,
            cursor: (0, 13).into(),
            ..Default::default()
        };
        let expected_buf = buf.clone();
        let expected_renderer = renderer;
        let res = handle_event(
            &mut buf,
            &mut renderer,
            None,
            &Event::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)),
        );
        assert!(
            matches!(res, EventResult::Continue),
            "expected {:?}, got {:?}",
            EventResult::Continue,
            res
        );
        assert_eq!(buf, expected_buf);
        assert_eq!(renderer, expected_renderer);
    }

    #[test]
    fn esc_nop_if_empty_history() {
        let mut buf = make_buf(&["abcdëf🎸"], ':');
        let mut renderer = Renderer {
            display_width: 10,
            display_height: 10,
            cursor: (0, 13).into(),
            ..Default::default()
        };
        let expected_buf = buf.clone();
        let expected_renderer = renderer;
        let res = handle_event(
            &mut buf,
            &mut renderer,
            None,
            &Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        );
        assert!(
            matches!(res, EventResult::Repaint),
            "expected {:?}, got {:?}",
            EventResult::Repaint,
            res
        );
        assert_eq!(buf, expected_buf);
        assert_eq!(renderer, expected_renderer);
    }

    #[test]
    fn down_nop_when_not_viewing_history() {
        let mut buf = make_buf(&["abcdëf🎸"], ':');
        buf.draft = Some("abcdë🎸".to_owned());
        let mut renderer = Renderer {
            display_width: 10,
            display_height: 5,
            cursor: (0, 13).into(),
            ..Default::default()
        };
        let mut hs = Some(HistoryStack::new());
        let expected_buf = buf.clone();
        let expected_renderer = renderer;
        let expected_hs = hs.clone();
        let res = handle_event(
            &mut buf,
            &mut renderer,
            hs.as_mut(),
            &Event::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)),
        );
        assert!(
            matches!(res, EventResult::Continue),
            "expected {:?}, got {:?}",
            EventResult::Continue,
            res
        );
        assert_eq!(buf, expected_buf);
        assert_eq!(renderer, expected_renderer);
        assert_eq!(hs, expected_hs);
    }

    #[test]
    fn enter_adds_non_empty_input_to_history() {
        let mut buf = make_buf(&["123456789", "abc"], ':');
        buf.draft = Some("abc".to_owned());
        let mut renderer = Renderer {
            display_width: 10,
            display_height: 5,
            cursor: (1, 3).into(),
            ..Default::default()
        };
        let mut hs = Some(HistoryStack::new());
        let expected_hs = HistoryStack {
            lines: vec!["123456789abc".to_owned()],
            edited: vec![None],
            index: 1,
        };
        let res = handle_event(
            &mut buf,
            &mut renderer,
            hs.as_mut(),
            &Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        );
        assert!(
            matches!(res, EventResult::Accept),
            "expected {:?}, got {:?}",
            EventResult::Accept,
            res
        );
        assert_eq!(hs.unwrap(), expected_hs);
    }

    #[test]
    fn up_editing_input_saves_input_and_views_most_recent_history() {
        let mut buf = make_buf(&["123456789", "abc"], ':');
        let mut renderer = Renderer {
            display_width: 10,
            display_height: 5,
            cursor: (1, 3).into(),
            ..Default::default()
        };
        let hs = HistoryStack {
            lines: vec!["foo".to_owned(), "bar".to_owned(), "baz".to_owned()],
            edited: vec![None, None, None],
            index: 3,
        };
        let expected_buf = EditBuffer {
            lines: vec![":baz".into()],
            input_start: (0, 1).into(),
            prompt: Some(':'),
            draft: Some("123456789abc".to_owned()),
        };
        let expected_hs = HistoryStack { index: 2, ..hs.clone() };
        let expected_renderer = Renderer { cursor: (0, 4).into(), ..renderer };
        let event = Event::Key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        let mut hs = Some(hs);
        let res = handle_event(&mut buf, &mut renderer, hs.as_mut(), &event);
        assert!(
            matches!(res, EventResult::Repaint),
            "expected {:?}, got {:?}",
            EventResult::Repaint,
            res
        );
        assert_eq!(buf, expected_buf);
        assert_eq!(renderer, expected_renderer);
        assert_eq!(hs.unwrap(), expected_hs);
    }

    #[test]
    fn up_editing_history_saves_edited_and_views_next_older_history() {
        let mut renderer = Renderer {
            display_width: 10,
            display_height: 5,
            cursor: (0, 3).into(),
            ..Default::default()
        };
        let mut buf = EditBuffer {
            lines: vec![":ba".into()],
            input_start: (0, 1).into(),
            prompt: Some(':'),
            draft: Some("123456789abc".to_owned()),
        };
        let hs = HistoryStack {
            lines: vec!["foo".to_owned(), "bar".to_owned(), "baz".to_owned()],
            edited: vec![None, None, None],
            index: 1,
        };

        let expected_renderer = Renderer { cursor: (0, 4).into(), ..renderer };
        let expected_buf =
            EditBuffer { lines: vec![":foo".into()], ..buf.clone() };
        let expected_hs = HistoryStack {
            index: 0,
            edited: vec![None, Some("ba".to_owned()), None],
            ..hs.clone()
        };
        let event = Event::Key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        let mut hs = Some(hs);
        let res = handle_event(&mut buf, &mut renderer, hs.as_mut(), &event);
        assert!(
            matches!(res, EventResult::Repaint),
            "expected {:?}, got {:?}",
            EventResult::Repaint,
            res
        );
        assert_eq!(buf, expected_buf);
        assert_eq!(renderer, expected_renderer);
        assert_eq!(hs.unwrap(), expected_hs);
    }
    #[test]
    fn accepting_history_item_resets_history_stack() {
        let mut renderer = Renderer {
            display_width: 10,
            display_height: 5,
            cursor: (0, 3).into(),
            ..Default::default()
        };
        let mut buf = EditBuffer {
            lines: vec![":ba".into()],
            input_start: (0, 1).into(),
            prompt: Some(':'),
            draft: Some("123456789abc".to_owned()),
        };
        let hs = HistoryStack {
            lines: vec!["foo".to_owned(), "bar".to_owned(), "baz".to_owned()],
            edited: vec![None, None, None],
            index: 1,
        };

        let expected_renderer = Renderer { cursor: (0, 4).into(), ..renderer };
        let expected_buf =
            EditBuffer { lines: vec![":foo".into()], ..buf.clone() };
        let expected_hs = HistoryStack {
            index: 0,
            edited: vec![None, Some("ba".to_owned()), None],
            ..hs.clone()
        };
        let event = Event::Key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        let mut hs = Some(hs);
        let res = handle_event(&mut buf, &mut renderer, hs.as_mut(), &event);
        assert!(
            matches!(res, EventResult::Repaint),
            "expected {:?}, got {:?}",
            EventResult::Repaint,
            res
        );
        assert_eq!(buf, expected_buf);
        assert_eq!(renderer, expected_renderer);
        assert_eq!(hs.as_ref(), Some(&expected_hs));

        let expected_hs = HistoryStack {
            lines: vec![
                "foo".to_owned(),
                "bar".to_owned(),
                "baz".to_owned(),
                "foo".to_owned(),
            ],
            edited: vec![None, None, None, None],
            index: 4,
        };
        let event =
            Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        let res = handle_event(&mut buf, &mut renderer, hs.as_mut(), &event);
        assert!(
            matches!(res, EventResult::Accept),
            "expected {:?}, got {:?}",
            EventResult::Accept,
            res
        );
        assert_eq!(buf, expected_buf);
        assert_eq!(renderer, expected_renderer);
        assert_eq!(hs.as_ref(), Some(&expected_hs));
    }

    #[test]
    fn up_viewing_history_views_next_oldest_history() {
        let mut buf = make_buf(&["baz"], ':');
        buf.draft = Some("123456789abc".to_owned());
        let mut renderer = Renderer {
            display_width: 10,
            display_height: 5,
            cursor: (0, 4).into(),
            ..Default::default()
        };
        let hs = HistoryStack {
            lines: vec!["foo".to_owned(), "bar".to_owned(), "baz".to_owned()],
            edited: vec![None, None, None],
            index: 2,
        };
        let expected_buf =
            EditBuffer { lines: vec![":bar".into()], ..buf.clone() };
        let expected_renderer = renderer;
        let expected_hs = HistoryStack { index: 1, ..hs.clone() };
        let event = Event::Key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        let mut hs = Some(hs);
        let res = handle_event(&mut buf, &mut renderer, hs.as_mut(), &event);
        assert!(
            matches!(res, EventResult::Repaint),
            "expected {:?}, got {:?}",
            EventResult::Repaint,
            res
        );
        assert_eq!(buf, expected_buf);
        assert_eq!(renderer, expected_renderer);
        assert_eq!(hs.unwrap(), expected_hs);
    }

    #[test]
    fn up_viewing_history_nop_after_oldest_history() {
        let mut buf = make_buf(&["foo"], ':');
        buf.draft = Some("123456789abc".to_owned());
        let mut renderer = Renderer {
            display_width: 10,
            display_height: 5,
            cursor: (0, 4).into(),
            ..Default::default()
        };
        let hs = HistoryStack {
            lines: vec!["foo".to_owned(), "bar".to_owned(), "baz".to_owned()],
            edited: vec![None, None, None],
            index: 0,
        };
        let expected_buf = buf.clone();
        let expected_renderer = renderer;
        let expected_hs = hs.clone();
        let event = Event::Key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        let mut hs = Some(hs);
        let res = handle_event(&mut buf, &mut renderer, hs.as_mut(), &event);
        assert!(
            matches!(res, EventResult::Repaint),
            "expected {:?}, got {:?}",
            EventResult::Repaint,
            res
        );
        assert_eq!(buf, expected_buf);
        assert_eq!(renderer, expected_renderer);
        assert_eq!(hs.unwrap(), expected_hs);
    }

    #[test]
    fn down_viewing_history_views_next_newest_history() {
        let mut buf = make_buf(&["foo"], ':');
        buf.draft = Some("123456789abc".to_owned());
        let mut renderer = Renderer {
            display_width: 10,
            display_height: 5,
            cursor: (0, 4).into(),
            ..Default::default()
        };
        let hs = HistoryStack {
            lines: vec!["foo".to_owned(), "bar".to_owned(), "baz".to_owned()],
            edited: vec![None, None, None],
            index: 0,
        };
        let expected_buf =
            EditBuffer { lines: vec![":bar".into()], ..buf.clone() };
        let expected_renderer = renderer;
        let expected_hs = HistoryStack { index: 1, ..hs.clone() };
        let event =
            Event::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        let mut hs = Some(hs);
        let res = handle_event(&mut buf, &mut renderer, hs.as_mut(), &event);
        assert!(
            matches!(res, EventResult::Repaint),
            "expected {:?}, got {:?}",
            EventResult::Repaint,
            res
        );
        assert_eq!(buf, expected_buf);
        assert_eq!(renderer, expected_renderer);
        assert_eq!(hs.unwrap(), expected_hs);
    }

    #[test]
    fn down_from_newest_history_returns_to_editing_draft() {
        let mut renderer = Renderer {
            display_width: 10,
            display_height: 5,
            cursor: (0, 4).into(),
            ..Default::default()
        };
        let mut buf = EditBuffer {
            lines: vec![":baz".into()],
            prompt: Some(':'),
            input_start: (0, 1).into(),
            draft: Some("123456789abc".to_owned()),
        };
        let hs = HistoryStack {
            lines: vec!["foo".to_owned(), "bar".to_owned(), "baz".to_owned()],
            edited: vec![None, None, None],
            index: 2,
        };
        let expected_renderer = Renderer { cursor: (1, 3).into(), ..renderer };
        let expected_buf = EditBuffer {
            lines: vec![":123456789".into(), "abc".into()],
            draft: None,
            ..buf.clone()
        };
        let expected_hs = HistoryStack { index: 3, ..hs.clone() };
        let event =
            Event::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        let mut hs = Some(hs);
        let res = handle_event(&mut buf, &mut renderer, hs.as_mut(), &event);
        assert!(
            matches!(res, EventResult::Repaint),
            "expected {:?}, got {:?}",
            EventResult::Repaint,
            res
        );
        assert_eq!(buf, expected_buf);
        assert_eq!(renderer, expected_renderer);
        assert_eq!(hs.unwrap(), expected_hs);
    }

    #[test]
    fn esc_editing_history_edits_draft() {
        let hs = HistoryStack {
            lines: vec!["foo".to_owned(), "bar".to_owned(), "baz".to_owned()],
            edited: vec![None, None, None],
            index: 0,
        };
        let mut buf = make_buf(&["fo"], ':');
        buf.draft = Some("123456789abc".to_owned());
        let mut renderer = Renderer {
            display_width: 10,
            display_height: 5,
            cursor: (0, 3).into(),
            ..Default::default()
        };
        let expected_buf = make_buf(&["123456789", "abc"], ':');
        let expected_renderer = Renderer { cursor: (1, 3).into(), ..renderer };
        let expected_hs = HistoryStack {
            lines: vec!["foo".to_owned(), "bar".to_owned(), "baz".to_owned()],
            edited: vec![None, None, None],
            index: 3,
        };
        let mut hs = Some(hs);
        let res = handle_event(
            &mut buf,
            &mut renderer,
            hs.as_mut(),
            &Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        );
        assert!(
            matches!(res, EventResult::Repaint),
            "expected {:?}, got {:?}",
            EventResult::Repaint,
            res
        );
        assert_eq!(buf, expected_buf);
        assert_eq!(renderer, expected_renderer);
        assert_eq!(hs.unwrap(), expected_hs);
    }

    #[test]
    fn esc_nop_when_editing_input() {
        let mut buf = make_buf(&["some text"], ':');
        let mut renderer = Renderer {
            display_width: 10,
            display_height: 5,
            cursor: (0, 10).into(),
            ..Default::default()
        };
        let expected_buf = buf.clone();
        let expected_renderer = renderer;
        let res = handle_event(
            &mut buf,
            &mut renderer,
            None,
            &Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        );
        assert!(
            matches!(res, EventResult::Repaint),
            "expected {:?}, got {:?}",
            EventResult::Repaint,
            res
        );
        assert_eq!(buf, expected_buf);
        assert_eq!(renderer, expected_renderer);
    }

    #[test]
    fn esc_viewing_history_after_editing_input_edits_input() {
        let mut buf = EditBuffer {
            lines: vec![":foo".into()],
            input_start: (0, 1).into(),
            prompt: Some(':'),
            draft: Some("123456789abc".to_owned()),
        };
        let mut renderer = Renderer {
            display_width: 10,
            display_height: 5,
            cursor: (0, 4).into(),
            ..Default::default()
        };
        let hs = HistoryStack {
            lines: vec!["foo".to_owned(), "bar".to_owned(), "baz".to_owned()],
            edited: vec![None, None, None],
            index: 0,
        };

        let expected_buf = EditBuffer {
            lines: vec![":123456789".into(), "abc".into()],
            draft: None,
            ..buf.clone()
        };
        let expected_renderer = Renderer { cursor: (1, 3).into(), ..renderer };
        let expected_hs = HistoryStack { index: 3, ..hs.clone() };
        let mut hs = Some(hs);
        let res = handle_event(
            &mut buf,
            &mut renderer,
            hs.as_mut(),
            &Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        );
        assert!(
            matches!(res, EventResult::Repaint),
            "expected {:?}, got {:?}",
            EventResult::Repaint,
            res
        );
        assert_eq!(buf, expected_buf);
        assert_eq!(renderer, expected_renderer);
        assert_eq!(hs.unwrap(), expected_hs);
    }
}
