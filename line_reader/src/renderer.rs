use std::cmp;
use std::io;
use std::io::Write;
use std::ops::Range;

use crossterm::ExecutableCommand;
use crossterm::QueueableCommand;
use crossterm::cursor::Hide;
use crossterm::cursor::MoveTo;
use crossterm::cursor::Show;
use crossterm::terminal;
use crossterm::terminal::Clear;
use crossterm::terminal::ClearType;
use crossterm::terminal::ScrollUp;

#[derive(Debug, Clone, PartialEq)]
pub struct View {
    size: DimWH,
    first_display_line: u16,
    cursor_position: Coord2D,
    visible_chars: Range<usize>,
    prompt: Option<char>,
    insertion_point: usize,
    state: ViewState,
}

#[derive(Debug, Copy, Clone, PartialEq)]
pub enum ViewState {
    Valid,
    Invalid,
}

#[derive(Debug, Copy, Clone, PartialEq)]
pub struct Coord2D(pub u16, pub u16);

impl From<(u16, u16)> for Coord2D {
    fn from(v: (u16, u16)) -> Self {
        Coord2D(v.0, v.1)
    }
}

#[derive(Debug, Copy, Clone, PartialEq)]
pub struct DimWH(pub u16, pub u16);

impl From<(u16, u16)> for DimWH {
    fn from(v: (u16, u16)) -> Self {
        DimWH(v.0, v.1)
    }
}

#[must_use]
fn char_width(ch: char, width_before: u16) -> u16 {
    if ch == '\t' {
        8 - (width_before % 8)
    } else {
        use unicode_width::UnicodeWidthChar;
        ch.width().unwrap_or(0).try_into().expect("width is at most 2 columns")
    }
}

#[must_use]
fn str_width(s: &str, width_before: u16) -> u16 {
    s.chars().fold(0, |width, ch| width + char_width(ch, width + width_before))
}

impl View {
    pub fn new(
        size: DimWH,
        first_display_line: u16,
        prompt: Option<char>,
    ) -> View {
        View {
            size,
            first_display_line,
            cursor_position: Coord2D(0, first_display_line),
            visible_chars: 0..0,
            prompt,
            state: ViewState::Invalid,
            insertion_point: 0,
        }
    }

    pub fn invalidate(&mut self) {
        self.state = ViewState::Invalid;
    }

    pub fn is_valid(&self) -> bool {
        self.state == ViewState::Valid
    }

    pub fn set_insertion_point(&mut self, i: usize) {
        self.insertion_point = i;
        self.state = ViewState::Invalid;
    }

    pub fn insertion_point(&self) -> usize {
        self.insertion_point
    }

    pub fn resize(
        &mut self,
        size: DimWH,
        cursor_position: Coord2D,
        buffer: &str,
    ) {
        // If nothing has changed, nothing to do
        if self.size == size && self.cursor_position == cursor_position {
            return;
        }

        self.size = size;
        self.cursor_position = cursor_position;

        let buf_lines = self.wrap_buffer_lines(buffer);

        let ip_buf_line = buf_lines
            .iter()
            .position(|l| l.contains(&self.insertion_point))
            .expect("insertion_point is in or just after buffer");

        let first_visible_buf_line =
            ip_buf_line.saturating_sub(usize::from(self.cursor_position.1));

        let last_visible_buf_line = first_visible_buf_line
            + cmp::min(usize::from(self.size.1), buf_lines.len())
            - 1;

        self.first_display_line = u16::try_from(
            usize::from(self.cursor_position.1).saturating_sub(ip_buf_line),
        )
        .expect("first_display_line fits u16");
        self.visible_chars.start = buf_lines[first_visible_buf_line].start;
        self.visible_chars.end = buf_lines[last_visible_buf_line].end
            - usize::from(
                last_visible_buf_line == ip_buf_line
                    && self.insertion_point == buffer.len(),
            );
        self.state = ViewState::Valid;
    }

    /// If View isn't in a Valid state (i.e., `insertion_point`
    /// has changed), update and return wrapped buffer lines
    /// and amount view needs to scroll, otherwise return None.
    fn update(&mut self, buffer: &str) -> Option<u16> {
        if self.is_valid() {
            return None;
        }

        let buf_lines = self.wrap_buffer_lines(buffer);

        let mut scroll_lines = 0;

        let ip_buf_line = buf_lines
            .iter()
            .position(|s| s.contains(&self.insertion_point))
            .expect("insertion_point is in or just after buffer");

        let prompt_width = if ip_buf_line == 0 {
            self.prompt.map_or(0, |p| char_width(p, 0))
        } else {
            0
        };

        let lines_to_bottom =
            usize::from(self.size.1 - 1 - self.first_display_line);

        let mut first_visible_line = buf_lines
            .iter()
            .position(|l| l.contains(&self.visible_chars.start))
            .expect("visible_chars are in the buffer");

        let new_cursor_x = prompt_width
            + str_width(
                &buffer[buf_lines[ip_buf_line].start..self.insertion_point],
                prompt_width,
            );

        let new_cursor_y = if first_visible_line + lines_to_bottom < ip_buf_line
        {
            // insertion_point below display
            let delta = ip_buf_line - (first_visible_line + lines_to_bottom);
            scroll_lines = u16::try_from(cmp::min(
                usize::from(self.first_display_line),
                delta,
            ))
            .expect("scroll_lines fits u16");
            self.first_display_line -= scroll_lines;
            self.size.1 - 1
        } else if ip_buf_line < first_visible_line {
            // Only possible if first_display_line was 0
            first_visible_line = ip_buf_line;
            0
        } else {
            self.first_display_line
                + u16::try_from(ip_buf_line - first_visible_line)
                    .expect("new cursor y fits u16")
        };
        self.cursor_position = Coord2D(new_cursor_x, new_cursor_y);

        let last_visible_line = cmp::min(
            buf_lines.len() - 1,
            first_visible_line
                + usize::from(self.size.1 - 1 - self.first_display_line),
        );
        self.visible_chars.start = buf_lines[first_visible_line].start;
        self.visible_chars.end = buf_lines[last_visible_line].end
            - usize::from(
                last_visible_line == ip_buf_line
                    && self.insertion_point == buffer.len(),
            );

        self.state = ViewState::Valid;
        Some(scroll_lines)
    }

