# Real-time audio rules

The audio callback is not ordinary code. It runs on a thread with a hard deadline
supplied by the operating system, and the consequence of missing that deadline is
not a slow program — it is an audible click, or silence.

At 48 kHz with a 128-sample buffer the callback is invoked every **2.67 ms** and
must return well inside it, **every single time**. Not on average. The 99.9th
percentile is the number that matters, because the one late buffer in a thousand
is the one the listener hears.

Everything below follows from that.

## The forbidden list

Inside `process` / `process_block`, or anything they call:

| Forbidden | Why |
|---|---|
| Allocating or freeing memory | The allocator takes a lock and may call into the kernel. Unbounded latency. |
| Locking a mutex | Priority inversion: a lower-priority thread holding the lock stops the audio thread indefinitely. |
| Any system call | File I/O, sockets, `println!`, logging. All unbounded. |
| `panic!`, `unwrap`, `expect`, `assert!` | A panic on the audio thread either aborts the process or unwinds through a C callback. Both are worse than any wrong sample. |
| Unbounded loops | `while !converged` has no worst case. Every loop needs a compile-time maximum. |
| Waiting on anything | Channels, condition variables, joins, sleeps. |
| Growing a `Vec`, `String`, `HashMap` | Hidden allocation. |
| `Box<dyn Trait>` in the per-sample path | An indirect call the optimiser cannot see through. See `PERF-003`. |

## How this project enforces it

Enforcement is structural, not a matter of remembering:

- **`piano-core` is `no_std` + `alloc`.** Most of the forbidden operations are not
  even reachable — there is no `std::fs`, no `std::sync::Mutex`, no `println!`.
- **`#![forbid(unsafe_code)]`** in every crate. Unsafe is opt-in per crate, with a
  written justification, and today no crate opts in.
- **Clippy denies `unwrap_used`, `expect_used`, `panic` and `unimplemented`** at
  the workspace level. Test modules opt out explicitly; production code cannot.
- **Every constructor validates and every hot-path function is total.** Invalid
  parameters are rejected by `ParamError` at construction. `read_interpolated`
  masks its index and saturates `NaN` rather than panicking. Filter coefficients
  are clamped below the unit circle so a loop cannot diverge.
- **All allocation happens in constructors.** `DelayLine::with_capacity` is the
  only allocating function in `piano-core`, and it is documented as such.
- **`panic = "abort"` in the release profile.** If a panic somehow happens, it
  will not unwind through a C audio callback and corrupt the host.

## Getting data in and out

Parameter changes, note-on and note-off come from a different thread. The rule
above forbids locking, so the only acceptable mechanism is a **lock-free
single-producer/single-consumer ring buffer** of plain-old-data command structs,
drained at the top of each callback.

```
UI / MIDI thread  ──push──▶  SPSC ring (fixed capacity)  ──drain──▶  audio thread
```

Consequences that are easy to get wrong:

- The ring has **fixed capacity**. When it is full, the producer drops or blocks —
  the audio thread never waits.
- Commands are **`Copy` plain data**. No `String`, no `Box`, no `Arc` whose last
  clone might be dropped (and therefore freed) on the audio thread.
- Audio-to-UI data (levels, voice counts) goes back the same way, or through
  atomics with relaxed ordering. Never a shared `Mutex<State>`.

## Numerical rules

- **Every recursive filter must be provably stable.** Coefficients are clamped at
  construction, not checked at runtime.
- **`NaN` is a permanent infection.** Once a `NaN` enters a feedback loop it never
  leaves. `math::clamp_or_low` maps `NaN` to the low bound instead of propagating
  it — that is why it exists and why `f32::clamp` is not used.
- **Denormals are a performance bug, not a correctness one.** See `PERF-002`.
- **Output is bounded by construction**, so a bug produces a wrong note rather
  than a full-scale square wave into someone's headphones.

## What "never crashes" means here

The user's requirement was code that never locks up. Concretely, this project
takes that to mean four properties, each of which is testable:

1. **Totality.** Every hot-path function returns a value for every input,
   including `NaN`, `±∞`, zero and `usize::MAX`. Verified with `proptest`.
2. **Boundedness.** Output magnitude stays finite for any reachable parameter
   combination, for arbitrarily long runs. Verified with long-run tests.
3. **Determinism.** The same seed and inputs produce byte-identical output, so a
   failure can be reproduced.
4. **Predictable timing.** No operation in the callback has unbounded worst-case
   duration.
