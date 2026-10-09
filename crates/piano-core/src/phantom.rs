//! Phantom partials: the sum-frequency components a stretched string's
//! tension modulation mixes into its own sound.
//!
//! H. A. Conklin, "Generation of partials due to nonlinear mixing in a
//! stringed instrument" (JASA 105(1), 1999), showed that a struck piano
//! string's spectrum holds partials at `f_m + f_n` and `2·f_n` that no
//! linear string model produces. Transverse motion stretches the string;
//! the tension change is proportional to the *square* of the transverse
//! displacement, and that squared term reaches the bridge longitudinally.
//! B. Bank and L. Sujbert, "Generation of longitudinal vibrations in piano
//! strings" (JASA 117(4), 2005), confirm the mechanism and that it matters
//! most in the bass and tenor, where it gives the low register its metallic,
//! "clangy" edge, and that it grows with the square of the strike amplitude
//! — so it is nearly absent at *piano* and plainly there at *fortissimo*.
//!
//! [`PhantomPartials`] models exactly that proportionality: it squares the
//! string's bridge signal, removes the DC the square carries, and adds the
//! result back scaled by a per-key gain. It does not model the free
//! longitudinal resonances (their frequencies are scale-design data this
//! project has no published source for, see issue #65), only the nonlinear
//! mixing Conklin measured.

use crate::filter::DcBlocker;
use crate::math;

/// Largest phantom gain [`PhantomPartials::set_gain`] accepts.
pub const MAX_PHANTOM_GAIN: f32 = 4.0;

/// The squared term's input is clamped to this magnitude first, so a
/// runaway or non-finite input can never square into infinity. Ordinary
/// output never comes near it: the engine's limiter sits at 1.
const MAX_TRANSVERSE_MAGNITUDE: f32 = 4.0;

/// Highpass corner, in hertz, that strips the DC a squared signal always
/// has. Below the lowest piano fundamental, so the difference-frequency
/// phantoms near `f0` survive.
const DC_CORNER_HZ: f32 = 20.0;

/// The nonlinear-mixing stage for one key — see the module docs.
#[derive(Debug, Clone, Copy)]
pub struct PhantomPartials {
    gain: f32,
    dc_blocker: DcBlocker,
}

impl PhantomPartials {
    /// A mixer at `gain` (clamped into `[0, MAX_PHANTOM_GAIN]`, `NaN` to
    /// `0`) for a stream at `sample_rate_hz`.
    #[must_use]
    pub fn new(gain: f32, sample_rate_hz: f32) -> Self {
        let rate = math::clamp_or_low(sample_rate_hz, 1.0, f32::MAX);
        let pole = math::exp(-core::f32::consts::TAU * DC_CORNER_HZ / rate);
        Self {
            gain: math::clamp_or_low(gain, 0.0, MAX_PHANTOM_GAIN),
            dc_blocker: DcBlocker::new(pole),
        }
    }

    /// Sets how strongly this key mixes; `0` turns phantoms off.
    pub fn set_gain(&mut self, gain: f32) {
        self.gain = math::clamp_or_low(gain, 0.0, MAX_PHANTOM_GAIN);
    }

    /// `transverse` plus the phantom partials it generates.
    #[inline]
    pub fn process(&mut self, transverse: f32) -> f32 {
        let bounded = math::clamp_or_low(
            transverse,
            -MAX_TRANSVERSE_MAGNITUDE,
            MAX_TRANSVERSE_MAGNITUDE,
        );
        let phantom = self.dc_blocker.process(self.gain * bounded * bounded);
        bounded + phantom
    }

    /// Clears the highpass memory.
    pub fn reset(&mut self) {
        self.dc_blocker.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const RATE: f32 = 48_000.0;

    /// Energy at `frequency` in `signal`, by correlation with a sinusoid.
    fn energy_at(signal: &[f32], frequency: f32) -> f32 {
        let omega = core::f32::consts::TAU * frequency / RATE;
        let (re, im) = signal
            .iter()
            .enumerate()
            .fold((0.0f32, 0.0f32), |(re, im), (n, &x)| {
                let phase = omega * n as f32;
                (re + x * phase.cos(), im + x * phase.sin())
            });
        re * re + im * im
    }

    fn two_partials(amplitude: f32) -> Vec<f32> {
        (0..9_600)
            .map(|n| {
                let t = n as f32 / RATE;
                amplitude
                    * ((core::f32::consts::TAU * 300.0 * t).sin()
                        + (core::f32::consts::TAU * 500.0 * t).sin())
            })
            .collect()
    }

    fn mixed(gain: f32, input: &[f32]) -> Vec<f32> {
        let mut mixer = PhantomPartials::new(gain, RATE);
        input.iter().map(|&x| mixer.process(x)).collect()
    }

    #[test]
    fn two_partials_gain_a_partial_at_their_sum_frequency() {
        let input = two_partials(0.3);
        let without = energy_at(&mixed(0.0, &input), 800.0);
        let with = energy_at(&mixed(1.0, &input), 800.0);
        assert!(with > 1_000.0 * (without + 1e-9), "{without} {with}");
    }

    #[test]
    fn the_phantom_grows_with_the_square_of_the_strike() {
        let ratio = |amplitude: f32| {
            let out = mixed(1.0, &two_partials(amplitude));
            energy_at(&out, 800.0) / energy_at(&out, 300.0)
        };
        let soft = ratio(0.05);
        let loud = ratio(0.4);
        let growth_db = 10.0 * (loud / soft).log10();
        assert!((16.0..20.0).contains(&growth_db), "{growth_db}");
    }

    #[test]
    fn zero_gain_passes_the_signal_through() {
        let input = two_partials(0.3);
        assert_eq!(mixed(0.0, &input), input);
    }

    proptest! {
        #[test]
        fn any_input_stays_finite_and_bounded(
            inputs in proptest::collection::vec(proptest::num::f32::ANY, 0..64),
            gain in proptest::num::f32::ANY,
            rate in proptest::num::f32::ANY,
        ) {
            let mut mixer = PhantomPartials::new(gain, rate);
            for input in inputs {
                let out = mixer.process(input);
                prop_assert!(out.is_finite());
                prop_assert!(out.abs() <= 200.0);
            }
        }
    }
}
