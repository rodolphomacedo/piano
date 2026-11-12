//! A second-order filter section that stays accurate at low frequencies.
//!
//! The textbook biquad stores its poles as `2·cos ω` and `r²`, two numbers
//! within a hair of 2 and 1 when the filter sits near the bottom of the
//! audio band. In single precision the pole position is then decided by the
//! last few bits, and round-off noise comes out amplified by tens of
//! decibels. The state-variable form keeps `tan(ω/2)` instead, a small
//! number held to full precision, integrates with the trapezoidal rule so
//! no delay-free loop is left to break, and is stable for every positive
//! frequency and damping (V. Zavalishin, *The Art of VA Filter Design*,
//! rev. 2.1.2, 2020, ch. 4, "State variable filter").
//!
//! Its lowpass, bandpass and highpass outputs share one denominator, so any
//! second-order response with that denominator is a weighted sum of the
//! input and two of them. [`StateVariableFilter::highpass`],
//! [`StateVariableFilter::bell`] and [`StateVariableFilter::high_shelf`]
//! take those weights from the analog prototypes of R. Bristow-Johnson's
//! "Cookbook formulae for audio equalizer biquad filter coefficients";
//! trapezoidal integration is the bilinear transform, so the response is
//! exactly the cookbook's.

use core::f32::consts::{PI, SQRT_2};

use crate::math;
use crate::units::SampleRate;

/// Lowest corner accepted, as a fraction of the sample rate.
const MIN_FREQUENCY_RATIO: f32 = 1.0e-5;

/// Highest corner accepted, as a fraction of the sample rate: just short
/// of Nyquist, where `tan` has its pole.
const MAX_FREQUENCY_RATIO: f32 = 0.49;

/// Least resonant section accepted.
pub const MIN_Q: f32 = 0.1;

/// Most resonant section accepted: a peak of 34 dB.
pub const MAX_Q: f32 = 50.0;

/// Largest boost or cut a shelf or a bell accepts, in decibels.
pub const MAX_SHELF_DB: f32 = 40.0;

/// Damping of a shelf, `1/Q` at `Q = 1/√2`: the steepest slope that neither
/// overshoots nor dips on the way from one plateau to the other.
const SHELF_DAMPING: f32 = SQRT_2;

/// One second-order section — see the module docs.
#[derive(Debug, Clone, Copy)]
pub struct StateVariableFilter {
    /// `1/(1 + g·(g + k))`, then the same times `g` and `g²`.
    feedback: [f32; 3],
    /// Weights of the input, the bandpass output and the lowpass output.
    mix: [f32; 3],
    band_state: f32,
    low_state: f32,
}

impl StateVariableFilter {
    /// A highpass with its corner at `cutoff_hz` and a resonance of `q`
    /// there; `q = 1/√2` is the flattest. `cutoff_hz` is clamped into what
    /// `sample_rate` can represent and `q` into `[MIN_Q, MAX_Q]`.
    #[must_use]
    pub fn highpass(cutoff_hz: f32, q: f32, sample_rate: SampleRate) -> Self {
        let damping = 1.0 / math::clamp_or_low(q, MIN_Q, MAX_Q);
        let warped = warped_frequency(cutoff_hz, sample_rate);
        Self::with_response(warped, damping, [1.0, -damping, -1.0])
    }

    /// A bell that raises the band around `centre_hz` by `gain_db` (lowers
    /// it, for a negative gain) and leaves the rest alone. `q` is the
    /// centre over the width of the band between the two frequencies of
    /// half the gain in decibels. Clamped like the other designs.
    #[must_use]
    pub fn bell(centre_hz: f32, gain_db: f32, q: f32, sample_rate: SampleRate) -> Self {
        let amplitude = math::powf(10.0, bounded_decibels(gain_db) / 40.0);
        let damping = 1.0 / (amplitude * math::clamp_or_low(q, MIN_Q, MAX_Q));
        let band = damping * (amplitude * amplitude - 1.0);
        let warped = warped_frequency(centre_hz, sample_rate);
        Self::with_response(warped, damping, [1.0, band, 0.0])
    }

