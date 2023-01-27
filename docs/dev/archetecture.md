# Archetecture

This document describes the high-evel architecture of the lned line editor.

## Bird's Eye View

lned is a line editor modeled after *nix ed, but with extensions to make
it more useful and ergonomic. It is intended primarily as a vehicle for
exploring the rust programming language, but is intended also to be
actually useable.

Implementation simplicity is a primary design goal.

## Repository Structure

This is a brief description of important directories.

### crates/lned

This crate defines the actual lned binary.
This encompases the UI, handling command line options, handling the config
file, and managing edit buffers.

### crates/edit_buffer

This implements data types and functions that store and manipulate text.
Also included are undo/redo stacks and file I/O.


## Design

These sections provide more detailed design decisions and questions.

### edit_buffer

### lned
