mod history_stack;
mod renderer;

use std::io::{self, BufRead, Write};
use std::ops::ControlFlow;
use std::sync::LazyLock;
use std::time::Duration;

use crossterm::cursor::{self};
use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use crossterm::terminal;

use regex::Regex;

use unicode_segmentation::UnicodeSegmentation;

use unicode_width::UnicodeWidthChar;

use crate::history_stack::HistoryStack;
use crate::renderer::Coord2D;
use crate::renderer::DimWH;
use crate::renderer::View;

pub trait LineInput {
    /// # Errors
    ///
    /// Will return `io::Error` if an error is encountered reading a line
    fn read(
        &mut self,
        buffer: &mut String,
        options: &LineInputOptions,
    ) -> io::Result<usize>;
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct InputEditor {
    history: Option<HistoryStack>,
}

#[derive(Debug, Default, Clone)]
pub struct LineInputOptions {
    pub prompt: Option<char>,
    pub history: bool,
    pub indent: Option<String>,
}

#[must_use]
pub fn native_eol() -> &'static str {
    if std::env::consts::FAMILY == "windows" { "\r\n" } else { "\n" }
}

impl InputEditor {
    #[must_use]
    pub fn new() -> InputEditor {
        InputEditor { ..Default::default() }
    }

    #[cfg(not(tarpaulin_include))]
    fn accept_line(
        &mut self,
        output_buffer: &mut String,
        options: &LineInputOptions,
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

        if let Some(indent) = options.indent.as_ref() {
            input_buffer.push_str(indent);
            view.set_insertion_point(input_buffer.len());
        }

        view.repaint(&input_buffer)?;
        while pump_event(&mut input_buffer, &mut view, history.as_mut())?
            .is_continue()
        {
            view.repaint(&input_buffer)?;
        }

        let _ = handle_cursor_to_end(&input_buffer, &mut view);
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
impl LineInput for InputEditor {
    fn read(
        &mut self,
        buffer: &mut String,
        options: &LineInputOptions,
    ) -> io::Result<usize> {
        self.accept_line(buffer, options)
    }
}

impl<T> LineInput for T
where
    T: BufRead,
{
    fn read(
        &mut self,
        buffer: &mut String,
        _options: &LineInputOptions,
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
        Event::Key(event) if event.is_press() => Ok(handle_key_pressed(
            (event.code, event.modifiers),
            buffer,
            view,
            history,
        )),
        &Event::Resize(mut w, mut h) => {
            while let Ok(true) = event::poll(Duration::from_millis(50)) {
                if let Event::Resize(w1, h1) = event::read()? {
                    (w, h) = (w1, h1);
                }
            }
            let cursor_position: Coord2D = cursor::position()?.into();
            Ok(handle_resize(buffer, view, DimWH(w, h), cursor_position))
        }
        Event::Key(_)
        | Event::FocusGained
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

fn handle_key_pressed(
    key: (KeyCode, KeyModifiers),
    buffer: &mut String,
    view: &mut View,
    history: Option<&mut HistoryStack>,
) -> ControlFlow<()> {
    // decode command
    let command =
        if let (KeyCode::Char(ch), KeyModifiers::NONE | KeyModifiers::SHIFT) =
            key
        {
            EditCommand::CharInput(ch)
        } else if let Some(binding) =
            KEY_BINDINGS.iter().find(|binding| binding.key == key)
        {
            binding.command
        } else {
            return ControlFlow::Continue(());
        };

    // dispatch command
    match command {
        EditCommand::CharInput(ch) => handle_char_input(buffer, view, ch),
        EditCommand::Backspace => handle_backspace(buffer, view),
        EditCommand::Delete => handle_delete(buffer, view),
        EditCommand::HistoryNextBack => {
            handle_history_next_back(buffer, view, history)
        }
        EditCommand::HistoryNext => handle_history_next(buffer, view, history),
        EditCommand::RestoreDraft => {
            handle_restore_draft(buffer, view, history)
        }
        EditCommand::CursorLeft => handle_cursor_left(buffer, view),
        EditCommand::CursorRight => handle_cursor_right(buffer, view),
        EditCommand::CursorToStart => handle_cursor_to_start(view),
        EditCommand::CursorToEnd => handle_cursor_to_end(buffer, view),
        EditCommand::DeleteToStart => handle_delete_to_start(buffer, view),
        EditCommand::DeleteToEnd => handle_delete_to_end(buffer, view),
        EditCommand::AcceptLine => handle_accept_line(buffer, history),
        EditCommand::HistoryRFind => {
            handle_history_rfind(buffer, view, history)
        }
        EditCommand::HistoryFind => handle_history_find(buffer, view, history),
        EditCommand::Indent => handle_indent(buffer, view),
        EditCommand::Dedent => handle_dedent(buffer, view),
        EditCommand::CursorSpanLeft => handle_cursor_span_left(buffer, view),
        EditCommand::CursorSpanRight => handle_cursor_span_right(buffer, view),
        EditCommand::DeleteSpanLeft => handle_delete_span_left(buffer, view),
        EditCommand::DeleteSpanRight => handle_delete_span_right(buffer, view),
    }
}

fn handle_accept_line(
    buffer: &str,
    history: Option<&mut HistoryStack>,
) -> ControlFlow<()> {
    if let Some(history) = history {
        history.rewind();
        if !buffer.is_empty()
            && history.last().is_none_or(|last| last != buffer)
        {
            history.push(buffer.to_string());
        }
    }
    ControlFlow::Break(())
}

fn handle_restore_draft(
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

fn handle_history_next(
    buffer: &mut String,
    view: &mut View,
    history: Option<&mut HistoryStack>,
) -> ControlFlow<()> {
    if let Some(history_line) = history.and_then(|h| h.next_newer()) {
        buffer.replace_range(.., history_line);
        view.set_insertion_point(buffer.len());
    }

    ControlFlow::Continue(())
}

fn handle_history_next_back(
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

fn handle_cursor_left(buffer: &str, view: &mut View) -> ControlFlow<()> {
    if view.insertion_point() != 0
        && let Some((prev_idx, _)) = buffer[..view.insertion_point()]
            .char_indices()
            .rfind(|(_, c)| *c == '\t' || c.width().unwrap_or(0) > 0)
    {
        view.set_insertion_point(prev_idx);
    }

    ControlFlow::Continue(())
}

fn handle_cursor_right(buffer: &str, view: &mut View) -> ControlFlow<()> {
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

fn handle_delete_to_start(
    buffer: &mut String,
    view: &mut View,
) -> ControlFlow<()> {
    // if at start of buffer, nothing to do
    if view.insertion_point() != 0 {
        buffer.replace_range(..view.insertion_point(), "");
        view.set_insertion_point(0);
    }

    ControlFlow::Continue(())
}

fn handle_delete_to_end(
    buffer: &mut String,
    view: &mut View,
) -> ControlFlow<()> {
    // if at end of buffer, nothing to do
    if view.insertion_point() != buffer.len() {
        buffer.replace_range(view.insertion_point().., "");
        view.set_insertion_point(buffer.len());
    }

    ControlFlow::Continue(())
}

fn handle_cursor_to_start(view: &mut View) -> ControlFlow<()> {
    if view.insertion_point() != 0 {
        view.set_insertion_point(0);
    }

    ControlFlow::Continue(())
}

fn handle_cursor_to_end(buffer: &str, view: &mut View) -> ControlFlow<()> {
    if view.insertion_point() != buffer.len() {
        view.set_insertion_point(buffer.len());
    }

    ControlFlow::Continue(())
}

fn handle_history_find(
    buffer: &mut String,
    view: &mut View,
    history: Option<&mut HistoryStack>,
) -> ControlFlow<()> {
    if let Some(line) =
        history.and_then(|h| h.find(&buffer[..view.insertion_point()]))
    {
        buffer.replace_range(.., line);
        view.invalidate();
    }

    ControlFlow::Continue(())
}

fn handle_history_rfind(
    buffer: &mut String,
    view: &mut View,
    history: Option<&mut HistoryStack>,
) -> ControlFlow<()> {
    if let Some(line) =
        history.and_then(|h| h.rfind(&buffer[..view.insertion_point()]))
    {
        buffer.replace_range(.., line);
        view.invalidate();
    }

    ControlFlow::Continue(())
}

fn handle_indent(buffer: &mut String, view: &mut View) -> ControlFlow<()> {
    // If the first buffer char is tab ('\t'), insert one additional
    // tab at start of line. If not, insert up to 4 space (' ') chars
    // at start of line, so that leading spaces are the next multiple
    // of four.
    if buffer.starts_with('\t') {
        buffer.insert(0, '\t');
        view.set_insertion_point(view.insertion_point() + 1);
    } else {
        let leading_spaces = buffer.chars().take_while(|c| *c == ' ').count();
        let next_stop = (leading_spaces + 1).next_multiple_of(4);
        let to_add = next_stop - leading_spaces;
        buffer.insert_str(0, &"    "[..to_add]);
        view.set_insertion_point(view.insertion_point() + to_add);
    }
    ControlFlow::Continue(())
}

fn handle_dedent(buffer: &mut String, view: &mut View) -> ControlFlow<()> {
    // If the first buffer char is tab ('\t'), delete it.
    // If not, delete up to 4 leading spaces so that the
    // number of remaining leading spaces is a multple of four.
    if buffer.starts_with('\t') {
        buffer.remove(1);
        view.set_insertion_point(view.insertion_point().saturating_sub(1));
    } else if buffer.starts_with(' ') {
        let leading_spaces = buffer.chars().take_while(|c| *c == ' ').count();
        let previous_stop = (leading_spaces / 4).saturating_sub(1) * 4;
        let to_remove = leading_spaces.saturating_sub(previous_stop);
        buffer.replace_range(..to_remove, "");
        view.set_insertion_point(
            view.insertion_point().saturating_sub(to_remove),
        );
    }
    ControlFlow::Continue(())
}

#[derive(Debug, PartialEq, Clone, Copy)]
enum SpanType {
    Empty,
    Word,
    Space,
    Symbol,
    Other,
}

static SYMBOL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[\p{S}\p{P}]").unwrap());

fn span_type(s: &str) -> SpanType {
    if s.is_empty() {
        return SpanType::Empty;
    }
    if s.starts_with(|c: char| c.is_alphanumeric() || c == '_') {
        SpanType::Word
    } else if s.starts_with(char::is_whitespace) {
        SpanType::Space
    } else if SYMBOL.is_match(s) {
        SpanType::Symbol
    } else {
        SpanType::Other
    }
}

fn handle_cursor_span_left(buffer: &str, view: &mut View) -> ControlFlow<()> {
    if view.insertion_point() == 0 {
        return ControlFlow::Continue(());
    }

    let mut gr_idxs = buffer[..view.insertion_point()]
        .grapheme_indices(true)
        .rev()
        .skip_while(|(_, gr)| span_type(gr) == SpanType::Space);
    if let Some((idx, target_span_type)) =
        gr_idxs.next().map(|(idx, gr)| (idx, span_type(gr)))
    {
        view.set_insertion_point(
            gr_idxs
                .take_while(|(_, gr)| span_type(gr) == target_span_type)
                .last()
                .map_or(idx, |(i, _)| i),
        );
    } else {
        view.set_insertion_point(0);
    }

    ControlFlow::Continue(())
}

fn handle_cursor_span_right(buffer: &str, view: &mut View) -> ControlFlow<()> {
    let mut gr_idxs = buffer
        .grapheme_indices(true)
        .skip_while(|(i, _)| *i < view.insertion_point());
    let mut current_span_type =
        gr_idxs.next().map_or(SpanType::Empty, |(_, gr)| span_type(gr));
    if current_span_type != SpanType::Empty {
        view.set_insertion_point(
            gr_idxs
                .find(|(_, gr)| match span_type(gr) {
                    SpanType::Space => {
                        current_span_type = SpanType::Space;
                        false
                    }
                    st => st != current_span_type,
                })
                .map_or(buffer.len(), |(i, _)| i),
        );
    }
    ControlFlow::Continue(())
}

fn handle_delete_span_left(
    buffer: &mut String,
    view: &mut View,
) -> ControlFlow<()> {
    if view.insertion_point() == 0 {
        return ControlFlow::Continue(());
    }

    let mut gr_idxs = buffer[..view.insertion_point()]
        .grapheme_indices(true)
        .rev()
        .skip_while(|(_, gr)| span_type(gr) == SpanType::Space);
    let (idx, target_span_type) = gr_idxs
        .next()
        .map_or((0, SpanType::Space), |(idx, gr)| (idx, span_type(gr)));

    let span_start = gr_idxs
        .take_while(|(_, gr)| span_type(gr) == target_span_type)
        .last()
        .map_or(idx, |(i, _)| i);
    buffer.replace_range(span_start..view.insertion_point(), "");
    view.set_insertion_point(span_start);
    ControlFlow::Continue(())
}

fn handle_delete_span_right(
    buffer: &mut String,
    view: &mut View,
) -> ControlFlow<()> {
    let mut gr_idxs = buffer
        .grapheme_indices(true)
        .skip_while(|(i, _)| *i < view.insertion_point());
    let mut current_span_type =
        gr_idxs.next().map_or(SpanType::Empty, |(_, gr)| span_type(gr));
    if current_span_type != SpanType::Empty {
        let span_end = gr_idxs
            .find(|(_, gr)| match span_type(gr) {
                SpanType::Space => {
                    current_span_type = SpanType::Space;
                    false
                }
                st => st != current_span_type,
            })
            .map_or(buffer.len(), |(i, _)| i);
        buffer.replace_range(view.insertion_point()..span_end, "");
        view.invalidate();
    }
    ControlFlow::Continue(())
}

#[derive(Debug, Copy, Clone)]
enum EditCommand {
    CharInput(char),
    Backspace,
    Delete,
    HistoryNextBack,
    HistoryNext,
    RestoreDraft,
    CursorLeft,
    CursorRight,
    CursorToStart,
    CursorToEnd,
    DeleteToStart,
    DeleteToEnd,
    AcceptLine,
    HistoryRFind,
    HistoryFind,
    Indent,
    Dedent,
    CursorSpanLeft,
    CursorSpanRight,
    DeleteSpanLeft,
    DeleteSpanRight,
}

#[derive(Debug)]
struct KeyBinding {
    key: (KeyCode, KeyModifiers),
    command: EditCommand,
}

const KEY_BINDINGS: [KeyBinding; 23] = [
    KeyBinding {
        key: (KeyCode::Enter, KeyModifiers::NONE),
        command: EditCommand::AcceptLine,
    },
    KeyBinding {
        key: (KeyCode::Left, KeyModifiers::NONE),
        command: EditCommand::CursorLeft,
    },
    KeyBinding {
        key: (KeyCode::Right, KeyModifiers::NONE),
        command: EditCommand::CursorRight,
    },
    KeyBinding {
        key: (KeyCode::Home, KeyModifiers::NONE),
        command: EditCommand::CursorToStart,
    },
    KeyBinding {
        key: (KeyCode::Home, KeyModifiers::CONTROL),
        command: EditCommand::DeleteToStart,
    },
    KeyBinding {
        key: (KeyCode::End, KeyModifiers::NONE),
        command: EditCommand::CursorToEnd,
    },
    KeyBinding {
        key: (KeyCode::End, KeyModifiers::CONTROL),
        command: EditCommand::DeleteToEnd,
    },
    KeyBinding {
        key: (KeyCode::Backspace, KeyModifiers::NONE),
        command: EditCommand::Backspace,
    },
    KeyBinding {
        key: (KeyCode::Delete, KeyModifiers::NONE),
        command: EditCommand::Delete,
    },
    KeyBinding {
        key: (KeyCode::Up, KeyModifiers::NONE),
        command: EditCommand::HistoryNextBack,
    },
    KeyBinding {
        key: (KeyCode::Down, KeyModifiers::NONE),
        command: EditCommand::HistoryNext,
    },
    KeyBinding {
        key: (KeyCode::Esc, KeyModifiers::NONE),
        command: EditCommand::RestoreDraft,
    },
    KeyBinding {
        key: (KeyCode::Tab, KeyModifiers::NONE),
        command: EditCommand::Indent,
    },
    KeyBinding {
        key: (KeyCode::BackTab, KeyModifiers::SHIFT),
        command: EditCommand::Dedent,
    },
    KeyBinding {
        key: (KeyCode::F(8), KeyModifiers::NONE),
        command: EditCommand::HistoryRFind,
    },
    KeyBinding {
        key: (KeyCode::Char('r'), KeyModifiers::CONTROL),
        command: EditCommand::HistoryRFind,
    },
    KeyBinding {
        key: (KeyCode::F(8), KeyModifiers::SHIFT),
        command: EditCommand::HistoryFind,
    },
    KeyBinding {
        key: (KeyCode::Char('s'), KeyModifiers::CONTROL),
        command: EditCommand::HistoryFind,
    },
    KeyBinding {
        key: (KeyCode::Char('i'), KeyModifiers::CONTROL),
        command: EditCommand::CharInput('\t'),
    },
    KeyBinding {
        key: (KeyCode::Left, KeyModifiers::CONTROL),
        command: EditCommand::CursorSpanLeft,
    },
    KeyBinding {
        key: (KeyCode::Right, KeyModifiers::CONTROL),
        command: EditCommand::CursorSpanRight,
    },
    KeyBinding {
        key: (KeyCode::Backspace, KeyModifiers::CONTROL),
        command: EditCommand::DeleteSpanLeft,
    },
    KeyBinding {
        key: (KeyCode::Delete, KeyModifiers::CONTROL),
        command: EditCommand::DeleteSpanRight,
    },
];

#[cfg(test)]
#[allow(clippy::unicode_not_nfc)]
mod tests {
    use super::*;
    use crate::history_stack::tests::HistoryStackBuilder;
    use crate::renderer::ViewState;
    use crate::renderer::tests::ViewBuilder;

    use crossterm::event::KeyEvent;
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
        let expected_buf = "🎸!";

        let mut view = ViewBuilder::new().with_insertion_point(0).build();

        let res = handle_event(
            &mut buf,
            &mut view,
            None,
            &Event::Key(KeyEvent::new(KeyCode::Char('🎸'), KeyModifiers::NONE)),
        )
        .unwrap();

        assert!(res.is_continue(),);
        let res = handle_event(
            &mut buf,
            &mut view,
            None,
            &Event::Key(KeyEvent::new(KeyCode::Char('!'), KeyModifiers::SHIFT)),
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
    fn delete_to_start_at_start_is_nop() {
        let mut buf = "aë🎸io".to_owned();
        let mut view = ViewBuilder::new()
            .with_insertion_point(0)
            .with_state(ViewState::Valid)
            .build();
        let expected_buf = buf.clone();
        let expected_view = view.clone();
        let res = handle_event(
            &mut buf,
            &mut view,
            None,
            &Event::Key(KeyEvent::new(KeyCode::Home, KeyModifiers::CONTROL)),
        )
        .unwrap();
        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert_eq!(view, expected_view);
    }

    #[test]
    fn delete_to_start_removes_chars_before_cursor() {
        let mut buf = "aë🎸io".to_owned();
        let expected_buf = "io";
        let mut view = ViewBuilder::new()
            .with_insertion_point(8)
            .with_state(ViewState::Valid)
            .build();

        let res = handle_event(
            &mut buf,
            &mut view,
            None,
            &Event::Key(KeyEvent::new(KeyCode::Home, KeyModifiers::CONTROL)),
        )
        .unwrap();
        assert!(res.is_continue());
        assert!(!view.is_valid());
        assert_eq!(&buf, expected_buf);
        assert_eq!(view.insertion_point(), 0);
    }

    #[test]
    fn delete_to_end_at_end_is_nop() {
        let mut buf = "aë🎸io".to_owned();
        let mut view = ViewBuilder::new()
            .with_insertion_point(buf.len())
            .with_state(ViewState::Valid)
            .build();
        let expected_buf = buf.clone();
        let expected_view = view.clone();
        let res = handle_event(
            &mut buf,
            &mut view,
            None,
            &Event::Key(KeyEvent::new(KeyCode::End, KeyModifiers::CONTROL)),
        )
        .unwrap();
        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert_eq!(view, expected_view);
    }

    #[test]
    fn delete_to_end_removes_chars_from_cursor_to_end() {
        let mut buf = "aë🎸io".to_owned();
        let mut view = ViewBuilder::new()
            .with_insertion_point(4)
            .with_state(ViewState::Valid)
            .build();
        let expected_buf = "aë".to_owned();

        let res = handle_event(
            &mut buf,
            &mut view,
            None,
            &Event::Key(KeyEvent::new(KeyCode::End, KeyModifiers::CONTROL)),
        )
        .unwrap();
        assert!(res.is_continue());
        assert_eq!(buf, expected_buf);
        assert!(!view.is_valid());
        assert_eq!(view.insertion_point(), expected_buf.len());
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
        let expected_hs =
            HistoryStackBuilder::new().with_entries(&["123456789abc"]).build();
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
        let mut hs =
            Some(hs_builder.with_entries(&["foo", "bar", "baz"]).build());
        let expected_hs = hs_builder.with_index(Some(2)).build();

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
    fn accepting_history_item_resets_history_stack() {
        let mut buf = "ba".to_owned();
        let expected_buf = "foo";

        let mut view = ViewBuilder::new().build();

        let mut hs_builder = HistoryStackBuilder::new();
        hs_builder
            .with_entries(&["foo", "bar", "baz"])
            .with_draft(Some("123456789abc"));
        let mut hs = Some(hs_builder.with_index(Some(1)).build());
        let expected_hs = hs_builder
            .with_entries(&["foo", "bar", "baz"])
            .with_index(Some(0))
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
            .with_entries(&["foo", "bar", "baz", "foo"])
            .with_index(None)
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
            .with_entries(&["foo", "bar", "baz"])
            .with_draft(Some("123456789abc"));
        let expected_hs = hs_builder.with_index(Some(1)).build();
        let mut hs = Some(hs_builder.with_index(Some(2)).build());

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
            .with_entries(&["foo", "bar", "baz"])
            .with_draft(Some("123456789abc"));
        let expected_hs = hs_builder.with_index(Some(0)).build();
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
            .with_entries(&["foo", "bar", "baz"])
            .with_draft(Some("123456789abc"));
        let expected_hs = hs_builder.with_index(Some(1)).build();
        let mut hs = Some(hs_builder.with_index(Some(0)).build());

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
        hs_builder.with_entries(&["foo", "bar", "baz"]);
        let expected_hs = hs_builder.with_draft(Some(draft)).build();
        let mut hs = Some(hs_builder.with_index(Some(2)).build());

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
        let expected_hs =
            hs_builder.with_entries(&["foo", "bar", "baz"]).build();
        let mut hs = Some(
            hs_builder
                .with_draft(Some("123456789abc"))
                .with_index(Some(0))
                .build(),
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
        hs_builder.with_entries(&["foo", "bar", "baz"]);
        let hs = hs_builder
            .with_draft(Some("123456789abc"))
            .with_index(Some(0))
            .build();

        let expected_buf = "123456789abc";
        let expected_hs =
            hs_builder.with_index(Some(3)).with_draft(None).build();

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

    #[test]
    fn rfind_shows_match() {
        let mut buf = "ol".to_owned();
        let mut vb = ViewBuilder::new();
        let mut view = vb
            .with_insertion_point(buf.len())
            .with_size(DimWH(80, 24))
            .with_cursor_position(Coord2D(buf.len().try_into().unwrap(), 23))
            .with_first_display_line(23)
            .build();

        let mut hs = HistoryStackBuilder::new()
            .with_entries(&["oldest", "older", "old", "newest"])
            .build();
        let res = handle_event(
            &mut buf,
            &mut view,
            Some(&mut hs),
            &Event::Key(KeyEvent::new(KeyCode::F(8), KeyModifiers::NONE)),
        )
        .unwrap();
        assert!(res.is_continue());
        assert!(!view.is_valid());
        assert_eq!(&buf, "old");
        assert_eq!(view.insertion_point(), 2);
    }

    #[test]
    fn rfind_uses_new_prefix() {
        let mut buf = "ol".to_owned();
        let mut vb = ViewBuilder::new();
        let mut view = vb
            .with_insertion_point(buf.len())
            .with_size(DimWH(80, 24))
            .with_cursor_position(Coord2D(buf.len().try_into().unwrap(), 23))
            .with_first_display_line(23)
            .build();

        let mut hs = HistoryStackBuilder::new()
            .with_entries(&["oldest", "older", "old", "newest"])
            .build();
        let _ = handle_event(
            &mut buf,
            &mut view,
            Some(&mut hs),
            &Event::Key(KeyEvent::new(KeyCode::F(8), KeyModifiers::NONE)),
        )
        .unwrap();
        buf.replace_range(.., "ne");
        let res = handle_event(
            &mut buf,
            &mut view,
            Some(&mut hs),
            &Event::Key(KeyEvent::new(
                KeyCode::Char('r'),
                KeyModifiers::CONTROL,
            )),
        )
        .unwrap();
        assert!(res.is_continue());
        assert_eq!(&buf, "newest");
        assert_eq!(view.insertion_point(), 2);
    }

    #[test]
    fn find_shows_match() {
        let mut buf = "ol".to_owned();
        let mut vb = ViewBuilder::new();
        let mut view = vb
            .with_insertion_point(buf.len())
            .with_size(DimWH(80, 24))
            .with_cursor_position(Coord2D(buf.len().try_into().unwrap(), 23))
            .with_first_display_line(23)
            .build();

        let mut hs = HistoryStackBuilder::new()
            .with_entries(&["oldest", "older", "old", "newest"])
            .build();
        let res = handle_event(
            &mut buf,
            &mut view,
            Some(&mut hs),
            &Event::Key(KeyEvent::new(KeyCode::F(8), KeyModifiers::SHIFT)),
        )
        .unwrap();
        assert!(res.is_continue());
        assert!(!view.is_valid());
        assert_eq!(&buf, "oldest");
        assert_eq!(view.insertion_point(), 2);
    }

    #[test]
    fn find_uses_new_prefix() {
        let mut buf = "ol".to_owned();
        let mut vb = ViewBuilder::new();
        let mut view = vb
            .with_insertion_point(buf.len())
            .with_size(DimWH(80, 24))
            .with_cursor_position(Coord2D(buf.len().try_into().unwrap(), 23))
            .with_first_display_line(23)
            .build();

        let mut hs = HistoryStackBuilder::new()
            .with_entries(&["oldest", "older", "old", "newest"])
            .build();
        let _ = handle_event(
            &mut buf,
            &mut view,
            Some(&mut hs),
            &Event::Key(KeyEvent::new(KeyCode::F(8), KeyModifiers::SHIFT)),
        )
        .unwrap();
        buf.replace_range(.., "ne");
        let res = handle_event(
            &mut buf,
            &mut view,
            Some(&mut hs),
            &Event::Key(KeyEvent::new(
                KeyCode::Char('s'),
                KeyModifiers::CONTROL,
            )),
        )
        .unwrap();
        assert!(res.is_continue());
        assert_eq!(&buf, "newest");
        assert_eq!(view.insertion_point(), 2);
    }

    #[test]
    fn ctrl_i_inputs_tab() {
        let mut buf = "text".to_owned();
        let mut view = ViewBuilder::new().with_insertion_point(1).build();
        let res = handle_event(
            &mut buf,
            &mut view,
            None,
            &Event::Key(KeyEvent::new(
                KeyCode::Char('i'),
                KeyModifiers::CONTROL,
            )),
        )
        .unwrap();
        assert!(res.is_continue());
        assert_eq!(&buf, "t\text");
        assert_eq!(view.insertion_point(), 2);
    }

    #[test]
    fn tab_indents_with_tab() {
        let mut buf = "\tline".to_owned();
        let mut view =
            ViewBuilder::new().with_insertion_point(buf.len()).build();
        let res = handle_event(
            &mut buf,
            &mut view,
            None,
            &Event::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)),
        )
        .unwrap();
        assert!(res.is_continue());
        assert!(!view.is_valid());
        assert_eq!(&buf, "\t\tline");
        assert_eq!(view.insertion_point(), buf.len());
    }

    #[test]
    fn tab_indents_with_spaces() {
        let mut buf = "line".to_owned();
        let mut view = ViewBuilder::new().with_insertion_point(2).build();
        let res = handle_event(
            &mut buf,
            &mut view,
            None,
            &Event::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)),
        )
        .unwrap();
        assert!(res.is_continue());
        assert!(!view.is_valid());
        assert_eq!(&buf, "    line");
        assert_eq!(view.insertion_point(), 6);
        let mut buf = "     line".to_owned();
        let mut view = ViewBuilder::new().with_insertion_point(6).build();
        let res = handle_event(
            &mut buf,
            &mut view,
            None,
            &Event::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)),
        )
        .unwrap();
        assert!(res.is_continue());
        assert!(!view.is_valid());
        assert_eq!(&buf, "        line");
        assert_eq!(view.insertion_point(), 9);
    }

    #[test]
    fn tab_indents_correctly_with_mixed_leading_blanks() {
        let mut buf = "     \tline".to_owned();
        let mut view =
            ViewBuilder::new().with_insertion_point(buf.len()).build();
        let res = handle_event(
            &mut buf,
            &mut view,
            None,
            &Event::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)),
        )
        .unwrap();
        assert!(res.is_continue());
        assert_eq!(&buf, "        \tline");
        assert_eq!(view.insertion_point(), buf.len());

        let mut buf = "\t\t  line".to_owned();
        view.set_insertion_point(buf.len());
        let res = handle_event(
            &mut buf,
            &mut view,
            None,
            &Event::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)),
        )
        .unwrap();
        assert!(res.is_continue());
        assert_eq!(&buf, "\t\t\t  line");
        assert_eq!(view.insertion_point(), buf.len());
    }

    #[test]
    fn backtab_dedents_with_tab() {
        let mut buf = "\t\tline".to_owned();
        let mut view = ViewBuilder::new().with_insertion_point(5).build();
        let res = handle_event(
            &mut buf,
            &mut view,
            None,
            &Event::Key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT)),
        )
        .unwrap();
        assert!(res.is_continue());
        assert_eq!(&buf, "\tline");
        assert_eq!(view.insertion_point(), 4);
    }

    #[test]
    fn backtab_dedents_with_spaces() {
        let mut buf = "        line".to_owned();
        let mut view = ViewBuilder::new().with_insertion_point(10).build();
        let res = handle_event(
            &mut buf,
            &mut view,
            None,
            &Event::Key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT)),
        )
        .unwrap();
        assert!(res.is_continue());
        assert!(!view.is_valid());
        assert_eq!(&buf, "    line");
        assert_eq!(view.insertion_point(), 6);
    }

    #[test]
    fn backtab_nop_with_no_indent() {
        let mut buf = "line".to_owned();
        let mut view = ViewBuilder::new().with_insertion_point(2).build();
        let expected_buf = buf.clone();
        let expected_view = view.clone();
        let res = handle_event(
            &mut buf,
            &mut view,
            None,
            &Event::Key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT)),
        )
        .unwrap();
        assert!(res.is_continue());
        assert!(view.is_valid());
        assert_eq!(buf, expected_buf);
        assert_eq!(view, expected_view);
    }

    #[test]
    fn cursor_span_right_jumps_to_next_word() {
        let mut buf = "word \t  (())".to_owned();
        let mut view = ViewBuilder::new().with_insertion_point(2).build();
        let res = handle_event(
            &mut buf,
            &mut view,
            None,
            &Event::Key(KeyEvent::new(KeyCode::Right, KeyModifiers::CONTROL)),
        )
        .unwrap();
        assert!(res.is_continue());
        assert_eq!(8, view.insertion_point());
        assert!(!view.is_valid());

        let res = handle_event(
            &mut buf,
            &mut view,
            None,
            &Event::Key(KeyEvent::new(KeyCode::Right, KeyModifiers::CONTROL)),
        )
        .unwrap();
        assert!(res.is_continue());
        assert_eq!(12, view.insertion_point());
    }

    #[test]
    fn cursor_span_right_nop_at_end() {
        let mut buf = "chars".to_owned();
        let mut view =
            ViewBuilder::new().with_insertion_point(buf.len()).build();
        let expected_view = view.clone();
        let res = handle_event(
            &mut buf,
            &mut view,
            None,
            &Event::Key(KeyEvent::new(KeyCode::Right, KeyModifiers::CONTROL)),
        )
        .unwrap();
        assert!(res.is_continue());
        assert!(view.is_valid());
        assert_eq!(view, expected_view);
    }

    #[test]
    fn cursor_span_right_nop_on_empty_buffer() {
        let mut buf = String::new();
        let mut view =
            ViewBuilder::new().with_insertion_point(buf.len()).build();
        let expected_view = view.clone();
        let res = handle_event(
            &mut buf,
            &mut view,
            None,
            &Event::Key(KeyEvent::new(KeyCode::Right, KeyModifiers::CONTROL)),
        )
        .unwrap();
        assert!(res.is_continue());
        assert!(view.is_valid());
        assert_eq!(view, expected_view);
    }

    #[test]
    fn cursor_span_left_jumps_to_start_of_previous_word() {
        let mut buf = "    word \t  (())".to_owned();
        let mut view =
            ViewBuilder::new().with_insertion_point(buf.len() - 2).build();
        let res = handle_event(
            &mut buf,
            &mut view,
            None,
            &Event::Key(KeyEvent::new(KeyCode::Left, KeyModifiers::CONTROL)),
        )
        .unwrap();
        assert!(res.is_continue());
        assert_eq!(12, view.insertion_point());
        assert!(!view.is_valid());

        let res = handle_event(
            &mut buf,
            &mut view,
            None,
            &Event::Key(KeyEvent::new(KeyCode::Left, KeyModifiers::CONTROL)),
        )
        .unwrap();
        assert!(res.is_continue());
        assert_eq!(4, view.insertion_point());

        let res = handle_event(
            &mut buf,
            &mut view,
            None,
            &Event::Key(KeyEvent::new(KeyCode::Left, KeyModifiers::CONTROL)),
        )
        .unwrap();
        assert!(res.is_continue());
        assert_eq!(0, view.insertion_point());
    }

    #[test]
    fn cursor_span_left_nop_at_start() {
        let mut buf = "chars".to_owned();
        let mut view = ViewBuilder::new().with_insertion_point(0).build();
        let expected_view = view.clone();
        let res = handle_event(
            &mut buf,
            &mut view,
            None,
            &Event::Key(KeyEvent::new(KeyCode::Left, KeyModifiers::CONTROL)),
        )
        .unwrap();
        assert!(res.is_continue());
        assert!(view.is_valid());
        assert_eq!(view, expected_view);
    }

    #[test]
    fn cursor_span_left_nop_on_empty_buffer() {
        let mut buf = String::new();
        let mut view = ViewBuilder::new().with_insertion_point(0).build();
        let expected_view = view.clone();
        let res = handle_event(
            &mut buf,
            &mut view,
            None,
            &Event::Key(KeyEvent::new(KeyCode::Left, KeyModifiers::CONTROL)),
        )
        .unwrap();
        assert!(res.is_continue());
        assert!(view.is_valid());
        assert_eq!(view, expected_view);
    }

    #[test]
    fn delete_span_right_nop_at_end() {
        let mut buffer = "    word    \t  (())".to_owned();
        let expected_buffer = buffer.clone();
        let mut view =
            ViewBuilder::new().with_insertion_point(buffer.len()).build();
        let expected_view = view.clone();
        let res = handle_event(
            &mut buffer,
            &mut view,
            None,
            &Event::Key(KeyEvent::new(KeyCode::Delete, KeyModifiers::CONTROL)),
        )
        .unwrap();
        assert!(res.is_continue());
        assert_eq!(buffer, expected_buffer);
        assert_eq!(view, expected_view);
    }

    #[test]
    fn delete_span_right_deletes_to_next_span_end() {
        let mut buffer = "    word    \t  (())".to_owned();
        let mut view = ViewBuilder::new().with_insertion_point(2).build();

        let res = handle_event(
            &mut buffer,
            &mut view,
            None,
            &Event::Key(KeyEvent::new(KeyCode::Delete, KeyModifiers::CONTROL)),
        )
        .unwrap();
        assert!(res.is_continue());
        assert_eq!(&buffer, "  word    \t  (())");
        assert_eq!(view.insertion_point(), 2);

        let res = handle_event(
            &mut buffer,
            &mut view,
            None,
            &Event::Key(KeyEvent::new(KeyCode::Delete, KeyModifiers::CONTROL)),
        )
        .unwrap();
        assert!(res.is_continue());
        assert_eq!(&buffer, "  (())");
        assert_eq!(view.insertion_point(), 2);

        let res = handle_event(
            &mut buffer,
            &mut view,
            None,
            &Event::Key(KeyEvent::new(KeyCode::Delete, KeyModifiers::CONTROL)),
        )
        .unwrap();
        assert!(res.is_continue());
        assert_eq!(&buffer, "  ");
        assert_eq!(view.insertion_point(), 2);
    }

    #[test]
    fn delete_span_left_deletes_to_previous_span_start() {
        let mut buffer = "    word    \t  (())".to_owned();
        let mut view = ViewBuilder::new().with_insertion_point(17).build();

        let res = handle_event(
            &mut buffer,
            &mut view,
            None,
            &Event::Key(KeyEvent::new(
                KeyCode::Backspace,
                KeyModifiers::CONTROL,
            )),
        )
        .unwrap();
        assert!(res.is_continue());
        assert_eq!(&buffer, "    word    \t  ))");
        assert_eq!(view.insertion_point(), 15);

        let res = handle_event(
            &mut buffer,
            &mut view,
            None,
            &Event::Key(KeyEvent::new(
                KeyCode::Backspace,
                KeyModifiers::CONTROL,
            )),
        )
        .unwrap();
        assert!(res.is_continue());
        assert_eq!(&buffer, "    ))");
        assert_eq!(view.insertion_point(), 4);

        let res = handle_event(
            &mut buffer,
            &mut view,
            None,
            &Event::Key(KeyEvent::new(
                KeyCode::Backspace,
                KeyModifiers::CONTROL,
            )),
        )
        .unwrap();
        assert!(res.is_continue());
        assert_eq!(&buffer, "))");
        assert_eq!(view.insertion_point(), 0);
    }

    #[test]
    fn delete_span_left_at_start_is_nop() {
        let mut buffer = "    word    \t  (())".to_owned();
        let expected_buffer = buffer.clone();
        let mut view = ViewBuilder::new().with_insertion_point(0).build();
        let expected_view = view.clone();
        let res = handle_event(
            &mut buffer,
            &mut view,
            None,
            &Event::Key(KeyEvent::new(
                KeyCode::Backspace,
                KeyModifiers::CONTROL,
            )),
        )
        .unwrap();
        assert!(res.is_continue());
        assert_eq!(buffer, expected_buffer);
        assert_eq!(view, expected_view);
    }
}
