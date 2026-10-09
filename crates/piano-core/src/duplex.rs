//! The duplex scale: string segments beyond the speaking length, left free
//! to ring in sympathy with it.
//!
//! On most pianos the short lengths of string between the bridge and the
//! hitch pins are muted with felt. C. F. Theodore Steinway's duplex scale
//! (US patent 126,848, 1872) leaves them free in the upper half of the
//! keyboard and sets their lengths so they resonate at a harmonic of the
//! speaking length; H. A. Conklin, "Design and tone in the mechanoacoustic
//! piano, Part III" (JASA 100(3), 1996), describes the result as a
//! shimmering "after-ring" added to the treble. The duplex segment is short
//! and almost unstiff, so its resonance sits at an exact multiple of the
//! fundamental, while the stiff speaking length's own partial sits slightly
//! sharp of it (inharmonicity): the two beat gently, which is the shimmer.
//!
//! A segment tuned to the *exact* multiple would hardly ring at all: a
//! treble string's fourth partial sits tens of hertz sharp of `4·f0`, far
//! outside a high-Q resonance. Technicians set the duplex bars by ear for
//! exactly this reason, so the model tunes each segment to its string's
//! real inharmonic partial, `n·f0·√((1 + B·n²)/(1 + B))`, and then
//! [`DUPLEX_DETUNE_CENTS`] off it — the residual mistuning a by-ear setting
//! leaves, heard as a slow beat.
//!
//! [`DuplexResonance`] is one lightly damped two-pole resonator per key,
//! driven by the key's bridge signal and added back at a small gain. Only
//! the keys a duplex scale leaves free get one.

use crate::math;
use crate::soundboard::{Resonator, SoundboardMode};

/// How far a by-ear duplex setting is left off its partial, in cents: a
/// beat of a few tenths of a hertz in the treble.
const DUPLEX_DETUNE_CENTS: f32 = 1.5;

/// Which partial of which string a duplex segment rings with.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DuplexTuning {
    /// The partial number the segment's length is set to.
    pub harmonic: f32,
    /// The speaking length's inharmonicity coefficient `B`.
    pub inharmonicity: f32,
}

impl DuplexTuning {
    /// The frequency the segment resonates at for a string at
    /// `fundamental_hz`: its real partial, then [`DUPLEX_DETUNE_CENTS`] off.
    fn frequency_hz(self, fundamental_hz: f32) -> f32 {
        let partial = math::clamp_or_low(self.harmonic, 0.0, 64.0);
        let stiffness = math::clamp_or_low(self.inharmonicity, 0.0, 1.0);
        let stretch = math::sqrt((1.0 + stiffness * partial * partial) / (1.0 + stiffness));
        let detune = math::powf(2.0, DUPLEX_DETUNE_CENTS / 1_200.0);
        partial * math::clamp_or_low(fundamental_hz, 0.0, 1.0e6) * stretch * detune
    }
}

/// How long the free segment rings, to `1/e`, in seconds — about 1.4 s to
/// silence, a little shorter than the treble strings it hangs off.
const DUPLEX_DECAY_SECONDS: f32 = 0.2;

/// Largest duplex gain a caller can ask for.
pub const MAX_DUPLEX_GAIN: f32 = 1.0;

/// Highest the resonance may sit, as a fraction of the sample rate, so it
/// never folds over Nyquist.
const MAX_FREQUENCY_FRACTION: f32 = 0.45;

/// One key's free duplex segment — see the module docs.
#[derive(Debug, Clone)]
pub struct DuplexResonance {
    resonator: Resonator,
    gain: f32,
    representable: bool,
}

