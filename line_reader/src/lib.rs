mod history_stack;
mod renderer;

use std::io::{self, BufRead, Write};
use std::ops::ControlFlow;
use std::time::Duration;

use crossterm::cursor::{self};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
use crossterm::terminal;

use unicode_width::UnicodeWidthChar;

use crate::history_stack::HistoryStack;
use crate::renderer::Coord2D;
use crate::renderer::DimWH;
use crate::renderer::View;

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

#[derive(Debug, Default, Clone, PartialEq)]
pub struct LineReader {
    history: Option<HistoryStack>,
}

#[derive(Debug, Default, Clone)]
pub struct LineReaderOptions {
    pub prompt: Option<char>,
    pub history: bool,
}

#[must_use]
pub fn native_eol() -> &'static str {
    if std::env::consts::FAMILY == "windows" { "\r\n" } else { "\n" }
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
        let term_size: DimWH = terminal::size()?.into();
        let (_, first_display_line) = cursor::position()?;

        // View has Drop impl to ensure terminal reset to cooked
        // and cursor not hidden.
        let mut view = View::new(term_size, first_display_line, options.prompt);
        terminal::enable_raw_mode()?;

        // instantiate and/or get history stack, if necessary
        let history = if options.history {
            self.history.get_or_insert_with(HistoryStack::new);
            &mut self.history
        } else {
            &mut None
        };

        let mut input_buffer = String::with_capacity(80);

        view.repaint(&input_buffer)?;
        while pump_event(&mut input_buffer, &mut view, history.as_mut())?
            .is_continue()
        {
            view.repaint(&input_buffer)?;
        }

        let _ = handle_end(&input_buffer, &mut view);
        let mut stdout = io::stdout().lock();
        stdout.write_all(b"\r\n")?;
        stdout.flush()?;

        let prev_bytes = output_buffer.len();
        output_buffer.push_str(&input_buffer);
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

#[cfg(not(tarpaulin_include))]
fn pump_event(
    buffer: &mut String,
    view: &mut View,
    history: Option<&mut HistoryStack>,
) -> io::Result<ControlFlow<()>> {
    let event = event::read()?;
    handle_event(buffer, view, history, &event)
}

#[cfg(not(tarpaulin_include))]
fn handle_event(
    buffer: &mut String,
    view: &mut View,
    history: Option<&mut HistoryStack>,
    event: &Event,
) -> io::Result<ControlFlow<()>> {
    match event {
        Event::Key(event) => {
            if event.kind == KeyEventKind::Press {
                Ok(handle_key_event(buffer, view, history, event))
            } else {
                Ok(ControlFlow::Continue(()))
            }
        }
        &Event::Resize(mut w, mut h) => {
            while let Ok(true) = event::poll(Duration::from_millis(50)) {
                if let Event::Resize(w1, h1) = event::read()? {
                    (w, h) = (w1, h1);
                }
            }
            let cursor_position: Coord2D = cursor::position()?.into();
            Ok(handle_resize(buffer, view, DimWH(w, h), cursor_position))
        }
        Event::FocusGained
        | Event::FocusLost
        | Event::Mouse(_)
        | Event::Paste(_) => Ok(ControlFlow::Continue(())),
    }
}

fn handle_resize(
    buffer: &str,
    view: &mut View,
    size: DimWH,
    cursor_position: Coord2D,
) -> ControlFlow<()> {
    view.resize(size, cursor_position, buffer);
    ControlFlow::Continue(())
}

fn handle_key_event(
    buffer: &mut String,
    view: &mut View,
    history: Option<&mut HistoryStack>,
    event: &KeyEvent,
) -> ControlFlow<()> {
    match event.code {
        KeyCode::Enter => {
            if let Some(history) = history {
                history.rewind();
                if !buffer.is_empty()
                    && history.last().is_none_or(|last| last != buffer)
                {
                    history.push(buffer.clone());
                }
            }
            ControlFlow::Break(())
        }
        KeyCode::Left => handle_left(buffer, view),
        KeyCode::Right => handle_right(buffer, view),
        KeyCode::Home => handle_home(view),
        KeyCode::End => handle_end(buffer, view),
        KeyCode::Backspace => handle_backspace(buffer, view),
        KeyCode::Delete => handle_delete(buffer, view),
        KeyCode::Char(c) => handle_char_input(buffer, view, c),
        KeyCode::Up => handle_up(buffer, view, history),
        KeyCode::Down => handle_down(buffer, view, history),
        KeyCode::Esc => handle_esc(buffer, view, history),
        KeyCode::Tab => handle_char_input(buffer, view, '\t'),
        _ => ControlFlow::Continue(()),
    }
}

