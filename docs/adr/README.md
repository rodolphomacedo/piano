# Architecture decision records

Short records of decisions that were expensive to make and would be expensive to
reverse. Each states the context, the decision, and what it costs — including
what it costs when it turns out to be wrong.

A decision that was obvious does not need a record. These are the ones that were
not.

| # | Decision | Status |
|---|---|---|
| [0001](0001-digital-waveguide-over-finite-differences.md) | Digital waveguide, not finite differences | Accepted |
| [0002](0002-no-std-core-with-allocation-only-in-constructors.md) | `no_std` core, allocation only in constructors | Accepted |
| [0003](0003-f32-as-the-sample-type.md) | `f32` as the sample type | Accepted |
| [0004](0004-permissive-licence-and-clean-room-policy.md) | MIT OR Apache-2.0, with a clean-room policy | Accepted |
| [0005](0005-lock-free-command-queue.md) | Lock-free SPSC command queue for control data | Accepted |
| [0006](0006-crate-boundaries.md) | Crate boundaries follow the real-time boundary | Accepted |
