//! The regulation gate: every key, rendered through the full engine at
//! *mezzo-forte*, must sit within [`TOLERANCE_DB`] of the regulated
//! loudness curve (`piano_audio::voicing::loudness_target_db`), measured
//! A-weighted over its first 0.68 s.
//!
//! Without regulation the keyboard spanned 26 dB with 5 dB cliffs at both
//! unison breaks. The ignored test below regenerates
//! `voicing::LEVEL_CORRECTION_DB` after any change that moves a key's level:
//!
//! ```sh
//! cargo test -p piano-audio --release --test keyboard_loudness -- --ignored --nocapture
//! ```

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use piano_audio::offline::OfflineEngine;
use piano_audio::voicing::{LEVEL_CORRECTION_DB, loudness_target_db};
use piano_core::SampleRate;
use piano_params::{HIGHEST_PIANO_KEY, LOWEST_PIANO_KEY, PianoKey, Tuning};
use rustfft::{FftPlanner, num_complex::Complex32};

const SAMPLE_RATE_HZ: f32 = 48_000.0;
const WINDOW: usize = 1 << 15;
const VELOCITY: f32 = 0.6;

/// The A-weighted level, in this test's units, the flat middle of the
/// keyboard is regulated to — where C3-C7 already sat on average before
/// regulation, so regulating moves no register's absolute level by much.
const REFERENCE_DB: f32 = -19.5;

/// How far a key may sit off the curve. A listener notices about 1 dB
/// between adjacent notes; two leaves room for the table to go a little
/// stale before the gate trips.
const TOLERANCE_DB: f32 = 2.0;

/// IEC 61672 A-weighting, as a linear amplitude factor.
fn a_weighting(frequency: f32) -> f32 {
    let f2 = frequency * frequency;
    let numerator = 12_194.0f32.powi(2) * f2 * f2;
    let denominator = (f2 + 20.6f32.powi(2))
        * ((f2 + 107.7f32.powi(2)) * (f2 + 737.9f32.powi(2))).sqrt()
        * (f2 + 12_194.0f32.powi(2));
    numerator / denominator * 1.258_9
}

fn a_weighted_level_db(midi: u8) -> f32 {
    let mut engine =
        OfflineEngine::new(SampleRate::new(SAMPLE_RATE_HZ).unwrap(), Tuning::default());
    engine.note_on(midi, VELOCITY);
    let samples = engine.render(WINDOW as f32 / SAMPLE_RATE_HZ);
    let mut spectrum: Vec<Complex32> = samples[..WINDOW]
        .iter()
        .map(|&sample| Complex32::new(sample, 0.0))
        .collect();
    FftPlanner::<f32>::new()
        .plan_fft_forward(WINDOW)
        .process(&mut spectrum);
    let energy: f32 = spectrum[1..WINDOW / 2]
        .iter()
        .enumerate()
        .map(|(bin, value)| {
            let weight = a_weighting((bin + 1) as f32 * SAMPLE_RATE_HZ / WINDOW as f32);
            value.norm_sqr() * weight * weight
        })
        .sum();
    10.0 * (energy / (WINDOW as f32 * WINDOW as f32)).log10()
}

fn deviations_db() -> Vec<(u8, f32)> {
    (LOWEST_PIANO_KEY..=HIGHEST_PIANO_KEY)
        .map(|midi| {
            let target = REFERENCE_DB + loudness_target_db(PianoKey::from_midi(midi).unwrap());
            (midi, a_weighted_level_db(midi) - target)
        })
        .collect()
}

#[test]
fn every_key_sits_on_the_regulated_loudness_curve() {
    for (midi, deviation) in deviations_db() {
        assert!(
            deviation.abs() < TOLERANCE_DB,
            "key {midi} is {deviation:+.1} dB off the regulated curve — regenerate \
             voicing::LEVEL_CORRECTION_DB (see this file's docs)"
        );
    }
}

#[test]
#[ignore = "generator: prints a new voicing::LEVEL_CORRECTION_DB"]
fn print_regenerated_level_table() {
    let table: Vec<String> = deviations_db()
        .iter()
        .zip(LEVEL_CORRECTION_DB.iter())
        .map(|((_, deviation), current)| format!("{:.1}", current - deviation))
        .collect();
    for row in table.chunks(11) {
        println!("    {},", row.join(", "));
    }
}
