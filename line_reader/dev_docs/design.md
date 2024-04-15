Non-normitive and possibly out of date notes to help explore design
issues and ideas. I'll try to delete stuff that's no longer relevent.

# Overall Design
Main data structure is a gap buffer. The gap is always at the current
cursor position, since typing in text is assumed to be the most frequent
action. 

The gap is moved by navigation actions (Left, Right, Home, etc.).

## Repaint

Original plan was to repaint separately from actual buffer changes,
similarly to how a game might render (process input, update world,
render world).

Might still be viable, but possibly less efficient? Would need to keep a
model of the current display in order to maintain consistency in rendering
(i.e., not have the cursor position move around unnecessarily).

Alternative model would be to do appropriate display update based upon
event handling. So a key event handler would use knowledge of what
changes it is making (cursor movement, buffer insertion or deletion, etc.)
to help efficiently update the screen.

Pros & cons:

Full repaint:
  + separation of buffer manipulation and rendering (good for
    testing, keeps related code togther)
  + might reduce duplicate code, since several event types would
    probably result in similar screen update code. This could
    be somewhat mitigated by pulling out common code into
    functions that could be used by several event handling
    routines.
  - would need to update more of screen (up to all of it) than
    using decentralized screen updates.
  - would probably need to store a more detailed model of the
    current screen state to facilitate rendering. This would
    duplicate information stored by the terminal emulator. OTOH,
    this might avoid depending upon terminal behavior that
    could differ between platforms.

Partial repaint:
  + Only need to consider related updates in any one piece of
    rendering code, which might keep complexity down, at least
    locally.
  + Might allow less actual screen updates, resulting in better
    performance. OTOH, even a large terminal window isn't very
    big relative to current processing power (132 x 72 window
    is only 9504 cells/characters)
  - Might result in duplicate code because of similar work that
    would need to be done in several cases. Could reduce by
    pulling shared code out into functions if that makes
    sense, but that would add function call overhead, as well
    as further increase complexity.

Worth sketching out a design for each before deciding, since
just running ahead (first with full repaint, then trying to
move to a distributed model to deal better with buffers larger
than the screen size) has been ... messy.


### Desired behavior

Cursor position shouldn't change unexpectedly, nor should the
screen contents scroll unnecessarily. Cursor will remain
within the viewport at all times.

The viewport is defined as the terminal size, minus one line at the
top and bottom if more text exists in the buffer beyond that visible
in the terminal window. If the terminal size is less than 3 lines,
the viewport is the full terminal window.

### Full repaint

Values tracked (i.e., the model):

Note that crossterm uses u16 for many of these values
(or for the values used to compute them). Converting
constantly between them is a PITA, though, so my
thinking is to convert to/from u16 at the interface
with crossterm, and use usize internally.

  * prompt: String or &'a str
  * prompt line: Option<usize> (needed?)
  * before_gap_lines: usize
  * before_gap_remainder: usize
  * terminal size: (usize, usize)
  * first_char: usize

The repaint algorithm would be:

    Hide cursor
    Identify last character of after_gap that will fit
    If whole buffer fits screen
        If more lines are needed than available
            scroll up enough lines
            adjust prompt_line
        Position cursor at (0, prompt_line)
        Clear from cursor to EOS
        Write prompt
        Write before_gap
        Save cursor pos
        Write after_gap
        Restore cursor position
    else
        Adjust RenderContext::first_char & prompt_line
        Position cursor at (0, 0)
        Clear from cursor to end of window
        If prompt_line is not None
            Write prompt
            Write before_gap
        else            
            Write before_gap[first_char_..]
        Save cursor position
        Write as much of after_gap as will fit on screen
        Restore cursor position
    Show cursor

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

### Distributed repaint

Essentially, try to do the least work possible to update. When an
event is handled, only repaint what is necessary. In the best case
this would mean just moving the cursor w/o repainting any text. In
the worst case, it would be the same as full repaint: repaint
whole screen from first_char on.

Because of the worst case, at least prompt_line and first_char
would need to be tracked the same as with full repaint.

