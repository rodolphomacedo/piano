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
/// to, A-weighted. Flat from C3 to C6; the bass sits lower on the meter
/// because A-weighting discounts it far more than the ear does at playing
/// level ([`BASS_TAPER_DB`]), and the top two octaves taper the way real
/// pianos' do ([`TREBLE_TAPER_START_MIDI`]).
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

/// Where the treble taper starts: C6. Above it a piano's radiated level
/// falls away (the short treble strings carry little energy and the
/// soundboard radiates them less efficiently), and the ear is already more
/// sensitive there than A-weighting credits: at playing level (70 phon,
/// ISO 226:2003) 2-4 kHz needs about 3 dB less sound pressure than 1 kHz
/// for the same loudness, where A-weighting allows only about 1 dB. A
/// treble regulated flat on an A-weighted meter therefore sounds too
/// bright and too loud — heard as shrill.
const TREBLE_TAPER_START_MIDI: f32 = 84.0;

/// How far below the middle C8 is regulated.
const TREBLE_TAPER_DB: f32 = 8.0;

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
    9.3, 9.6, 9.0, 9.4, 9.8, 10.1, 10.5, 10.0, 10.4, 11.2, 10.0, 9.2, 4.9, 4.8, 4.1, 3.1, 3.9, 4.2,
    3.2, 3.2, 3.2, 3.8, 3.0, 3.8, 3.6, 3.8, 3.3, 3.5, 4.8, 4.1, -1.6, -2.7, -1.5, -1.9, -1.4, -1.3,
    -1.4, -1.1, -1.8, -0.8, -0.3, 0.2, 0.1, 0.0, 0.4, -0.1, 0.9, 1.6, 1.3, 1.7, 2.5, 1.9, 2.6, 2.7,
    4.3, 3.2, 2.7, 2.9, 2.4, 2.7, 1.7, 2.5, 2.8, 1.7, -0.8, -1.0, -3.1, -3.4, -5.9, -7.4, -8.4,
    -6.2, -8.8, -8.9, -7.8, -9.2, -9.4, -7.2, -5.7, -7.3, -4.5, -2.3, -1.5, -1.2, 3.0, -1.7, -2.7,
    -2.9,
];

/// How strongly `key` mixes phantom partials ([`piano_core::phantom`]):
/// [`PHANTOM_GAIN_IN_BASS`] up to A2, tapering to nothing by E5. Conklin
/// (1999) and Bank & Sujbert (2005) find the nonlinear mixing prominent in
/// the bass and tenor, where long, heavily wound strings stretch the most,
/// and negligible in the short treble strings.
#[must_use]
pub fn phantom_gain_for_key(key: PianoKey) -> f32 {
    let midi = f32::from(key.midi_number());
    let taper =
        (PHANTOM_TAPER_END_MIDI - midi) / (PHANTOM_TAPER_END_MIDI - PHANTOM_FULL_UNTIL_MIDI);
    PHANTOM_GAIN_IN_BASS * math::clamp_or_low(taper, 0.0, 1.0)
}

/// Phantom gain below A2, set by measurement: see `PHYSICS.md`'s phantom
/// partials section for the level it puts the sum-frequency partials at.
pub const PHANTOM_GAIN_IN_BASS: f32 = 0.1;

/// Last key with the full bass phantom gain: A2.
const PHANTOM_FULL_UNTIL_MIDI: f32 = 45.0;

/// First key with no phantom partials: E5.
const PHANTOM_TAPER_END_MIDI: f32 = 76.0;

/// How strongly `key`'s duplex segment rings ([`piano_core::duplex`]):
/// [`DUPLEX_GAIN`] from [`DUPLEX_FROM_MIDI`] up, `0` below, where a duplex
/// scale leaves the rear lengths muted.
#[must_use]
pub fn duplex_gain_for_key(key: PianoKey) -> f32 {
    if f32::from(key.midi_number()) >= DUPLEX_FROM_MIDI {
        DUPLEX_GAIN
    } else {
        0.0
    }
}

/// Which partial `key`'s duplex segment is set to: the double octave up to
/// B5, the octave from C6, where the string's fourth partial is too weak
/// (and its segment too short) to ring. A reasoned choice: published
/// accounts give harmonic duplex lengths but not a per-key table.
#[must_use]
pub fn duplex_harmonic_for_key(key: PianoKey) -> f32 {
    if f32::from(key.midi_number()) >= DUPLEX_OCTAVE_FROM_MIDI {
        2.0
    } else {
        4.0
    }
}

/// Where duplex segments switch from the double octave to the octave: C6.
const DUPLEX_OCTAVE_FROM_MIDI: f32 = 84.0;

/// The lowest key with a free duplex segment: C4, roughly where a
/// concert grand's duplex scale begins.
const DUPLEX_FROM_MIDI: f32 = 60.0;

/// Duplex resonance level, set by measurement: see `PHYSICS.md`'s duplex
/// section for where it puts the segment's ring relative to the note.
pub const DUPLEX_GAIN: f32 = 0.2;