    /// A shelf that leaves the bass alone and raises everything above
    /// `corner_hz` by `gain_db` (lowers it, for a negative gain), clamped
    /// into `±MAX_SHELF_DB`, `NaN` to no gain at all. Half the gain is
    /// reached at the corner.
    #[must_use]
    pub fn high_shelf(corner_hz: f32, gain_db: f32, sample_rate: SampleRate) -> Self {
        let amplitude = math::powf(10.0, bounded_decibels(gain_db) / 40.0);
        let warped = warped_frequency(corner_hz, sample_rate) * math::sqrt(amplitude);
        let band = SHELF_DAMPING * (1.0 - amplitude) * amplitude;
        let mix = [amplitude * amplitude, band, 1.0 - amplitude * amplitude];
        Self::with_response(warped, SHELF_DAMPING, mix)
    }

    /// A section with the denominator `s² + damping·s + 1` at the warped
    /// frequency `warped`, read out through `mix`.
    fn with_response(warped: f32, damping: f32, mix: [f32; 3]) -> Self {
        let base = 1.0 / (1.0 + warped * (warped + damping));
        Self {
            feedback: [base, warped * base, warped * warped * base],
            mix,
            band_state: 0.0,
            low_state: 0.0,
        }
    }

    /// Filters one sample.
    #[inline]
    pub fn process(&mut self, input: f32) -> f32 {
        let [base, single, double] = self.feedback;
        let [input_mix, band_mix, low_mix] = self.mix;
        let excess = input - self.low_state;
        let band = base * self.band_state + single * excess;
        let low = self.low_state + single * self.band_state + double * excess;
        self.band_state = math::flush_denormal(2.0 * band - self.band_state);
        self.low_state = math::flush_denormal(2.0 * low - self.low_state);
        input_mix * input + band_mix * band + low_mix * low
    }

    /// Clears the filter memory.
    pub fn reset(&mut self) {
        self.band_state = 0.0;
        self.low_state = 0.0;
    }
}

/// `gain_db` held within `±MAX_SHELF_DB`; a `NaN` asks for nothing.
fn bounded_decibels(gain_db: f32) -> f32 {
    if gain_db.is_nan() {
        return 0.0;
    }
    gain_db.clamp(-MAX_SHELF_DB, MAX_SHELF_DB)
}

