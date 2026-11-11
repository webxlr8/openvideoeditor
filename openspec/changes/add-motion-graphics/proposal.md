# Proposal: add-motion-graphics

## Why

Titles, lower thirds and animated graphics are the most requested additions after cuts. Agents need to write them as code they can reason about, and people need templates they can fill in.

## What Changes

- `ove-graphics`: TSX compositions transpiled with oxc and run per frame in a QuickJS sandbox (64 MB, 250 ms per frame, no file system, network, timers or modules; fixed `Date`/`Math.random`), laid out and painted by Takumi.
- `check`: syntax, imports, globals, runtime errors, endless loops, memory, unsupported tags/props/styles and the slowest frame, with line:column.
- Eight templates of our own design (lower third, title card, counter, progress bar, quote card, logo sting, kinetic text, end card); Motion panel and the inspector Code tab with a props form generated from each template's schema.
