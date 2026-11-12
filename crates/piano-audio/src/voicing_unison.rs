//! How far apart a key's unison strings are tuned.
//!
//! No tuner leaves a unison at zero beats, and none should: G. Weinreich,
//! "Coupled piano strings", JASA 62(6), 1977, shows that what the strings do
//! together is set by their mistuning *measured against how fast the note
//! decays*. Far apart on that scale, each string rings on its own and the
//! note beats. Close together, they act as one string that loses energy to
//! the bridge quickly, and the mistuning only decides how much of that
//! energy escapes into the slow aftersound.
//!
//! So the spread is not a fixed number of cents. Each key's strings sit
//! [`UNISON_MISTUNING_RATIO`] of the fundamental's own prompt decay rate
//! apart, which keeps the first half-dozen partials on the "one string"
//! side everywhere on the keyboard — where a concert grand's are: resolved
//! into decaying modes, each of its partials is one fast mode with slower
//! companions 10-30 dB under it, not two equals beating
//! (`docs/CALIBRATION.md`). Higher partials, whose mistuning in hertz grows
//! with their number, drift into a slow shimmer. It works out near 0.17
//! cents at A4, a unison a good tuner would call dead on.
//!
//! [`MAX_TUNED_SPREAD_CENTS`] bounds the rule for a string voiced to decay
//! faster than any on the default scale.

use piano_core::math;
use piano_core::string::SILENCE_THRESHOLD;
use piano_params::{PianoKey, Tuning};

use super::scale::{self, DecayCurve};

/// The unison's spread in angular frequency, as a fraction of twice the
/// fundamental's prompt decay rate in nepers per second. Weinreich's two
/// regimes meet at `0.5`.
const UNISON_MISTUNING_RATIO: f32 = 0.07;

/// The widest a tuned unison is left, in cents.
const MAX_TUNED_SPREAD_CENTS: f32 = 1.0;

/// Cents in an octave.
const CENTS_PER_OCTAVE: f32 = 1_200.0;

/// How far apart, in cents, `key`'s flattest and sharpest strings are tuned
/// under `tuning`. See the module docs.
#[must_use]
pub fn unison_spread_cents_for_key(key: PianoKey, tuning: Tuning) -> f32 {
    let frequency = key.frequency(tuning).hertz();
    let prompt_seconds = scale::scale_seconds(frequency, tuning, DecayCurve::Fundamental);
    spread_cents(frequency, prompt_seconds)
}

/// The spread for a string at `frequency_hz` whose prompt sound takes
/// `prompt_seconds` to reach [`SILENCE_THRESHOLD`]. Total: a degenerate
/// frequency or time gives a spread inside `[0, MAX_TUNED_SPREAD_CENTS]`.
fn spread_cents(frequency_hz: f32, prompt_seconds: f32) -> f32 {
    let nepers_per_second = -math::ln(SILENCE_THRESHOLD) / prompt_seconds;
    let spread_hz = UNISON_MISTUNING_RATIO * nepers_per_second / core::f32::consts::PI;
    let octaves = math::ln(1.0 + spread_hz / frequency_hz) / core::f32::consts::LN_2;
    let cents = CENTS_PER_OCTAVE * octaves;
    math::clamp_or_low(cents, 0.0, MAX_TUNED_SPREAD_CENTS)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::*;
    use proptest::prelude::*;

    fn spread_at(midi: u8) -> f32 {
        let key = PianoKey::from_midi(midi).expect("a piano key");
        unison_spread_cents_for_key(key, Tuning::default())
    }

    #[test]
    fn concert_a_is_tuned_a_fraction_of_a_cent_apart() {
        let spread = spread_at(69);
        assert!((0.1..0.3).contains(&spread), "{spread}");
    }

    #[test]
    fn every_key_is_tuned_apart_but_within_the_widest_tuned_unison() {
        for midi in 21..=108 {
            let spread = spread_at(midi);
            assert!(
                spread > 0.05 && spread < MAX_TUNED_SPREAD_CENTS,
                "{midi}: {spread}"
            );
        }
    }

    #[test]
    fn a_string_that_dies_at_once_stops_at_the_widest_tuned_unison() {
        assert!((spread_cents(440.0, 0.01) - MAX_TUNED_SPREAD_CENTS).abs() < 1e-6);
    }

    proptest! {
        #[test]
        fn any_frequency_and_decay_time_give_a_bounded_spread(
            frequency in proptest::num::f32::ANY,
            seconds in proptest::num::f32::ANY,
        ) {
            let cents = spread_cents(frequency, seconds);
            prop_assert!((0.0..=MAX_TUNED_SPREAD_CENTS).contains(&cents), "{cents}");
        }
    }
}
