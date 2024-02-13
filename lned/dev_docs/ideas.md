#### File saving

https://stackoverflow.com/questions/18260899/adequetely-safe-method-of-overwriting-a-save-file

https://danluu.com/file-consistency/

https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-replacefilea?redirectedfrom=MSDN

https://stackoverflow.com/questions/1812115/how-to-safely-write-to-a-file

https://stackoverflow.com/questions/18260899/adequetely-safe-method-of-overwriting-a-save-file

Files are hard.

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
