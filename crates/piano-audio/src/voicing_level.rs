//! Per-key regulation: the gain a technician's voicing would give each key
//! so that the keyboard sounds even.
//!
//! Nothing in the string model makes neighbouring keys equally loud. The
//! unison count steps from one string to two at A1 and to three at C#3, and
//! each step adds 4-6 dB; the per-register hammer and the treble's
//! mass-loading filter move the top two octaves by several more. Measured
//! A-weighted at *mezzo-forte*, the raw keyboard spanned 26 dB with 5 dB
//! cliffs at both unison breaks — heard as notes jumping out of a scale.
//! A real piano is evened out the same way, after the physics: the
//! technician needles or hardens individual hammers until the scale is
//! smooth. Physically the per-key gain also stands in for how efficiently
//! the soundboard radiates from where each string meets the bridge, which
//! this model's single modal board does not vary along the bridge.
//!
//! [`LEVEL_CORRECTION_DB`] is a measured table, not a formula: the
//! difference between [`loudness_target_db`] and the A-weighted level each
//! key renders at velocity 0.6 through the full engine. It is regenerated
//! by `cargo test -p piano-audio --release --test keyboard_loudness --
//! --ignored --nocapture`, and `keyboard_loudness`'s gate fails if any key
//! drifts more than its tolerance off the target — so a physics change that
//! moves a key's level cannot ship with a stale table.

use piano_core::math;
use piano_params::{HIGHEST_PIANO_KEY, LOWEST_PIANO_KEY, PianoKey};

/// The level, relative to the middle of the keyboard, a key is regulated
/// to, A-weighted. Flat from C3 to C7; the bass sits lower on the meter
/// because A-weighting discounts it far more than the ear does at playing
/// level ([`BASS_TAPER_DB`]), and the top octave tapers the way real
/// pianos' do.
#[must_use]
pub fn loudness_target_db(key: PianoKey) -> f32 {
    let midi = f32::from(key.midi_number());
    let bass =
        (BASS_TAPER_END_MIDI - midi).max(0.0) / (BASS_TAPER_END_MIDI - f32::from(LOWEST_PIANO_KEY));
    let treble = (midi - TREBLE_TAPER_START_MIDI).max(0.0)
        / (f32::from(HIGHEST_PIANO_KEY) - TREBLE_TAPER_START_MIDI);
    -BASS_TAPER_DB * bass - TREBLE_TAPER_DB * treble
}

/// Where the bass taper reaches the flat middle: C3.
const BASS_TAPER_END_MIDI: f32 = 48.0;

/// How far below the middle A0's A-weighted level is regulated.
/// A-weighting follows the 40-phon equal-loudness contour; at playing
/// level (70-80 phon, ISO 226:2003) the ear discounts 50-200 Hz by 10-15 dB
/// less than A-weighting does, so a bass regulated to sound even reads this
/// much lower on an A-weighted meter.
const BASS_TAPER_DB: f32 = 12.0;

/// Where the treble taper starts: C7.
const TREBLE_TAPER_START_MIDI: f32 = 96.0;

/// How far below the middle C8 is regulated.
const TREBLE_TAPER_DB: f32 = 4.0;

/// The gain [`crate::engine`] applies to `key`'s voice.
#[must_use]
pub fn level_for_key(key: PianoKey) -> f32 {
    let correction = LEVEL_CORRECTION_DB
        .get(usize::from(key.key_index()))
        .copied()
        .unwrap_or(0.0);
    math::powf(10.0, correction / 20.0)
}

/// Per-key regulation, in dB, A0 first. Generated — see the module docs.
pub const LEVEL_CORRECTION_DB: [f32; 88] = [
    9.3, 9.4, 9.2, 9.6, 9.8, 9.9, 10.2, 10.0, 10.5, 11.1, 10.2, 9.5, 4.8, 4.4, 3.8, 2.8, 3.7, 4.0,
    3.0, 2.8, 2.9, 3.5, 2.7, 3.4, 3.2, 3.4, 3.0, 3.2, 4.3, 3.6, -1.5, -2.4, -1.6, -1.5, -1.0, -1.0,
    -1.5, -1.0, -1.6, -0.5, -0.3, 0.1, 0.3, 0.1, 0.7, 0.1, 1.3, 1.7, 0.9, 1.4, 1.9, 1.6, 2.1, 2.6,
    4.1, 3.2, 3.0, 2.4, 2.8, 2.5, 1.7, 1.9, 2.2, 1.6, -0.1, -0.7, -1.2, -2.7, -3.7, -4.6, -4.7,
    -4.9, -5.2, -4.9, -4.7, -5.1, -5.0, -3.0, -1.8, -3.3, -1.1, 1.1, 3.9, 3.2, 6.0, 4.5, 2.8, 0.7,
];
