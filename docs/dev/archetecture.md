# Archetecture

This document describes the high-evel architecture of the lned line editor.

## Bird's Eye View

lned is a line editor modeled after *nix ed, but with extensions to make
it more useful and ergonomic. It is intended primarily as a vehicle for
exploring the rust programming language, but is intended also to be
actually useable.

Implementation simplicity is a primary design goal.

## Repository Structure

This is a brief description of important directories.

### crates/lned

This crate defines the actual lned binary.
This encompases the UI, handling command line options, handling the config
file, and managing edit buffers.

### crates/edit_buffer

This implements data types and functions that store and manipulate text.
Also included are undo/redo stacks and file I/O.


## Design

These sections provide more detailed design decisions and questions.

### edit_buffer

API is a combination of submitted commands (for revertable actions) and
function calls (for non-revertable actions). This makes it easy to 
tell the difference between mutations that can be undone/redone and
actions that cannot.

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
