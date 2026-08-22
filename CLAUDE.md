# Project instructions for AI agents

This file is read automatically at the start of a session in this repository.
It is the short version; the long versions are the documents it points to.

## What this project is

A physically modelled piano synthesiser in Rust. Sound is computed from a model of
a vibrating string — there are no samples and no recordings anywhere in this repo,
and none may be added.

Everything in this repository is written in **English**: code, comments, docs,
commit messages, issues.

## Read these before writing code

| When | Read |
|---|---|
| Touching anything in `piano-core` | `docs/REALTIME-AUDIO-RULES.md` |
| Adding or changing physics | `docs/PHYSICS.md`, then the paper it comes from |
| Anything performance-related | `docs/PERFORMANCE.md` |
| Deciding where code belongs | `docs/ARCHITECTURE.md` |
| Looking at another project for reference | `docs/PRIOR-ART.md` — **mandatory** |

## Hard rules

1. **The audio thread allocates nothing, locks nothing, panics nowhere, and has no
   unbounded loops.** This is not a guideline. If a change makes any of those
   possible, it is wrong regardless of how it sounds.

2. **No copyleft source may be read for a component you are writing.** This project
   is MIT/Apache-2.0. OpenPiano and similar AGPL projects are auditory benchmarks
   and pointers to papers, never sources. Implement from the published literature
   and cite it in a doc comment.

3. **New bottlenecks get a `PERF-xxx` entry when identified, not when fixed.**
   `docs/PERFORMANCE.md` is meant to run ahead of the code. An entry closes only
   with a measurement.

4. **No `unwrap`, `expect`, `panic!` or `unimplemented!` outside test modules.**
   Clippy denies them at workspace level. Do not add `#[allow]` to production code
   to get around it — restructure so the failure case is representable.

5. **Every hot-path function is total.** It returns a value for `NaN`, `±∞`, zero
   and `usize::MAX`. Prove it with a `proptest`, not with an argument.

## Before claiming anything works

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build -p piano-core --no-default-features
```

Run them. Quote the output. "Should work" is not a result, and a green claim
without a green run is worse than no claim.

For audio changes, also **render and listen**, and measure the fundamental if
tuning could have been affected.

## Style

See `CONTRIBUTING.md`. The short form: functions under 20 lines, names specific
enough that every grep hit is relevant, newtypes for anything confusable, error
messages that carry the offending value, comments that explain why and never what.
