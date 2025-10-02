#   Requirements
    1. Unless the terminal is less than three lines high, the cursor
       doesn't move to the first or last line of the terminal unless
       that would put it in the last line of the buffer (i.e., there
       are no lines "off screen" in that direction).
    2. Lines above the input should be scrolled out of window as needed,
       to preserve scrollback buffer history.
    3. Cursor shouldn't move around needlessly, to avoid disorienting
       the user.
    4. Modulo terminl bugs, line_input should support terminal window
       resizing.

#   Data model

Previous designes assumed a gap buffer for input, but the overwhelmingly
most common case will be a sigle display line, and virtually all use cases
will result in single digit display lines worth of text. Therefore, most
of the complexity in the previous models is unnecessary.

##  A.  Simplified Data Model

The new model is a buffer of lines limited to display line width, along
with display position related values, and a bit of metadata to eliminate
excessive scanning of line contents.

struct BufferLine {
  text: String,
  width: usize,
}

struct BufferIndex {
  line: usize,
  offset: usize,
}
  
struct DisplayPosition {
  column: usize,
  line: usize,
  buffer_offset: BufferIndex,
}


struct InputEditor {
  buffer: Vec<BufferLine>,
  input_start: BufferIndex,
  display_width: usize,
  display_height: usize,
  cursor: DisplayPosition,
  display_start: DisplayPosition,
  scroll_needed: usize,
}

##  B.  Update and Rendering
    The buffer is maintained such that lines exist for every line a cursor
    could occupy, which implies that, if the last buffer line with text is
    full, an empty buffer line should be appended.
    
    1.  In event handlers, make associated upates to buffer model.        
        a.  Resize
            i.      Update display columns & lines
            ii.     Reflow the entire buffer
        b.  Char(c)
            i.      If c.width() > 0, or there is at least one
                    preceding non-zero width character before the cursor,
                    append new character after the cursor, adjusting the
                    line width and cursor offset accordingly
            ii.     If character width > 0, update the buffer line's width
                    and update the cursor display location
            iii.    If line.width > display_width, reflow buffer from
                    cursor line
        c.  Left
            i.      Move cursor buffer offset to first preceding non-zero
                    width character in the buffer
            ii.     Adjust viewport
        d.  Right
            i.      Move cursor forward to next non-zero with character in
                    the buffer.
            ii.     Adjust viewport
        e.  Backspace
            i.      If at input_start, do nothing.
            ii.     If column is 0, set cursor_position to one past last
                    char on previous buffer line.
            iii.    Remove char before cursor and subtract it's len_utf8
                    from the cursor offset.
            iv.     If removed char had non-zero width, reflow buffer from
                    cursor line
            v.      Reflow from earlier of first line or one line before
                    cursor
        f.  Delete
            i.      If at end of last buffer line, do nothing.
            ii.     Remove character at cursor and any following zero width
                    characters.
            ii.     Reflow buffer from cursor line
        g.  Home
            i.      Move cursor to first input character
            ii.     Adjust viewport
        e.  End
            i.      Move cursor to end of buffer
            ii.     Adjust viewport
        f.  Return
            i.      Move cursor to end of buffer
            ii.     Adjust viewport
            iii.    Move cursor to beginning of nextg line
    2.  Reflow
        The reflow routine needs to:
        a.  iterate the buffer lines, filling lines as full as possible,
            and wrapping any overflow to following lines
        b.  maintain final buffer line of < display_width
    3.  Adjusting viewport
        a.  constrain the cursor to the viewport
            i.  If it's below the viewport, pan the buffer down enough
                to bring it to the last line of the viewport by adjusting
                first_buffer_line and, if it wasn't already 0,
                first_display_line. If first_display_line is adjusted,
                scroll_needed is the difference between the old and new
                values.
            ii. If it's above the viewport, pan the buffer up enough
                to bring it to the first line of the viewport by adjusting
                first_buffer_line.
    
    4.  Rendering/repaint
        a. Compute last buffer line to display
            i.  display_lines =
                    self.display_height - self.display_start.line
            ii. last_buffer_line = buffer.len().min(display_lines)
        b. Hide cursor
        c. ScrollUp if needed.
        d. Move cursor to (0, self.display_start.line)
        d. Clear to end
        e. Write buffer[self.display_start.offset.line..last_buffer_line]
            *   if terminal emulators don't wrap as expected, move terminal
                cursor to beginning of each line prior to writing
        f. Move cursor to (self.cursor.column, self.cursor.line)
        g. Show cursor
        
## Test cases
I.  Viewport bounds
    The cursor is limited to one less line in each direction than the
    display_height would indicate if more buffer lines exist "off screen"
    in that direction.
    A.  Test cases
        1.  viewport_top is 1 unless the first buffer line is displayed
        2.  viewport_bottom is one less than the last display line unless
            the last buffer line is displayed
