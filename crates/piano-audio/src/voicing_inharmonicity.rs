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

use piano_core::dispersion::MAX_INHARMONICITY;
use piano_core::math;

use super::{RegisterOverrides, interpolate_two_segments};

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

const A0_HZ: f32 = 27.5;
const A4_HZ: f32 = 440.0;
const SEMITONES_PER_OCTAVE: f32 = 12.0;

/// The two-asymptote default `B` at `frequency`.
#[must_use]
pub(super) fn default_inharmonicity(frequency: f32) -> f32 {
    let semitones_above = |reference: f32| {
        SEMITONES_PER_OCTAVE * math::ln(frequency / reference) / core::f32::consts::LN_2
    };
    let bass = BASS_ASYMPTOTE_AT_A0
        * math::exp(-BASS_ASYMPTOTE_FALL_PER_SEMITONE * semitones_above(A0_HZ));
    let treble = TREBLE_ASYMPTOTE_AT_A4
        * math::exp(TREBLE_ASYMPTOTE_GROWTH_PER_SEMITONE * semitones_above(A4_HZ));
    math::clamp_or_low(bass + treble, 0.0, MAX_INHARMONICITY)
}

/// `B` for a key at `frequency`: the default curve, multiplied by a
/// correction that is exactly `override / default` at each overridden
/// anchor and `1` at every other anchor, interpolated in log space between
/// them. An anchor's override therefore lands on that anchor exactly, and
/// with no overrides the default curve passes through untouched.
#[must_use]
pub(super) fn inharmonicity_for(
    frequency: f32,
    bass_hz: f32,
    mid_hz: f32,
    treble_hz: f32,
    registers: RegisterOverrides,
) -> f32 {
    let log_correction = |anchor_hz: f32, wanted: Option<f32>| {
        wanted.map_or(0.0, |value| {
            let value = math::clamp_or_low(value, MIN_OVERRIDE, MAX_INHARMONICITY);
            math::ln(value / default_inharmonicity(anchor_hz))
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
        default_inharmonicity(frequency) * math::exp(correction),
        0.0,
        MAX_INHARMONICITY,
    )
}
