# 0002 — `no_std` core, allocation only in constructors

**Status**: Accepted

## Context

The audio callback must not allocate: the allocator takes a lock and may enter the
kernel, and neither has a bounded worst case. The usual approach is a coding
guideline and a code review. Guidelines are forgotten under deadline pressure,
and the failure shows up as an intermittent click that nobody can reproduce.

## Decision

`piano-core` is `#![no_std]` with `extern crate alloc`, and all allocation happens
in constructors. `DelayLine::with_capacity` is the only allocating function in the
crate, and it is documented as such.

## Consequences

**What it buys.** Most real-time violations become *unwriteable* rather than
merely discouraged: there is no `std::fs`, no `std::sync::Mutex`, no `println!`,
no `std::time`. A reviewer does not have to notice a mistake that the compiler
already rejected. It also makes the WASM target nearly free, since it was never
depending on a hosted environment.

**What it costs.** `core` has no transcendental float functions, so everything
goes through `math`, which forwards to `std` or `libm`. That is one indirection
and one module of boilerplate. It also means CI must build the crate with
`--no-default-features`, or the constraint silently rots.

**Why not `#![no_std]` without `alloc`.** Delay-line length depends on the note,
so buffers cannot be sized at compile time without fixing the lowest note and
wasting memory on every other one. Allocating once, at construction, off the audio
thread, is the honest trade.
