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

Intended usage is for lned to instantiate a InputEditor, then make
calls to read_line() or read_line_or_cancel() as needed to get
user input lines. The GapBuffer would thus only need to be
allocated once, rather than when each line is read.

LineInput would need to ensure that a few things (e.g., raw mode,
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

##  InputHistory

When instantiated, InputEditor has empty history. As input lines are
accepted, non-empty lines are pushed to the history. The history stack can
be navigated on later calls, via [Up] and [Down]. Two edited buffers are
potentially maintaind: edited_input is saved when history is accessed,
and edited_history is separately saved when edits are made after viewing
a history line, but the user navigates away.

### States and transitions

To manage the various history actions, InputEditor may be in one of several
states. These states may or may not need to be explicitly tracked, since
the current state may be inferrable from history_iter, edited_history, and
edited_input.

* EditInput     - Making changes to a new input line
* ViewHistory   - Viewing one of the previously accepted input lines
* EditHistory   - Making changes to the content of a previously accepted
                  input line. Doesn't modify actual history, nor discard
                  edited input line.  
* Accept        - Accepting input

Certain KeyEvents trigger transitions between these states.

[Up] -> NOP, or to ViewHistory
    if history.is_empty() { NOP }
    if history_idx.is_none() {
        if Some(edited_input) { save buffer to edited_history }
        else { save buffer to edited_input }
        init history_idx to history.len()
    }
    if history_idx > 0 {
        history_idx -= 1;
        init buffer with history[history_idx]
    } else {
        history_idx = None
    }

[Down] -> NOP, or to ViewHistory, EditHistory, or EditInput
    if Some(history_idx) {
        history_idx += 1
        if history_idx < history.len() {
            init buffer with history line
        } else {
            history_idx = None
            if Some(edited_history) { init buffer with edited_history }
            else { init buffer with edited_input }
        }
    }

[Esc] -> NOP, or to EditHistory or EditInput
    history_idx = None
    if Some(edited_history) {
        init buffer from edited_history
        edited_history = None
    } else if Some(edited_input) {
        init buffer from edited_input
        edited_input = None
    }

[Delete], [Backspace], [Char(c)] -> NOP or to EditHistory, then handle evt.
    history_idx = None
    handle event

[Enter] -> Accept
    if !buffer.is_empty() {
        push buffer content to history
        copy buffer content to output buffer
    }

### Test cases

 1. [Up], [Down], and [Esc] do nothing if history is empty.
 2. [Down] does nothing if not viewing history.
 3. [Enter] Accepts the input line, copying it to output buffer, and, if
    non-empty, adding it to history.
 4. [Up] when editing input saves edited input and begins viewing history
 5. [Up] when editing history saves edited history and begins viewing
    history
 6. [Up] when viewing history iterates backwards through history, doing
    nothing once oldest line has been reached.
 7. [Delete], [Backspace], or [Char(c)] when viewing history edits history
 8. [Esc] when editing history returns to editing input
 9. [Esc] when editing input does nothing
10. [Esc] when viewing history returns first of editing history or editing
    input that is_some()


# New history design

The current history design is unnecessarily complex and because of this
has several annoying and somewhat intractable bugs. I propose a new
design that is simpler, and therefore straightforward to implement,
but still should retain the desired functionality.

## Data model:

struct EditBuffer {
    lines: Vec<BufferLine>,     // text split to fit on display lines
    prompt_char_count: usize,   // length of prompt string in chars
    input_start: BufferIndex,   // BufferIndex of first non-prompt char
    draft: Option<String>,      // Input line before viewing/editing
                                // history.
}

)struct HistoryStack {
    lines: Vec<String>,          // accepted input lines
    edited: Vec<Option<String>>, // edited copy of accepted lines
    index: usize,
}

## Actions:

### [Up]:

[Up] traverses to older history lines until the oldest saved line
has been viewed.
If no older history to view, this is a NOP.
If not viewing history, save buffer to draft, otherwise if buffer
differs from current edited history, if some, or current history, if
not, save buffer to current edited history.
Advance to next oldest history and load buffer from edited, if it
exists, otherwise accepted.

### [Down]:

[Down] traverses to more recent history lines until the most recent one
has been displayed, then finally to the draft input line.
If not viewing history (i.e., history at end) this is a NOP.
If buffer differs from current edited history, if there is one, or else current accepted history, save it to current edited history.
Advance to next newer history.
If at end, take draft to load buffer.
Otherwise load buffer from current edited hstory, if there is one, or
else from current accepted.

### [Esc]:

Load buffer from draft, clear draft, and reset history to end, clearing any edited history.

### [Enter]:

[Enter] causes current buffer text to be saved to history if it is not
empty and is different from the most recent line in history. The
history index is also reset to 1 past the end. Then the text input loop
exits so that the input (terminated with native_eol) can be copied into
the output buffer and control returned to the caller.