fn handle_esc(
    buffer: &mut String,
    view: &mut View,
    history: Option<&mut HistoryStack>,
) -> ControlFlow<()> {
    if let Some(draft) = history.and_then(HistoryStack::rewind) {
        buffer.replace_range(.., &draft);
        view.set_insertion_point(buffer.len());
    }
    ControlFlow::Continue(())
}

fn handle_down(
    buffer: &mut String,
    view: &mut View,
    history: Option<&mut HistoryStack>,
) -> ControlFlow<()> {
    if let Some(history_line) = history.and_then(|h| h.next_newer(buffer)) {
        buffer.replace_range(.., history_line);
        view.set_insertion_point(buffer.len());
    }

    ControlFlow::Continue(())
}

fn handle_up(
    buffer: &mut String,
    view: &mut View,
    history: Option<&mut HistoryStack>,
) -> ControlFlow<()> {
    if let Some(line) = history.and_then(|h| h.next_older(buffer)) {
        buffer.replace_range(.., line);
        view.set_insertion_point(buffer.len());
    }

    ControlFlow::Continue(())
}

fn handle_char_input(
    buffer: &mut String,
    view: &mut View,
    c: char,
) -> ControlFlow<()> {
    // if char is zero width, but no previous chars exist to
    //  which it can  be combined, do nothing (i.e., don't accept
    // the input)
    if c != '\t'
        && c.width().unwrap_or(0) == 0
        && !buffer[..view.insertion_point()]
            .chars()
            .rev()
            .take_while(|c| *c != '\t')
            .any(|c| c.width().unwrap_or(0) > 0)
    {
        return ControlFlow::Continue(());
    }

    buffer.insert(view.insertion_point(), c);
    view.set_insertion_point(view.insertion_point() + c.len_utf8());

    ControlFlow::Continue(())
}

fn handle_backspace(buffer: &mut String, view: &mut View) -> ControlFlow<()> {
    if view.insertion_point() != 0
        && let Some((i, _)) =
            buffer[..view.insertion_point()].char_indices().next_back()
    {
        buffer.remove(i);
        view.set_insertion_point(i);
    }

    ControlFlow::Continue(())
}

fn handle_left(buffer: &str, view: &mut View) -> ControlFlow<()> {
    if view.insertion_point() != 0
        && let Some((prev_idx, _)) = buffer[..view.insertion_point()]
            .char_indices()
            .rfind(|(_, c)| *c == '\t' || c.width().unwrap_or(0) > 0)
    {
        view.set_insertion_point(prev_idx);
    }

    ControlFlow::Continue(())
}

fn handle_right(buffer: &str, view: &mut View) -> ControlFlow<()> {
    // If aleady at end, nothing to do
    if view.insertion_point() != buffer.len() {
        let next_idx = buffer[view.insertion_point()..]
            .char_indices()
            .skip(1)
            .find(|(_, c)| *c == '\t' || c.width().unwrap_or(0) > 0)
            .map_or_else(|| buffer.len(), |(i, _)| i + view.insertion_point());
        view.set_insertion_point(next_idx);
    }

    ControlFlow::Continue(())
}

fn handle_delete(buffer: &mut String, view: &mut View) -> ControlFlow<()> {
    // if at end of buffer, nothing to do
    let cur_idx = view.insertion_point();
    if cur_idx != buffer.len() {
        let next_idx = buffer[view.insertion_point()..]
            .char_indices()
            .skip(1)
            .find(|(_, c)| *c == '\t' || c.width().unwrap_or(0) > 0)
            .map_or_else(|| buffer.len(), |(i, _)| i + view.insertion_point());
        buffer.replace_range(view.insertion_point()..next_idx, "");
        view.invalidate();
    }

    ControlFlow::Continue(())
}