    /// render current buffer to display
    #[cfg(not(tarpaulin_include))]
    pub fn repaint(&mut self, buffer: &str) -> io::Result<()> {
        let Some(scroll_lines) = self.update(buffer) else {
            return Ok(());
        };

        // redraw display
        let mut stdout = io::stdout().lock();

        stdout.queue(Hide)?;

        if scroll_lines > 0 {
            stdout.queue(ScrollUp(scroll_lines))?;
        }

        stdout
            .queue(MoveTo(0, self.first_display_line))?
            .queue(Clear(ClearType::FromCursorDown))?;

        write!(
            stdout,
            "{}{}",
            self.prompt.unwrap_or_default(),
            &buffer[self.visible_chars.clone()],
        )?;

        stdout
            .queue(MoveTo(self.cursor_position.0, self.cursor_position.1))?
            .queue(Show)?
            .flush()
    }

    /// Generate list of spans representing
    /// the chars that would be displayed, wrapped
    /// to display width, leaving room for cursor
    /// at end if necessary.
    #[must_use]
    fn wrap_buffer_lines(&self, buffer: &str) -> Vec<Range<usize>> {
        let mut lines = Vec::new();
        let mut cols = self.prompt.map_or(0, |ch| char_width(ch, 0));
        let mut begin = 0;
        let mut end;
        for (i, ch) in buffer.char_indices() {
            let w = char_width(ch, cols);
            end = i;
            if self.size.0 - cols < w {
                lines.push(begin..end);
                cols = 0;
                begin = i;
            }
            cols += w;
        }

        // leave room for cursor at end, if necessary
        end = buffer.len();
        if self.insertion_point == end {
            if cols == self.size.0 {
                lines.push(begin..end);
                begin = end;
            }
            end = buffer.len() + 1;
        }
        lines.push(begin..end);

        lines
    }
}

impl Drop for View {
    #[cfg(not(tarpaulin_include))]
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
        let _ = io::stdout().execute(Show);
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub struct ViewBuilder {
        size: DimWH,
        first_display_line: u16,
        cursor_position: Coord2D,
        visible_chars: Range<usize>,
        prompt: Option<char>,
        insertion_point: usize,
        state: ViewState,
    }

    impl ViewBuilder {
        pub fn new() -> Self {
            ViewBuilder {
                size: DimWH(10, 5),
                first_display_line: 0,
                cursor_position: Coord2D(0, 0),
                visible_chars: 0..0,
                prompt: Some(':'),
                insertion_point: 0,
                state: ViewState::Valid,
            }
        }

        pub fn build(&self) -> View {
            let mut v =
                View::new(self.size, self.first_display_line, self.prompt);
            v.cursor_position = self.cursor_position;
            v.visible_chars.start = self.visible_chars.start;
            v.visible_chars.end = self.visible_chars.end;
            v.insertion_point = self.insertion_point;
            v.state = self.state;
            v
        }

        pub fn with_size(&mut self, size: DimWH) -> &mut Self {
            self.size = size;
            self
        }

        pub fn with_first_display_line(&mut self, fdl: u16) -> &mut Self {
            self.first_display_line = fdl;
            self
        }

        pub fn with_cursor_position(&mut self, pos: Coord2D) -> &mut Self {
            self.cursor_position = pos;
            self
        }

        pub fn with_visible_chars(&mut self, cs: Range<usize>) -> &mut Self {
            self.visible_chars = cs;
            self
        }

        pub fn with_prompt(&mut self, p: Option<char>) -> &mut Self {
            self.prompt = p;
            self
        }

        pub fn with_insertion_point(&mut self, i: usize) -> &mut Self {
            self.insertion_point = i;
            self
        }

        pub fn with_state(&mut self, s: ViewState) -> &mut Self {
            self.state = s;
            self
        }
    }

    #[test]
    fn coord2d_from_u16_u16() {
        let f = (169u16, 13u16);
        let t = Coord2D(169, 13);

        assert_eq!(t, Coord2D::from(f));
    }

