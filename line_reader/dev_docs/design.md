Non-normitive and possibly out of date notes to help explore design
issues and ideas. I'll try to delete stuff that's no longer relevent.

# Overall Design
Main data structure is a gap buffer. The gap is always at the current
cursor position, since typing in text is assumed to be the most frequent
action. 

The gap is moved by navigation actions (Left, Right, Home, etc.).

Cursor position shouldn't change unexpectedly, nor should the
screen contents scroll unnecessarily. Cursor will remain
within the viewport at all times.

The viewport is defined as the terminal size, minus one line at the
top and bottom if more text exists in the buffer beyond that visible
in the terminal window. If the terminal size is less than 3 lines,
the viewport is the full terminal window.

Intended usage is for lned to instantiate a LineReader, then make
calls to read_line() or read_line_or_cancel() as needed to get
user input lines. The GapBuffer would thus only need to be
allocated once, rather than when each line is read.

LineReader would need to ensure that a few things (e.g., raw mode,
hidden cursor) are safely reset after each read_line() call
regardless of errors.

Since those things are all rendering related, having a Renderer
struct that is Drop and resets those items when dropped would be
the obvious solution.

## Repaint

Values tracked (i.e., the model):

Crossterm uses u16 for most attributes, so associated
values will be stored in that format to reduce the
need for conversion unless it makes other aspects
(i.e., calculating new cursor position) too clumsy.

Some of the fields shown here for Renderer might not be necessary,
since they can be derived from other fields (eg., cursor_line,
cursor_col), but I'm listing them for now for completeness. Those
found to be redundant will be removed.

enum DisplayStart {
	/// Terminal line of prompt
	Prompt(u16),

	/// Index in buffer.before_gap of char at terminal 0,0
	CharIndex(usize),
}

struct Renderer {
	// reference to current prompt
	prompt: &'a str,

	/// width of prompt, in columns
	prompt_width: u16,

	// current terminal window width
	terminal_cols: u16,

	// current terminal window height
	terminal_lines: u16,

	// Description of beginning of text in terminal
	display_start: DisplayStart,

	// Terminal lines current buffer would occupy before the cursor line
	lines_before_cursor: usize,

	// Terminal columns text before the cursor occupies
	// on the cursor line
	cols_before_cursor: u16,

	// Terminal lines current buffer would occupy after the
	// cursor line.
	lines_after_cursor: usize,

	// Terminal line of cursor position (0 based)
	cursor_line: u16,

	// Terminal column of cursor position (0 based)
	cursor_col: u16,

	/// Index of last char in buffer.after_gap that fits in terminal
	last_char_idx: usize,
	
}
	
### Notes on rendering

#### Repaint entire viewport (i.e., w/o specific event)

Update terminal size
Compute new cursor location given current terminal size, buffer,
	and display_start value.
If cursor would be above or below the viewport, compute a new display_start
	to place it on first or last line of viewport, respectively. ScrollUp
    if previous display_start was Prompt(l) and l > 0.
Hide the cursor
Move cursor to display_start
Clear from to end of terminal	
Print from display_start to cursor (prompt and before_gap, or
	before_gap[first_char_idx..], depending upon display_start)
If are characters after the cursor, compute the last character that will
	fit in the terminal (last_char_idx), then print those characters
	(after_gap[..last_char_idx-1])
Move cursor to new cursor location
Show the cursor

This should always result in a correct display and avoid jumping the
cursor around in unexpected ways, but will often result in clearing
and printing unnecessary characters to the terminal.

As an optimization, we could have repaint take the last event as
an argument. This would  allow more optimal repainting in many cases.

