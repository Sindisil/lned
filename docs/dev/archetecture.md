# Archetecture

This document describes the high-evel architecture of the lned line editor.

## Bird's Eye View

lned is a line editor modeled after *nix ed, but with extensions to make
it more useful and ergonomic. It is intended primarily as a vehicle for
exploring the rust programming language, but is intended also to be
actually useable.

Implementation simplicity is a primary design goal.

## Design

These sections provide more detailed design decisions and questions.

### edit_buffer

API is a combination of submitted commands (for revertable actions) and
function calls (for non-revertable actions). This makes it easy to 
tell the difference between mutations that can be undone/redone and
actions that cannot.

#### Data Structures

In the spirit of "simplest thing that works adequately", I'm going
to try a simple String for the text buffer. That may end up being
*too* simple, but I think it may well work just fine for a line
based editor, at least until and unless I want to be able to handle
very long text files.

If that ends up being a bad choice (because of ease of development,
performance, or some other reason), I'll choose another way to manage
the text buffer (e.g., Vec of lines, gap buffer, piece table, or some
combination).

If I was building this only for the end product
(as opposed to a combination of utility and an excuse to exercise 
the rust programming language), I might use a pre-built crate
for this, such as ropey. That isn't to say that I won't be using
various crates for elements that are either way too complex to take
on given my goals (e.g., file watching: notify, string slice indexing:
str_indicies, small vector optimaization:smallvec).

#### File modification detection

Initially plan to not do active file watching, only indicating when
buffer has been changed since create/read/write. Will try to detect
file changed on write.

Later feature add will be to use something like the notify crate to
monitor files for change/delete, updating prompt accordingly.

#### File saving

https://stackoverflow.com/questions/18260899/adequetely-safe-method-of-overwriting-a-save-file

https://danluu.com/file-consistency/

https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-replacefilea?redirectedfrom=MSDN

https://stackoverflow.com/questions/1812115/how-to-safely-write-to-a-file

https://stackoverflow.com/questions/18260899/adequetely-safe-method-of-overwriting-a-save-file

Files are hard.


### lned

Features are defined in the user docs.

The lned crate defines the UI (terminal I/O, prompt, command loop, error
messaging, possibly help display, command line arguement processing,
and config file handling.

### Commands

Commands are defined in command.rs, as are the parsing functions.
main_loop accepts command input, parses into a Command using
command::parse(), then executes the commands (delegating to edit_buffer
for edit commands).

Owned & traced by main_loop are:
  * edit_buffer (just one, at least initially)
    - may be queried for buffer modification status
    - edit commands are delegated to edit_buffer
  * previous_command (to facilitate override of warning when
    executing commands that will result in throwing away or
    overwriting changes that cannot be undone)
  * Current filename, if any
  * Current line number in buffer

#### Commands associated data

Commands for the MVP are listed below. Where relevant, a default line span (used if
none is specified) is shown within parantheses. Associated data for the command is
listed in each entry.

##### Immediate commands
q
  Exits (quits) lned.
  If buffer has unwritten changes, a warning is displayed
  instead. A second consecutive quit command will exit unconditionally,
  tossing the unwritten changes.

  * buffer modification status

e
  Edit a file.
  If the buffer has unwritten changes, a warning is displayed instead.
  A second consecutive edit command will be executed unconditionally, tossing
  unwritten changes. If a filename is specified, the file's content is read in,
  replacing the current contents of the buffer. If no filename is specified, the
  buffer is cleared, providing a fresh, empty buffer. The current line is set to
  the last line in the buffer.

  * optional new filename

(1,$)w _file_

  Write a line span to a file.
  Previous file contents are overwritten unconditionally.
  If no file is specified, the current filename is used if it has been specified.
  If neither is specified, an error is displayed. The current line address is
  not changed in any case.

  * optional line span to write to file
  * optional new filename

f _file_
  Set or display current filename.
  The current filename is set to the file specified. If no file is specified,
  the current filename is displayed.

  * Optional new filename

(.,.)n
  Print line span with line numbers.
  The specified line span is printed to stdout with line numbers. The current line
  address is set to the last line printed.

  * Optional line span to output with line numbers

(.,.)p
  Print line span.
  The specifed line span is printed to stdout. The current line address is
  set to the last line printed.

  * Optional line span to print

##### Edit commands
(.)a
  Append text.
  Accept text in input mode and append it after the specified line. The current
  line address is set to the last line entered.

  * Optional address of line after which text will be appended

(.,.)d
  Delete line span.
  The specified line span is deleted from the buffer. The current line address is
  set to the line after the deleted span, if there is one, otherwise to the line
  before the deleted span.

  * Optional line span to delete

(.,.)c
  The current line address is set to the last line entered.
  Change line span.
  Deletes the addressed lines and accepts text in input mode to append in their place.

  * Optional line span to delete