    #[test]
    fn dimwh_from_u16_u16() {
        let f = (169u16, 13u16);
        let t = DimWH(169, 13);

        assert_eq!(t, DimWH::from(f));
    }

    #[test]
    fn update_empty_buffer_with_prompt() {
        let buffer = String::new();
        let mut vb = ViewBuilder::new();

        let mut view = vb
            .with_prompt(Some(':'))
            .with_size(DimWH(80, 24))
            .with_insertion_point(0)
            .with_cursor_position(Coord2D(0, 23))
            .with_first_display_line(23)
            .with_state(ViewState::Invalid)
            .build();

        let expected_view = vb
            .with_cursor_position(Coord2D(1, 23))
            .with_first_display_line(23)
            .with_state(ViewState::Valid)
            .build();

        let scroll_lines = view.update(&buffer);

        assert_eq!(view, expected_view);
        assert_eq!(scroll_lines, Some(0));
    }

    #[test]
    fn update_one_char_added() {
        let buffer = "\u{1f3b8}".to_owned();
        let mut vb = ViewBuilder::new();

        let mut view = vb
            .with_prompt(Some(':'))
            .with_size(DimWH(80, 24))
            .with_insertion_point(buffer.len())
            .with_cursor_position(Coord2D(1, 23))
            .with_first_display_line(23)
            .with_state(ViewState::Invalid)
            .with_visible_chars(0..0)
            .build();

        let expected_view = vb
            .with_cursor_position(Coord2D(3, 23))
            .with_visible_chars(0..buffer.len())
            .with_state(ViewState::Valid)
            .build();

        let scroll_lines = view.update(&buffer);

        assert_eq!(scroll_lines, Some(0));
        assert_eq!(view, expected_view);
    }

    #[test]
    fn update_ip_moved_above_display() {
        let buffer = "012345678\
                      9012345678\
                      9012345678\
                      9012345678\
                      9012345678\
                      9012345678"
            .to_owned();

        let mut vb = ViewBuilder::new();

        let mut view = vb
            .with_size(DimWH(10, 5))
            .with_prompt(Some(':'))
            .with_insertion_point(0) // ip moved to start
            .with_cursor_position(Coord2D(0, 4)) // ip was at end
            .with_first_display_line(0)
            .with_visible_chars(19..buffer.len())
            .with_state(ViewState::Invalid)
            .build();

        let expected_view = vb
            .with_cursor_position(Coord2D(1, 0)) // cursor at start of input
            .with_visible_chars(0..buffer.len() - 10) // view moved up one line
            .with_state(ViewState::Valid)
            .build();

        let scroll_lines = view.update(&buffer);

        assert_eq!(scroll_lines, Some(0));
        assert_eq!(view, expected_view);
    }

    #[test]
    fn update_on_valid_view_is_nop() {
        let buffer = "buffer text".to_owned();
        let mut vb = ViewBuilder::new();

        let mut view = vb
            .with_size(DimWH(80, 24))
            .with_prompt(Some(':'))
            .with_insertion_point(buffer.len())
            .with_cursor_position(Coord2D(
                u16::try_from(buffer.len()).unwrap() + 1,
                23,
            ))
            .with_first_display_line(23)
            .with_visible_chars(0..buffer.len())
            .with_state(ViewState::Valid)
            .build();

        let expected_view = view.clone();

        let scroll_lines = view.update(&buffer);

        assert_eq!(scroll_lines, None);
        assert_eq!(view, expected_view);
    }

    #[test]
    fn update_backspace_past_column_0() {
        let buffer = "12345678".to_owned();
        let mut vb = ViewBuilder::new();

        let mut view = vb
            .with_size(DimWH(10, 5))
            .with_prompt(Some(':'))
            .with_insertion_point(buffer.len())
            .with_cursor_position(Coord2D(0, 4))
            .with_first_display_line(3)
            .with_visible_chars(0..buffer.len())
            .with_state(ViewState::Invalid)
            .build();

        let expected_view = vb
            .with_cursor_position(Coord2D(9, 3))
            .with_state(ViewState::Valid)
            .build();

        let scroll_lines = view.update(&buffer);

        assert_eq!(scroll_lines, Some(0));
        assert_eq!(view, expected_view);
    }

    #[test]
    fn update_added_char_at_display_end() {
        let buffer = "012345678".to_owned();
        let mut vb = ViewBuilder::new();

        let mut view = vb
            .with_prompt(Some(':'))
            .with_size(DimWH(10, 5))
            .with_insertion_point(buffer.len())
            .with_cursor_position(Coord2D(9, 4))
            .with_first_display_line(4)
            .with_visible_chars(0..9)
            .with_state(ViewState::Invalid)
            .build();

        let expected_view = vb
            .with_cursor_position(Coord2D(0, 4))
            .with_first_display_line(3)
            .with_visible_chars(0..9)
            .with_state(ViewState::Valid)
            .build();

        let scroll_lines = view.update(&buffer);

        assert_eq!(view, expected_view);
        assert_eq!(scroll_lines, Some(1));
    }
}
