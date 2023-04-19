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

Easiest option would be to clone ed and add some functionality to make it more
productive (e.g., better join command, auto indent in input mode, some sort of
wrap and/or justify, multiple buffers, useful prompt).

Another option would be to try to make a non-modal line editor. The idea being
that just typing would append text, and other commands would be accessed similarly
to non-modal screen editors. Would therefor need to take input in raw mode or
use a crate to do something similar so that hotkeys can be accepted.

Common commands would have their own hotkeys, less common commands might need to
be accessed at a command promp (sometimes called a command pallette). Examples
might be:

ctrl+HOME = navigate to 0th line to allow input before first line.
ctrl+END = navigate to last line and display it
PgUp = equivalant of .-<WinSz>zn in ed (i.e., "scroll" WinSz lines, starting
       back WinSz lines, leaving current_line as last line displayed
ctrl+g = prompt for line number, set current_line to the specified line, display
         it if it's not line 0.
ctrl+shift+p = prompt for command
Delete = delete current line
DownArrow = if current_line isn't already last line, increment current_line and
            display it.

Upside: no modal input
Downside: need raw mode, not amenable to scripting, possibly less discoverable
          if modal editor has decent help & error messages (somewhat offset by
          using CUA & other common key bindings, maybe).

**Decision: continue with modal 'ed' clone with extensions, for two reasons:
  1. Will continue to be useful for scripted editing once scrned is completed
  2. Simpler to implement. Since lned is inteded as an experiment, not necessarily
     a long term tool, that is the overriding consideration.


## Data Types

struct CmdArgs {
  files: Vec
}

struct EditBuffer {
  text: TextBuffer;
  filename: Option(PathBuf);
  current_line: isize;
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
