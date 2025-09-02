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

The specific syntax of _lned_'s commands are listed in the
[_Commands_](#commands) section, below, but in general they consist of
zero or more line addresses, followed by a one character command
specifier, possibly followed by any arguments to the specific command.

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

Commands consist of zero or more line addresses, followed by a command,
possibly folowed by additional parameters:

	[address[,address]]command[parameters]

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
	  the regex searching *forward* through the buffer starting with the
	  _current line_
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

,	    : 1,$
,addr   : 1,addr
addr,	: addr,addr
;		: .;$
;addr   : .;addr
addr;   : addr;addr

## Print Suffixes

Any command other than e, f, g, q, r, w, and ! may have an 'l', 'n', or
'p' added to their end. If this is the case, the command will be executed
and then the new current line will be written as described under the
'l' (list), 'n' (enumerate), or p (print) commands. Only one print suffix
is supported per command.

Note that, although the 'g' (global) command cannot itself have a print
suffix applied, commands supplied to the global command can.

## Commands

### Append ('a')

#### Syntax

(.)a
\<input text\>
.

#### Behavior

Text is accepted in input mode, and the resulting lines are apended after
the addressed line. The last appended line, or, if none, the addressed
line, becomes the current line. A line address of '0' is valid for the
append command; the input text will then be placed at the beginning of the
buffer.

### Change ('c')

#### Syntax

(.,.)c
\<input text\>
.

#### Behavior

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

### Copy ('t')

See Transfer.

### Delete ('d')

#### Syntax

(.,.)d

#### Behavior

The addressed lines are deleted from the buffer.

The current line is set to the first line after the deleted span. If the
deleted lines were atthe end of the buffer, the new last line becomes the
current line. If the buffer is empty after addressed lines are deleted,
the current line becomes 0.

### Edit ('e')

#### Syntax

e [file]

#### Behavior

If buffer not empty, delete contents, then read specified file into
buffer. If the last line of the file isn't terminated, a line terminator
will be appended.

The current line number will be set to the address of the last line in the
buffer.

If no filename is specified, the currently remembered filename will be
used, if it is set, otherwise an error message will be output.

If a file is read, the number of bytes read are displayed.

The remembered filename will be changed to the filename specified, if any.

It is not an error for the filename to not exist, though a message will be
displayed in that case.

If the buffer has changed since the last time the entire buffer has been
written, the user will be warned. As with the quit command, a second
successive edit command will proceed, even if the buffer has been changed.

### Enumerate ('n')

#### Syntax

(.,.)n

#### Behavior

Write the addressed lines to stdout, prefixing each line with its line
number. The line number will be right justified within a field wide enough
to hold the largest line number in the file, and will be separated from
the line content by two spaces.

The last line written becomes the current line.

### File ('f')

#### Syntax

f [filename]

#### Behavior

Set the buffer's remembered filename to the specified value, if any.
Regardless of whether a new filename is specified, the buffer's currently
remembered filename is printed to stdout.

The current line is not affected by this command.

### Global ('g')

#### Syntax

(1,$)g/__RE__/__commands__

#### Behavior

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

### Insert ('i')

#### Syntax

(.)i
\<input text\>
.

#### Behavior

Text is accepted in input mode, and the resulting lines are inserted
before the addressed line. The last inserted line, or, if none, the
addressed line, becomes the current line. A line address of '0' is valid
for the insert command; the input text will then be placed at the
beginning of the buffer.

### Join ('j')

#### Syntax

(.,.+1)j

#### Behavior

Join addressed contiguous lines by removing the intervening line
terminators.

If exactly one address is given, no action will be taken.

If any lines are joined, the current line will be set tot he address of
the resulting joined line, otherwise the current line number will not
be set.

### Line Number ('=')

#### Syntax

($)=

#### Behavior

Writes the line number of the addressed line to stdout.
The current line number will be unchanged.

### List ('l')

#### Syntax

(.)l

#### Behavior

The addressed lines are written to stdout with the
some special characters, and the end of line, displayed
visually as follows:

* HT (horizontal tab):   \t
* CR (carriage return):  \r
* LF (line feed):        \n
* EOL (end of line):      $
* $ within text:         \$


### Move ('m')

#### Syntax

(.,.)m\<destination\>

#### Behavior

Move the addressed lines to just after the last line specified by
\<destination\>.

If '0' is specified as the destination, the addressed lines are moved to the
beginning of the buffer. The destination may not fall within the span of moved
lines.

The current line number will be set to the resulting address of the last line
moved.

### Null command ('')

#### Syntax

(.,+1)

#### Behavior

An address alone on a line will display the addressed line. A newline
alone on a line will display the next line (i.e., equivalent to +1p). The
line displayed becomes the current line.

### Print ('p')

#### Syntax

(.,.)p

#### Behavior

The addressed lines are written to stdout. The last line written becomes
the current line.

### Quit ('q')

#### Syntax

q

#### Behavior

Exits the editor. If there are unsaved changes, a warning will be
printed. Repeating the Quit command will discard the changes and
exit.

### Read ('r')

#### Syntax

($)r [file]

#### Behavior

Inserts the contents of the specified file into the buffer after
the specified address, or after the current_line if no address
is specified.

If the last line of the file isn't terminated, a line terminator
will be appended.

The current line number will be set to the address of the last line
inserted.

If no filename is specified, the currently rememberd filename will be
used, if it is set, otherwise an error message will be output.

If a file is read, the number of lines and bytes read are displayed.

The remembered filename will be set to the filename specified, if any.

If the filename doesn't exist, an error message will be displayed,
and the current line will not change.

A read may be undone as if it were an Insert command. As such, if
a Read is undone, then redone by issuing a Redo command, the file
is *not* reread; the lines previously read are simply reinserted.

### Redo ('U')

#### Syntax

U

#### Behavior

Reverts the most recently undone command.
The most recent item is popped from the undo stack and executed.

As with direct commands, the redone command is then pushed to the
undo stack.

For more details about the undo/redo system, see the 'u' (undo) command.

### Scroll ('z')

#### Syntax

(.)z[count]

#### Behavior

Prints 'count' display lines from buffer, setting the scroll window size
to 'count'. Printing will begin with the addressed line, or current_line
if no address is given.

If 'count' is not given, the current scroll window size is used.
The scroll window size defaults to display height - 2, or 22 if
the display height can't be determined.

Note that the scroll window size is a number of display lines, not buffer
lines.

The current_line is set to one past the last line displayed, or buffer
end, whichever is smaller.

If any print suffixes are specified, all lines will be displayed
accordingly.

### Show diff ('S')

#### Syntax

S [filename]

#### Behavior

Shows the differences between the current buffer contents and
the specified filename's contents.

If no filename is specified, the current remembered filename is
used if it is set, otherwise an error is given.

The current remembered filename is not changed by this command.

### Substitute ('s')

#### Syntax

(.,.)s/regex/replacement/flags

#### Behavior

Matches each line in the addressed range witht he specified regular
expression pattern, replacing one or all (depending upon flags)
non-overlapping occurances with the specified replacement pattern.
An error will be reported if no matches are found.

Any character other than ' ' (space) or '\n' (new line) may be used
instead of '/' (slash) to delimit the regex, and within the regex
the delimiter may be used as a literal character if escaped by a
'\' (backslash) character.

The current line will be set to the line on which the last replacement
was made.

The regex syntax is that supported by the Rust regex crate, and
the replacement pattern syntax is that supported by that crate's
replace() method.

See the regex crate's documentation for more details:

[regex](https://docs.rs/regex/1.11.0/regex/index.html#syntax).
[replace()](https://docs.rs/regex/1.11.0/regex/struct.Regex.html#method.replace) method.

Flags may be either (but not both) of:

* 'g'    Globaly replace all non-overlapping of regex with replacement
* _number_    Replace the _number_th occurrance of regex with replacement

### Copy ('t')

See Transfer.

### Transfer ('t')

#### Syntax

(., .)t\<destination\>

#### Behavior

Copy the addressed lines to just after the last line specified by
\<destination\>.

If '0' is specified as the destination address, the
addressed lines are copied to the beginning of the buffer. The
destination may not fall within the span of copied lines.

The current line number will be set to the resulting address of the
last line copied.

### Undo ('u')

#### Syntax

u

#### Behavior

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

### Write ('w')

#### Syntax

(1,$)w [filename]

#### Behavior

The *w* command writes the addressed lines into the file with the
specified filename. If no filename is given, the currently remembered
filename will be used, or an error shown if there is none.

If the file named doesn't exist, it will be created. If it already
exists, a warning will be displayed and no write will occur, nor will
the remembered filename be changed. A second identical write command
will override the warning, overwriting the file's contents. No warning
will be displayed if the remembered filename is used (i.e., if no
filename is specified) -- this is essentially a "save" command.

If the file is written, the remembered filename will be set to the
specified filename if it had not already been set, otherwise it will
remain unchanged.

The current line number will not be changed.

The number of bytes written  is printed to stdout if successful.

If the full buffer is written, buffer is marked clean.
