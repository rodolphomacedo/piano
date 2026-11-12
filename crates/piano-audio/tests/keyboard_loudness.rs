//! The regulation gate: every key, rendered through the full engine at
//! *mezzo-forte*, must sit on the regulated loudness curve
//! (`piano_audio::voicing::loudness_target_db`), measured A-weighted over
//! its first 0.68 s as the mean power of the stereo pair.
//!
//! Without regulation the keyboard spanned 26 dB with 5 dB cliffs at both
//! unison breaks. What is regulated is the trend along each unison
//! register, not each key on its own: the soundboard's diffuse field
//! (`piano_core::body`) answers a single partial several decibels louder or
//! softer depending on where it falls among the field's resonances, as a
//! real board does, and that pattern moves with the sample rate and the
//! tuning. A table that cancelled it key by key would be right at one
//! sample rate and concert pitch and twice as uneven at any other. So each
//! key's deviation is averaged with its neighbours' ([`NEIGHBOURS`] either
//! side, never across a change of unison count) before it is judged or
//! corrected.
//!
//! The ignored test below regenerates `voicing::LEVEL_CORRECTION_DB` after
//! any change that moves a register's level. Run it until the table stops
//! changing; the limiter makes the first pass approximate:
//!
//! ```sh
//! cargo test -p piano-audio --release --test keyboard_loudness -- --ignored --nocapture
//! ```

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use piano_audio::offline::OfflineEngine;
use piano_audio::voicing::{LEVEL_CORRECTION_DB, loudness_target_db, unison_count_for_key};
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

/// How far a register's trend may sit off the curve at any key. A
/// listener notices about 1 dB between adjacent notes; two leaves room for
/// the table to go a little stale before the gate trips.
const TOLERANCE_DB: f32 = 2.0;

/// How far one key may sit off the curve on its own: the spread the
/// diffuse field gives single partials, with room to spare. A key further
/// out than this is broken, not coloured.
const SINGLE_KEY_TOLERANCE_DB: f32 = 8.0;

/// Keys averaged on either side of each key.
const NEIGHBOURS: usize = 5;

/// IEC 61672 A-weighting, as a linear amplitude factor.
fn a_weighting(frequency: f32) -> f32 {
    let f2 = frequency * frequency;
    let numerator = 12_194.0f32.powi(2) * f2 * f2;
    let denominator = (f2 + 20.6f32.powi(2))
        * ((f2 + 107.7f32.powi(2)) * (f2 + 737.9f32.powi(2))).sqrt()
        * (f2 + 12_194.0f32.powi(2));
    numerator / denominator * 1.258_9
}

/// A-weighted energy of one channel, in this test's units.
fn a_weighted_energy(samples: &[f32]) -> f32 {
    let mut spectrum: Vec<Complex32> = samples[..WINDOW]
        .iter()
        .map(|&sample| Complex32::new(sample, 0.0))
        .collect();
    FftPlanner::<f32>::new()
        .plan_fft_forward(WINDOW)
        .process(&mut spectrum);
    let weighted = spectrum[1..WINDOW / 2]
        .iter()
        .enumerate()
        .map(|(bin, value)| {
            let weight = a_weighting((bin + 1) as f32 * SAMPLE_RATE_HZ / WINDOW as f32);
            value.norm_sqr() * weight * weight
        });
    weighted.sum::<f32>() / (WINDOW as f32 * WINDOW as f32)
}

fn a_weighted_level_db(midi: u8) -> f32 {
    let mut engine =
        OfflineEngine::new(SampleRate::new(SAMPLE_RATE_HZ).unwrap(), Tuning::default());
    engine.note_on(midi, VELOCITY);
    let (left, right) = engine.render_stereo(WINDOW as f32 / SAMPLE_RATE_HZ);
    10.0 * f32::midpoint(a_weighted_energy(&left), a_weighted_energy(&right)).log10()
}

/// How far each key sits off the curve, on its own.
fn deviations_db() -> Vec<(u8, f32)> {
    (LOWEST_PIANO_KEY..=HIGHEST_PIANO_KEY)
        .map(|midi| {
            let target = REFERENCE_DB + loudness_target_db(PianoKey::from_midi(midi).unwrap());
            (midi, a_weighted_level_db(midi) - target)
        })
        .collect()
}

/// Each key's value averaged with its neighbours of the same unison
/// count: the trend along a register.
fn register_trend_db(per_key: &[(u8, f32)]) -> Vec<f32> {
    let strings = |midi: u8| unison_count_for_key(PianoKey::from_midi(midi).unwrap());
    (0..per_key.len())
        .map(|index| {
            let from = index.saturating_sub(NEIGHBOURS);
            let to = (index + NEIGHBOURS + 1).min(per_key.len());
            let own = strings(per_key[index].0);
            let alike: Vec<f32> = per_key[from..to]
                .iter()
                .filter(|(midi, _)| strings(*midi) == own)
                .map(|(_, value)| *value)
                .collect();
            alike.iter().sum::<f32>() / alike.len() as f32
        })
        .collect()
}

#[test]
fn every_register_sits_on_the_regulated_loudness_curve() {
    let deviations = deviations_db();
    let trend = register_trend_db(&deviations);
    for ((midi, deviation), trend) in deviations.iter().zip(&trend) {
        assert!(
            trend.abs() < TOLERANCE_DB,
            "the keys around {midi} are {trend:+.1} dB off the regulated curve — regenerate \
             voicing::LEVEL_CORRECTION_DB (see this file's docs)"
        );
        assert!(
            deviation.abs() < SINGLE_KEY_TOLERANCE_DB,
            "key {midi} alone is {deviation:+.1} dB off the regulated curve"
        );
    }
}

#[test]
#[ignore = "generator: prints a new voicing::LEVEL_CORRECTION_DB"]
fn print_regenerated_level_table() {
    let deviations = deviations_db();
    // What would put each key exactly on the curve, then its trend: the
    // table stays as smooth after any number of passes.
    let wanted: Vec<(u8, f32)> = deviations
        .iter()
        .zip(LEVEL_CORRECTION_DB.iter())
        .map(|((midi, deviation), current)| (*midi, current - deviation))
        .collect();
    let table: Vec<String> = register_trend_db(&wanted)
        .iter()
        .map(|correction| format!("{correction:.1}"))
        .collect();
    for row in table.chunks(11) {
        println!("    {},", row.join(", "));
    }
    let spread: Vec<String> = deviations
        .iter()
        .map(|(_, deviation)| format!("{deviation:+.1}"))
        .collect();
    println!(
        "each key's own deviation before this pass: {}",
        spread.join(" ")
    );
}
