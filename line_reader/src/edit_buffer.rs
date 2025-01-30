use std::cmp::Ordering;
use std::ops::ControlFlow;

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::render_context::Cursor;
use crate::render_context::RenderContext;

#[derive(Debug, Clone, PartialEq)]
pub struct EditBuffer {
    pub(crate) lines: Vec<BufferLine>,
    pub(crate) prompt_char_count: usize,
    pub(crate) input_start: BufferIndex,
    pub(crate) draft: Option<String>,
}

impl EditBuffer {
    #[must_use]
    pub fn new() -> EditBuffer {
        EditBuffer { ..Default::default() }
    }

    pub fn reset(&mut self, render_ctx: &mut RenderContext, prompt: &str) {
        let prompt_line =
            BufferLine { text: prompt.to_owned(), width: prompt.width() };
        self.input_start = (0, prompt_line.text.len()).into();
        self.prompt_char_count = prompt.chars().count();
        render_ctx.cursor = Cursor {
            column: prompt_line.width,
            line: render_ctx.first_display_line,
            index: self.input_start,
        };
        self.lines.splice(.., [prompt_line]);
        self.reflow(render_ctx, 0);
    }

    #[must_use]
    pub fn prompt(&self) -> String {
        self.lines
            .iter()
            .flat_map(|l| l.text.chars())
            .take(self.prompt_char_count)
            .collect()
    }

    pub fn len(&self) -> usize {
        self.lines.len()
    }

    pub fn is_empty(&self) -> bool {
        self.input_start == self.buffer_end()
    }

    /// Compute index one past last char in buffer
    pub fn buffer_end(&self) -> BufferIndex {
        (self.lines.len() - 1, self.lines.last().map(|l| l.text.len()).unwrap())
            .into()
    }

    pub fn save_draft(&mut self) {
        let draft = self.draft.get_or_insert_with(String::new);
        draft.clear();
        draft.extend(
            self.lines
                .iter()
                .flat_map(|l| l.text.chars())
                .skip(self.prompt_char_count),
        );
    }

