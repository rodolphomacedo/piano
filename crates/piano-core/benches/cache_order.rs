//! Criterion benchmark for `PERF-010` (`docs/PERFORMANCE.md`): does loop
//! order (voice-outer, block-inner vs. sample-outer, voice-inner) actually
//! matter at the working-set size M6 introduced — up to 222 delay lines,
//! roughly 2 MB total, past the reference machine's 256 KB L2 and a third
//! of its 6 MB L3?
//!
//! `piano_audio::engine::Engine::process_block` already loops voice-outer
//! (`docs/ARCHITECTURE.md`); this measures that choice against the naive
//! sample-outer alternative directly, isolated from dispatch/bridge/
//! soundboard cost — closing the "left to M7" note `PERF-010`'s own M5/M6
//! status entries both carry.

#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    missing_docs
)]

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use piano_core::string::{PluckedString, StringConfig};
use piano_core::{Hz, SampleRate};

/// M6's full 88-key unison-string total (`docs/PHYSICS.md`).
const VOICE_COUNT: usize = 222;
const BLOCK_SAMPLES: usize = 128;

fn strings() -> Vec<PluckedString> {
    let rate = SampleRate::new(48_000.0).expect("48 kHz is valid");
    (0..VOICE_COUNT)
        .map(|index| {
            // Spread frequencies across the keyboard so every voice's delay
            // line has a realistic, distinct size, matching
            // `piano_audio::Engine`'s real working set rather than 222
            // identical (and therefore identically cache-behaved) copies.
            let octave_span = VOICE_COUNT as f32 / 7.25;
            let frequency = 27.5 * 2f32.powf(index as f32 / octave_span);
            let config = StringConfig::new(Hz::new(frequency).expect("frequency is valid"));
            let mut string = PluckedString::new(config, rate).expect("frequency is tunable");
            string.pluck(0.9);
            string
        })
        .collect()
}

fn bench_voice_outer_block_inner(c: &mut Criterion) {
    // The order `Engine::process_block` actually uses: one voice's whole
    // delay line stays hot in cache for a full block before moving to the
    // next voice.
    c.bench_function("cache_order_voice_outer_block_inner", |b| {
        let mut voices = strings();
        b.iter(|| {
            let mut output = [0.0f32; BLOCK_SAMPLES];
            for voice in &mut voices {
                for slot in &mut output {
                    *slot += voice.process();
                }
            }
            black_box(output)
        });
    });
}

fn bench_sample_outer_voice_inner(c: &mut Criterion) {
    // The naive alternative PERF-010 warns against: every sample touches
    // every voice's delay line, so the full ~2 MB working set is read once
    // per sample instead of once per block.
    c.bench_function("cache_order_sample_outer_voice_inner", |b| {
        let mut voices = strings();
        b.iter(|| {
            let mut output = [0.0f32; BLOCK_SAMPLES];
            for slot in &mut output {
                for voice in &mut voices {
                    *slot += voice.process();
                }
            }
            black_box(output)
        });
    });
}

criterion_group!(
    benches,
    bench_voice_outer_block_inner,
    bench_sample_outer_voice_inner
);
criterion_main!(benches);
