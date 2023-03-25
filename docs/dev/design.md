# Working lned design document

## Goals and purpose

The lned editor is intended primarily to serve as a way to evaluate Rust in a
"real" project. Additionally, I'm interrested in exploring the theory that I
work best using relatively straightforward and simple tools. (for values of "best"
that encompass comfort, happiness, and effectiveness/productivity).

To that end, and just for fun, I'm bootstrapping lned using ed, and then intend
to se lned to complete its own development. As of this writing, the plan is then
to use lned to bootstrap development on a terminal based screen editor (scrned).

For these reasons, simplicity (to the point of "simplest thing that will reasonably work")
should be the overarching design focus for lned.

This document is intended to flesh out the design of lned using pseudocode.


## Data Types

struct CmdArgs {
  files: Vec
}

struct EditBuffer {
  text: TextBuffer;
  filename: Option(PathBuf);
}

## Design

args = parse_command_line()
initialize_buffers(&buffers, &args.files)
current_buffer = &buffers[0]
Loop {
  Show_Prompt(current_buffer)
  Accept_Command
  Parse_Command
  Execute_Command
}

initialize_buffers(buffers, files) {
  for file in args.files {
    buffers.add(create_buffer(Some(file)))
  }
  if buffers.is_empty() {
    buffers.add(create_buffer(None))
  }
}
