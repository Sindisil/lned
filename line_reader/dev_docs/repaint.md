# Requirements
    1. Unless the terminal is less than three lines high, the cursor
       doesn't move to the first or last line of the terminal unless
       that would put it in the last line of the buffer (i.e., there
       are no lines "off screen" in that direction).
    2. Lines above the input should be scrolled out of window as needed,
       to preserve scrollback buffer history.
    3. Cursor shouldn't move around needlessly, to avoid disorienting
       the user.
    4. Modulo terminl bugs, line_reader should support terminal window
       resizing.

# Rendering methods

##  I.  Cursor centric

Essentially, calculate everything at repaint time, using cursor
position (column, line) as main point of reference. Basic update is
straight forward, but becomes inefficient and unnecessarily complex
in cases of large buffers and large changes to the buffers.

### Top level procedure

	1.	Compute new cursor position given current display start
		and buffer content.
	2.	If cursor would be outside viewport, compute new display start
	   	and cursor position so that the cursor will be on the last line
	   	of the viewport in that direction.
	3.	If new display start is above current display start, compute
		the appropriate number of lines to scroll to preserve any
		scrollback buffer history.
    4.  Render buffer to display
    5.  Save new cursor position and display start

##  II. Line centric

Not entirely different from the first method, but uses a vector of
display line indicies rather than scanning the buffer as needed.
Should solve most of the issues of the first method.

### Top level procedure

    1.  If the buffer contents (not just the cursor position) or the
        display dimentions change, update the display line indices.
        Probably most efficiently done in the appropriate event handlers.
    2.  Using line indicies, compute new cursor position given current
        display start.
	2.	If cursor would be outside viewport, compute new display start
	   	and cursor position so that the cursor will be on the last line
	   	of the viewport in that direction.
	3.	If new display start is above current display start, compute
		the appropriate number of lines to scroll to preserve any
		scrollback buffer history.
    4.  Render buffer to display
    5.  Save new cursor position and display start
    
##  III.    Event driven

Given that the most efficient time to update the display line offsets
seems to be in the associated event handlers, it seems worth
considering computing the full display model at event handling time,
leaving only actual rendering to be done during repaint.

In addition to likely being even more efficient, it would afford the
opportunity to unit test the display model generation, leaving only
actual I/O untested.

### Top level prodedure

** TODO: differentiate between buffer gap and cursor position!!!

    1.  In event handlers, make associated upates to display model.        
        a.  Resize
            i.      Update display columns & lines
            ii.     If dimensions have changed, update bg_buf display line
                    offsets
            iii.    If line offsets have changed, update cursor position
            iv.     If new cursor position is outside viewport, update
                    display start and cursor postion to place cursor on
                    last line of viewport.
            v.      Compute any necessary scroll distance
            vi.     If ag_buf isn't empty, compute end bound of displayed
                    slice
        b.  Char(c)
            i.      If character width > 0, or there is at least one
                    preceding non-zero width character before the gap,
                    append new character before the gap.
            ii.     If character width > 0, update bg_buf display line
                    offsets starting from cursor line
            iii.    If line offsets have changed, update cursor position
            iv.     If new cursor position is outside viewport, update
                    display start and cursor postion to place cursor on
                    last line of viewport.
            v.      Compute any necessary scroll distance
            vi.     If ag_buf isn't empty, compute end bound of displayed
                    slice
        c.  Left
            i.      Move last non-zero width char from back of bg_buf
                    to front of ag_buf, along with any following zero
                    width chars
            ii.     Truncate last display line offset if it's beyond
                    bg_buf len
            iii.    Compute new cursor position
            iv.     If new cursor position is outside viewport, update
                    display start and cursor postion to place cursor on
                    first line of viewport.
            v.      If ag_buf isn't empty, compute end bound of displayed
                    slice
        d.  Right
            i.      Move first char from front of ag_buf to back of
                    bg_buf, along with any following zero width chars
            ii.     Update bg_buf display line offsets starting from
                    cursor line
            iii.    Compute new cursor position
            iv.     If new cursor position is outside viewport, update
                    display start and cursor postion to place cursor on
                    first line of viewport.
            v.      Compute any necessary scroll distance
            vi.     If ag_buf isn't empty, compute end bound of displayed
                    slice
        e.  Backspace
            i.      Remove last char before gap
            ii.     Truncate last display line offset if it's beyond
                    bg_buf len
            iii.    If removed char had non-zero width, compute new
                    cursor position.
            iv.     If new cursor position is outside viewport, update
                    display start and cursor postion to place cursor on
                    last line of viewport.
            v.      If ag_buf isn't empty, compute end bound of displayed
                    slice
        f.  Delete
            i.      Remove first char and any following zero width chars
                    from front of ag_buf
            ii.     If ag_buf isn't empty, compute end bound of displayed
                    slice
        g.  Home
            i.      Move content of bg_buf to front of ag_buf
            ii.     Compute new cursor position.
            iii.    If new cursor position is outside viewport, update
                    display start and cursor postion to place cursor on
                    last line of viewport.
            iv.     If ag_buf isn't empty, compute end bound of displayed
                    slice
        e.  End
            i.      Move contents of ag_buf to back of bg_buf
            ii.     Update display line indices, starting with cursor
                    line.
            iii.    Compute new cursor position
            iv.     If new cursor position is outside viewport, update
                    display start and cursor postion to place cursor on
                    first line of viewport.
            v.      Compute any necessary scroll distance
        f.  Return
            i.      Move to end
        g.  Cancel  (ctrl-d)
            i.      Moe to end
    2.  Render buffer to display.
        a. Hide cursor
        b. ScrollUp if needed.
        c. Move cursor to (0, first_display_line)
        d. Clear to end
        e. Write bg_buf from first_buffer_line
        f. Write as much of ag_buf as fits in display
        g. Move cursor to cursor position
        f. Show cursor
        
### Test cases
I.  Char(c)
    A.  Insertion sizes
        1.  0w (e.g., combining mark u0308 '̈¨')
        2.  1w (e.g., the letter 'a')
        3.  2w (e.g., the guitar symbol u1f3b8 '🎸')
    B.  Test cases
        1.  Insertion of each size works as expected in base case
            a.  char_typed_non_0w_inserts
            b.  char_typed_0w_requires_base_char
                0w as first character doesn't insert
            c.  char_typed_beore_eol_moves_cursor_char_width
        2.  char_typed_to_eol_before_bottom_wraps_cursor_to_0
            Insertion of each size that fills line to last column wraps
            cursor to column zero of next line.
        3.  char_typed_past_eol_before_bottom_wraps_cursor_to_1
            Insertion that won't fit line wraps character to start of next
            line and moves cursor to first colum after character.
        4.  Insertion that puts cursor below viewport causes display
            start to adjust to keep cursor on last line of viewport,
            scrolling up as necessary.
            a.  char_typed_to_bottom_when_bg_fits_pans_display
            b.  char_typed_to_bottom_when_bg_overflows_pans_buffer    
        5.  char_typed_ag_display_only_to_display_end
        

