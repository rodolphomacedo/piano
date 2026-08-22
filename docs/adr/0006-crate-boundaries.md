# 0006 — Crate boundaries follow the real-time boundary

**Status**: Accepted

## Context

The project could be one crate with modules. Modules are cheaper to work with:
no version numbers, no dependency declarations, no re-exports.

But the most important invariant in this codebase is "this code may run on the
audio thread; that code may not", and module boundaries do not enforce anything.
A `use crate::wav::write` inside a DSP function compiles fine.

## Decision

Crate boundaries are drawn along the real-time boundary, and dependencies point in
one direction only:

```
piano-core  ←  piano-params  ←  piano-render  ←  piano-cli
                     ↑                ↑
                piano-audio      piano-wasm
```

`piano-core` cannot reach file I/O, because `piano-render` depends on it and not
the reverse. That is a compile error, not a review comment.

## Consequences

**What it buys.** The real-time contract is enforced by the dependency graph. It
also means the engine can be compiled for WASM, for a plugin, or into a test,
without a single `#[cfg]` in the DSP — and each front end pays only for what it
uses.

**What it costs.** More `Cargo.toml` files, workspace dependency plumbing, and the
occasional type that has to be moved when it turns out to belong on the other
side of a boundary. Feature unification across crates also needs care —
`default-features = false` must be declared in the workspace table, not per crate.

**Where the line is drawn.** `piano-params` is `no_std` and could live in the core.
It is separate because the DSP genuinely does not need to know what a note name is,
and keeping "what is A4" away from "how does a string vibrate" is what makes both
testable in isolation.