For example, if repaint() knows that it was a KeyEvent::Char(ch) that
triggered the repaint call, we know:

	a. display_start and current cursor position will change by at
		most one line, and that display_start will never transition
		from ::CharIndex() to ::Prompt() as a result (though it could
		transition the other direction)
	b. we only need to clear at most from the current cursor position
		down, and then only if after_gap isn't empty.
	c. we only need to print the new character, plus as much of
		after_gap as will fit (if it isn't empty)

Whether or not such optimization is worthwhile is an open question.
It probably makes sense to implement the general algorithm first,
and implement the optimizations only if performance is an issue in
practice, since we'll need the general algorithm anyway, both for
the initial prompt display, as well as for resize events (since
the aren't a result of a buffer content or cursor location change
that would provide the information necessary to do smarter updates).

To update display_start:

last_vp_line = min(
		terminal_lines - 1,
		terminal_lines - 1
				- (buffer.after_gap.width()
						+ cur_col > terminal_cols) as u16)
match display_start {
	::Prompt(l) => {
		if cur_line > last_vp_line {
			d_cur = cur_line - last_vp_line
			if d_cur <= l {
				d_start = l - d_cur
				ScrollUp(d_start)
				display_start = ::Prompt(d_start)
			} else {
				ScrollUp(l)
				display_start =
						::CharIndex(self.skip_lines(buffer, d_cur - l))
			}
		}
	}
	::CharIndex(i) => {
			
	}
}

compute (lines_before_cursor, cols_before_cursor)
lines_needed = if after_gap.width() + cols_before_cursor >= terminal_cols
    	lines_before_cursor + 1
   	else
   		lines_before_cursor
match display_start
	Prompt(l) => {
   		if lines_needed > terminal_lines {
   			// we need more lines than will fit on screen
   			// compute new index and set to CharIndex
   			lines_to_skip = 
  		} else if lines_needed + l > terminal_lines {
  			display_start = Prompt(terminal_lines - lines_needed)
  		}
  	CharIndex(i) => {
  		if lines_needed <= terminal_lines {
  			display_start = Prompt(0);
  		} else {
  		    let mut lines_to_skip = lines_needed - terminal_lines;
  		    let mut cols = 0;
  		    for (i, c) in buffer.before_gap.chars().enumerate() {
  		    	let w = c.width().unwrap_or(0);
  		    	if cols + w > self.terminal_cols {
  		    		lines_to_skip -= 1;
  		    		if lines_to_skip == 0 {
  		    			display_start = CharIndex(i);
  		    			break;
  		    		}
  		    	}
  		    }
  		}
  	}
}

To compute screen space before cursor:

let mut lines = 1;
let (mut cols, first_char) = match display_start {
	DisplayStart::Prompt(l) => (prompt.width(), 0),
	DisplayStart::CharIndex(i) => (0, i),
}
for c in before_gap[i..] {
	let w = c.width.unwrap_or(0);
	if cols + w > terminal_cols {
		lines += 1;
		cols = w
	}
}
(lines, cols)

To compute display end:

let (mut line, mut col) = cursor::position();
for (i, c) in after_gap[..].chars() {
	let w = c.width().unwrap_or(0);
	if col + w > terminal_cols {
		line += 1;
		if line >= terminal_lines {
			return i
		}
		col = w
	}
}
after_gap.len()


An event would cause first_char_idx to change if:

  * first character after before_gap would be on first line of
    terminal screen and prompt_line is None: move
    first_char_idx back enough to fill one additional screen
    line, setting prompt_line to Some(0) if first_char_idx is 0.
    
  * first character after before_gap would end up on last line
    of the terminal screen and after_gap doesn't fit on screen:
    move first_char_idx forward enough to start display with the
    first character on the following screen line, effectively
    "scrolling up" (or "panning down") in the buffer.


Paint helper functions:

renderctx.first_displayed_back(buffer: &GapBuffer, n: impl Into<usize>)
        -> usize
    takes a number of columns to move first_displayed_char back, and
    returns the offset into the buffer of the first char that would
    fit (given character display widths). Saturates to 0.

renderctx.first_displayed_forward(buffer: &GapBuffer, n: Impl Into<usize>)
        -> usize
    takes a number of columns to move first_displayed_char forward,
    and returns the offset into the buffer of the first char that
    would fit (given character display widths). Will never be larger
    than buffer.before_gap.len(), since the cursor is always on the
    screen.

Calculating lines needed:

 Need to be able to calculate lines needed to display the buffer,
 or part of it (i.e., after_gap), since any one "character" (i.e.,
 glyph/grapheme cluster) might take more than one cell to render,
 even in terminals that don't fully support grapheme clustering.
 For example, emoji characters are two cells wide.

 At least some of the buffer will need to be iterated over in order
 to make this calculation, but we could possibly cache and update
 some of the information as buffer is modified, which which should
 be more efficient, especially in the common case of typing text.

 Essentially:
 
     track:
        before_gap_lines (including prompt)
        before_gap_remainder (chars on last partial line)
 
     update on event:
        Char(c) =>
          get c_width
          add c_width to before_gap_remainder
          if before_gap_remainder > terminal_width
             before_gap_lines += 1
             before_gap_remainder -= terminal_width
        Backspace =>
            if removed char's width == 0 return
            if removed char's width > before_gap_remainder
                before_gap_lines -= 1
                before_gap_remainder = terminal_width - removed_char's width            else
                before_gap_remainder -= removed char's width
        Resize =>
            Calcuate before_gap_lines and before_gap_remainder by
            iterating over prompt and before_gap. This could be
            optimized to only be done when terminal_width changes,
            if it seems worthwhile.
                
Event handling:

Char(c) =>
    buffer.before_gap.push(c);
    render_ctx.before_gap_remainder += c.width().unwrap_or(0);
Backspace =>
    c = buffer.before_gap.pop();
    render_ctx.before_gap_remainder -= c.width().unwrap_or(0);
Delete =>
    i = buffer.after_gap.char_indices();
    c = i.next();
    buffer.after_gap.truncate(/* first cluster */)
Home =>
    buffer.after_gap.insert_str(0, buffer.before_gap[..]));
    buffer.before_gap.clear();
    render_ctx.compute_before_gap_lines(&buffer);