impl DuplexResonance {
    /// The duplex segment of a string at `fundamental_hz`, set to `tuning`
    /// and ringing at `gain` (clamped into `[0, MAX_DUPLEX_GAIN]`; `0` for
    /// a muted segment). A resonance that would sit above Nyquist is
    /// silenced.
    #[must_use]
    pub fn new(fundamental_hz: f32, tuning: DuplexTuning, gain: f32, sample_rate_hz: f32) -> Self {
        let rate = math::clamp_or_low(sample_rate_hz, 1.0, f32::MAX);
        let frequency = tuning.frequency_hz(fundamental_hz);
        let fits = frequency < MAX_FREQUENCY_FRACTION * rate;
        let mode = SoundboardMode {
            frequency_hz: frequency,
            decay_seconds: DUPLEX_DECAY_SECONDS,
            gain: 1.0,
            bridge_coupling: 0.0,
        };
        let gain = if fits { gain } else { 0.0 };
        Self {
            resonator: Resonator::new(mode, rate),
            gain: math::clamp_or_low(gain, 0.0, MAX_DUPLEX_GAIN),
            representable: fits,
        }
    }

    /// Sets how strongly the segment rings; `0` mutes it. Clamped into
    /// `[0, MAX_DUPLEX_GAIN]`, `NaN` to `0`. A segment that could not be
    /// represented at construction stays silent whatever the gain.
    pub fn set_gain(&mut self, gain: f32) {
        if self.representable {
            self.gain = math::clamp_or_low(gain, 0.0, MAX_DUPLEX_GAIN);
        }
    }

    /// `bridge` plus the duplex segment's answer to it.
    #[inline]
    pub fn process(&mut self, bridge: f32) -> f32 {
        if self.gain <= 0.0 {
            return bridge;
        }
        bridge + self.gain * self.resonator.process(bridge)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp)]

    use super::*;
    use alloc::vec::Vec;
    use proptest::prelude::*;

    const RATE: f32 = 48_000.0;

    const OCTAVE: DuplexTuning = DuplexTuning {
        harmonic: 2.0,
        inharmonicity: 0.0,
    };

    fn sine(frequency: f32, samples: usize) -> Vec<f32> {
        (0..samples)
            .map(|n| math::sin(core::f32::consts::TAU * frequency * n as f32 / RATE))
            .collect()
    }

    fn added_energy(duplex: &mut DuplexResonance, input: &[f32]) -> f32 {
        input
            .iter()
            .map(|&x| {
                let added = duplex.process(x) - x;
                added * added
            })
            .sum()
    }

    #[test]
    fn the_segment_answers_its_own_partial_far_more_than_a_neighbour() {
        let mut on = DuplexResonance::new(1_000.0, OCTAVE, 0.1, RATE);
        let mut off = DuplexResonance::new(1_000.0, OCTAVE, 0.1, RATE);
        let tuned = added_energy(&mut on, &sine(2_001.7, 9_600));
        let detuned = added_energy(&mut off, &sine(2_200.0, 9_600));
        assert!(tuned > 100.0 * detuned, "{tuned} {detuned}");
    }

    #[test]
    fn a_stiff_string_moves_the_segment_up_to_its_sharp_partial() {
        let stiff = DuplexTuning {
            harmonic: 4.0,
            inharmonicity: 1e-3,
        };
        let frequency = stiff.frequency_hz(1_000.0);
        assert!((4_030.0..4_040.0).contains(&frequency), "{frequency}");
    }

    #[test]
    fn a_muted_or_unrepresentable_segment_passes_the_signal_through() {
        let input = sine(500.0, 256);
        let mut muted = DuplexResonance::new(1_000.0, OCTAVE, 0.0, RATE);
        let mut too_high = DuplexResonance::new(12_000.0, OCTAVE, 0.5, RATE);
        too_high.set_gain(1.0);
        assert!(input.iter().all(|&x| muted.process(x) == x));
        assert!(input.iter().all(|&x| too_high.process(x) == x));
    }

    proptest! {
        #[test]
        fn any_input_stays_finite(
            fundamental in proptest::num::f32::ANY,
            harmonic in proptest::num::f32::ANY,
            inharmonicity in proptest::num::f32::ANY,
            gain in proptest::num::f32::ANY,
            rate in proptest::num::f32::ANY,
            inputs in proptest::collection::vec(-4.0f32..4.0, 0..256),
        ) {
            let tuning = DuplexTuning { harmonic, inharmonicity };
            let mut duplex = DuplexResonance::new(fundamental, tuning, gain, rate);
            for input in inputs {
                prop_assert!(duplex.process(input).is_finite());
            }
        }
    }
}
