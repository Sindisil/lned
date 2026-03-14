## Command line

    lned [filename]

If a filename is specified on the command line, lned will attempt to
open and read the file's contents into an edit buffer. If not, an empty
edit buffer will be presented.

## Description

_lned_ is a line-oriented text editor in the spirit of _ed_, the
standard Unix editor. It implements much of _ed_'s featuers, and is
intended to be expanded to provide functional parity. In addition, there
are additional features planned for _lned_ to make using it more enjoyable
and productive.

As with most line editors, editing with _lned_ is done in two modes:
_command mode_ and _input mode_.

### Command mode

Commands consist of zero or more line addresses, followed by a command,
possibly folowed by additional parameters:

	[address[,address]]command[parameters]

The specific syntax of _lned_'s commands are listed in the
[_Commands_](#commands) section, below.

In command mode, a ':' prompt is presented, and a single command is
accepted. If the command accepts lines of text, those are entered in
_input mode_.

### Input Mode

In _input mode_, lines of text are accepted until a line containing a
single '.' character is entered. The terminating line is not considered
part of the input text. No commands or character escapes are recognized
while in _input mode_.

## Line Addressing

All lned commands act on whole lines or spans of lines.

The addresses specify the line or span of lines the command will affect.
Default addresses apply if fewer addresses are specified than the command
can accept.

Regular expressions may be used to specify some line addresses. Several
commands also accept regular expressions as parameters.

An address specifies the number of a line in an edit buffer. The
_current address_ is kept track of by _lned_. It is ofen used as the
default address when none is specified.

The current address is usually set to the last line affected by a command
(e.g., the last line of a file when a file is first read, the last line
typed after an insert command, the first line after a deleted span of
lines).

The address 0 (zero) points before the first line and is only valid with
certain commands. This isspecified in those commands' detailed
descriptions below.

An address may be a literal number, one of the address symbols defined
below, or an address symbol followed by a numeric offset expression.

An address range, or span, consists of two addresses separated by a comma
or semicolon. It is an error for a line address to be smaller than one
that precedes it on the command line.

### Address symbols

Several symbols have special meaning within a line address.

* '.' is iterpreted as the address of the _current line_
* '$' is interprete as the address of the last line in the buffer
* '/' characters delimiting a regex address the first line found to match
	the _current line_
* '?' characters delimiting a regex address the first line found matching
	the regex searching *backward*, starting with the _current line_
* '+n' or '-n', where 'n' is a decimal number, addresses the
	_current line_ plus or minus the specified number. If the number is
	left out, it is assumed to be 1.
* 'n', where 'n' is a decimal number, addresses the 'n'th line in the
	buffer

In addition, line addresses can be followed by zero or more address
offsets, which may optionally be separated by blanks ('\t' or ' '
characters). Address offsets consist of '+' or '-', followed by a decimal
number, to add or subtract that number from the address. If no number is
specified, it is assumed to be 1, and if no '+' or '-' is specified,
addition will be assumed.

Addresses are separated by ',' (comma) or ';' (semicolon). Comma is a
simple separator, but semi-colon causes the _current line_ to take the
value of the preceding address before evaluating the next. This has
several uses, such as determining the starting line for regex specified
line addresses.

Any blank characters ('\t' or ' ') between addresses, address separators,
or address offsets are ignored.

Addresses omittd on either side of an address separator are evaluated as
follows:

,	: 1,$
,addr	: 1,addr
addr,	: addr,addr
;	: .;$
;addr	: .;addr
addr;	: addr;addr

## Print Suffixes

Any command other than e, f, g, q, r, w, and ! may have an 'l', 'n', or
'p' added to their end. If this is the case, the command will be executed
and then the new current line will be written as described under the
'l' (list), 'n' (enumerate), or p (print) commands. Only one print suffix
is supported per command.

Note that, although the 'g' (global) command cannot itself have a print
suffix applied, commands supplied to the global command can.

## Commands

### '' Null command

    (.,+1)

An address alone on a line will display the addressed line. A newline
alone on a line will display the next line (i.e., equivalent to +1p). The
line displayed becomes the current line.

### '=' Line Number 

    ($)=

Writes the line number of the addressed line to stdout.
The current line number will be unchanged.

### 'a' Append 

    (.)a
    \<input text\>
    .

Text is accepted in input mode, and the resulting lines are apended after
the addressed line. The last appended line, or, if none, the addressed
line, becomes the current line. A line address of '0' is valid for the
append command; the input text will then be placed at the beginning of the
buffer.

### 'c' Change 

    (.,.)c
    \<input text\>
    .

Text is accepted in input mode, the addressed lines are deleted, and the
input text is inserted in their place. After a change command, the
current address will be set to the first of:

    * The last replacement line
    * If no replacement lines, the line after those deleted
    * If lines deleted were at the buffer's end, the last remaining line
      in the buffer
    * If no lines remain, 0

An address of '0' is valid, and will result in no lines deleted and any
replacement text inserted at the beginning of the buffer. An address with
a lower bound of 0 is also valid, and will be interpreted in this case as
the first line in the buffer (i.e., 0 if empty, 1 otherwise).

### 'd' Delete

    (.,.)d

The addressed lines are deleted from the buffer.

The current line is set to the first line after the deleted span. If the
deleted lines were atthe end of the buffer, the new last line becomes the
current line. If the buffer is empty after addressed lines are deleted,
the current line becomes 0.

### 'e' Reload

    e

Reload the current file, replacing the current buffer contents.

Line terminators are normalized to the prevailing newline, and a final
newline is appended if missing. The current line number is set to the
address of the last line in the buffer. The number of lines and bytes
read is displayed, as is the prevailing newline.

If there are unsaved buffer changes, the user will be warned. Repeating
the command will procede, discarding changes.

### 'E' Edit  (a.k.a. Open)

    E file

Load the specified file, replacing the current buffer contents.

Line terminators are normalized to the prevailing newline, and a final
newline is appended if missing. The specified file becomes the new
current filename. The current line number is set to the address of the last
line in the buffer. The number of lines and bytes read is displayed, as
is the prevailing newline.

If there are unsaved buffer changes, the user will be warned. Repeating
the command will procede, discarding changes.

### 'L' Line Terminator (a.k.a. Newline)

    L [CR|CRLF]

Set the buffer's prevailing newline to the one specified, if any. If a
newline is specified, buffer lines are normalized to that newline.
Regardless of whether a new newline is specified, the prevailing newline
is printed to stdout.

The current line is not affected by this command.

### 'f' Filename

    f [filename]

Set the current filename to the specified value, if any. Regardless of
whether a new filename is specified, the currently current filename is
printed to stdout.

The current line is not affected by this command.

### 'g' Global 

    (1,$)g/__RE__/__commands__

The g command will first note every line matching the specified regex.
Then, working from beginning to end, the command list will be executed for
each matching line, with the current line set to the address of that line.
Any matched line modified by the command list will be removed from the
list of matching lines. Any error will immediately stop execution. Any
character other than ' ' (space) or '\n' (new line) may be used instead of
'/' to delimit the regex, and within the regex the delimiter may be used
as a literal character if escaped by a '\' character.

Unless errors are encountered, the current line will eventually be set to
the value assigned by the last command in the command list. If there were
no matching lines, the current line will not change.

The first command in the command list must appear on the same line as the
global command. All additional lines in the command list but the last must
be backslash terminated to escape the line terminator.

The list of permitted commands in a global command list includes any of:
'a', 'c', 'd', 'i', 'j', 'm', 'n', 'p', 's', and 't'. Input lines
associated with the 'a', 'c', and 'i' commands must be included in the
command list. The terminating '.' may be omitted if it would be the last
line in the command list.

If no command is provided, it will be interpreted as if a 'p' command were
given.

Only those commands in the command list that successfully modify the edit
buffer will be included when *undo*ing or *redo*ing a global command.

### 'i' Insert 

    (.)i
    \<input text\>
    .

Text is accepted in input mode, and the resulting lines are inserted
before the addressed line. The last inserted line, or, if none, the
addressed line, becomes the current line. A line address of '0' is valid
for the insert command; the input text will then be placed at the
beginning of the buffer.

### 'j' Join 

    (.,.+1)j[/separator/]

Join addressed contiguous lines by removing the intervening line
terminators, optionally inserting a separator string between each.
If a single address is specified, that line is joined with the next.

If a separator string is given, it replaces any leading whitespace
in each joined line past the first. Any character other than ' '
(space), '\n' (newline), 'n', 'l', or 'p' may be used instead of '/'
(slash) to delimit the separator string, and within the separator
string the delimiter may be used as a literal character if escaped
by a '\\' (backslash) character. The terminating delimiter is optional,
but eliding it precludes the use of a print suffix.

If any lines are joined, the current line will be set to the address of
the resulting joined line, otherwise the current line number will not
be set.

### 'l' List 

    (.)l

The addressed lines are written to stdout with the
some special characters, and the end of line, displayed
visually as follows:

* HT (horizontal tab):   \t
* CR (carriage return):  \r
* LF (line feed):        \n
* EOL (end of line):      $
* $ within text:         \$


### 'm' Move 

    (.,.)m\<destination\>

Move the addressed lines to just after the last line specified by
\<destination\>.

If '0' is specified as the destination, the addressed lines are moved to
the beginning of the buffer. The destination may not fall within the
span of moved lines.

The current line number will be set to the resulting address of the last
line moved.

### 'n' Enumerate

    (.,.)n

Write the addressed lines to stdout, prefixing each line with its line
number. The line number will be right justified within a field wide enough
to hold the largest line number in the file, and will be separated from
the line content by two spaces.

The last line written becomes the current line.

### 'N' New

Discard the buffer contents and unset current file.

A waring will be given if there are unsaved buffer changes. Repeating
the command will procede, discarding changes.

### 'p' Print 

    (.,.)p

The addressed lines are written to stdout. The last line written becomes
the current line.

### 'q' Quit 

    q

Exits the editor. If there are unsaved changes, a warning will be
printed. Repeating the command will discard the changes and exit.

### 'r' Read 

    ($)r [file]

Inserts the contents of the specified file (or the current file, if none
is specified) into the buffer after the specified address (or after the
current_line if no address is specified).

Line terminators are normalized to the prevailing newline, and a final
newline is appended if missing. The current line number is set to the
address of the last line inserted. The number of lines and bytes
read is displayed, as is the prevailing newline.

A read may be undone as if it were an Insert command. As such, if
a Read is undone, then redone by issuing a Redo command, the file
is *not* reread; the lines previously read are simply reinserted.

### 'S' Show diff 

    S [filename]

Shows the differences between the current buffer contents and the
specified filename's contents.

If no filename is specified, the current current filename is used if it
is set, otherwise an error is given.

The current current filename is not changed by this command.

### 's' Substitute 

    (.,.)s/regex/replacement/flags

Matches each line in the addressed range witht he specified regular
expression pattern, replacing one or all (depending upon flags)
non-overlapping occurances with the specified replacement pattern. An
error will be reported if no matches are found.

Any character other than ' ' (space) or '\n' (new line) may be used
instead of '/' (slash) to delimit the regex, and within the regex the
delimiter may be used as a literal character if escaped by a '\'
(backslash) character.

The current line will be set to the line on which the last replacement
was made.

The regex syntax is that supported by the Rust regex crate, and the
replacement pattern syntax is that supported by that crate's replace()
method.

See the regex crate's documentation for more details:

[regex](https://docs.rs/regex/1.11.0/regex/index.html#syntax).
[replace()](https://docs.rs/regex/1.11.0/regex/struct.Regex.html#method.replace) method.

Flags may be either (but not both) of:

* 'g'    Globaly replace all non-overlapping of regex with replacement
* _number_    Replace the _number_th occurrance of regex with
  replacement

### 't' Transfer (a.k.a. Copy)

    (., .)t\<destination\>

Copy the addressed lines to just after the last line specified by
\<destination\>.

If '0' is specified as the destination address, the addressed lines are
copied to the beginning of the buffer. The destination may not fall
within the span of copied lines.

The current line number will be set to the resulting address of the last
line copied.

### 'u' Undo 

    u

The most recent command is reverted.

Revertible actions are kept on an undo stack. The 'u' command pops the top
item and uses that information to revert the associated action (e.g.,
undoing a 'd' command causes the deleted lines to be re-inserted into the
buffer).

The current line is reset to its value before the reverted command was
executed.

All commands executed as part of a 'g' command are reverted as one
action.

Undone actions are themselves remembered on a redo stack, so that
they can be redone (effectively "undoing the undo").

If the redo stack was non-empty when a direct command is saved to the
undo stack, those commands are moved back to the undo stack, first in
reverse order, then in forward order but with inverted effect (i.e.,
deletes become inserts, transfers become deletes, etc.). This is so
that no history of edit actions are lost, including 'undo' commands.

### 'U' Redo 

    U

Reverts the most recently undone command.

The most recent item is popped from the redo stack and executed. As with
direct commands, the redone command is then pushed to the undo stack.

For more details about the undo/redo system, see the 'u' (Undo) command.

### 'w' Write (a.k.a. Save)

    w

Writes the buffer contents to the current file.

A warning will be displayed if the current file contents has changed
since last loaded or saved. Repeating the command will overwrite the
current file.

### 'W' Write As (a.k.a. Save As)

    (1,$)w [filename]

Writes the addressed lines into the file with the specified filename.

If the file named doesn't exist, it will be created. If it already
exists, a warning will be displayed and no write will occur. A second
identical write command will override the warning, overwriting the
file's contents.

If the full buffer is written and the current filename had not yet been
set, the current filename is set to the specified filename and the
buffer is considered to be saved.

The current line number will not be changed in any case.

The number of lines and bytes written is printed to stdout if successful.

### 'z' Scroll 

    (.)z[count]

Prints 'count' display lines from buffer, setting the scroll window size
to 'count'. Printing will begin with the addressed line, or current_line
if no address is given.

If 'count' is not given, the current scroll window size is used. The
scroll window size defaults to display height - 2, or 22 if the display
height can't be determined.

Note that the scroll window size is a number of display lines, not
buffer lines.

The current_line is set to one past the last line displayed, or buffer
end, whichever is smaller.

If any print suffixes are specified, all lines will be displayed
accordingly.
