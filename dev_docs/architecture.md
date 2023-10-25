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

