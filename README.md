# lned
Acutally useful line editor -- similar to an extended *nix ed.

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
   such as IntelliJ IDEA, NetBeans, Eclipse/Eclips CDE, Visual Studio,
   and Visual Studio Code, but felt no more productive and honestly
   enjoyed it less.


To that end, my intent is to use good ol' _ed_ to write the MVP of a
line based programmer's text editor. Once the MVP is operational, I
then plan to extend it via stepwise refinement, driven primarily by
the pain points I experience using it.

My hope is that lned will actually become a useful development tool,
though I suspect that the line-based nature will put an upper bound
on how productive it will feel. Perhaps, if I'm still interested, I
will then move on to writing a screen based editor.
