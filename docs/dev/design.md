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

Lned implementation should probably be:

Update clean fingerprint after:
1. initial file read to buffer
2. new empty buffer
3. after (e) command
4. new buffer with content
5. after (w) command, if whole buffer is written to default filename


The buffer fingerprint contains:
1. line count 
2. total length
3. hash of file

A buffer is dirty if the current state doesn't match the fingerprint.

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