/// `tan(π·f/fs)`: the frequency the trapezoidal integrators must be given
/// for the section's corner to land on `frequency_hz` exactly.
fn warped_frequency(frequency_hz: f32, sample_rate: SampleRate) -> f32 {
    let ratio = math::clamp_or_low(
        frequency_hz / sample_rate.hertz(),
        MIN_FREQUENCY_RATIO,
        MAX_FREQUENCY_RATIO,
    );
    math::tan(PI * ratio)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use proptest::prelude::*;

    use super::*;
    use crate::test_signal::sine_gain_db;

    const RATE_HZ: f32 = 48_000.0;

    fn rate() -> SampleRate {
        SampleRate::new(RATE_HZ).unwrap()
    }

    fn gain_db(mut filter: StateVariableFilter, frequency_hz: f32) -> f32 {
        sine_gain_db(RATE_HZ, frequency_hz, |input| filter.process(input))
    }

    #[test]
    fn a_flat_highpass_is_three_decibels_down_at_its_corner() {
        let filter = StateVariableFilter::highpass(200.0, core::f32::consts::FRAC_1_SQRT_2, rate());
        assert!((gain_db(filter, 200.0) + 3.01).abs() < 0.1);
        assert!(gain_db(filter, 4_000.0).abs() < 0.05);
    }

    #[test]
    fn a_highpass_falls_twelve_decibels_per_octave_below_its_corner() {
        let filter = StateVariableFilter::highpass(800.0, core::f32::consts::FRAC_1_SQRT_2, rate());
        let slope = gain_db(filter, 100.0) - gain_db(filter, 50.0);
        assert!((slope - 12.04).abs() < 0.3, "{slope}");
    }

    #[test]
    fn a_resonant_highpass_peaks_by_its_q_at_the_corner() {
        let filter = StateVariableFilter::highpass(86.0, 3.2, rate());
        let expected = 20.0 * 3.2_f32.log10();
        assert!((gain_db(filter, 86.0) - expected).abs() < 0.2);
    }

    #[test]
    fn a_shelf_reaches_its_gain_above_and_leaves_the_bass_alone() {
        let filter = StateVariableFilter::high_shelf(1_300.0, 7.0, rate());
        assert!(gain_db(filter, 60.0).abs() < 0.1);
        assert!((gain_db(filter, 1_300.0) - 3.5).abs() < 0.1);
        assert!((gain_db(filter, 16_000.0) - 7.0).abs() < 0.1);
    }

    #[test]
    fn a_negative_shelf_cuts_by_as_much() {
        let filter = StateVariableFilter::high_shelf(2_000.0, -9.0, rate());
        assert!((gain_db(filter, 18_000.0) + 9.0).abs() < 0.1);
    }

    #[test]
    fn a_bell_reaches_its_gain_at_the_centre_and_leaves_the_rest_alone() {
        let filter = StateVariableFilter::bell(128.0, 7.0, 4.0, rate());
        assert!((gain_db(filter, 128.0) - 7.0).abs() < 0.1);
        assert!(gain_db(filter, 30.0).abs() < 0.3);
        assert!(gain_db(filter, 1_000.0).abs() < 0.3);
    }

    #[test]
    fn a_bell_is_half_its_gain_at_the_edges_its_q_sets() {
        let (centre, q) = (1_000.0_f32, 2.0_f32);
        let filter = StateVariableFilter::bell(centre, 6.0, q, rate());
        let half_width = (0.5 / q + (1.0 + 0.25 / (q * q)).sqrt()).ln() / core::f32::consts::LN_2;
        let upper_edge = centre * 2.0_f32.powf(half_width);
        assert!((gain_db(filter, upper_edge) - 3.0).abs() < 0.25);
    }

    #[test]
    fn a_negative_bell_cuts_by_as_much() {
        let filter = StateVariableFilter::bell(560.0, -3.5, 1.2, rate());
        assert!((gain_db(filter, 560.0) + 3.5).abs() < 0.1);
    }

    #[test]
    fn a_corner_in_the_deep_bass_is_still_placed_exactly() {
        let filter = StateVariableFilter::highpass(30.0, core::f32::consts::FRAC_1_SQRT_2, rate());
        assert!((gain_db(filter, 30.0) + 3.01).abs() < 0.1);
    }

    #[test]
    fn reset_silences_the_section() {
        let mut filter = StateVariableFilter::highpass(100.0, 5.0, rate());
        for _ in 0..64 {
            filter.process(1.0);
        }
        filter.reset();
        assert!(filter.process(0.0).abs() < f32::EPSILON);
    }

    proptest! {
        #[test]
        fn any_design_stays_finite_on_any_bounded_input(
            design in proptest::array::uniform3(proptest::num::f32::ANY),
            rate_hz in 1_000.0_f32..400_000.0,
            inputs in proptest::collection::vec(-1.0e6_f32..1.0e6, 0..256),
        ) {
            let rate = SampleRate::new(rate_hz).unwrap();
            let mut highpass = StateVariableFilter::highpass(design[0], design[1], rate);
            let mut shelf = StateVariableFilter::high_shelf(design[0], design[2], rate);
            let mut bell = StateVariableFilter::bell(design[0], design[2], design[1], rate);
            for input in inputs {
                prop_assert!(highpass.process(input).is_finite());
                prop_assert!(shelf.process(input).is_finite());
                prop_assert!(bell.process(input).is_finite());
            }
        }
    }
}
