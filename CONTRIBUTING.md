# Contributing

## Before you write DSP code

Read [`docs/REALTIME-AUDIO-RULES.md`](docs/REALTIME-AUDIO-RULES.md). It is short,
and it is the difference between code that works and code that clicks.

Read [`docs/PRIOR-ART.md`](docs/PRIOR-ART.md) too. This project is
MIT/Apache-2.0 and takes licence hygiene seriously: **do not read GPL or AGPL
source for a component you intend to write.** Implement from the papers.

## The quality gate

Everything below must pass before a pull request is reviewed. CI runs all of it.

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build -p piano-core --no-default-features
```

The lint configuration lives in the workspace `Cargo.toml`. It denies
`unwrap_used`, `expect_used`, `panic` and `unimplemented` in production code.
Test modules opt out with an inner `#![allow(...)]` at the top of `mod tests`;
production code does not get to.

## Code standards

These are the rules this codebase is actually held to.

- **Functions under 20 lines, files under 500.** If a function does not fit, it is
  doing two things.
- **Names specific enough that every grep hit is relevant.** No `data`, `handler`,
  `manager`, `process_thing`.
- **Explicit types. No `unwrap`, no `expect`, no `panic!` outside tests.**
  Constructors validate and return `Result`; hot-path functions are total.
- **Error messages carry the offending value.** `expected X, got {value}`, never
  just "invalid input".
- **Early returns over nested `if`s.** Three levels of indentation is the ceiling.
- **No magic numbers.** A named constant with a doc comment explaining *why* that
  value.
- **Newtypes for anything confusable.** `SampleRate` and `Hz` are both "some
  number of hertz"; making them different types turns a silent bug into a compile
  error.
- **Comments explain why, never what.** The default is no comment. Write one when
  there is a hidden constraint, a subtle invariant, or a workaround — the phase
  delay correction in `PluckedString::new` is a good example of a comment that
  earns its place.

## Tests

- **Behaviour, not implementation.** `step_response_converges_to_the_input`, not
  `test_process`.
- **Property tests for anything that must hold universally.** If the claim is "this
  never panics" or "this never diverges", it needs a `proptest`, not three
  examples. These are what make the project's safety claims checkable.
- **Physical tests for physical claims.** A string must decay monotonically, reach
  silence, and be in tune. Those are testable and they catch real regressions.
- **Determinism is required.** The excitation is a seeded PRNG precisely so that a
  failing render can be reproduced exactly.

Coverage is not yet gated, but every new module is expected to arrive with tests.
The long-term target is 100 % line and branch coverage in `piano-core`.

## Performance work

Two rules, both non-negotiable:

1. **Measure before optimising.** An entry in
   [`docs/PERFORMANCE.md`](docs/PERFORMANCE.md) is a hypothesis until a benchmark
   confirms it. Several entries there may turn out to cost nothing.
2. **Measure the right thing.** Mean callback time is almost useless; the p99.9 is
   what the listener hears. Report worst case.

If you discover a new bottleneck, add a `PERF-xxx` entry *when you identify it*,
not when you fix it. The register is meant to be ahead of the code.

## Commits

Conventional commits, and the subject says **why**, not what:

```
feat: tune the loop delay against the loss filter's phase delay
fix: reach exact zero at the end of the fade instead of 1/length
perf: skip voices below the audibility threshold
docs: record the O(N²) trap in sympathetic coupling before anyone writes it
```

Never amend a commit unless explicitly asked to.

## Pull requests

- One concern per PR.
- If it touches DSP, say what you listened to and what you measured.
- If it touches the audio thread, say which real-time rule could have been broken
  and why it was not.
