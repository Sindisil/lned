# Working lned design document

## Goals and purpose

The lned editor is intended primarily to serve as a way to evaluate Rust in a
"real" project. Additionally, I'm interrested in exploring the theory that I
work best using relatively straightforward and simple tools. (for values of "best"
that encompass comfort, happiness, and effectiveness/productivity).

To that end, and just for fun, I'm bootstrapping lned using ed, and then intend
to use lned to complete its own development. As of this writing, the plan is then
to use lned to bootstrap development on a terminal based screen editor (scrned).

For these reasons, simplicity (to the point of "simplest thing that will reasonably work")
should be the overarching design focus for lned.

This document is intended to flesh out the design of lned using pseudocode,
as well as provide a space to hash out design alternatives.

## Design thoughts

### Dirty buffer

A buffer is dirty if:
  * Changes have been made since last write

What counts as a change?
  * Insertions of text not undone
  * Deletions of text not undone
  * Substituions of text not undone

After testing several editors, they all consider a buffer dirty if there are any changes not
undone via the undo function. A few are even simpler, and consider it dirty after the
first change -- even undo doesn't clear the flag.

There seem to be two major styles of dirty buffer detection:

I. If there are any items on the undo stack since the last file write,
   the buffer is considered dirty (regardless of whether the contents are
   actually different or, as in the case where the changes have been
   manually "undone", not).

   For this case the clean fingerprint would a hash of the undo stack's
   contents, and could optionally include the number of undo records to
   allow for faster detection of a dirty buffer (check for differing
   record count first, and compare hashes if counts are equal).

   This method's performance isn't sensitive to file size, but will
   consider a buffer that has been changed, but has been manually corrected
   to match the clean state, to be dirty. This does, however, seem to be
   the more widely used idiom.
   
II. If the file contents from after the last file write (or initial
    buffer creation, as with the (e) command), it is considered dirty.
    If not, not.

   For this case, the snapshot would be a hash of the file contents,
   and could optionally include the line count to allow for faster
   detection of a changed file (check for differing line count first,
   and if same, compare hashes).

   This method may potentially lead to performance issues, since the entire
   file must be hashed, rather than just the undo stack. However, since
   most text files that are actually edited are quite small, this is unlikely
   to occur in practice. It is also less commonly used: out of Nvim, Micro,
   and VSCode, only VSCode uses this method. 

After consideration, lned will use the more common idiom I.
Update clean fingerprint after:
1. buffer creation (from file or programmatic contents, or empty)
2. buffer re-creation (via (e) command)
3. after (w) command that writes the full buffer (i.e., address is equivalent to .,$).

### Undo/Redo

Two design quesions are fundamental to the undo/redo design:

1. Should undo/redo be tracked editor wide, or individually for each buffer?
2. Should undo/redo be a stack or tree model?

#### Undo/Redo scope

Tracking per buffer seems like the obvious choice (and seems to be the common choice
with other editors). It provides simpler implementation in some ways, and would cause
less "spooky action at a distance" (i.e., either switching buffers or causing changes
in other than the current active buffer).

After looking at other editors, definately track per buffer.

#### Undo/Redo model

For simplicity, we'll use a two stack model: undo stack for the linear history, redo stack for ... well, undoing undos.

To ensure no loss of history, if an undoable edit action is executed while there are items on the redo stack (i.e., after the user has used undo one or more times), those items are played back onto the undo stack both forwards and backwards (to track the original edits and the undo actions).

### Command model

I orginally used an enum to define commands, because it was simple, worked, and ensured full coveage of all commands at key points.

However I may want to move away from that in order to properly support the undo/redo model. For each command, lned needs to be able to:

1. Store the necessary data (e.g., address, target, lines of input, etc.) associated with normal execution.
2. Define the code to actually perform the command's action.
3. Produce the inverse commadn for use by undo. This would need to include items 1 & 2, above, but for the inverse of the original command.
4. Produce the original command again later, for use by redo (could store orignal, could construct from undo info).

There are multiple ways to do this, of course, but it seems like storing the data in the enum and then writing free functions to implement everything is messier than necessary.

One option would be to define each command as a struct containing the pertinent data items, then implement Execute (or Do), Undo, and Redo traits for each. This loses the ability to check statically that we're covering every command, but that might be an acceptable tradeoff.

It might still be worthwhile to have a CommandType or CommandId enum, but it might not add enough value.

