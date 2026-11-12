//! The parts a feedback delay network is made of, shared by the room
//! ([`crate::room`]) and the soundboard's diffuse field ([`crate::body`]).
//!
//! J.-M. Jot and A. Chaigne, "Digital delay networks for designing
//! artificial reverberators" (AES 90th Convention, 1991): delay lines of
//! mutually unrelated length, fed back through an orthonormal matrix so the
//! loop itself loses nothing, each line followed by an absorption filter
//! that takes the same decibels per second from every line whatever its
//! length. All decay then comes from the absorption, and can be set in
//! seconds.

use crate::delay::DelayLine;
use crate::math;

/// Delay lines in a network. Eight is the usual minimum for a tail dense
/// enough not to flutter (Jot & Chaigne 1991).
pub(crate) const LINES: usize = 8;

/// `1/sqrt(LINES)`: the normalisation that makes the Hadamard matrix
/// orthonormal, hence lossless.
pub(crate) const HADAMARD_SCALE: f32 = 0.353_553_4;

/// Sign patterns the two outputs read the lines through. Orthogonal (their
/// dot product is zero), so left and right are uncorrelated.
pub(crate) const LEFT_TAPS: [f32; LINES] = [1.0, -1.0, 1.0, -1.0, 1.0, -1.0, 1.0, -1.0];
/// See [`LEFT_TAPS`].
pub(crate) const RIGHT_TAPS: [f32; LINES] = [1.0, 1.0, -1.0, -1.0, 1.0, 1.0, -1.0, -1.0];

/// Steepest absorption pole accepted, short of the unit circle.
const MAX_ABSORPTION_POLE: f32 = 0.999;

/// One delay line and the one-pole absorption filter that follows it.
#[derive(Debug, Clone)]
pub(crate) struct AbsorbingLine {
    delay: DelayLine,
    length: usize,
    dc_gain: f32,
    pole: f32,
    state: f32,
}

impl AbsorbingLine {
    /// A silent, fully absorbing line able to hold `samples`. The only
    /// allocating call here.
    pub(crate) fn with_capacity(samples: usize) -> Self {
        Self {
            delay: DelayLine::with_capacity(samples),
            length: 1,
            dc_gain: 0.0,
            pole: 0.0,
            state: 0.0,
        }
    }

    /// Sets the line `samples` long, clamped into what it was allocated
    /// for, and returns the length it took. Never allocates.
    pub(crate) fn set_length(&mut self, samples: f32) -> f32 {
        let longest = (self.delay.max_delay() as f32).max(1.0);
        let length = math::clamp_or_low(math::round(samples), 1.0, longest);
        self.length = length as usize;
        length
    }

    /// Sets what one pass through the line keeps: `dc_gain` at DC, tilted
    /// above it by a one-pole filter at `pole` — a lowpass for a positive
    /// pole and, for a negative one, a rise towards Nyquist, which is how a
    /// network is given a treble that outlasts its bass.
    ///
    /// A line that amplified at any frequency would make its network
    /// unstable, so `dc_gain` is held to what keeps the gain at Nyquist at
    /// or below one.
    pub(crate) fn set_absorption(&mut self, dc_gain: f32, pole: f32) {
        let pole = if pole.is_nan() {
            0.0
        } else {
            pole.clamp(-MAX_ABSORPTION_POLE, MAX_ABSORPTION_POLE)
        };
        let nyquist_tilt = (1.0 - pole) / (1.0 + pole);
        self.dc_gain = math::clamp_or_low(dc_gain, 0.0, (1.0 / nyquist_tilt).min(1.0));
        self.pole = pole;
    }

    /// The line's output this sample, after absorption.
    #[inline]
    pub(crate) fn read_absorbed(&mut self) -> f32 {
        let raw = self.delay.read(self.length.saturating_sub(1));
        let filtered = (1.0 - self.pole) * raw + self.pole * self.state;
        self.state = math::flush_denormal(filtered);
        self.dc_gain * self.state
    }

    #[inline]
    pub(crate) fn write(&mut self, sample: f32) {
        self.delay.write(math::flush_denormal(sample));
    }

    /// Empties the line and its filter.
    pub(crate) fn clear(&mut self) {
        self.delay.clear();
        self.state = 0.0;
    }
}

/// The gain that, applied once every `seconds`, decays 60 dB in
/// `reverb_seconds`.
pub(crate) fn decay_gain(seconds: f32, reverb_seconds: f32) -> f32 {
    math::powf(10.0, -3.0 * seconds / reverb_seconds)
}

/// The pole of the one-pole lowpass `(1 − p)/(1 − p·z⁻¹)` whose gain at
/// `omega` radians per sample is `ratio` — how an absorption filter is made
/// to hit a second decay time at a chosen frequency.
///
/// From `(1 − p)² = ratio²·(1 − 2p·cos ω + p²)`, the root inside the unit
/// circle. A `ratio` of one or more asks for no extra loss and gets `0`;
/// anything unusable (`NaN`, a ratio at or below zero) gets the steepest
/// pole accepted.
pub(crate) fn pole_for_gain_at(ratio: f32, omega: f32) -> f32 {
    if ratio >= 1.0 {
        return 0.0;
    }
    if ratio.is_nan() || ratio <= 0.0 {
        return MAX_ABSORPTION_POLE;
    }
    let squared = ratio * ratio;
    let centre = 1.0 - squared * math::cos(omega);
    let spread = 1.0 - squared;
    let root = math::sqrt((centre * centre - spread * spread).max(0.0));
    math::clamp_or_low((centre - root) / spread, 0.0, MAX_ABSORPTION_POLE)
}

