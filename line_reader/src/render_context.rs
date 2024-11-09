use std::io;
use std::io::Write;

use crossterm::cursor::Hide;
use crossterm::cursor::MoveTo;
use crossterm::cursor::Show;
use crossterm::terminal::Clear;
use crossterm::terminal::ClearType;
use crossterm::terminal::ScrollUp;
use crossterm::QueueableCommand;

use crate::edit_buffer::BufferIndex;
use crate::edit_buffer::EditBuffer;

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct RenderContext {
    pub(crate) display_width: usize,
    pub(crate) display_height: usize,
    pub(crate) cursor: Cursor,
    pub(crate) first_display_line: usize,
    pub(crate) first_buffer_line: usize,
    pub(crate) scroll_needed: usize,
}

impl RenderContext {
    pub fn new(
        display_width: usize,
        display_height: usize,
        first_display_line: usize,
    ) -> RenderContext {
        RenderContext {
            display_width,
            display_height,
            first_display_line,
            ..Default::default()
        }
    }

    /// Compute first line of viewport
    pub fn viewport_top(&self) -> usize {
        (self.first_buffer_line > 0).into()
    }

    /// Compute last line of viewport
    fn viewport_bottom(&self, buffer: &EditBuffer) -> usize {
        if self.cursor.index.line == buffer.lines.len() - 1
            || (buffer.lines.len() - self.first_buffer_line)
                <= (self.display_height - self.first_display_line)
        {
            self.display_height - 1
        } else {
            self.display_height - 2
        }
    }

    #[cfg(not(tarpaulin_include))]
    /// render current buffer to display
    pub fn repaint(&mut self, buffer: &EditBuffer) -> io::Result<()> {
        let display_lines = self.display_height - self.first_display_line;
        let last_displayed =
            self.first_buffer_line + buffer.lines.len().min(display_lines);

        let mut stdout = io::stdout().lock();

        stdout.queue(Hide)?;

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

        if self.scroll_needed > 0 {
            let scroll_needed = u16::try_from(self.scroll_needed)
                .expect("scroll needed fits in u16");
            stdout.queue(ScrollUp(scroll_needed - 1))?;
            self.scroll_needed = 0;
        }

        for line in &buffer.lines[self.first_buffer_line..last_displayed] {
            stdout.write_all(line.text.as_bytes())?;
        }

        stdout.queue(MoveTo(cursor_column, cursor_line))?.queue(Show)?.flush()
    }

    pub fn adjust_viewport(&mut self, buffer: &EditBuffer) {
        if self.cursor.line > self.viewport_bottom(buffer) {
            let diff = self.cursor.line - self.viewport_bottom(buffer);
            self.cursor.line = self.viewport_bottom(buffer);
            if self.first_display_line == 0 {
                self.first_buffer_line += diff;
            } else {
                self.scroll_needed = self.first_display_line.min(diff);
                self.first_display_line =
                    self.first_display_line.saturating_sub(diff);
                self.first_buffer_line += diff - self.scroll_needed;
            }
        } else if self.cursor.line < self.viewport_top() {
            let diff = self.viewport_top() - self.cursor.line;
            self.cursor.line = self.viewport_top();
            self.first_buffer_line =
                self.first_buffer_line.saturating_sub(diff);
        }
        if buffer.lines.len() <= self.display_height {
            if self.first_buffer_line != 0 {
                // lines above display
                self.cursor.line += self.first_buffer_line;
                self.first_buffer_line = 0;
            } else if self.display_height - self.first_display_line
                < buffer.lines.len()
            {
                // lines below display
                self.scroll_needed = buffer.lines.len()
                    - (self.display_height - self.first_display_line);
                self.cursor.line -= self.scroll_needed;
                self.first_display_line -= self.scroll_needed;
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Cursor {
    pub column: usize,
    pub line: usize,
    pub index: BufferIndex,
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn viewport_all_within_display() {
        let buffer = EditBuffer {
            lines: vec![
                ":123456789".into(),
                "0123456789".into(),
                "012345".into(),
            ],
            prompt_char_count: 1,
            input_start: (0, 1).into(),
            draft: None,
        };
        let render_ctx = RenderContext {
            display_width: 10,
            display_height: 5,
            cursor: Cursor { column: 6, line: 2, index: (2, 6).into() },
            ..Default::default()
        };
        assert_eq!(
            render_ctx.viewport_bottom(&buffer),
            render_ctx.display_height - 1
        );
        assert_eq!(render_ctx.viewport_top(), 0);
    }

    #[test]
    fn viewport_buffer_beyond_top() {
        let buffer = EditBuffer {
            lines: vec![
                ":123456789".into(),
                "0123456789".into(),
                "0123456789".into(),
                "0123456789".into(),
                "0123456789".into(),
                "0123456789".into(),
                "012345".into(),
            ],
            prompt_char_count: 1,
            input_start: (0, 1).into(),
            draft: None,
        };
        let render_ctx = RenderContext {
            display_width: 10,
            display_height: 5,
            cursor: Cursor { column: 6, line: 4, index: (6, 6).into() },
            first_buffer_line: 2,
            ..Default::default()
        };
        let vp_bottom = render_ctx.viewport_bottom(&buffer);
        let vp_top = render_ctx.viewport_top();
        assert_eq!(vp_bottom, render_ctx.display_height - 1);
        assert_eq!(vp_top, 1);
    }

    #[test]
    fn viewport_buffer_beyond_bottom() {
        let buffer = EditBuffer {
            lines: vec![
                ":123456789".into(),
                "0123456789".into(),
                "0123456789".into(),
                "0123456789".into(),
                "0123456789".into(),
                "0123456789".into(),
                "012345".into(),
            ],
            prompt_char_count: 1,
            input_start: (0, 1).into(),
            draft: None,
        };
        let render_ctx = RenderContext {
            display_width: 10,
            display_height: 5,
            cursor: Cursor { column: 1, line: 0, index: (0, 1).into() },
            ..Default::default()
        };
        assert_eq!(
            render_ctx.viewport_bottom(&buffer),
            render_ctx.display_height - 2
        );
        assert_eq!(render_ctx.viewport_top(), 0);
    }

    #[test]
    fn viewport_buffer_beyond_both() {
        let buffer = EditBuffer {
            lines: vec![
                ":123456789".into(),
                "0123456789".into(),
                "0123456789".into(),
                "0123456789".into(),
                "0123456789".into(),
                "0123456789".into(),
                "012345".into(),
            ],
            input_start: (0, 1).into(),
            prompt_char_count: 1,
            draft: None,
        };
        let render_ctx = RenderContext {
            display_width: 10,
            display_height: 5,
            cursor: Cursor { column: 5, line: 2, index: (3, 5).into() },
            first_buffer_line: 1,
            ..Default::default()
        };
        assert_eq!(
            render_ctx.viewport_bottom(&buffer),
            render_ctx.display_height - 2
        );
        assert_eq!(render_ctx.viewport_top(), 1);
    }
}
