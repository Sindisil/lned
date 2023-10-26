
# Features and tasks

## Create "what_why_how" doc in dev_docs

Status: planned
Issue: 71

Create a document to provide context to anyone reading the code:

1. What does 'lned' do?
2. Why does it exist?
3. How is it constructed (overall design, design & development philosophy, etc.)

A bare bones version is acceptable for the first milestone, and the expectation is
that the 'how' section may be extended or edited as development proceeds.

Readme.md should also contain at least a short version of "what" & "why", along
with a pointer to the w-w-h file, basic build instructions, and a pointer to user docs.

##  Create a feature tracking document in dev_docs

Status: Complete
Issue: 70

Create dev_docs/features.md so that it's possible to track proposed features not
yet worth entering as issues, as well as to allow work when GitHub is inaccessable.
It should contain (issue number(when known) & status, as well as other information such a

Status is one of:
    * Proposed: Would like to do feature, but not defined well enough to open a task
      or enhancement issue yet
    * Planned: Issue has been opened and at least a first cut at syntax and
      behavior has been defined
    * Functional: Code & most tests written and committed
    * Complete: All tests written and passing, docs updated, tracking issue closed

## Commenting pass

Status: planned
Issue: 69

Go over existing code and add comments as appropriate. Intent is for this to
be roughly production quality code (within my current mastery of idiomatic Rust, anyway).

## Read command line specified file into buffer

Status: planned
Issue: 4

Accept a single filename (path) on the command line.
Acts essentially as if an edit (e) command was issued as part of
launching lned, but w/o adding a Revert record to the undo stack.

## Rework error messages

Status: planned
Issue: 22

1. Wording doesn't match API guidelines (i.e., no need for word "error", etc)
2. Should probably populate source field as appropriate
3. Add test coverage for error message formatting.

## Basic global (g) command

Status: planned
Issue: 46

Syntax:

(1,$)g/RE/command

Behavior:

A subset of the full global command implementing essentially a "print matching lines" command.
Only '/' delimiters supported for search expression, and only a single
command accepted, which must e one of 'n', 'p', or the null command.

## U (redo) command

Status: planned
Issue: 53

Syntax:

U

Behavior:

Reverts the most recently undone command (i.e., redoes the command).

## Refactor, clean up, and extend unit tests

Status: planned
Issue: 60

1. I believe excessive redundancy has crept into the unit test coverage. Especially worth looking at tests for Address and read().
2. EditBuffer tests are crying for refactoring WRT boilerplate.
    a. Probably add a builder
    b. Maybe add setup functions as well.
3. None of the Display implementations for Errors are checked.

# Bugs

## Null cmd isn't parsed correctly

Status: planned
Issue: 62

No parse function to catch invalid suffix.

## d command shouldn't allow span beginning with 0 

Status: planned
Issue: 38

0,5d for example, should give an "invalid address" error.

Starting to think it would be easier to just always use spans internally,
and represent line actions as a span over a single line.

## Errors don't appear to be propegated out of EditBuffer::do_user_cmd()

Status: planned
Issue: 66

# Proposed work items

There are several features and commands planned for lned that will
differ from the basic __ed__ functionality which serves as the
inspiration for lned.

* Auto-indent in input mode
* Multi-buffer support
  - __b__ command to list & navigate buffers
    - b alone lists current buffer name (also probaby shown in prompt?)
    - b <regex> switches to buffer specifed by regex, if it's unique,
        otherwise lists buffers matching the regex.
    - b __n__ switches to buffer #n
    - b __name__ switches to buffer matching __name__, or shows error
  - __B__ <compiler> works similarly to __e__ !command, but opens in
    error buffer and contents is parsed as error/warning messages from
    the configured compiler error parser associated with the specified
    compiler name. Initially support only "cargo" as comiler.
* Improved j (join) command.
  - Adds optional arguments to the command to provide better control
    of the results.
  - (.,+1)j[<separator>]
    Joins the specified lines using the given separator string, eliding
    leading whitespace for each line beyond the first. If no separator string
    is given, a single space (" ") is used by default.
  - (.,+1)j"<separator>"
    Joins the specified lines using the given separator string. Lines
    to be joined are used unchanged.

* Wrap command.

# Possible future additions

There are features and commands which might be nice additions, but
of which I'm not yet certain. They may or may not make sense within
this projecet's goals.

* Error buffer
  A special read-only buffer that supports display of compiler output,
  parsing of the errors & warnings, and navigation to the location of
  those errors and warnings.

FIXME: E is now used for Edit in a new buffer, so a new command name would be required
       here, or a different way of specifying editing in a new buffer would be needed.
       Might be OK to eliminate the special edit command, since the same can be done
       with with separate "buffer new", "buffer change", and "edit" commands.
  - E<n> Navigate to the file and line indicated in the error on line <n>
    of the error buffer. If the error indicates other line locations
    (eg. the context information shown by cargo/rustc), specifiying
    a line containing that context will instead navigate to that file &
    line (if that is feasible). If there is already a buffer associated
    with that file, lned will switch to that buffer and change the buffer's
    current line to the error line. If that buffer is dirty, the command will
    instead show a warning. Repeating the command will re-read the file
    and set the current line to the error line. If the buffer & file have
    changed, the line address may no longer be valid. If there is no
    buffer associated with the file containing the specified error, a new
    buffer will be created, the file read into the buffer, and the buffer's
    current line set to the error line.
  - E+ (or just E) will navigate to the next error in the error buffer, as
    described above.
  - E- will navigate to the previous error in the error buffer, as described
    above.
