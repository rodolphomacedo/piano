//! The decay half of the scale design: how fast each key's fundamental, mid
//! partial and bright partial die away, as a table of anchor keys rather
//! than a curve drawn through three points (issue #88).
//!
//! A piano does not change smoothly from bass to treble. Its scale has
//! breaks where the construction itself changes, and a smooth curve blends
//! every one of them away, leaving a keyboard that sounds like one
//! instrument stretched across 88 notes. Each break is written here as two
//! anchors on *adjacent* keys, so interpolation runs up to the last key on
//! one side and starts again from the first key on the other: the step is
//! represented as a step, not blended across an octave.
//!
//! # What an anchor's rate means
//!
//! Each rate is how fast the note's **prompt sound** falls, in decibels per
//! second: the motion in which all of a key's strings push the bridge
//! together (G. Weinreich, "Coupled piano strings", JASA 62(6), 1977). The
//! slower aftersound that follows is not in this table; it comes out of the
//! unison itself (`piano_core::unison`). The rest of the voicing works in
//! seconds to [`piano_core::string::SILENCE_THRESHOLD`], so an anchor is
//! stored as that: [`DECAY_SPAN_DB`] divided by its rate.
//!
//! # Where the rates come from
//!
//! They are calibrated against a concert grand, by resolving each partial
//! of its notes into exponentially decaying modes and reading the dominant
//! one (`docs/CALIBRATION.md`; only the derived rates are kept here). What
//! that shows, and what the table follows:
//!
//! - **Through the middle of the keyboard every partial falls about as fast
//!   as the fundamental**, 15-25 dB/s for the first dozen. The loss is the
//!   bridge's, and the bridge takes all partials alike. An earlier table
//!   let the fundamental ring four times longer than this and killed the
//!   eighth partial seven times faster than it, which is a plucked string,
//!   not a struck one: a tone that goes dull while it is still loud.
//! - **The bass rings long at every partial**: a few dB/s for the first
//!   twenty, where the earlier table lost them at 40-90 dB/s. Individual
//!   bass partials differ widely, by whether a soundboard mode sits under
//!   them; that part is `piano_core::bridge_load`'s, on top of this table.
//! - **The treble dies in well under a second**, 80-170 dB/s at the top.
//!
//! # The break
//!
//! **Two wound strings to three plain ones** (D3 | D♯3). Three strings
//! load the bridge harder than two, and in the measurement the upper
//! partials fall about a fifth faster on the three-string side of the break
//! while the fundamental carries straight across. That is the opposite of
//! the "wound strings are darker" step an earlier table guessed at.
//!
//! The monochord-to-bichord boundary ([`super::unison_count_for_key`])
//! shows no such step in the measured rates, so the table has none there.
//!
//! Between anchors a key's three targets are interpolated in log frequency,
//! and partials between and above the three are placed by
//! `voicing::DecayTargets`.

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

/// How far [`piano_core::string::SILENCE_THRESHOLD`] is below a struck
/// note, in decibels: what an anchor's "seconds" are seconds *to*.
const DECAY_SPAN_DB: f32 = 80.0;

/// The anchor at `midi` whose fundamental, mid partial and bright partial
/// fall at `db_per_second`.
const fn falling(midi: u8, db_per_second: [f32; 3]) -> ScaleAnchor {
    ScaleAnchor {
        midi,
        fundamental_seconds: DECAY_SPAN_DB / db_per_second[0],
        mid_partial_seconds: DECAY_SPAN_DB / db_per_second[1],
        brightness_seconds: DECAY_SPAN_DB / db_per_second[2],
    }
}

/// A0, the lowest and longest-ringing string.
pub(super) const A0_ANCHOR: ScaleAnchor = falling(21, [3.0, 3.3, 5.0]);

/// A1.
pub(super) const A1_ANCHOR: ScaleAnchor = falling(33, [3.5, 3.8, 5.5]);

/// E2, where the fundamental has climbed into the soundboard's own modes
/// and `piano_core::bridge_load` starts taking its share on top of this.
pub(super) const E2_ANCHOR: ScaleAnchor = falling(40, [6.0, 6.3, 7.5]);

/// A2.
pub(super) const A2_ANCHOR: ScaleAnchor = falling(45, [7.5, 7.8, 9.0]);

/// D3, the last wound, two-string key.
pub(super) const LAST_WOUND_ANCHOR: ScaleAnchor = falling(50, [11.0, 11.4, 12.8]);

/// D♯3, the first of the three-string keys: its upper partials a step
/// shorter-lived than its two-string neighbour's.
pub(super) const FIRST_PLAIN_ANCHOR: ScaleAnchor = falling(51, [11.0, 11.6, 15.5]);

/// A3.
pub(super) const A3_ANCHOR: ScaleAnchor = falling(57, [13.5, 14.5, 19.0]);

/// A4.
pub(super) const A4_ANCHOR: ScaleAnchor = falling(69, [17.0, 18.0, 21.0]);

/// E5.
pub(super) const E5_ANCHOR: ScaleAnchor = falling(76, [26.0, 28.0, 40.0]);

/// C6.
pub(super) const C6_ANCHOR: ScaleAnchor = falling(84, [40.0, 42.0, 64.0]);

/// G6.
pub(super) const G6_ANCHOR: ScaleAnchor = falling(91, [42.0, 50.0, 100.0]);

/// C7.
pub(super) const C7_ANCHOR: ScaleAnchor = falling(96, [80.0, 100.0, 200.0]);

/// C8, the shortest string.
pub(super) const C8_ANCHOR: ScaleAnchor = falling(108, [130.0, 180.0, 400.0]);

/// The scale, bass to treble. Two anchors on adjacent keys are a break.
pub(super) const SCALE_ANCHORS: [ScaleAnchor; 13] = [
    A0_ANCHOR,
    A1_ANCHOR,
    E2_ANCHOR,
    A2_ANCHOR,
    LAST_WOUND_ANCHOR,
    FIRST_PLAIN_ANCHOR,
    A3_ANCHOR,
    A4_ANCHOR,
    E5_ANCHOR,
    C6_ANCHOR,
    G6_ANCHOR,
    C7_ANCHOR,
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
        .map_or((C7_ANCHOR, C8_ANCHOR), |(low, high)| (*low, *high))
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
    fn the_two_to_three_string_break_is_a_step_not_a_slope() {
        let step = |midi: u8| {
            seconds_at(midi + 1, DecayCurve::Brightness) / seconds_at(midi, DecayCurve::Brightness)
        };
        let across_break = step(LAST_WOUND_ANCHOR.midi);
        assert!(across_break < 0.85, "{across_break}");
        for midi in [40, 48, 55, 60] {
            assert!(
                (0.9..1.0).contains(&step(midi)),
                "MIDI {midi} steps by {}",
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
    fn no_anchor_lets_an_upper_partial_outlast_a_lower_one() {
        for anchor in SCALE_ANCHORS {
            assert!(
                anchor.fundamental_seconds >= anchor.mid_partial_seconds,
                "{anchor:?}"
            );
            assert!(
                anchor.mid_partial_seconds > anchor.brightness_seconds,
                "{anchor:?}"
            );
        }
    }

    #[test]
    fn the_table_runs_from_the_lowest_key_to_the_highest_in_order() {
        assert_eq!(SCALE_ANCHORS.first().map(|anchor| anchor.midi), Some(21));
        assert_eq!(SCALE_ANCHORS.last().map(|anchor| anchor.midi), Some(108));
        assert!(SCALE_ANCHORS.is_sorted_by(|low, high| low.midi < high.midi));
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
