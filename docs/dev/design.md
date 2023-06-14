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

Does an insert followed by a delete of the same lines count? I don't think so, as that's
equivalent to an undo, but how to tell?

We'll create a hash at the start of a buffer session (new empty buffer or after initial
read of file content), as well as a current_hash (initially the same as the initial_hash,
obviously). When a command is executed that might cause the buffer to become dirty, the
current_hash is recalculated. buffer.is_dirty() is then just:

impl EditBuffer {
  fn is_dirty() -> bool {
    initial_hash != current_hash
  }
}

If we eventually add support for file watching, activity on the file could trigger
recalculation of initial_hash from new file content.

When a "file" command is executed, a new initial_hash should be calculated from
the contents of the new file, if it exists.

Variables considered:
  * char count
  * line count
  * hash of content

New buffer:
  * char_count = 0;
  * line_count = 0;
  * hash = hash of empty line vec

In case of no file on disk, *any* content beyond empty means dirty.

"File on disk" is determined by default_file_name of buffer, so read from file,

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
