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
certain commands. This is specified in those commands' detailed
descriptions below.

An address may be a literal number, one of the address symbols defined
below, or an address symbol followed by a numeric offset expression.

An address range, or span, consists of two addresses separated by a comma
or semicolon. It is an error for a line address to be smaller than one
that precedes it on the command line.

### Address symbols

Several symbols have special meaning within a line address.

* '.' is iterpreted as the address of the _current line_
* '$' is interpreted as the address of the last line in the buffer
* '%' is interpreted as all lines in the buffer (i.e., equivalant to .,$)
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

If more addresses are specified than a command accepts, they are still
evaluated, but if 'n' addresses are required, only the final 'n' will be
passed to the command. For example, if a command takes a single line, and
the address '1,5' is passed, the line '5' will be passed to the command.

Addresses omitted on either side of an address separator are evaluated as
follows:

,	: 1,$
,addr	: 1,addr
addr,	: addr,addr
;	: .;$
;addr	: .;addr
addr;	: addr;addr

## Print Suffixes

Most commands may have an 'l', 'n', or 'p' added to their end. This is
called a "print suffix". If this is the case, the command will be
executed and then the new current line will be written as described
under the 'l' (list), 'n' (enumerate), or p (print) commands. Only one
print suffix is supported per command.

Note that, although the 'g' (global) command cannot itself have a print
suffix applied, commands supplied to the global command can.

## Commands

### '' Null command

    (+1)

An address alone on a line will display the addressed lines. A newline
alone on a line will display the next line (i.e., equivalent to +1p). The
last line displayed becomes the current line.

### '=' Line Number 

    ($)=

Writes the line number of the addressed line to stdout.
The current line number will be unchanged.

### 'a' Append

Text is read from the input source, and the resulting lines are inserted
into the buffer after the addressed line. If lines are appended, the
current line is set to the last line appended, otherwise the addressed
line. Newline sequences for the input lines are normalized to match the
buffer's prevailing style. Other details specific to the input source are
detailed below.

#### Append from terminal

    (.)a

Lines input at the terminal are the input source.

In addition to the base append behavior, input text is prompted for with an
auto-indent prefilled. The indent prefill is set to match the first non-
blank line at or before the addressed line, and each additional input
prompt is auto-indented to match the previously entered line.

#### Append from clipboard

    (.)av

Lines from the application clipboard are used as the input source.

#### Append from file

    (.)a filename

Lines read from the file specified are used as the input source.

If the final line read is unterminated, a newline sequence matching the
buffer's prevailing EOL style is appended to it. A message detailing the
number of lines and bytes read is printed to the terminal, as well as an
indication if a missing final newline was appended.

If there are errors opening or reading the specified file, the current
line remains unchanged.

### 'A' Append raw

Text is read from the input source, and the resulting lines are inserted
into the buffer after the addressed line. If lines are appended, the
current line is set to the last line appended, otherwise the addressed
line. No other modifications (e.g. normalization of newline style) are
performed. Other details specific to the input source are detailed below.

#### Append raw from terminal

    (.)A

Lines input at the terminal are the input source.

#### Append raw from clipboard

    (.)Av

Lines from the application clipboard are used as the input source.

#### Append raw from file

    (.)A filename

Lines read from the file specified are used as the input source.

If the final line read is unterminated, a newline sequence is appended to
it. The appended newline will match the prevailing style for the lines read. A message detailing the number of lines and bytes read is printed to
the terminal, as well as an indication if a missing final newline was appended.

If there are errors opening or reading the specified file, the current
line remains unchanged.

### 'c' Copy

    (.)c

The addressed lines replace any existing lines in the application
clipboard. The current line remains unchanged.

### 'd' Delete

    (.,.)d

The addressed lines are deleted from the buffer.

The current line is set to the first line after the deleted span. If the
deleted lines were at the end of the buffer, the new last line becomes the
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

    L (LF|CRLF)

Set the buffer's prevailing newline to the one specified, if any. If a
newline is specified, buffer lines are normalized to that newline.
Regardless of whether a new newline is specified, the prevailing newline
is printed to stdout.

The current line is not affected by this command.

### 'f' Filename

    f

Prints the current filename and the prevailing newline.
The current line is not affected by this command.

### 'g' Global 

    (1,$)g/__RE__/__commands__

