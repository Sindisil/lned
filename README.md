# lned
Acutally useful line editor -- similar to an extended *nix ed, though
lned will not necessarily exactly duplicate _ed_ functionality in all
cases. For example, the regex syntax supported will almost certainly
be that of the Rust _regex_ crate, since I have no desire to implement
POSIC basic regular expressions as part of this project.

My intent is to use the project to acomplish several goals:

1. Exercise Rust on a non-trivial project.
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
   and Visual Studio Code, but felt no more productive and honestly
   enjoyed it less.

To that end, my intent is to use good ol' _ed_ to write the MVP of a
line based programmer's text editor. Once the MVP is operational, I
then plan to extend it via stepwise refinement, bootstrapping lned
into rough feature parity, and eventual superiority, to _ed_.

My hope is that lned will actually become a useful development tool,
though I suspect that the line-based nature will put an upper bound
on how productive it will feel.

I expect I will then move on to writing a screen based editor as the
next step in this experiment, though if my editor implemtation itch
has been scratched, I may instead go back to using some other editor
(or perhaps even an IDE, though that seems less likely at this point).

## **Important Note**
I'm making this PR public in case the code (or maybe even the editor)
might be of some use to others. However, at the very least until
_lned_ hits 1.0, I don't really intend it as a tool for others to
depend upon, nor do I intend to take PRs, since either would limit the
project's primary purpose. I will edit or remove this note if and when
I change either policy.