I. Char(c)
    A. Insertion widths
        1.  0w (e.g., combining mark u0308 '̈¨')
        2.  1w (e.g., the letter 'a')
        3.  2w (e.g., the guitar symbol u1f3b8 '🎸')
    B. Test cases
        1.  Input of each width works as expected in base case
            a.  0w requires preceding base character
            b.  each width inserts character and moves cursor appropriate
                width and offset
            c.  input at start of line is appended to preceding line if it
                fits (eg. 0w char, or 1w char if first char on cursor line
                is a 2w char that didn't fit preceding line)
            d.  Input that results in line width > display_width
                causes reflow, moving excess to start of next line
                iteratively until resulting in a line with no excess.
        2.  Cursor is maintained in the first available display cell after
            the new input.
            a.  Input that fills line to last column wraps cursor to start
                of next line.
            b.  Input that pushes characters at cursor to next line moves
                cursor with the reflowed characters
        3.  Cursor is bound to the viewport
            a.  Input at end of buffer smaller than display that moves
                cursor below last line of display decrements
                first_display_line and scrolls existing display lines up
                one line
            b.  Input at end of large buffer moving cursor below last line
                decrements first_display_line without scrolling
            c.  Input in buffer smaller than display extending beyond
                bottom that moves cursor to last line of display decrements
                first_display_line and scrolls existing display lines up
                one line
            d.  Input in buffer larger than display that moves cursor to
                last line of display decrements_first_display line, but
                does not scroll existing display lines
II. Backspace
    A. Removed widths
        1.  0w (e.g., combining mark u0308 '̈¨')
        2.  1w (e.g., the letter 'a')
        3.  2w (e.g., the guitar symbol u1f3b8 '🎸')
    B. Test cases
        1.  Backspace from start of input does nothing.
        2.  Backspace removes only one char before cursor.
        3.  Bacskapce moves cursor back the width of the removed character
        4.  Backspacing over the first char on a display line leaves the
            cursor in column 0 if previous line is full
        5.  Backspacing over the char in column 0 wraps cursor to previous
            line if previous line is not full.
        6.  Backspacing that results in the cursor moving above the
            first line of the viewport adjusts display start to keep the
            cursor on the first line of the viewport.
        7.  Backspacing reflows the buffer from the new cursor line down.
III. Left
    A. Traversed widths
        1.  0w (e.g., combining mark u0308 '̈¨')
        2.  1w (e.g., the letter 'a')
        3.  2w (e.g., the guitar symbol u1f3b8 '🎸')
    B. Test cases
        1.  Left from input_start does nothing.
        2.  Left moves cursor back to previous base (>0w) character.
        3.  Left from column 0 wraps cursor to column of last base
            character on previous display line.
        4.  Left that wraps cursor to above top pans buffer down one line
IV. Right
    A. Traversed widths
        1.  0w (e.g., combining mark u0308 '̈¨')
        2.  1w (e.g., the letter 'a')
        3.  2w (e.g., the guitar symbol u1f3b8 '🎸')
    B. Test cases
        1.  Right when at end of buffer does nothing.
        2.  Right moves cursor to next base (>0w) character
        3.  Right from last char on display line wraps cursor to column
            0 on next display line
        4.  Right that wraps cursor to below bottom pans buffer up one
            line, scrolling if first display line is greater than 0
V. Delete
    A. Deleted widths
        1.  0w (e.g., combining mark u0308 '̈¨')
        2.  1w (e.g., the letter 'a')
        3.  2w (e.g., the guitar symbol u1f3b8 '🎸')
    B. Test cases
        1.  Delete at end of buffer does nothing
        2.  Delete removes base character at cursor, along with any
            following 0w characters.
        3.  Delete moves cursor to end of previous line if prev line wasn't
            full and next char will fit.
        4.  Delete reflows buffer from new cursor line down.
VI. Home
    A. Test cases
        1.  Home when cursor is at input_start does nothing.
        2.  Home moves cursor to input_start
        3.  Home that moves cursor above display top pans buffer down so
            first displayed line is on line 0.
VII. End
    A. Test cases
        1.  End when cursor is at end of buffer does nothing.
        2.  End moves cursor to end of buffer (i.e., the first column after
            the last input character)
        3.  End that moves cursor below bottom pans buffer up so cursor
            line is on last display line
        4.  End that causes buffer to pan down scrolls lesser of lines
            panned or lines above display top
VIII. Resize
    A. Resize changes
        1. Smaller
            a.  Height only
            b.  Width, with or without height
        2. Larger
            a.  Height only
            b.  Width, with or without height
    A. Test cases
        1.  Resize that changes only display height doesn't reflow
        2.  Resize that changes columns or both lines and columns reflows
        3.  Resize smaller takes space first from before cursor util cursor
            is at top line, then from after cursor until cursor is at
            bottom, then keeping cursor on bottom line.
        4.  Resize larger keeps first buffer line until end of buffer fits
            display, then adjusts first buffer line until beginning of
            buffer fits, and only then ajusts first display line.