/// The orthonormal 8-point Hadamard transform, as three butterfly stages.
#[inline]
pub(crate) fn hadamard(values: [f32; LINES]) -> [f32; LINES] {
    let transformed = butterfly(butterfly(butterfly(values, 1), 2), 4);
    transformed.map(|value| HADAMARD_SCALE * value)
}

/// One radix-2 stage: every pair `stride` apart becomes `(sum, difference)`.
#[inline]
fn butterfly(values: [f32; LINES], stride: usize) -> [f32; LINES] {
    core::array::from_fn(|index| {
        let own = values.get(index).copied().unwrap_or(0.0);
        let partner = values.get(index ^ stride).copied().unwrap_or(0.0);
        if index & stride == 0 {
            own + partner
        } else {
            partner - own
        }
    })
}

/// The lines' outputs read through one sign pattern.
#[inline]
pub(crate) fn tap(outputs: &[f32; LINES], signs: [f32; LINES]) -> f32 {
    outputs
        .iter()
        .zip(signs)
        .map(|(output, sign)| output * sign)
        .sum()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp)]

    use proptest::prelude::*;

    use super::*;

    fn energy(samples: &[f32]) -> f32 {
        samples.iter().map(|s| s * s).sum()
    }

    /// Gain of `(1 − p)/(1 − p·z⁻¹)` at `omega`.
    fn one_pole_gain(pole: f32, omega: f32) -> f32 {
        (1.0 - pole) / (1.0 - 2.0 * pole * omega.cos() + pole * pole).sqrt()
    }

    #[test]
    fn the_hadamard_matrix_preserves_energy() {
        let input = [0.3, -1.2, 0.5, 2.0, -0.7, 0.1, 0.9, -0.4];
        let output = hadamard(input);
        assert!((energy(&input) - energy(&output)).abs() < 1e-4);
    }

    #[test]
    fn the_two_tap_patterns_are_orthogonal() {
        let dot: f32 = LEFT_TAPS.iter().zip(RIGHT_TAPS).map(|(l, r)| l * r).sum();
        assert_eq!(dot, 0.0);
    }

    #[test]
    fn a_pole_solved_for_a_gain_gives_that_gain() {
        for (ratio, omega) in [
            (0.5, 1.0),
            (0.1, 0.3),
            (0.9, 3.0),
            (0.25, core::f32::consts::PI),
        ] {
            let pole = pole_for_gain_at(ratio, omega);
            let gain = one_pole_gain(pole, omega);
            assert!((gain - ratio).abs() < 1e-3, "{ratio} at {omega}: {gain}");
        }
    }

    #[test]
    fn asking_for_no_loss_gives_no_filter() {
        assert_eq!(pole_for_gain_at(1.0, 1.0), 0.0);
        assert_eq!(pole_for_gain_at(2.0, 1.0), 0.0);
    }

    #[test]
    fn a_line_returns_what_was_written_its_length_ago() {
        let mut line = AbsorbingLine::with_capacity(64);
        assert_eq!(line.set_length(10.0), 10.0);
        line.set_absorption(1.0, 0.0);
        line.write(1.0);
        let heard: f32 = (0..9)
            .map(|_| {
                let out = line.read_absorbed();
                line.write(0.0);
                out
            })
            .sum();
        assert_eq!(heard, 0.0);
        assert_eq!(line.read_absorbed(), 1.0);
    }

    #[test]
    fn a_length_beyond_the_allocation_is_clamped() {
        let mut line = AbsorbingLine::with_capacity(16);
        assert!(line.set_length(1.0e9) <= 16.0);
        assert_eq!(line.set_length(f32::NAN), 1.0);
    }

    #[test]
    fn a_line_tilted_towards_the_treble_still_never_amplifies() {
        let mut line = AbsorbingLine::with_capacity(8);
        line.set_length(1.0);
        line.set_absorption(1.0, -0.5);
        let loudest = (0..200)
            .map(|n| {
                let out = line.read_absorbed();
                line.write(if n % 2 == 0 { 1.0 } else { -1.0 });
                out.abs()
            })
            .fold(0.0_f32, f32::max);
        assert!(loudest <= 1.0 + 1e-4, "{loudest}");
    }

    proptest! {
        #[test]
        fn any_gain_and_frequency_give_a_stable_pole(
            ratio in proptest::num::f32::ANY,
            omega in proptest::num::f32::ANY,
        ) {
            let pole = pole_for_gain_at(ratio, omega);
            prop_assert!((0.0..=MAX_ABSORPTION_POLE).contains(&pole));
        }

        #[test]
        fn a_line_stays_finite_for_any_setting_and_input(
            length in proptest::num::f32::ANY,
            dc_gain in proptest::num::f32::ANY,
            pole in proptest::num::f32::ANY,
            inputs in proptest::collection::vec(proptest::num::f32::ANY, 0..128),
        ) {
            let mut line = AbsorbingLine::with_capacity(32);
            line.set_length(length);
            line.set_absorption(dc_gain, pole);
            for input in inputs {
                let out = line.read_absorbed();
                prop_assert!(out.is_finite());
                line.write(math::bounded_signal(input));
            }
        }
    }
}
