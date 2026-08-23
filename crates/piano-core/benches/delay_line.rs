//! Criterion benchmark for `PERF-001` (`docs/PERFORMANCE.md`): does the
//! delay line's bounds-masking branch cost anything at a realistic voice
//! count?
//!
//! `piano_core::delay::DelayLine::read`/`write` mask the index into range,
//! then index a boxed slice — logically always in bounds, but LLVM cannot
//! always prove that from the mask alone, so every access may pay a bounds
//! check the entry estimates at 2-4 cycles. This compares the shipped, safe
//! implementation against a `get_unchecked` equivalent built only for this
//! benchmark — never shipped in `piano-core`, which is
//! `#![forbid(unsafe_code)]` — to measure the actual ceiling `unsafe` could
//! buy, per PERF-001's own rule: "do not act before a benchmark shows the
//! branches actually cost something."

#![allow(
    unsafe_code,
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    missing_docs
)]

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use piano_core::delay::DelayLine;

/// M6's full 88-key unison-string total (`docs/PHYSICS.md`).
const VOICE_COUNT: usize = 222;
/// The realtime block-size convention (`PERF-010`, `PERF-011`).
const BLOCK_SAMPLES: usize = 128;
/// A0's own rounded-up delay-line size (`PERF-010`'s own worked example).
const CAPACITY: usize = 2048;

fn bench_safe_delay_lines(c: &mut Criterion) {
    c.bench_function("delay_line_safe_read_write_222_voices_per_block", |b| {
        let mut lines: Vec<DelayLine> = (0..VOICE_COUNT)
            .map(|_| DelayLine::with_capacity(CAPACITY))
            .collect();
        b.iter(|| {
            for line in &mut lines {
                for sample in 0..BLOCK_SAMPLES {
                    line.write(black_box(sample as f32 * 0.001));
                    black_box(line.read(black_box(37)));
                }
            }
        });
    });
}

/// A `get_unchecked`-based stand-in for [`DelayLine::read`]/`write`, built
/// only to measure `PERF-001`'s ceiling. Never shipped: this is a bench
/// crate, entirely separate from `piano-core`'s
/// `#![forbid(unsafe_code)]` library target.
struct UncheckedDelayLine {
    buffer: Box<[f32]>,
    mask: usize,
    write_index: usize,
}

impl UncheckedDelayLine {
    fn with_capacity(capacity: usize) -> Self {
        let capacity = capacity.next_power_of_two();
        Self {
            buffer: vec![0.0; capacity].into_boxed_slice(),
            mask: capacity - 1,
            write_index: 0,
        }
    }

    #[inline]
    fn write(&mut self, sample: f32) {
        self.write_index = (self.write_index + 1) & self.mask;
        let index = self.write_index;
        // SAFETY: `index` is `self.write_index & self.mask`, and `self.mask`
        // is set once in `with_capacity` to exactly `self.buffer.len() - 1`
        // and never changed afterwards, so `index < self.buffer.len()` holds
        // for every possible `write_index`. This mirrors the exact invariant
        // `piano_core::delay::DelayLine` already relies on for its own safe
        // indexing — this type exists solely to measure that invariant's
        // bounds-check cost when asserted via `unsafe` instead.
        unsafe {
            *self.buffer.get_unchecked_mut(index) = sample;
        }
    }

    #[inline]
    fn read(&self, delay: usize) -> f32 {
        let index = self.write_index.wrapping_sub(delay) & self.mask;
        // SAFETY: see `write` above — the same masking invariant applies.
        unsafe { *self.buffer.get_unchecked(index) }
    }
}

fn bench_unchecked_delay_lines(c: &mut Criterion) {
    c.bench_function(
        "delay_line_unchecked_read_write_222_voices_per_block",
        |b| {
            let mut lines: Vec<UncheckedDelayLine> = (0..VOICE_COUNT)
                .map(|_| UncheckedDelayLine::with_capacity(CAPACITY))
                .collect();
            b.iter(|| {
                for line in &mut lines {
                    for sample in 0..BLOCK_SAMPLES {
                        line.write(black_box(sample as f32 * 0.001));
                        black_box(line.read(black_box(37)));
                    }
                }
            });
        },
    );
}

criterion_group!(benches, bench_safe_delay_lines, bench_unchecked_delay_lines);
criterion_main!(benches);