End =>
    buffer.before_gap.push_str(buffer.after_gap[..]);
    buffer.after_gap.clear();
    render_ctx.compute_before_gap_lines(&buffer);
Left =>
    w = buffer.before_gap[last_base_char_idx..].width();
    buffer.after_gap.insert_str(0, buffer.before_gap[last_base_char_idx..]);
    buffer.before_gap.truncate(last_base_char_idx);
    render_ctx.before_gap_remainder -= w;
Right =>
    i = buffer.after_gap.char_indices();
    w = i.next().width().unwrap_or(0);
    nxt = /* first char of next cluster, or after_gap.len() */
    buffer.before_gap.push_str(buffer.after_gap[..nxt]);
    buffer.after_gap.drain(..nxt);
    render_ctx.before_gap_remainder += w;


## Edit states

LineReader has several ReaderStates:

    EditInput
    ViewHistory
    EditHistory
    Accept

When history search is added, it will probably result in another
ReaderState (e.g., SearchHistory).

### EditInput

LineReader starts in this state. The input_buffer is displayed and
manipulated in this state. Transitions out of this state are:

#### EditInput

In this state, the input_buffer is displayed and manipulated.

[Enter] => Accept
[Up] => ViewHistory
[Esc] if previous state was EditHistory => EditHistory
_ => EditInput

#### ViewHistory

In this state, a line from the line_history list is displayed. When
transitioning into this state, the most recent (i.e., last) line of
line_history is displayed.

[Up] => ViewHistory (moves to next older line of history)
[Down] if at end of line_history => EditInput
[Down] => ViewHistory (move to next most recent line of history)
[Esc] if previous state was EditInput => EditInput
[Esc] if previous state was EditHistory => EditHistory
[Enter] => Accept
_ => EditHistory

#### EditHistory

When entering this state from ViewHistory, history_buffer is initialized
from the currently displayed line in line_history. The contents and state
persist until accepted or replaced by a subsequent transition from
ViewHistory.

[Up] => ViewHistory
[Esc] => EditInput
[Enter] => Accept
_ => EditHistory

#### Accept

If non-empty, the currently displayed line (whether input_buffer,
history_buffer, or an item in line_history) is pushed onto end of
line_history and copied into output buffer. In future, might shrink
one or both of the buffers if they're beyond some limit, as well, to
optimize memory consumption.


### Input rendering - "buffer zone"
One UX design question is whether or not LineInput should attempt
to keep a one line "buffer zone" ("buffer" in text editor development
seems to be as overloaded as "level" in RPGs) visible (i.e., the
cursor should only be on the top or bottom line of the terminal
if it is in the first or last line of buffer text, respectively).

It would be simpler to not worry about it, but being able to see
what text would be affected by a delete in the last position, or
a BS in the first position, would be desirable, I think. Also,
seeing more text would allow for more efficient navigation. Of
course, in the common case, the input would be a handfull of
lines at most, and often only a single line, so the whole bufer
would most often be on screen at all times. Still an edge case
possibly worth handling.
