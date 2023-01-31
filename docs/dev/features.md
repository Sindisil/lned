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
* limited line range specification
  - literal single line number
  - 0   indicates the location before the first line.
  - %   All lines in the buffer.

* <range>a     Append after specified line range
  - '.' alone as first character on a line terminates input
* <range>d     Delete specified line range
* <range>n     Print specified line range prefixed with their line numbers
* q!           quits lned unconditionally, discarding unwritten changes
* w            W buffer to current file path
  - Show error if no current file path, or if error occurs writing file.
* f _file_     set current file path
  - If the file path isn't specified, the current file path is printed
