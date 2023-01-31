## Command line

  **lned [_file_ ...]**

lned can be launched with one or more specified file paths. lned
will attempt to open each in their own edit buffer, the the first
successfully loaded buffer, or a new empty buffer, will be initially
active.

## Description

_lned_ is a line-oriented text editor in the spirit of _ed_, the standard
Unix editor. it duplicates many of the commands of _ed_, with additional
functionality to improve ergonomics and usability.

If one or more file paths are specified when lned is run, an edit buffer
is created for each, and each file that exists is read into its associated
buffer. If no file paths are specified, a single empty edit buffer is created.
In either case, the first edit buffer is made active.

Editing is done modally, with two modes: _command_ and _input_. When first
run, lned is in command mode. In command mode, commands are read from
standard input and executed to modify the current edit buffer.

When an input command is given (e.g., a(ppend) or i(nsert)), lned enters
input mode. In this mode, standard input is written into the active edit
buffer. Lines consist of text up to and including a line terminator, which
is configurable, but defaults to CR (carriage return). A single period ('.')
entered on a line exits input mode.

All lned commands act on whole lines or spans of lines.

Commands consist of zero or more line addresses, followed by a
command, possibly folowed by additional parameters:

	[address[,address]]command[parameters]

The addresses specify the line or span of lines the command will affect.
Default addresses apply if fewer addresses are specified than the
command can accept.

Regular expressions may be used to specify some line addresses. Several
commands also accept regular expressions as parameters.

## Line Addressing

An address specifies the number of a line in an edit buffer.
__lned__ tracks the _current address_ for each edit buffer. This is
typically used as the default address when none is specified.
The current address is usually set to the last line affected by a command
(e.g., the last line of a file when a file is first read, the last line
typed after an insert command, the first line after a deleted span of lines).

The address 0 (zero) points before the first line.

An address may be a literal number, one of the address symbols defined
below, or an address symbol followed by a numeric offset expression.

An address range, or span, consists of two addresses separated by a
comma or semicolon.
The value of the first address must be equal to or less than the second.