    pub fn input_chars(&self) -> impl Iterator<Item = char> + use<'_> {
        self.lines
            .iter()
            .flat_map(|l| l.text.chars())
            .skip(self.prompt_char_count)
    }

    /// Reflow buffer lines to fit `display_width`, and
    /// snap cursor location to within viewport.
    /// Also might result in setting scroll needed.
    pub fn reflow(&mut self, render_ctx: &mut RenderContext, start: usize) {
        let mut tl_idx = start;
        while tl_idx < self.lines.len() {
            match self.lines[tl_idx].width.cmp(&render_ctx.display_width) {
                Ordering::Less => {
                    if self.try_fill_from_next(render_ctx, tl_idx).is_none()
                        || self.lines[tl_idx].width == render_ctx.display_width
                    {
                        tl_idx += 1;
                    }
                }
                Ordering::Greater => {
                    self.move_overflow_to_next(render_ctx, tl_idx);
                    tl_idx += 1;
                }
                Ordering::Equal => {
                    if tl_idx == render_ctx.cursor.index.line
                        && render_ctx.cursor.column >= render_ctx.display_width
                    {
                        render_ctx.cursor.line += 1;
                        render_ctx.cursor.column = 0;
                        render_ctx.cursor.index.line += 1;
                        render_ctx.cursor.index.offset = 0;
                    }
                    tl_idx += 1;
                }
            }
        }

        render_ctx.adjust_viewport(self);
    }

    // attempt to fill this line, up to but not beyond,
    // display_width.
    // returns Some(prev_line_len) (i.e., idx of first
    // moved char), or None if no chars moved
    fn try_fill_from_next(
        &mut self,
        render_ctx: &mut RenderContext,
        tl_idx: usize,
    ) -> Option<(usize, usize)> {
        if tl_idx == self.lines.len() - 1 {
            return None;
        }

        let tl_width = self.lines[tl_idx].width;
        let nl_idx = tl_idx + 1;
        let moved = self.lines[nl_idx].text.char_indices().try_fold(
            (0, 0),
            |(res_idx, cols_moved), (i, c)| {
                let c_width = c.width().unwrap_or(0);
                if render_ctx.display_width >= (tl_width + cols_moved + c_width)
                {
                    ControlFlow::Continue((i + 1, cols_moved + c_width))
                } else {
                    ControlFlow::Break((res_idx, cols_moved))
                }
            },
        );
        let (res_idx, cols_moved) = match moved {
            ControlFlow::Continue(result) | ControlFlow::Break(result) => {
                result
            }
        };
        if res_idx > 0 {
            if render_ctx.cursor.index.line == nl_idx {
                // if cursor was on next line, adjust cursor
                if render_ctx.cursor.index.offset < res_idx
                    || res_idx == self.lines[nl_idx].text.len()
                {
                    // char at cursor moved to this line
                    render_ctx.cursor.line -= 1;
                    render_ctx.cursor.column += tl_width;
                    render_ctx.cursor.index.line -= 1;
                    render_ctx.cursor.index.offset += self.lines[tl_idx].len();
                } else {
                    // cursor still on next line
                    render_ctx.cursor.index.offset -= res_idx;
                    render_ctx.cursor.column -= cols_moved;
                }
            }

            if self.input_start.line == nl_idx {
                // if input_start was on next line, adjust it
                if self.input_start.offset < res_idx
                    || res_idx == self.lines[nl_idx].text.len()
                {
                    // input_start moved to this line
                    self.input_start.line -= 1;
                    self.input_start.offset += self.lines[tl_idx].len();
                } else {
                    // input_start still on next line
                    self.input_start.offset -= res_idx;
                }
            }

            let (this_part, next_part) = self.lines.split_at_mut(nl_idx);
            let this_line = &mut this_part[tl_idx];
            let next_line = &mut next_part[0];
            this_line.text.extend(next_line.text.drain(..res_idx));
            this_line.width += cols_moved;
            next_line.width -= cols_moved;
        }

        if self.lines[nl_idx].text.is_empty()
            && self.lines[tl_idx].width < render_ctx.display_width
        {
            self.lines.remove(nl_idx);
            if render_ctx.cursor.index.line > tl_idx {
                render_ctx.cursor.index.line -= 1;
                render_ctx.cursor.line -= 1;
            }
        }

        match res_idx {
            0 => None,
            _ => Some((res_idx, cols_moved)),
        }
    }

    fn move_overflow_to_next(
        &mut self,
        render_ctx: &mut RenderContext,
        tl_idx: usize,
    ) {
        assert!(self.lines[tl_idx].width > render_ctx.display_width);
        // check to see if there's a next_line & push one if not
        if tl_idx == self.lines.len() - 1 {
            self.lines.push(BufferLine::new());
        }

        // move this_line's residue to beginning of next line
        let mut cols = 0;
        let (this, next) = self.lines.split_at_mut(tl_idx + 1);
        let (this, next) = (&mut this[tl_idx], &mut next[0]);
        let (res_idx, _) = this
            .text
            .char_indices()
            .find(|(_, c)| {
                let c_width = c.width().unwrap_or(0);
                if render_ctx.display_width - cols < c_width {
                    true
                } else {
                    cols += c_width;
                    false
                }
            })
            .unwrap();
        let cols_moved = this.width - cols;
        let bytes_moved = this.len() - res_idx;
        this.width = cols;
        next.width += cols_moved;
        next.text.insert_str(0, this.text.drain(res_idx..).as_str());

        if tl_idx == render_ctx.cursor.index.line
            && res_idx <= render_ctx.cursor.index.offset
        {
            // if this was the cursor line & char at cursor moved,
            // adjust cursor
            render_ctx.cursor.line += 1;
            render_ctx.cursor.column -= this.width;
            render_ctx.cursor.index.line += 1;
            render_ctx.cursor.index.offset -= res_idx;
        } else if render_ctx.cursor.index.line == tl_idx + 1 {
            // if next line was cursor line, adjust cursor column
            render_ctx.cursor.column += cols_moved;
            render_ctx.cursor.index.offset += bytes_moved;
        }

        if tl_idx == self.input_start.line && res_idx <= self.input_start.offset
        {
            // if the line where input_start is located, and chars at or
            // before input start moved, adjust input_start
            self.input_start.line += 1;
            self.input_start.offset -= res_idx;
        } else if self.input_start.line == tl_idx + 1 {
            // if next line was input_start.line, adjust input_start column
            self.input_start.offset += bytes_moved;
        }
    }

    pub fn set_input_text(
        &mut self,
        render_ctx: &mut RenderContext,
        line: impl AsRef<str>,
    ) {
        let mut text = self.prompt();
        self.input_start = (0, text.len()).into();
        text.push_str(line.as_ref());
        let width = text.width();
        let cursor = Cursor {
            column: width,
            line: render_ctx.first_display_line,
            index: (0, text.len()).into(),
        };
        self.lines.splice(.., [BufferLine { text, width }]);
        render_ctx.cursor = cursor;
        self.reflow(render_ctx, 0);
    }

    pub fn set_from_draft(&mut self, render_ctx: &mut RenderContext) {
        if let Some(draft) = self.draft.take() {
            if draft.chars().ne(self.input_chars()) {
                self.set_input_text(render_ctx, draft);
            }
        }
    }
}

impl Default for EditBuffer {
    fn default() -> EditBuffer {
        EditBuffer {
            lines: Vec::new(),
            prompt_char_count: 0,
            input_start: (0, 0).into(),
            draft: None,
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct BufferLine {
    pub(crate) text: String,
    // Display width, in columns, including wide glyphs
    pub(crate) width: usize,
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

// Location within edit buffer
#[derive(Debug, Clone, Copy, Default)]
pub struct BufferIndex {
    // [0, buffer.len())
    pub line: usize,
    // Byte offset within line [0, buf[line].len())
    pub offset: usize,
}

impl From<(usize, usize)> for BufferIndex {
    fn from((line, offset): (usize, usize)) -> BufferIndex {
        BufferIndex { line, offset }
    }
}

impl From<BufferIndex> for (usize, usize) {
    fn from(i: BufferIndex) -> (usize, usize) {
        (i.line, i.offset)
    }
}

impl PartialOrd for BufferIndex {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for BufferIndex {
    fn cmp(&self, other: &Self) -> Ordering {
        if self.line == other.line {
            self.offset.cmp(&other.offset)
        } else {
            self.line.cmp(&other.line)
        }
    }
}

impl PartialEq for BufferIndex {
    fn eq(&self, other: &Self) -> bool {
        self.line == other.line && self.offset == other.offset
    }
}

impl Eq for BufferIndex {}
