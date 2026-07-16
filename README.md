# lned
Acutally useful line editor, similar to an extended _ed_, though
lned does not exactly duplicate _ed_ functionality.

lned should run in any terminal that supports ANSI escape sequences,
but has only been used extensively in the following environments:

* Windows 11
    - Windows Terminal running cmd.exe
    - Windows conhost running cmd.exe
* Linux (mostly used on Fedora 43 & 44)
    - GNOME Terminal
    - ptyxis
    - Konsole

See [the manual](docs/manual.md) (also viewable using the Help command
('h') within lned) for information on how to use the editor.

## Current Project Status

I have completed the initial goals I intended for the lned project.
It was, and is, sufficiently usable for programming to actually be
pretty comfortable. I used it to develop both the vast majority of
the editor itself, including its documentation, as well as the
line_edit library lned uses to accept user input.

That said, as I expected I have begun to find a few aspects of using
the line editor constraining. It's not usually so bad with respect to
code editing, but prose editing is a right pain in the ass with a line
editor.

Because of this, I have indeed begun work on a screen editor. I am
still using lned as my primary editor for that work, since I legitimately
find it comfortable, though I not infrequently go back to vim when
I need to edit any quanitity of text so we'll see if that remains the case.
In any case, I will certainly dog food the new editor once it's at a basic
level of functionality.

I don't expect to make significant additions to lned going forward,
but may well do so if I find pain points while working on the next
project.

## Project History

This project is primarily an experiment. My intent is to use the project
to acomplish several goals:

1. Use Rust on a non-trivial project.
2. Recapture some of the enjoyment I've had in the past developing
   tools for my own use.
3. Test a theory I've been mulling over that I am more productive and
   happier using simpler development tools, as long as they're
   sufficiently powerful to not actually *hinder* me. For example,
   in the past I produced a significant amount of good quality
   Java and C code using Vim, and found it a comfortable and
   mostly enjoyable experience.

   I've also produced significant amounts of good quality Java, and
   a lesser amount of C and C++, using various more powerful tools,
   such as IntelliJ IDEA, NetBeans, Eclipse/Eclipse CDE, Visual Studio,
   and Visual Studio Code, but seldom felt more productive and often
   less so as I spent time making my tools work correctly, rather than
   doing the same for the code I was writing.

   And I usually enjoyed it less.

To that end, the plan was to use good ol' _ed_ to write the MVP of a
line based programmer's text editor. Once the MVP was operational, I
then planned to extend it via stepwise refinement, bootstrapping lned
into rough feature parity, and eventual superiority, to _ed_.

My hope at the start of the project was that lned will actually become
a useful development tool, though I suspect that the line-based nature
will put an upper bound on how productive it will feel.

I expect I will then move on to writing a screen based editor as the
next step in this experiment, though if my editor implemtation itch
has been scratched, I may instead go back to using vim or some other
editor (or perhaps even an IDE, though that seems less likely at this
point).

## Contributing

I'm making this repository public in case the code (or maybe even the
editor) might be of some use to others. However, I don't really intend
it as a tool for others to depend upon at this time, nor do I intend
to accept PRs right now, since either would limit the project's primary
purpose.

That said, I'm open to bug reports and/or feature requests, if, by some
chance, anyone *does* use __lned__ (or at least try it), keeping in mind
the project's status and intent.

If the situation with respect to any aspect of contributing to lned
changes in the future, I will update this README.

## Generative AI Policy

No Generative AI has ben used in the developement of lned.

Issues generated with AI agents will be rejected. If lned eventually
opens up to external pull requests, any pull requests generated with
AI agents will also be rejected.

## License

SPDX-License-Identifier: GPL-3.0-only

This program is free software; you can redistribute it and/or modify it
under the terms of the GNU General Public License as published by the
Free Software Foundation, version 3.

This program is distributed in the hope that it will be useful, but
WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the GNU
General Public License for more details. 

Copyright © 2023 Greg A. Jandl
