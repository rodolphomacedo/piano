# 0005 — Lock-free SPSC command queue for control data

**Status**: Accepted (design fixed; implementation lands in M2)

## Context

Note-on, note-off, pedal state and parameter changes originate on a UI or MIDI
thread and must reach the audio thread. The obvious mechanism, a
`Arc<Mutex<State>>`, is unusable: if the audio thread ever blocks on a mutex held
by a lower-priority thread, it misses its deadline. Priority inversion is not a
rare case here — it is the normal case, because the audio thread runs at a
priority above everything else.

## Decision

Control data crosses the boundary through a **fixed-capacity, lock-free,
single-producer/single-consumer ring buffer** of `Copy` plain-data commands,
drained at the top of each audio callback.

Data flowing back to the UI (levels, voice counts) uses the same mechanism in
reverse, or relaxed atomics.

## Consequences

**What it buys.** The audio thread never waits for anything. Worst-case time to
drain the queue is bounded by its capacity, which is a compile-time constant.

**What it costs, and the traps.**

- The queue is **fixed capacity**. When full, the *producer* drops or blocks. The
  consumer never does. Dropping a note-on is bad; missing a buffer is worse.
- Commands must be **`Copy` plain data**. No `String`, no `Box`, and in particular
  no `Arc` — if the audio thread holds the last clone, dropping it frees memory on
  the audio thread, which is the exact thing ADR-0002 exists to prevent.
- Anything large or dynamically sized (a new impulse response, a reconfigured
  voice pool) is prepared off-thread and handed over as a pointer swap, with the
  old value freed by the *other* thread.

**Rejected alternative.** A `try_lock` with a fallback path. It works, but it makes
behaviour depend on timing, which makes bugs unreproducible — and reproducibility
is one of this project's stated safety properties.
