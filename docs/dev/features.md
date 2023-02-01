# Feature summary

This document describes lned's feature set. It is split into two main
sections: MVP (minimum viable product) and Enhancements.

# MVP Feature Set

This section describes the minimum features to allow lned to be used
(albeit painfully) for its own continued development. Once these
features are complete I could (and will try to) dogfood lned
development elopment.

* launch, with optional file path to read in
  - If no file is specified, create empty edit buffer reading file.
* Simple Prompt (":")

## Line Addressing

A address is the number of a line in a buffer. *lned* keeps track of the
_current address_, which is commands typically use if no address is
specified. When a file is read into a buffer, thecurrent address is
the last line read. After a command executes, the current address
is set to the last line affected by a command.

A line address is specified by one of:

.        The current line address.
$        The last line in the buffer.
_n_      The _n_th line in the buffer, in the range [0,$]
0        Before the first line.

An address range is a closed range, represented by two line
addresses separated by a comma. The value of the first address must not
exceed the value  of the second. If only one address is given
where a range is expected, it is treated as if the specified
line was given as both the beginning and end of the range.
If a line range is given where a command expects a single line
address, the last line specified by the range is used.

Two special shortcuts for common ranges exist:

, or %   All the lines in the buffer; equivalent to the range 1,$.
;        The current through last lines in the buffer; equivalent to
         the range .,$.

## Commands

* <line>a      Append after specified line
  - '.' alone as first character on a line terminates input
* <line>d      Delete specified line range
* <line>n      Print specified line range prefixed with their line numbers
* q!           quits lned unconditionally, discarding unwritten changes
* w            W buffer to current file path
  - Show error if no current file path, or if error occurs writing file.
* f _file_     set current file path
  - If the file path isn't specified, the current file path is printed
