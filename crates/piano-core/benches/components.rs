//! Criterion benchmarks isolating the DSP components `PERF-005`, `PERF-007`,
//! `PERF-008` and `PERF-009` (`docs/PERFORMANCE.md`) flagged as
//! "implemented, unmeasured" or "measured only in aggregate" — the
//! decomposition those entries and M7's own `docs/ROADMAP.md` line both
//! explicitly deferred. Run with `cargo bench -p piano-core --bench components`.
//!
//! Not part of `cargo test`: these are wall-clock measurements for a human
//! to read and record in `docs/PERFORMANCE.md`, the same "measure by hand,
//! quote the number" discipline the `#[ignore]`d tests in `piano-audio`
//! already use for the whole-engine aggregate number.
//!
//! Every type used here is `pub` at the `piano-core` level, which is what
//! lets this isolate one component's own cost — `piano_audio::Engine` itself
//! is `pub(crate)` and cannot be reached from an external bench crate, so a
//! true decomposition of its 697.2 µs/block number has to happen at this
//! layer instead, summing the same components that number's block actually
//! runs.

#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    missing_docs
)]

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use piano_core::dispersion::DispersionCascade;
use piano_core::string::{PluckedString, StringConfig};
use piano_core::unison::unison_count_for_key_index;
use piano_core::{BridgeBus, Hz, SampleRate, Soundboard, UnisonGroup};

/// Matches the block size `piano_audio::engine::Engine` actually chunks
/// into (`BRIDGE_BLOCK_SAMPLES`) and the realtime rules' own 128-sample
/// convention (`PERF-010`, `PERF-011`).
const BLOCK_SAMPLES: usize = 128;

/// M6's real full-keyboard string count: 12 monochord + 18 bichord·2 +
/// 58 trichord·3 (`docs/PHYSICS.md`, `piano_core::unison`).
const KEY_COUNT: usize = 88;

fn sample_rate() -> SampleRate {
    SampleRate::new(48_000.0).expect("48 kHz is valid")
}

fn bench_dispersion_cascade(c: &mut Criterion) {
    // PERF-005: "the single largest CPU consumer", never cycle-counted on
    // its own. A0 uses the documented 8-section cascade; A4 uses 2.
    let mut group = c.benchmark_group("dispersion_cascade_one_block");
    for (label, frequency, inharmonicity) in [
        ("a0_8_sections", 27.5, 0.000_1),
        ("a4_2_sections", 440.0, 0.000_4),
    ] {
        group.bench_function(label, |b| {
            let mut cascade = DispersionCascade::new(frequency, inharmonicity);
            b.iter(|| {
                let mut sample = 0.0f32;
                for _ in 0..BLOCK_SAMPLES {
                    sample = cascade.process(black_box(sample + 0.01));
                }
                black_box(sample)
            });
        });
    }
    group.finish();
}

fn bench_hammer_contact(c: &mut Criterion) {
    // PERF-007: a bounded spike at note-on, not a steady load — benchmarked
    // as one call, not per-block, since that is how `PluckedString::pluck`
    // actually uses it.
    c.bench_function("hammer_simulate_contact_one_strike", |b| {
        b.iter(|| {
            black_box(piano_core::hammer::simulate_contact(
                black_box(0.8),
                black_box(48_000.0),
            ))
        });
    });
}

fn bench_bridge_bus(c: &mut Criterion) {
    // PERF-008's own cost in isolation, at the full M6 voice count, one
    // block's worth of writes+reads.
    c.bench_function("bridge_bus_add_and_read_222_voices_one_block", |b| {
        let mut bus = BridgeBus::with_capacity(BLOCK_SAMPLES);
        b.iter(|| {
            bus.begin_block();
            for index in 0..BLOCK_SAMPLES {
                for voice in 0..222 {
                    black_box(bus.add_and_read(index, black_box(voice as f32 * 0.001)));
                }
            }
        });
    });
}

fn bench_soundboard(c: &mut Criterion) {
    // PERF-009's own cost in isolation: a post-mix stage run once per
    // sample regardless of voice count, one block's worth.
    c.bench_function("soundboard_process_one_block", |b| {
        let mut board = Soundboard::new(sample_rate());
        b.iter(|| {
            let mut sum = 0.0f32;
            for _ in 0..BLOCK_SAMPLES {
                sum += board.process(black_box(0.1));
            }
            black_box(sum)
        });
    });
}

fn bench_single_voice_block(c: &mut Criterion) {
    // The isolated single-voice `process_block` cost the M7 dispatch asked
    // for, separate from the full-engine aggregate.
    c.bench_function("single_plucked_string_process_block", |b| {
        let config = StringConfig::new(Hz::new(440.0).expect("440 Hz is valid"));
        let mut string =
            PluckedString::new(config, sample_rate()).expect("A4 is tunable at 48 kHz");
        string.pluck(0.9);
        b.iter(|| {
            let mut buffer = [0.0f32; BLOCK_SAMPLES];
            string.process_block_add(&mut buffer);
            black_box(buffer)
        });
    });
}

fn bench_unison_group_block(c: &mut Criterion) {
    c.bench_function("trichord_unison_group_process_block_local_coupling", |b| {
        let config = StringConfig::new(Hz::new(440.0).expect("440 Hz is valid"));
        let mut group = UnisonGroup::new(config, 3, sample_rate()).expect("A4 trichord is tunable");
        group.pluck(0.9);
        b.iter(|| {
            let mut sum = 0.0f32;
            for _ in 0..BLOCK_SAMPLES {
                sum += group.process();
            }
            black_box(sum)
        });
    });
}

fn bench_full_polyphony_block(c: &mut Criterion) {
    // The `piano-core`-level equivalent of `Engine::process_chunk`: every
    // one of 88 keys' real unison-string count (222 total), the shared
    // bridge bus and the soundboard, all active for one block — so its
    // result can be compared against the sum of the isolated benchmarks
    // above (dispersion + hammer's one-time cost + bridge + soundboard +
    // per-voice dispatch) to see how much of the 697.2 µs/block aggregate
    // each piece actually accounts for.
    c.bench_function("full_88_key_222_string_block_core_level", |b| {
        let rate = sample_rate();
        let mut groups: Vec<UnisonGroup> = (0u8..KEY_COUNT as u8)
            .map(|key_index| {
                let count = unison_count_for_key_index(key_index);
                let frequency = 27.5 * 2f32.powf(f32::from(key_index) / 12.0);
                let config = StringConfig::new(Hz::new(frequency).expect("frequency is valid"));
                let mut group =
                    UnisonGroup::new(config, count, rate).expect("every key is tunable at 48 kHz");
                group.pluck(0.9);
                group
            })
            .collect();
        let mut bridge = BridgeBus::with_capacity(BLOCK_SAMPLES);
        let mut board = Soundboard::new(rate);
        b.iter(|| {
            let mut chunk = [0.0f32; BLOCK_SAMPLES];
            bridge.begin_block();
            for group in &mut groups {
                for (index, sample) in chunk.iter_mut().enumerate() {
                    *sample += group.process_with_bridge(&mut bridge, index);
                }
            }
            for sample in &mut chunk {
                *sample += 0.5 * board.process(*sample);
            }
            black_box(chunk)
        });
    });
}

criterion_group!(
    benches,
    bench_dispersion_cascade,
    bench_hammer_contact,
    bench_bridge_bus,
    bench_soundboard,
    bench_single_voice_block,
    bench_unison_group_block,
    bench_full_polyphony_block,
);
criterion_main!(benches);
