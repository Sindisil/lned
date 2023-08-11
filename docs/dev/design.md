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

### Undo/Redo

Two design quesions are fundamental to the undo/redo design:

1. Should undo/redo be tracked editor wide, or individually for each buffer?
2. Should undo/redo be a stack or tree model?

#### Undo/Redo scope

Tracking per buffer seems like the obvious choice (and seems to be the common choice
with other editors). It provides simpler implementation in some ways, and would cause
less "spooky action at a distance" (i.e., either switching buffers or causing changes
in other than the current active buffer).

It is increasingly seeming that the current abstractions aren't quite right, though. Right
now we have the following:

 * main_loop
   - contains the vector of EditBuffers
   - interprets commands, interacting with buffers as needed
 * EditBuffer
   - contains the actual text
   - provides operations on the text
   - tracks current_line and the buffers's dirty status (needs_write)

This is awkward for undo/redo, as they are per buffer (so should probably live in EditBuffer),
but seem easiest to define in terms of Cmds, which main_loop currently translates into
operations on EditBuffer.

One solution would be to track undo/redo stacks in parallel with EditBuffers, and probably also
track buffer dirty status in parallel, as well. This feels a bit hacky, but would be pretty
simple to implement, would not require any signficant refactoring of the current code, and
should be rather performant (though I doubt any differene would be significant in lned's case).

Another option is to add another layer of abstraction, which would provide for separation of concerns,
and maybe make it easier to tell where to put (and find) specific functionality.

  * main_loop
    - contain vector of EditBuffer
    - interpret non-buffer specific commands (primarily quit and the buffer manipulation comands).
    - could still interpret buffer specific commands, but could instead pass them along to
      the correct EditBuffer (typically the active_buffer).
  * EditBuffer
    - contain's TextBuffer
    - contains undo/redo stacks
    - tracks current_line & dirty status
    - possibly (probably) interprets buffer specific commands, interacting with TextBuffer as
      necessary. Otherwise could simply provide a reference to the TextBuffer if main_loop
      would still interpret the commands.
  * TextBuffer
    - contains actual text
    - provides operations on the text lines
    - provides I/O operations for the text content
    - probably doesn't need to track needs_write anymore

Both options seem viable. I'll make a decision when I implement undo/redo, right after the first
opposing commands are complete (append & delete).


## Data Types

struct CmdArgs {
  files: Vec
}

struct EditBuffer {
  lines: Vec<String>;
  filename: Option<PathBuf>;
  current_line: usize;
}

enum Cmd {
  Quit,
  File(Option<PathBuf>),
  Edit(Option<PathBuf>),
}

## Design

main() {
  args = parse_command_line()
  main_loop(&args)
}

main_loop(args) {
  initialize_buffers(&buffers, &args.files)
  current_buffer = &buffers[0]
  cmd_input: String
  Loop {
    show_prompt(stdout.lock(), current_buffer)
    accept_command(stdin.lock(), &cmd_input)
    cmd = parse_command(&cmd_input)?
    match &cmd {
      Cmd::Quit => {
        // clean up and exit
      },
      _ => {
        // unsupported command
      }
    }
  }
}

initialize_buffers(buffers, files) {
  for file in args.files {
    buffers.add(create_buffer(Some(file)))
  }
  if buffers.is_empty() {
    buffers.add(create_buffer(None))
  }
}

show_prompt(writer, bufferuffer) {
  // will eventually test for changed buffer
  write(writer, ':')
}
accept_command(reader, cmd_input) {
  cmd_input.clear()
  read_str(reader, &cmd_input)
}

parse_command(cmd_input) -> Result<Cmd> {
  // parse the command string
}