The problem is, in the worst case (i.e., when the buffer is too
large to fit the terminal window), we need to do something
similar to the full repaint. Unfortunately, in order to
tell we need to do the full repaint, we'd need to track or
calculate the same information as with full repaint, so
at best we save the time of writing a partial screen of text
on each keystroke. Not as large a win as might it might seem,
but perhaps worth it, since I/O is way more expensive than
math.

Values tracked (i.e., the model):

Same as for full repaint.

Event handling:

Char(c) =>
    // update buffer
    buffer.before_gap.push(c);
    
    // render view
    render_ctx.before_gap_remainder += c.width().unwrap_or(0);
    if before_gap_remainder > terminal_width
        before_gap_lines += 1
        before_gap_remainder -= terminal_width
    compute lines needed for display
    if fits on screen
        if need more lines
            scroll down enough lines
            move cursor up and prompt_line same number
        clear to EOS
        write c
        save cursor location
        write after_gap to EOS
        restore cursor
    else
        // todo!
        
Backspace =>
    // update buffer
    c = buffer.before_gap.pop();
    // update view
    w = c.width().unwrap_or(0);
    if w > 0
        render_ctx.before_gap_remainder -= w;
        move cursor back one, handling wrap
        compute lines needed for display
        if cursor on first line && prompt_line is None
            scroll up one line
            move cursor one line
            compute offset of previous line in buffer
            if 0
                prompt_line = 0
                write prompt + chars to fill new line
            else
                write chars to fill new line
Delete =>
    // update buffer
    i = buffer.after_gap.char_indices();
    c = i.next();
    buffer.after_gap.truncate(/* first cluster */)
    // update view
    clear from cursor to EOS
    write after_gap to EOS
Home =>
    // update buffer
    buffer.after_gap.insert_str(0, buffer.before_gap[..]));
    buffer.before_gap.clear();
    // update view
    render_ctx.compute_before_gap_lines(&buffer);
    compute lines to display
    if larger than screen
        prompt_line = 0
        clear to EOS
        write after_gap to EOS
    move cursor to (prompt_width, prompt_line)
End =>
    // update buffer
    buffer.before_gap.push_str(buffer.after_gap[..]);
    buffer.after_gap.clear();
    // update view
    render_ctx.compute_before_gap_lines(&buffer);
    compute lines to display
    if larger than screen
        compute first char to display
        move cursor to (0, 0)
        clear to EOS
        write from first char to EOS
    else
        compute cursor location
        if off_screen
            scroll until cursor on last line
            adust prompt_line
        move cursor to new location
Left =>
    // update buffer
    find last_base_char_idx
    w = buffer.before_gap[last_base_char_idx..].width();
    buffer.after_gap.insert_str(0, buffer.before_gap[last_base_char_idx..]);
    buffer.before_gap.truncate(last_base_char_idx);
    // update view
    render_ctx.before_gap_remainder -= w;
    compute lines to display
    compute new cursor location
    if cursor on first line && prompt_line is None
        scroll up one line
        move cursor down one line
        compute offset of previous line in buffer
        if 0
            prompt_line = 0
            write prompt + chars to fill new line
        else
            write chars to fill new line
    move cursor to new position
Right =>
    // update buffer
    i = buffer.after_gap.char_indices();
    w = i.next().width().unwrap_or(0);
    nxt = /* first char of next cluster, or after_gap.len() */
    buffer.before_gap.push_str(buffer.after_gap[..nxt]);
    buffer.after_gap.drain(..nxt);
    // update view
    render_ctx.before_gap_remainder += w;
    compute lines to display
    compute new cursor location
    if off_screen
        scroll down 1 line
        write after_gap to fill new line
        adust cursor line and prompt_line
        move cursor to new location
Resize =>
    // update view
    Calcuate before_gap_lines and before_gap_remainder by
        iterating over prompt and before_gap. (This could be
        optimized to only be done when terminal_width changes,
        if it seems worthwhile.)
    calculate lines to display
    if larger than screen
        compute first char to display so cursor is on last line
        move cursor to (0, 0)
    else
        move cursor to (0, prompt_line)
    clear to EOS
    write from first char to EOS or EOB, whichever comes first
            

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