The g command will first note every line matching the specified regex.
Then, working from beginning to end, the command list will be executed
for each matching line, with the current line set to the address of that
line. Any matched line modified by the command list will be removed from
the list of matching lines. Any error will immediately stop execution.
Any character other than ' ' (space) or '\n' (new line) may be used
instead of '/' to delimit the regex, and within the regex the delimiter
may be used as a literal character if escaped by a '\' character.

Unless errors are encountered, the current line will eventually be set to
the value assigned by the last command in the command list. If there were
no matching lines, the current line will not change.

The first command in the command list must appear on the same line as the
global command. If more than one command is specified in the global command
list, each but the last must end with an ampersand ('&') character as a
separator. Newlines within commands may be escaped with a backslash ('\').

The list of permitted commands in a global command list includes any of:
'a', 'A', 'c', 'd', 'i', 'I', 'j', 'n', 'o', 'O', 'p', 's', and 'x'.
Input lines associated with the append, insert, and overwrite commands
must be included in the command list. The terminating '.' may be omitted
if it would be the last line in the command list.

If no command is provided, it will be interpreted as if a 'p' command were
given.

Only those commands in the command list that successfully modify the edit
buffer will be included when *undo*ing or *redo*ing a global command.

### 'i' Insert

Text is read from the input source, and the resulting lines are inserted
into the buffer before the addressed line. If lines are inserted, the
current line is set to the last one, otherwise to the addressed line.
Newline sequences for the input lines are normalized to match the
buffer's prevailing style. Other details specific to the input source are
detailed below.

#### Insert from terminal

    (.)i

Lines input at the terminal are the input source.

In addition to the base insert behavior, input text is prompted for with an
auto-indent prefilled. The indent prefill is set to match the first non-
blank line at or before the addressed line, and each additional input
prompt is auto-indented to match the previously entered line.

#### Insert from clipboard

    (.)iv

Lines from the application clipboard are used as the input source.

#### Insert from file

    (.)i filename

Lines read from the file specified are used as the input source.

If the final line read is unterminated, a newline sequence matching the
buffer's prevailing EOL style is appended to it. A message detailing the
number of lines and bytes read is printed to the terminal, as well as an
indication if a missing final newline was appended.

If there are errors opening or reading the specified file, the current
line remains unchanged.

### 'I' Insert raw

Text is read from the input source, and the resulting lines are inserted
into the buffer before the addressed line. If lines are inserted, the
current line is set to the last line appended, otherwise the addressed
line. No other modifications (e.g. normalization of newline style) are
performed. Other details specific to the input source are detailed below.

#### Insert raw from terminal

    (.)I

Lines input at the terminal are the input source.

#### Insert raw from clipboard

    (.)Iv

Lines from the application clipboard are used as the input source.

#### Insert raw from file

    (.)I filename

Lines read from the file specified are used as the input source.

If the final line read is unterminated, a newline sequence is appended to
it. The appended newline will match the prevailing style for the lines read. A message detailing the number of lines and bytes read is printed to
the terminal, as well as an indication if a missing final newline was appended.

If there are errors opening or reading the specified file, the current
line remains unchanged.

### 'j' Join 

    (.,.+1)j(/separator/)

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

### 'J' Justify

    (.)J(wrapping_style)(left_margin)( line_width)

Justifies text lines according to the specified parameters by replacing
leading whitespace with the appropriate number of spaces (' '), leaving
the right margin ragged and setting current line to last line in newly
justified span of lines.

The optional left_margin defaults to the number of columns preceding the
first printable character on the first line to be justified.

The optional line_width defaults to terminal width, and must be preceded
by one or more blanks (' ' or '\t') if specified.

The optional wrapping style may be specified with one of the following
characters and it defaults to Fill if unspecified.

'/' NoFill  Move words to start of next line as necessary to maintain
            line width and left margin, possibly inserting new lines
            after those addressed, but do not fill from next line even
            if there would be room. Only overflow margins if unbroken
            word is wider than the space between margins.
'^' Fill    Move words to or from the ends of lines as necessary to
            maintain line width and left margin, possibly inserting
            new lines after those addressed. Only overflow margins
            if unbroken word is wider than the space between margins.
'!' None    Do not wrap. Line width, if specified, is ignored.
 
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

### 'o' Overwrite

Text is read from the input source, the addressed lines are deleted, and
the input lines are inserted after the first line preceding the addressed
span. If lines are inserted, the current line is set to the last one,
otherwise to the first line after the deleted span, or the last buffer
line if the span was at the end of the buffer. Newline sequences for the
input lines are normalized to match the buffer's prevailing style. Other
details specific to the input source are detailed below.

#### Overwrite from terminal

    (.)o

Lines input at the terminal are the input source.

In addition to the base overwrite behavior, input text is prompted for with
an auto-indent prefilled. The indent prefill is set to match the first non-
blank line at or before the addressed line, and each additional input
prompt is auto-indented to match the previously entered line.

#### Overwrite from clipboard

    (.)ov

Lines from the application clipboard are used as the input source.

#### Overwrite from file

    (.)o filename

Lines read from the file specified are used as the input source.

If the final line read is unterminated, a newline sequence matching the
buffer's prevailing EOL style is appended to it. A message detailing the
number of lines and bytes read is printed to the terminal, as well as an
indication if a missing final newline was appended.

If there are errors opening or reading the specified file, the current
line remains unchanged.

### 'O' Overwrite raw

Text is read from the input source, the addressed lines are deleted, and
the input lines are inserted after the first line preceding the addressed
span. If lines are inserted, the current line is set to the last one,
otherwise to the first line after the deleted span, or the last buffer
line if the span was at the end of the buffer. No other modifications
(e.g. normalization of newline style) are performed. Other details specific
to the input source are detailed below.

#### Overwrite raw from terminal

    (.)O

Lines input at the terminal are the input source.

#### Overwrite raw from clipboard

    (.)Ov

Lines from the application clipboard are used as the input source.

#### Overwrite raw from file

    (.)O filename

Lines read from the file specified are used as the input source.

If the final line read is unterminated, a newline sequence is appended to
it. The appended newline will match the prevailing style for the lines read. A message detailing the number of lines and bytes read is printed to
the terminal, as well as an indication if a missing final newline was appended.

If there are errors opening or reading the specified file, the current
line remains unchanged.

### 'p' Print 

    (.,.)p

The addressed lines are written to stdout. The last line written becomes
the current line.

### 'q' Quit 

    q

Exits the editor. If there are unsaved changes, a warning will be
printed. Repeating the command will discard the changes and exit.

### 'S' Show diff 

    S (filename)

Shows the differences between the current buffer contents and the
specified filename's contents.

If no filename is specified, the current current filename is used if it
is set, otherwise an error is given.

The current current filename is not changed by this command.

### 's' Substitute 

    (.,.)s/regex/replacement/(target_match)

Matches each line in the addressed range with the specified regular
expression pattern, replacing all non-overlapping occurances with the
specified replacement pattern, or a single occurance indicated by an
integer indicating which match should be replaced. An error will be
reported if no matches are found.

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

### 't' Transfer (a.k.a. Copy)

    (., .)t(destination)

Copy the addressed lines to just after the last line specified by
destination.

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

The clipboard is unaffected by undo/redo commands.

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

    (1,$)w (filename)

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

### 'x' Cut

    (.)x

The addressed lines replace any existing lines in the application clipboard
and are deleted from the buffer. The first line after the addressed lines
becomes the current line. If the last addressed lines were at the end of
the buffer, the last remaining buffer line becomes the current line.

### 'z' PageDown

    (.+1)z(page_length)

Print lines filling the specified number of display rows, up to one less
than the display height, beginning with the addressed line. Any specified
print_suffixes will be applied to each line printed. If no address is
specified, the line following the current_line is used. If no page_length
is specified, the previously remembered page_length is used, or, if none
is remembered, the default of display height - 3 rows is used. If a
page_length > 0 is
specified, it is remembered. If a page_length of 0 is specified, the
remembered page number, if any, is forgotten and the default is used.

After printing, the current_line is set equal to the last line printed.

It is not an error if there are less lines to print than will fit the
page_length.

### 'Z' PageUp

    (.-1)Z(page_length)

Print lines filling the specified number of display rows, up to one less
than the display height, such that the addressed line is the last printed.
Any specified print_suffixes will be applied to each line printed. If no
address is specified, the line preceding the current line is used. If no
page_length is specified, the previously remembered page_length is used,
or, if none is remembered, the default of display height - 3 rows is used.
If a page_length > 0 is specified, it is remembered. If a page_length of 0
is specified, the remembered page number, if any, is forgotten and the
default is used.

After printing, the current_line is set equal to the first line printed.

It is not an error if there are less lines to print than will fit the
page_length.
