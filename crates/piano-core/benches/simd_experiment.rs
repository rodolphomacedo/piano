//! An experiment for `PERF-005`'s own suggestion (`docs/PERFORMANCE.md`):
//! SIMD across a note's unison strings' dispersion cascade — measured, not
//! assumed.
//!
//! `piano_core::unison::UnisonGroup` already processes a note's 1-3 strings
//! together, one call per sample, which is exactly the "natural 4-wide SIMD
//! group" `PERF-005` describes. This compares the shipped, sequential-
//! per-string dispersion cascade against a structure-of-arrays rewrite
//! (three parallel coefficient/state arrays, processed in lockstep) written
//! in **safe Rust**, in the hope LLVM auto-vectorises the lockstep version.
//! No `unsafe`, no explicit SIMD intrinsics: `piano-core` is
//! `#![forbid(unsafe_code)]`, and this experiment's whole point is to find
//! out whether reaching for `unsafe` SIMD would even be worth the risk
//! before anyone does — per this project's own "measure before optimising"
//! rule (`CONTRIBUTING.md`).

#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    missing_docs
)]

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use piano_core::dispersion::DispersionCascade;

/// Matches [`piano_core::dispersion::MAX_SECTIONS`].
const MAX_SECTIONS: usize = 8;
const SAMPLES_PER_ITER: usize = 128;

fn bench_sequential_trichord(c: &mut Criterion) {
    // The shape `UnisonGroup::process_impl` actually uses today: three
    // independent `DispersionCascade`s, called one after another.
    c.bench_function("dispersion_trichord_sequential", |b| {
        let mut cascades = [
            DispersionCascade::new(27.5, 0.000_1),
            DispersionCascade::new(27.6, 0.000_1),
            DispersionCascade::new(27.4, 0.000_1),
        ];
        b.iter(|| {
            let mut sums = [0.0f32; 3];
            for _ in 0..SAMPLES_PER_ITER {
                for (cascade, sum) in cascades.iter_mut().zip(sums.iter_mut()) {
                    *sum = cascade.process(black_box(*sum + 0.01));
                }
            }
            black_box(sums)
        });
    });
}

/// Structure-of-arrays restructuring of three cascades' state, so the three
/// strings' identical arithmetic runs in lockstep across parallel arrays —
/// the shape an auto-vectoriser can turn into SIMD instructions without any
/// `unsafe` on this project's part. A direct reimplementation of
/// [`piano_core::dispersion::DispersionCascade`]'s single-section
/// arithmetic (`Section::process`'s doc comment), applied to three lanes at
/// once instead of one.
struct TrichordCascade {
    coefficients: [[f32; 3]; MAX_SECTIONS],
    state: [[f32; 3]; MAX_SECTIONS],
    active: usize,
}

impl TrichordCascade {
    fn new(coefficient: f32, active: usize) -> Self {
        Self {
            coefficients: [[coefficient; 3]; MAX_SECTIONS],
            state: [[0.0; 3]; MAX_SECTIONS],
            active,
        }
    }

    #[inline]
    fn process(&mut self, input: [f32; 3]) -> [f32; 3] {
        let mut sample = input;
        for section in 0..self.active {
            let Some(coefficients) = self.coefficients.get(section).copied() else {
                break;
            };
            let Some(state) = self.state.get_mut(section) else {
                break;
            };
            let mut next_state = [0.0f32; 3];
            let mut output = [0.0f32; 3];
            for lane in 0..3 {
                let (Some(c), Some(s), Some(x)) =
                    (coefficients.get(lane), state.get(lane), sample.get(lane))
                else {
                    continue;
                };
                let next = x - c * s;
                if let Some(slot) = next_state.get_mut(lane) {
                    *slot = next;
                }
                if let Some(slot) = output.get_mut(lane) {
                    *slot = c * next + s;
                }
            }
            *state = next_state;
            sample = output;
        }
        sample
    }
}

fn bench_structure_of_arrays_trichord(c: &mut Criterion) {
    c.bench_function("dispersion_trichord_structure_of_arrays", |b| {
        let mut cascade = TrichordCascade::new(-0.02, 8);
        b.iter(|| {
            let mut sums = [0.0f32; 3];
            for _ in 0..SAMPLES_PER_ITER {
                sums = cascade.process(black_box(sums.map(|s| s + 0.01)));
            }
            black_box(sums)
        });
    });
}

criterion_group!(
    benches,
    bench_sequential_trichord,
    bench_structure_of_arrays_trichord
);
criterion_main!(benches);
