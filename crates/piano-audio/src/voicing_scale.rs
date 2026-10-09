//! The decay half of the scale design: how long each key's fundamental, mid
//! partial and bright partial ring, as a table of anchor keys rather than a
//! curve drawn through three points (issue #88).
//!
//! A piano does not change smoothly from bass to treble. Its scale has
//! breaks where the construction itself changes, and a three-anchor curve
//! smooths every one of them away, leaving a keyboard that sounds like one
//! instrument stretched across 88 notes. Each break is written here as two
//! anchors on *adjacent* keys, so interpolation runs up to the last key on
//! one side and starts again from the first key on the other: the step is
//! represented as a step, not blended across an octave.
//!
//! # The breaks
//!
//! - **Wound to plain wire.** Below the break every string is a steel core
//!   wrapped in copper; above it, bare steel. The winding rubs against its
//!   core as the string bends, a friction loss that grows with frequency,
//!   so a wound string's upper partials die sooner than a plain string's at
//!   the same pitch: the last wound note is darker, and the first plain
//!   note brighter, than a curve through both would make them. That is the
//!   audible "break" tuners and technicians talk about. The split sits at
//!   this model's bichord-to-trichord boundary (D3 | D♯3), where many grand
//!   scales also change wire; the size of the step — 15 % on the bright
//!   partial, 5 % on the mid one, the fundamental continuous — is this
//!   project's own reasoned choice, not a measured figure, the way
//!   `piano_core::unison` labels its boundaries.
//! - **Monochord, bichord, trichord.** Not in this table: the number of
//!   strings per key already changes abruptly
//!   ([`super::unison_count_for_key`]), and a coupled unison's two-stage
//!   decay follows from the strings themselves.
//!
//! # The decay law between anchors
//!
//! A real string does not lose every partial at the same rate: air damping
//! and the wire's internal friction both grow with frequency, so partial `n`
//! dies roughly `n` times faster than the fundamental (Fletcher & Rossing,
//! *The Physics of Musical Instruments*, the piano-string damping section).
//! The anchors follow that `1/n` law loosely, flattened across the low
//! partials where measured pianos hold their first few closer together
//! than `1/n` predicts: at A0, `35 / 18 / 6 s` is a `1 : 1.9 : 5.8` spread
//! against `1/n`'s `1 : 3 : 8`. The fundamentals are the middle of
//! `docs/PHYSICS.md`'s "Typical decay" row at each end and at A4; the
//! partial anchors are this project's own, first solved against in
//! `docs/TIMBRE-PLAN.md`'s D2 table.

use piano_core::math;
use piano_params::Tuning;

use super::{RegisterOverrides, anchor_hz, interpolate_log_frequency, interpolate_two_segments};

/// Which of a key's three decay targets a lookup is for.
#[derive(Debug, Clone, Copy)]
pub(super) enum DecayCurve {
    /// The fundamental.
    Fundamental,
    /// [`super::MID_PARTIAL`].
    MidPartial,
    /// [`super::BRIGHTNESS_PARTIAL`].
    Brightness,
}

/// One key's place in the scale table, its three ring-out times in seconds.
#[derive(Debug, Clone, Copy)]
pub(super) struct ScaleAnchor {
    pub(super) midi: u8,
    pub(super) fundamental_seconds: f32,
    pub(super) mid_partial_seconds: f32,
    pub(super) brightness_seconds: f32,
}

impl ScaleAnchor {
    fn seconds(self, curve: DecayCurve) -> f32 {
        match curve {
            DecayCurve::Fundamental => self.fundamental_seconds,
            DecayCurve::MidPartial => self.mid_partial_seconds,
            DecayCurve::Brightness => self.brightness_seconds,
        }
    }
}

/// A0, the lowest and longest-ringing string.
pub(super) const A0_ANCHOR: ScaleAnchor = ScaleAnchor {
    midi: 21,
    fundamental_seconds: 35.0,
    mid_partial_seconds: 18.0,
    brightness_seconds: 6.0,
};

/// D3, the last wound string: its upper partials a step darker than the
/// curve from A0 to A4 would give it — see the module docs.
pub(super) const LAST_WOUND_ANCHOR: ScaleAnchor = ScaleAnchor {
    midi: 50,
    fundamental_seconds: 20.5,
    mid_partial_seconds: 9.6,
    brightness_seconds: 2.8,
};

/// D♯3, the first plain string: a step brighter than the same curve.
pub(super) const FIRST_PLAIN_ANCHOR: ScaleAnchor = ScaleAnchor {
    midi: 51,
    fundamental_seconds: 20.0,
    mid_partial_seconds: 10.4,
    brightness_seconds: 3.7,
};

/// A4.
pub(super) const A4_ANCHOR: ScaleAnchor = ScaleAnchor {
    midi: 69,
    fundamental_seconds: 11.0,
    mid_partial_seconds: 5.0,
    brightness_seconds: 1.5,
};

/// C8, the shortest string.
pub(super) const C8_ANCHOR: ScaleAnchor = ScaleAnchor {
    midi: 108,
    fundamental_seconds: 1.5,
    mid_partial_seconds: 0.8,
    brightness_seconds: 0.3,
};

/// The scale, bass to treble. Two anchors on adjacent keys are a break.
pub(super) const SCALE_ANCHORS: [ScaleAnchor; 5] = [
    A0_ANCHOR,
    LAST_WOUND_ANCHOR,
    FIRST_PLAIN_ANCHOR,
    A4_ANCHOR,
    C8_ANCHOR,
];

