//! The default inharmonicity curve across the keyboard, and how a file's
//! register overrides bend it.
//!
//! A piano's `B` is not a straight line from bass to treble. Plotted on a log
//! scale against key number it follows two asymptotes: in the bass, wound
//! strings get relatively *less* stiff going up the compass, so `B` falls
//! from A0 to a minimum around the bass break; above it, plain wire strings
//! get shorter much faster than thinner, so `B` grows exponentially into the
//! top treble. F. Rigaud, B. David & L. Daudet, "A parametric model and
//! estimation techniques for the inharmonicity and tuning of the piano",
//! JASA 133(5), 2013, model `B` along the compass as exactly this sum of two
//! exponentials, one per asymptote.
//!
//! The four numbers below are this project's reasoned anchors inside the
//! ranges Fletcher & Rossing (*The Physics of Musical Instruments*, 2nd ed.,
//! §12.4) report for full-size pianos — about `3·10⁻⁴` at A0, a minimum
//! near `10⁻⁴` in the low tenor, `4–5·10⁻⁴` at A4 and `1–2·10⁻²` at C8 — not
//! values fitted to a particular instrument. The previous default, a
//! straight line from `10⁻⁴` to `0.05`, gave A4 roughly forty times a real
//! string's `B` (issue #96).
//!
//! # The wound-to-plain break
//!
//! A sum of two smooth exponentials cannot step, but a real scale does: a
//! wound string's copper winding adds mass without adding bending
//! stiffness, so the last wound string is noticeably less inharmonic than a
//! plain wire at the same pitch, and `B` jumps up where the wire changes.
//! Rigaud, David & Daudet (2013) fit their two asymptotes on either side of
//! exactly this discontinuity. Here the break sits where the decay table
//! puts it ([`super::scale::LAST_WOUND_ANCHOR`] | the next key), as a factor
//! on the smooth curve: `1` at A0, falling to [`WOUND_SIDE_OF_BREAK`] at the
//! last wound key, jumping to [`PLAIN_SIDE_OF_BREAK`] on the first plain
//! key and returning to `1` at A4, all in log space. The factor keeps the
//! curve's sourced ends untouched and makes the step a step; its size, a
//! ratio of about 1.35, is this project's reasoned choice, not a measured
//! figure.

use piano_core::dispersion::MAX_INHARMONICITY;
use piano_core::math;
use piano_params::Tuning;

use super::scale::{A0_ANCHOR, A4_ANCHOR, FIRST_PLAIN_ANCHOR, LAST_WOUND_ANCHOR};
use super::{RegisterOverrides, anchor_hz, interpolate_log_frequency, interpolate_two_segments};

/// The bass asymptote's value at A0.
const BASS_ASYMPTOTE_AT_A0: f32 = 3.0e-4;

/// How fast the bass asymptote falls, per semitone above A0.
const BASS_ASYMPTOTE_FALL_PER_SEMITONE: f32 = 0.08;

/// The treble asymptote's value at A4.
const TREBLE_ASYMPTOTE_AT_A4: f32 = 4.5e-4;

/// How fast the treble asymptote grows, per semitone above A4. Reaches
/// `1.5·10⁻²` at C8.
const TREBLE_ASYMPTOTE_GROWTH_PER_SEMITONE: f32 = 0.09;

/// Smallest override honoured, so a file asking for `B = 0` cannot send the
/// log-space correction to `-∞`. Far below any real string.
const MIN_OVERRIDE: f32 = 1.0e-7;

/// The smooth curve's factor at the last wound string.
pub(super) const WOUND_SIDE_OF_BREAK: f32 = 0.85;

/// The smooth curve's factor at the first plain string.
pub(super) const PLAIN_SIDE_OF_BREAK: f32 = 1.15;

const A0_HZ: f32 = 27.5;
const A4_HZ: f32 = 440.0;
const SEMITONES_PER_OCTAVE: f32 = 12.0;

/// The two-asymptote default `B` at `frequency`.
#[must_use]
fn default_inharmonicity(frequency: f32) -> f32 {
    let semitones_above = |reference: f32| {
        SEMITONES_PER_OCTAVE * math::ln(frequency / reference) / core::f32::consts::LN_2
    };
    let bass = BASS_ASYMPTOTE_AT_A0
        * math::exp(-BASS_ASYMPTOTE_FALL_PER_SEMITONE * semitones_above(A0_HZ));
    let treble = TREBLE_ASYMPTOTE_AT_A4
        * math::exp(TREBLE_ASYMPTOTE_GROWTH_PER_SEMITONE * semitones_above(A4_HZ));
    math::clamp_or_low(bass + treble, 0.0, MAX_INHARMONICITY)
}

/// The factor the wound-to-plain break puts on the smooth curve at
/// `frequency`; see the module docs.
fn break_factor(frequency: f32, tuning: Tuning) -> f32 {
    let last_wound_hz = anchor_hz(LAST_WOUND_ANCHOR.midi, tuning);
    let log_factor = if frequency <= last_wound_hz {
        let a0_hz = anchor_hz(A0_ANCHOR.midi, tuning);
        let wound = math::ln(WOUND_SIDE_OF_BREAK);
        interpolate_log_frequency(frequency, a0_hz, 0.0, last_wound_hz, wound)
    } else {
        let first_plain_hz = anchor_hz(FIRST_PLAIN_ANCHOR.midi, tuning);
        let a4_hz = anchor_hz(A4_ANCHOR.midi, tuning);
        let plain = math::ln(PLAIN_SIDE_OF_BREAK);
        interpolate_log_frequency(frequency, first_plain_hz, plain, a4_hz, 0.0)
    };
    math::exp(log_factor)
}

/// The built-in `B` at `frequency`: the two-asymptote curve with the
/// wound-to-plain break cut into it.
#[must_use]
pub(super) fn scale_inharmonicity(frequency: f32, tuning: Tuning) -> f32 {
    math::clamp_or_low(
        default_inharmonicity(frequency) * break_factor(frequency, tuning),
        0.0,
        MAX_INHARMONICITY,
    )
}

/// `B` for a key at `frequency`: the scale's curve, multiplied by a
/// correction that is exactly `override / default` at each overridden
/// anchor and `1` at every other anchor, interpolated in log space between
/// them. An anchor's override therefore lands on that anchor exactly, and
/// with no overrides the default curve passes through untouched.
#[must_use]
pub(super) fn inharmonicity_for(
    frequency: f32,
    tuning: Tuning,
    register_hz: (f32, f32, f32),
    registers: RegisterOverrides,
) -> f32 {
    let (bass_hz, mid_hz, treble_hz) = register_hz;
    let log_correction = |anchor_hz: f32, wanted: Option<f32>| {
        wanted.map_or(0.0, |value| {
            let value = math::clamp_or_low(value, MIN_OVERRIDE, MAX_INHARMONICITY);
            math::ln(value / scale_inharmonicity(anchor_hz, tuning))
        })
    };
    let correction = interpolate_two_segments(
        frequency,
        (
            bass_hz,
            log_correction(bass_hz, registers.bass.inharmonicity),
        ),
        (mid_hz, log_correction(mid_hz, registers.mid.inharmonicity)),
        (
            treble_hz,
            log_correction(treble_hz, registers.treble.inharmonicity),
        ),
    );
    math::clamp_or_low(
        scale_inharmonicity(frequency, tuning) * math::exp(correction),
        0.0,
        MAX_INHARMONICITY,
    )
}
