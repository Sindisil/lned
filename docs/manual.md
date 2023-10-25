## Command line

    lned [filename]

If a filename is specified on the command line, lned will attempt to open
and read the file's contents into an edit buffer. If not, an empty edit
buffer will be presented.

## Description

_lned_ is a line-oriented text editor in the spirit of _ed_, the standard
Unix editor. It implements much of _ed_'s featuers, and is intended to be
expanded to provide functional parity. In addition, there are additional features
planned for _lned_ to make using it more enjoyable and productive.

As with most line editors, editing with _lned_ is done in two modes:
_command mode_ and _input mode_.

### Command mode

The specific syntax of _lned_'s commands are listed in the [_Commands_](#commands)
section, below, but in general they consist of zero or more line addresses, followed
by a one character command specifier, possibly followed by any arguments to the
specific command.

In command mode, a ':' prompt is presented, and a single command is accepted. If the
command accepts lines of text, those are entered in _input mode_.

### Input Mode

In _input mode_, lines of text are accepted until a line containing a single '.'
character is entered. The terminating line is not considered part of the input text.
No commands or character escapes are recognized while in _input mode_.

## Line Addressing

All lned commands act on whole lines or spans of lines.

Commands consist of zero or more line addresses, followed by a
command, possibly folowed by additional parameters:

	[address[,address]]command[parameters]

The addresses specify the line or span of lines the command will affect.
Default addresses apply if fewer addresses are specified than the
command can accept.

Regular expressions may be used to specify some line addresses. Several
commands also accept regular expressions as parameters.

An address specifies the number of a line in an edit buffer.
The _current address_ is kept track of by _lned_. It is
ofen used as the default address when none is specified.

The current address is usually set to the last line affected by a command
(e.g., the last line of a file when a file is first read, the last line
typed after an insert command, the first line after a deleted span of lines).

The address 0 (zero) points before the first line and is only valid with
certain commands. This isspecified in those commands' detailed descriptions below.

An address may be a literal number, one of the address symbols defined
below, or an address symbol followed by a numeric offset expression.

An address range, or span, consists of two addresses separated by a
comma or semicolon. It is an error for a line address to be smaller
than one that precedes it on the command line.

### Address symbols

Several symbols have special meaning within a line address.

* '.' is iterpreted as the address of the _current line_
* '$' is interprete as the address of the last line in the buffer
* '/' characters delimiting a regex address the first line found 
      to match the regex searching *forward* through the buffer
      starting with the _current line_
* '?' characters delimiting a regex address the first line found
      matching the regex searching *backward*, starting with the
      _current line_
* '+n' or '-n', where 'n' is a decimal number, addresses the
      _current line_ plus or minus the specified number. If the
      number is left out, it is assumed to be 1.
* 'n', where 'n' is a decimal number, addresses the 'n'th line
      in the buffer

In addition, line addresses can be followed by zero or more address
offsets, which may optionally be separated by blanks ('\t' or ' '
characters). Address offsets consist of:

* A '+' or '-', followed by a decimal number, to add or subtract
that number from the address. If no number is specified, it is assumed
to be 1, and if no '+' or '-' is specified, addition will be assumed.


Addresses are separated by ',' (comma) or ';' (semicolon). Comma is a simple separator,
but semi-colon causes the _current line_ to take the value of the preceding
address before evaluating the next. This has several uses, such as determining the
starting line for regex specified line addresses.

Any blank characters ('\t' or ' ') between addresses, address separators, or address
offsets are ignored.

Addresses omittd on eithr side of an address separator are evaluated as follows:

,	: 1,$
,addr	: 1,addr
addr,	: addr,addr
;	: .;$
;addr	: .;addr
addr;	: addr;addr


## Commands

### (null command)

#### Syntax

(.,+1)

#### Behavior

An address alone on a line will display the addressed line.
A newline alone on a line will display the next line (i.e., equivalent to +1p).
The line displayed becomes the current line.

### 'a' (append)

#### Syntax

(.)a
<input text>
.

#### Behavior

Text is accepted in input mode, and the resulting lines are appended after the addressed line.
The last appended line, or, if none, the addressed line, becomes the current line.
A line address of '0' is valid for the append command; the input text will then be placed at the
beginning of the buffer.

### 'd' (delete)


#### Syntax

(.,.)d

#### Behavior

The addressed lines are deleted from the buffer.

The current line is set to the first line after the deleted span.
If the deleted lines were atthe end of the buffer, the new last line becomes the current line.
If the buffer is empty after addressed lines are deleted, the current line becomes 0.

### 'e' (edit)

#### Syntax

e [file]

#### Behavior

If buffer not empty, delete contents, then read specified file into buffer.

The current line number will be set to the address of the last line in the buffer.

If no filename is specified, the currently remembered filename will be used,
if it is set, otherwise an error message will be output.

If a file is read, the number of bytes read are displayed.

The remembered filename will be changed to the filename specified, if any.

It is not an error for the filename to not exist, though a message will be
displayed in that case.

If the buffer has changed since the last time the entire buffer has been written,
the user will be warned. As with the quit command, a second successive edit command
will proceed, even if the buffer has been changed.

An 'e' command may be undone.

### 'f' (file)

#### Syntax

f [filename]

#### Behavior

Set the buffer's remembered filename to the specified value, if any.
Regardless of whether a new filename is specified, the buffer's currently
remembered filename is printed to stdout.

The current line is not affected by this command.

'n' (number, or enumerate)

#### Syntax

(.,.)n

#### Behavior

Write the addressed lines to stdout, prefixing each line with its line number. The line number will be right justified within a field wide enough to hold the largest line number in the addressed span, and will be separated from the line content by two sp

The last line written becomes the current line.

'p' (print)

#### Syntax

(.,.)p

#### Behavior

The addressed lines are written to stdout. The last line written
becomes the current line.


'u' (undo)

#### Syntax

u

#### Behavior

The most recent command is reverted.

Revertible actions are kept on an undo stack. The 'u' command
pops the top item and uses that information to revert the associated
action (e.g., undoing a 'd' command causes the deleted lines to be
re-inserted into the buffer).

The current line is reset to its value before the reverted command was
executed.

All commands executed as part of a 'g' command are reverted as one
action.

'w' (write)

#### Syntax

(1,$)w [filename]

#### Behavior

The *w* command writes the addressed lines into the file with the specified filename, replacing the existing contents, or creating the file if it does not exist.

The currently remembered filename will not be changed unless there is not a remembered filename.

If no filename is given, the currently remembered filename will be used, or an error shown if there is none.

The current line number will not be changed.

The number of bytes written  is printed to stdout if successful.

If the full buffer is written, buffer is marked clean.
