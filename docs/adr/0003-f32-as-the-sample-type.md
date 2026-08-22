# 0003 — `f32` as the sample type

**Status**: Accepted

## Context

`f64` has 53 bits of mantissa against `f32`'s 24. Recursive filters with poles
very close to the unit circle — which is exactly what a 40-second bass decay is —
are where that difference can become audible.

But `PERF-010` establishes that the binding constraint at high polyphony is memory
bandwidth, not arithmetic: 240 delay lines are about 2 MB in `f32` and 4 MB in
`f64`, against 6 MB of shared L3 on the target CPU. `f64` also halves SIMD lane
count.

## Decision

`f32` is the sample type, the delay-line type, and the default filter-state type.

## Consequences

**What it buys.** Half the memory traffic, twice the SIMD width, and a working set
that has some chance of fitting in cache.

**What it costs.** Precision headroom in the longest decays. The specific risk is
a bass note whose envelope stalls at a small constant instead of reaching zero.

**The mitigation, if measured.** Keep `f32` for samples and delay lines, and widen
only the **filter state** to `f64` in the lowest octave. State is a handful of
values per string, so widening it costs essentially no bandwidth. Tracked as
`PERF-013`, with the test that would catch it named there.

**The signal to revisit.** A 60-second A0 render whose envelope does not decay
monotonically to exact zero.