/// Bounds a file's `decay_seconds` override is clamped into, so a zero,
/// negative or `NaN` value cannot send the log-space correction to `±∞`.
const MIN_DECAY_OVERRIDE_SECONDS: f32 = 1.0e-3;
const MAX_DECAY_OVERRIDE_SECONDS: f32 = 1.0e3;

/// `curve`'s built-in target for a string at `frequency`: linear between
/// the two anchors around it, positioned in log frequency.
pub(super) fn scale_seconds(frequency: f32, tuning: Tuning, curve: DecayCurve) -> f32 {
    let (low, high) = segment_around(frequency, tuning);
    interpolate_log_frequency(
        frequency,
        anchor_hz(low.midi, tuning),
        low.seconds(curve),
        anchor_hz(high.midi, tuning),
        high.seconds(curve),
    )
}

/// The pair of adjacent anchors `frequency` falls between; the top pair
/// for anything above C8 or a `NaN`, which interpolation then clamps.
fn segment_around(frequency: f32, tuning: Tuning) -> (ScaleAnchor, ScaleAnchor) {
    SCALE_ANCHORS
        .iter()
        .zip(SCALE_ANCHORS.iter().skip(1))
        .find(|(_, high)| frequency <= anchor_hz(high.midi, tuning))
        .map_or((A4_ANCHOR, C8_ANCHOR), |(low, high)| (*low, *high))
}

/// The fundamental's target at `frequency`: the scale table, times a
/// correction that is exactly `override / table` at each register anchor a
/// `.piano.json` file overrides and `1` at the others, interpolated in log
/// space between them. An override lands exactly on its anchor, and the
/// breaks in the table survive any override.
pub(super) fn fundamental_seconds(
    frequency: f32,
    tuning: Tuning,
    register_hz: (f32, f32, f32),
    registers: RegisterOverrides,
) -> f32 {
    let log_correction = |anchor: f32, wanted: Option<f32>| {
        wanted.map_or(0.0, |value| {
            let value = math::clamp_or_low(
                value,
                MIN_DECAY_OVERRIDE_SECONDS,
                MAX_DECAY_OVERRIDE_SECONDS,
            );
            math::ln(value / scale_seconds(anchor, tuning, DecayCurve::Fundamental))
        })
    };
    let (bass_hz, mid_hz, treble_hz) = register_hz;
    let correction = interpolate_two_segments(
        frequency,
        (
            bass_hz,
            log_correction(bass_hz, registers.bass.decay_seconds),
        ),
        (mid_hz, log_correction(mid_hz, registers.mid.decay_seconds)),
        (
            treble_hz,
            log_correction(treble_hz, registers.treble.decay_seconds),
        ),
    );
    scale_seconds(frequency, tuning, DecayCurve::Fundamental) * math::exp(correction)
}

#[cfg(test)]
mod tests {
    use super::*;
    use piano_params::PianoKey;
    use proptest::prelude::*;

    fn seconds_at(midi: u8, curve: DecayCurve) -> f32 {
        let tuning = Tuning::default();
        let frequency = PianoKey::from_midi(midi).map_or(0.0, |key| key.frequency(tuning).hertz());
        scale_seconds(frequency, tuning, curve)
    }

    fn assert_close(actual: f32, expected: f32) {
        assert!(
            (actual / expected - 1.0).abs() < 1e-5,
            "{actual} vs {expected}"
        );
    }

    #[test]
    fn every_anchor_key_gets_its_table_values() {
        for anchor in SCALE_ANCHORS {
            assert_close(
                seconds_at(anchor.midi, DecayCurve::Fundamental),
                anchor.fundamental_seconds,
            );
            assert_close(
                seconds_at(anchor.midi, DecayCurve::MidPartial),
                anchor.mid_partial_seconds,
            );
            assert_close(
                seconds_at(anchor.midi, DecayCurve::Brightness),
                anchor.brightness_seconds,
            );
        }
    }

    #[test]
    fn the_wound_to_plain_break_is_a_step_not_a_slope() {
        let step = |midi: u8| {
            seconds_at(midi + 1, DecayCurve::Brightness) / seconds_at(midi, DecayCurve::Brightness)
        };
        let across_break = step(LAST_WOUND_ANCHOR.midi);
        assert!(across_break > 1.25, "{across_break}");
        for midi in [40, 48, 55, 60] {
            assert!(
                step(midi) < 1.0,
                "MIDI {midi} brightens going up: {}",
                step(midi)
            );
        }
    }

    #[test]
    fn the_fundamental_stays_continuous_across_the_break() {
        let below = seconds_at(LAST_WOUND_ANCHOR.midi, DecayCurve::Fundamental);
        let above = seconds_at(FIRST_PLAIN_ANCHOR.midi, DecayCurve::Fundamental);
        assert!((below / above - 1.0).abs() < 0.05, "{below} {above}");
    }

    #[test]
    fn every_anchor_rings_its_partials_out_in_order() {
        for anchor in SCALE_ANCHORS {
            assert!(
                anchor.fundamental_seconds > anchor.mid_partial_seconds,
                "{anchor:?}"
            );
            assert!(
                anchor.mid_partial_seconds > anchor.brightness_seconds,
                "{anchor:?}"
            );
        }
    }

    proptest! {
        #[test]
        fn any_frequency_gets_a_finite_positive_target(frequency in proptest::num::f32::ANY) {
            for curve in [DecayCurve::Fundamental, DecayCurve::MidPartial, DecayCurve::Brightness] {
                let seconds = scale_seconds(frequency, Tuning::default(), curve);
                prop_assert!(seconds.is_finite() && seconds > 0.0, "{frequency} {seconds}");
            }
        }
    }
}
