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

Commands are listed with the default address or address range
(in parentheses) used if none is given. Possible arguements are shown
as applicable.

* (.)a            Appends text entered in input mode after the specified
                  line, setting current address to the last line entered.
* (.,.)d          Delete specified line range
* (.,.)n          Print specified line range prefixed with their line
                  numbers
* q               quits lned
* q!              quits lned unconditionally, discarding unwritten
                  changes
* (1,$)w _file_   Write the specified lines to __file__, overwriting
                  previous contents without warning. If there is no
                  default filename, it is set to __file_, otherwise it
                  is unchanged. If __file__ is not given, the default
                  filename is used.
                  The current address is not changed.
* f _file_        Set default filename to _file_. If no _file_ is given,
                  prints the default filename.

## Input Mode

When an input command like *a* (append) is given, lned enters input mode.
When in input mode, commands are not availabile -- standard input is
instead collected until input mode is terminated by a single period ('.')
on a line. Lines are terminated by CR or CRLF.