fn handle_home(view: &mut View) -> ControlFlow<()> {
    if view.insertion_point() != 0 {
        view.set_insertion_point(0);
    }

    ControlFlow::Continue(())
}

fn handle_end(buffer: &str, view: &mut View) -> ControlFlow<()> {
    if view.insertion_point() != buffer.len() {
        view.set_insertion_point(buffer.len());
    }

    ControlFlow::Continue(())
}

#[cfg(test)]
#[allow(clippy::unicode_not_nfc)]
mod tests {
    use super::*;
    use crate::history_stack::tests::HistoryStackBuilder;
    use crate::renderer::tests::ViewBuilder;

    use crossterm::event::KeyModifiers;
    use similar_asserts::assert_eq;

    #[test]
    fn unimplemented_event_ignored() {
        let mut buf = String::new();
        let expected_buf = buf.clone();

        let mut view = ViewBuilder::new().build();
        let expected_view = view.clone();

        let res =
            handle_event(&mut buf, &mut view, None, &Event::FocusLost).unwrap();

        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert_eq!(view, expected_view);
    }

    #[test]
    fn unimplemented_key_event_ignored() {
        let mut buf = String::new();
        let expected_buf = buf.clone();

        let mut view = ViewBuilder::new().build();
        let expected_view = view.clone();

        let res = handle_event(
            &mut buf,
            &mut view,
            None,
            &Event::Key(KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE)),
        )
        .unwrap();

        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert_eq!(view, expected_view);
    }

    #[test]
    fn enter_breaks_input_loop() {
        let res = handle_event(
            &mut String::new(),
            &mut ViewBuilder::new().build(),
            None,
            &Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        )
        .unwrap();
        assert!(res.is_break());
    }

    #[test]
    fn char_input_non_0w_inserts() {
        let mut buf = String::new();
        let expected_buf = "🎸";

        let mut view = ViewBuilder::new().with_insertion_point(0).build();

        let res = handle_event(
            &mut buf,
            &mut view,
            None,
            &Event::Key(KeyEvent::new(KeyCode::Char('🎸'), KeyModifiers::NONE)),
        )
        .unwrap();

        assert!(res.is_continue(),);
        assert_eq!(&buf, expected_buf);
        assert!(!view.is_valid());
        assert_eq!(view.insertion_point(), expected_buf.len());
    }

    #[test]
    fn char_input_0w_requires_base_char() {
        let mut buf = String::with_capacity(80);

        let mut vb = ViewBuilder::new();
        let mut view = vb.build();
        let expected_view = view.clone();

        let res = handle_event(
            &mut buf,
            &mut view,
            None,
            &Event::Key(KeyEvent::new(
                KeyCode::Char('\u{0308}'),
                KeyModifiers::NONE,
            )),
        )
        .unwrap();

        assert!(res.is_continue());
        assert!(buf.is_empty());
        assert_eq!(view, expected_view);

        buf.push('a');
        let expected_buf = "ä";

        let mut view = vb.with_insertion_point(buf.len()).build();

        let res = handle_event(
            &mut buf,
            &mut view,
            None,
            &Event::Key(KeyEvent::new(
                KeyCode::Char('\u{0308}'),
                KeyModifiers::NONE,
            )),
        )
        .unwrap();

        assert!(res.is_continue());
        assert_eq!(&buf, expected_buf);
        assert!(!view.is_valid());
        assert_eq!(view.insertion_point(), expected_buf.len());
    }

    #[test]
    fn backspace_0w() {
        let mut buf = "AëZ".to_owned();
        let mut view =
            ViewBuilder::new().with_insertion_point(buf.len() - 1).build();

        let res = handle_event(
            &mut buf,
            &mut view,
            None,
            &Event::Key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE)),
        )
        .unwrap();

        assert!(res.is_continue());
        assert_eq!(&buf, "AeZ");
        assert!(!view.is_valid());
        assert_eq!(view.insertion_point(), 2);
    }

    #[test]
    fn backspace_1w() {
        let mut buf = "AeZ".to_owned();
        let mut view =
            ViewBuilder::new().with_insertion_point(buf.len() - 1).build();

        let res = handle_event(
            &mut buf,
            &mut view,
            None,
            &Event::Key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE)),
        )
        .unwrap();

        assert!(res.is_continue());
        assert_eq!(&buf, "AZ");
        assert!(!view.is_valid());
        assert_eq!(view.insertion_point(), 1);
    }

    #[test]
    fn backspace_2w() {
        let mut buf = "a🎸z".to_owned();
        let mut view =
            ViewBuilder::new().with_insertion_point(buf.len() - 1).build();

        let res = handle_event(
            &mut buf,
            &mut view,
            None,
            &Event::Key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE)),
        )
        .unwrap();

        assert!(res.is_continue());
        assert_eq!(&buf, "az");
        assert!(!view.is_valid());
        assert_eq!(view.insertion_point(), 1);
    }

    #[test]
    fn backspace_at_input_start_does_nothing() {
        let mut buf = "input text".to_owned();
        let expected_buf = buf.clone();

        let mut view = ViewBuilder::new().with_insertion_point(0).build();
        let expected_view = view.clone();

        let res = handle_event(
            &mut buf,
            &mut view,
            None,
            &Event::Key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE)),
        )
        .unwrap();

        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert_eq!(view, expected_view);
    }

    #[test]
    fn left_from_input_start_does_nothing() {
        let mut buf = "12345".to_owned();
        let expected_buf = buf.clone();

        let mut view = ViewBuilder::new().with_insertion_point(0).build();
        let expected_view = view.clone();

        let res = handle_event(
            &mut buf,
            &mut view,
            None,
            &Event::Key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE)),
        )
        .unwrap();

        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert_eq!(view, expected_view);
    }

    #[test]
    fn left_moves_cursor_to_preceding_base_char() {
        let mut buf = "aë🎸iou".to_owned();
        let expected_buf = buf.clone();

        let mut vb = ViewBuilder::new();

        let mut view = vb.with_insertion_point(8).build();
        let res = handle_event(
            &mut buf,
            &mut view,
            None,
            &Event::Key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE)),
        )
        .unwrap();
        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert!(!view.is_valid());
        assert_eq!(view.insertion_point(), 4);

        let mut view = vb.with_insertion_point(4).build();
        let res = handle_event(
            &mut buf,
            &mut view,
            None,
            &Event::Key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE)),
        )
        .unwrap();
        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert!(!view.is_valid());
        assert_eq!(view.insertion_point(), 1);

        let mut view = vb.with_insertion_point(1).build();
        let res = handle_event(
            &mut buf,
            &mut view,
            None,
            &Event::Key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE)),
        )
        .unwrap();
        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert!(!view.is_valid());
        assert_eq!(view.insertion_point(), 0);
    }

    #[test]
    fn home_from_input_start_does_nothing() {
        let mut buf = "input text".to_owned();
        let expected_buf = buf.clone();

        let mut view = ViewBuilder::new().with_insertion_point(0).build();
        let expected_view = view.clone();

        let res = handle_event(
            &mut buf,
            &mut view,
            None,
            &Event::Key(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE)),
        )
        .unwrap();

        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert_eq!(view, expected_view);
    }

    #[test]
    fn home_moves_cursor_to_input_start() {
        let mut buf = "input text".to_owned();
        let expected_buf = buf.clone();

        let mut vb = ViewBuilder::new();
        let mut view = vb.with_insertion_point(5).build();

        let res = handle_event(
            &mut buf,
            &mut view,
            None,
            &Event::Key(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE)),
        )
        .unwrap();

        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert!(!view.is_valid());
        assert_eq!(view.insertion_point(), 0);
    }

    #[test]
    fn right_at_buffer_end_does_nothing() {
        let mut buf = "input text".to_owned();
        let expected_buf = buf.clone();

        let mut view =
            ViewBuilder::new().with_insertion_point(buf.len()).build();
        let expected_view = view.clone();

        let res = handle_event(
            &mut buf,
            &mut view,
            None,
            &Event::Key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE)),
        )
        .unwrap();

        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert_eq!(view, expected_view);
    }

    #[test]
    fn right_moves_cursor_to_next_base_char_until_end() {
        let mut buf = "aë🎸o".to_owned();
        let expected_buf = buf.clone();

        let mut vb = ViewBuilder::new();
        let mut view = vb.build();

        let res = handle_event(
            &mut buf,
            &mut view,
            None,
            &Event::Key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE)),
        )
        .unwrap();
        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert!(!view.is_valid());
        assert_eq!(view.insertion_point(), 1);

        let mut view = vb.with_insertion_point(1).build();

        let res = handle_event(
            &mut buf,
            &mut view,
            None,
            &Event::Key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE)),
        )
        .unwrap();
        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert!(!view.is_valid());
        assert_eq!(view.insertion_point(), 4);

        let mut view = vb.with_insertion_point(4).build();

        let res = handle_event(
            &mut buf,
            &mut view,
            None,
            &Event::Key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE)),
        )
        .unwrap();

        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert!(!view.is_valid());
        assert_eq!(view.insertion_point(), 8);

        let mut view = vb.with_insertion_point(8).build();

        let res = handle_event(
            &mut buf,
            &mut view,
            None,
            &Event::Key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE)),
        )
        .unwrap();

        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert!(!view.is_valid());
        assert_eq!(view.insertion_point(), 9);
    }

    #[test]
    fn end_at_buffer_end_does_nothing() {
        let mut buf = "buffer text".to_owned();
        let expected_buf = buf.clone();

        let mut view =
            ViewBuilder::new().with_insertion_point(buf.len()).build();
        let expected_view = view.clone();

        let res = handle_event(
            &mut buf,
            &mut view,
            None,
            &Event::Key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE)),
        )
        .unwrap();

        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert_eq!(view, expected_view);
    }

    #[test]
    fn end_moves_cursor_to_buffer_end() {
        let mut buf = "buffer text".to_owned();
        let expected_buf = buf.clone();

        let mut view = ViewBuilder::new().with_insertion_point(3).build();

        let res = handle_event(
            &mut buf,
            &mut view,
            None,
            &Event::Key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE)),
        )
        .unwrap();

        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert!(!view.is_valid());
        assert_eq!(view.insertion_point(), expected_buf.len());
    }

    #[test]
    fn delete_at_buffer_end_does_nothing() {
        let mut buf = "aë🎸io".to_owned();
        let expected_buf = buf.clone();

        let mut view =
            ViewBuilder::new().with_insertion_point(buf.len()).build();
        let expected_view = view.clone();

        let res = handle_event(
            &mut buf,
            &mut view,
            None,
            &Event::Key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE)),
        )
        .unwrap();

        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert_eq!(view, expected_view);
    }

    #[test]
    fn delete_removes_chars_from_cursor_to_next_base_char() {
        let mut buf = "aë🎸io".to_owned();

        let mut view = ViewBuilder::new().with_insertion_point(1).build();

        let res = handle_event(
            &mut buf,
            &mut view,
            None,
            &Event::Key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE)),
        )
        .unwrap();
        assert!(res.is_continue());
        assert_eq!(&buf, "a🎸io");
        assert!(!view.is_valid());
        assert_eq!(view.insertion_point(), 1);

        let res = handle_event(
            &mut buf,
            &mut view,
            None,
            &Event::Key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE)),
        )
        .unwrap();

        assert!(res.is_continue());
        assert_eq!(&buf, "aio");

        assert!(!view.is_valid());
        assert_eq!(view.insertion_point(), 1);

        let res = handle_event(
            &mut buf,
            &mut view,
            None,
            &Event::Key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE)),
        )
        .unwrap();
        assert!(res.is_continue());
        assert_eq!(&buf, "ao");
        assert!(!view.is_valid());
        assert_eq!(view.insertion_point(), 1);
    }

    #[test]
    fn up_nop_if_no_history() {
        let mut buf = "abcdëf🎸".to_owned();
        let expected_buf = buf.clone();

        let mut view = ViewBuilder::new().build();
        let expected_view = view.clone();

        let res = handle_event(
            &mut buf,
            &mut view,
            None,
            &Event::Key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)),
        )
        .unwrap();

        assert!(res.is_continue(),);
        assert_eq!(buf, expected_buf);
        assert_eq!(view, expected_view);
    }

    #[test]
    fn down_nop_if_no_history() {
        let mut buf = "abcdëf🎸".to_owned();
        let expected_buf = buf.clone();

        let mut view = ViewBuilder::new().build();
        let expected_view = view.clone();

        let res = handle_event(
            &mut buf,
            &mut view,
            None,
            &Event::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)),
        )
        .unwrap();

        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert_eq!(view, expected_view);
    }

    #[test]
    fn esc_nop_if_no_history() {
        let mut buf = "abcdëf🎸".to_owned();
        let expected_buf = buf.clone();

        let mut view = ViewBuilder::new().build();
        let expected_view = view.clone();

        let res = handle_event(
            &mut buf,
            &mut view,
            None,
            &Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        )
        .unwrap();

        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert_eq!(view, expected_view);
    }

    #[test]
    fn down_nop_when_not_viewing_history() {
        let mut buf = "abcdëf🎸".to_owned();
        let expected_buf = buf.clone();

        let mut view = ViewBuilder::new().build();
        let expected_view = view.clone();

        let mut hs = Some(HistoryStack::new());
        let expected_hs = hs.clone();

        let res = handle_event(
            &mut buf,
            &mut view,
            hs.as_mut(),
            &Event::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)),
        )
        .unwrap();
        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert_eq!(view, expected_view);
        assert_eq!(hs, expected_hs);
    }

    #[test]
    fn enter_adds_non_empty_input_to_history() {
        let mut buf = "123456789abc".to_owned();
        let mut view = ViewBuilder::new().build();

        let mut hs = Some(HistoryStack::new());
        let expected_hs = HistoryStackBuilder::new()
            .with_entries(&[("123456789abc", None)])
            .with_index(1)
            .build();
        let res = handle_event(
            &mut buf,
            &mut view,
            hs.as_mut(),
            &Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        )
        .unwrap();
        assert!(res.is_break());
        assert_eq!(hs.unwrap(), expected_hs);
    }

    #[test]
    fn up_editing_input_saves_input_and_views_most_recent_history() {
        let mut buf = "123456789abc".to_owned();
        let expected_buf = "baz";

        let mut view = ViewBuilder::new().build();

        let mut hs_builder = HistoryStackBuilder::new();
        hs_builder.with_draft(Some("123456789abc"));
        let mut hs = Some(
            hs_builder
                .with_entries(&[("foo", None), ("bar", None), ("baz", None)])
                .with_index(3)
                .build(),
        );
        let expected_hs = hs_builder.with_index(2).build();

        let res = handle_event(
            &mut buf,
            &mut view,
            hs.as_mut(),
            &Event::Key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)),
        )
        .unwrap();

        assert!(res.is_continue());
        assert_eq!(&buf, expected_buf);
        assert_eq!(hs.unwrap(), expected_hs);
        assert!(!view.is_valid());
        assert_eq!(view.insertion_point(), expected_buf.len());
    }

    #[test]
    fn up_editing_history_saves_edited_and_views_next_older_history() {
        let mut buf = "ba".to_owned();
        let expected_buf = "foo";

        let mut view = ViewBuilder::new().build();

        let mut hs_builder = HistoryStackBuilder::new();
        let mut hs = Some(
            hs_builder
                .with_entries(&[("foo", None), ("bar", None), ("baz", None)])
                .with_index(1)
                .with_draft(Some("123456789abc"))
                .build(),
        );
        let expected_hs = hs_builder
            .with_entries(&[("foo", None), ("bar", Some("ba")), ("baz", None)])
            .with_index(0)
            .build();

        let res = handle_event(
            &mut buf,
            &mut view,
            hs.as_mut(),
            &Event::Key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)),
        )
        .unwrap();

        assert!(res.is_continue());
        assert_eq!(hs.unwrap(), expected_hs);
        assert_eq!(&buf, expected_buf);
        assert!(!view.is_valid());
        assert_eq!(view.insertion_point(), expected_buf.len());
    }

    #[test]
    fn accepting_history_item_resets_history_stack() {
        let mut buf = "ba".to_owned();
        let expected_buf = "foo";

        let mut view = ViewBuilder::new().build();

        let mut hs_builder = HistoryStackBuilder::new();
        hs_builder
            .with_entries(&[("foo", None), ("bar", None), ("baz", None)])
            .with_draft(Some("123456789abc"));
        let mut hs = Some(hs_builder.with_index(1).build());
        let expected_hs = hs_builder
            .with_entries(&[("foo", None), ("bar", Some("ba")), ("baz", None)])
            .with_index(0)
            .build();

        let res = handle_event(
            &mut buf,
            &mut view,
            hs.as_mut(),
            &Event::Key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)),
        )
        .unwrap();

        assert!(res.is_continue());
        assert_eq!(&buf, expected_buf);
        assert!(!view.is_valid());
        assert_eq!(view.insertion_point(), expected_buf.len());
        assert_eq!(hs.as_ref(), Some(&expected_hs));

        let expected_hs = hs_builder
            .with_entries(&[
                ("foo", None),
                ("bar", None),
                ("baz", None),
                ("foo", None),
            ])
            .with_index(4)
            .with_draft(None)
            .build();
        let res = handle_event(
            &mut buf,
            &mut view,
            hs.as_mut(),
            &Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        )
        .unwrap();

        assert!(res.is_break());
        assert_eq!(&buf, expected_buf);
        assert_eq!(hs.as_ref(), Some(&expected_hs));
    }

    #[test]
    fn up_viewing_history_views_next_oldest_history() {
        let mut buf = "baz".to_owned();
        let expected_buf = "bar";

        let mut view = ViewBuilder::new().build();

        let mut hs_builder = HistoryStackBuilder::new();
        hs_builder
            .with_entries(&[("foo", None), ("bar", None), ("baz", None)])
            .with_draft(Some("123456789abc"));
        let expected_hs = hs_builder.with_index(1).build();
        let mut hs = Some(hs_builder.with_index(2).build());

        let res = handle_event(
            &mut buf,
            &mut view,
            hs.as_mut(),
            &Event::Key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)),
        )
        .unwrap();

        assert!(res.is_continue());
        assert_eq!(&buf, expected_buf);
        assert!(!view.is_valid());
        assert_eq!(view.insertion_point(), expected_buf.len());
        assert_eq!(hs.unwrap(), expected_hs);
    }

    #[test]
    fn up_viewing_history_nop_after_oldest_history() {
        let mut buf = "foo".to_owned();

        let mut view = ViewBuilder::new().build();

        let mut hs_builder = HistoryStackBuilder::new();
        hs_builder
            .with_entries(&[("foo", None), ("bar", None), ("baz", None)])
            .with_draft(Some("123456789abc"));
        let expected_hs = hs_builder.with_index(0).build();
        let mut hs = Some(hs_builder.build());

        let expected_buf = buf.clone();
        let expected_view = view.clone();

        let res = handle_event(
            &mut buf,
            &mut view,
            hs.as_mut(),
            &Event::Key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)),
        )
        .unwrap();

        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert_eq!(view, expected_view);
        assert_eq!(hs.unwrap(), expected_hs);
    }

    #[test]
    fn down_viewing_history_views_next_newest_history() {
        let mut buf = "foo".to_owned();
        let expected_buf = "bar";

        let mut view = ViewBuilder::new().build();

        let mut hs_builder = HistoryStackBuilder::new();
        hs_builder
            .with_entries(&[("foo", None), ("bar", None), ("baz", None)])
            .with_draft(Some("123456789abc"));
        let expected_hs = hs_builder.with_index(1).build();
        let mut hs = Some(hs_builder.with_index(0).build());

        let res = handle_event(
            &mut buf,
            &mut view,
            hs.as_mut(),
            &Event::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)),
        )
        .unwrap();

        assert!(res.is_continue());
        assert_eq!(&buf, expected_buf);
        assert!(!view.is_valid());
        assert_eq!(view.insertion_point(), expected_buf.len());
        assert_eq!(hs.unwrap(), expected_hs);
    }

    #[test]
    fn down_from_newest_history_returns_to_editing_draft() {
        let draft = "123456789abc";

        let mut buf = "baz".to_owned();
        let expected_buf = draft;

        let mut view = ViewBuilder::new().build();

        let mut hs_builder = HistoryStackBuilder::new();
        hs_builder.with_entries(&[("foo", None), ("bar", None), ("baz", None)]);
        let expected_hs =
            hs_builder.with_index(3).with_draft(Some(draft)).build();
        let mut hs = Some(hs_builder.with_index(2).build());

        let res = handle_event(
            &mut buf,
            &mut view,
            hs.as_mut(),
            &Event::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)),
        )
        .unwrap();

        assert!(res.is_continue());
        assert_eq!(&buf, expected_buf);
        assert!(!view.is_valid());
        assert_eq!(view.insertion_point(), expected_buf.len());
        assert_eq!(hs.unwrap(), expected_hs);
    }

    #[test]
    fn esc_editing_history_edits_draft() {
        let mut hs_builder = HistoryStackBuilder::new();
        let expected_hs = hs_builder
            .with_entries(&[("foo", None), ("bar", None), ("baz", None)])
            .with_index(3)
            .build();
        let mut hs = Some(
            hs_builder.with_draft(Some("123456789abc")).with_index(0).build(),
        );

        let expected_buf = "123456789abc";
        let mut buf = "fo".to_owned();

        let mut view = ViewBuilder::new().build();

        let res = handle_event(
            &mut buf,
            &mut view,
            hs.as_mut(),
            &Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        )
        .unwrap();

        assert!(res.is_continue());
        assert_eq!(&buf, expected_buf);
        assert!(!view.is_valid());
        assert_eq!(view.insertion_point(), expected_buf.len());
        assert_eq!(hs.unwrap(), expected_hs);
    }

    #[test]
    fn esc_nop_when_editing_input() {
        let mut buf = "some text".to_owned();
        let mut view = ViewBuilder::new().build();

        let expected_buf = buf.clone();
        let expected_view = view.clone();

        let res = handle_event(
            &mut buf,
            &mut view,
            None,
            &Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        )
        .unwrap();

        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert_eq!(view, expected_view);
    }

    #[test]
    fn esc_viewing_history_after_editing_input_edits_input() {
        let mut buf = "foo".to_owned();
        let mut view = ViewBuilder::new().build();
        let mut hs_builder = HistoryStackBuilder::new();
        hs_builder.with_entries(&[("foo", None), ("bar", None), ("baz", None)]);
        let hs =
            hs_builder.with_draft(Some("123456789abc")).with_index(0).build();

        let expected_buf = "123456789abc";
        let expected_hs = hs_builder.with_index(3).with_draft(None).build();

        let mut hs = Some(hs);
        let res = handle_event(
            &mut buf,
            &mut view,
            hs.as_mut(),
            &Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        )
        .unwrap();

        assert!(res.is_continue());
        assert_eq!(&buf, expected_buf);
        assert_eq!(view.insertion_point(), expected_buf.len());
        assert!(!view.is_valid());
        assert_eq!(hs.unwrap(), expected_hs);
    }

    #[test]
    fn resize_with_no_change_does_nothing() {
        let size = DimWH(10, 5);
        let cursor_pos = Coord2D(11, 0);

        let buf = "buffer text".to_owned();

        let mut view = ViewBuilder::new()
            .with_size(size)
            .with_insertion_point(buf.len())
            .with_cursor_position(cursor_pos)
            .build();
        let expected_view = view.clone();

        let res = handle_resize(&buf, &mut view, size, cursor_pos);

        assert!(res.is_continue());
        assert_eq!(view, expected_view);
    }

    #[test]
    fn resize_saves_values_and_revalidates() {
        let buf =
            "0123456789012345678901234567890123456789012345678".to_owned();

        let mut vb = ViewBuilder::new();
        let mut view = vb
            .with_insertion_point(buf.len())
            .with_size(DimWH(80, 24))
            .with_cursor_position(Coord2D(buf.len().try_into().unwrap(), 23))
            .with_first_display_line(23)
            .build();

        let expected_view = vb
            .with_size(DimWH(10, 5))
            .with_first_display_line(0)
            .with_cursor_position(Coord2D(0, 4))
            .with_visible_chars(9..buf.len())
            .build();
        let res = handle_resize(&buf, &mut view, DimWH(10, 5), Coord2D(0, 4));

        assert!(res.is_continue());
        assert!(view.is_valid());
        assert_eq!(view, expected_view);
    }
}
