//! The room a piano is heard in: a stereo feedback delay network.
//!
//! A piano is never heard dry. Every recording a listener compares this
//! instrument with was made in a hall or studio, and most of what makes a
//! close-miked piano sound "expensive" is the room answering it: a few
//! milliseconds of silence, then a dense, decorrelated tail that dies
//! faster in the treble than in the bass, because air and walls absorb
//! high frequencies first.
//!
//! [`Room`] is the feedback delay network of J.-M. Jot and A. Chaigne,
//! "Digital delay networks for designing artificial reverberators" (AES
//! 90th Convention, 1991): [`ROOM_LINES`] delay lines of mutually unrelated
//! length, fed back through an orthonormal Hadamard matrix (lossless, so
//! all decay comes from the absorption filters), each line followed by a
//! one-pole absorption filter whose gain at DC and at Nyquist is solved
//! from the two target reverberation times, so every line loses energy at
//! the same rate per second regardless of its length. Left and right read
//! the lines through two orthogonal sign patterns, so the two channels are
//! uncorrelated — what makes a room sound wide. No recording, no impulse
//! response: a synthetic room, like everything else here.

use crate::delay::DelayLine;
use crate::math;

/// Delay lines in the network. Eight is the usual minimum for a tail
/// dense enough not to flutter (Jot & Chaigne 1991).
pub const ROOM_LINES: usize = 8;

/// Line lengths in milliseconds, chosen with no small common ratios so
/// their echoes never line up into a pitched flutter. 30-75 ms is a
/// medium room's spacing of first reflections.
const LINE_MILLISECONDS: [f32; ROOM_LINES] = [29.7, 37.1, 41.3, 43.9, 53.1, 59.3, 67.7, 73.1];

/// Silence before the room answers, in milliseconds: the extra path length
/// of the first wall reflection over the direct sound.
const PREDELAY_MILLISECONDS: f32 = 12.0;

/// Reverberation time at low frequencies, in seconds — a small recital
/// hall or a large studio, the rooms concert pianos are usually recorded in.
const LOW_REVERB_SECONDS: f32 = 1.8;

/// Reverberation time at Nyquist, in seconds: air and soft surfaces
/// absorb the top octaves several times faster than the bass.
const HIGH_REVERB_SECONDS: f32 = 0.5;

/// Largest wet level [`Room::set_mix`] accepts.
pub const MAX_ROOM_MIX: f32 = 2.0;

/// The wet level a player hears by default through the live engine: the
/// room clearly present behind the instrument without washing it out.
pub const DEFAULT_ROOM_MIX: f32 = 0.25;

/// Largest delay any line needs, in seconds, used to size them.
const LONGEST_LINE_SECONDS: f32 = 0.1;

/// `1/sqrt(ROOM_LINES)`: the normalisation that makes the Hadamard matrix
/// orthonormal, hence lossless.
const HADAMARD_SCALE: f32 = 0.353_553_4;

/// Sign patterns the two outputs read the lines through. Orthogonal (their
/// dot product is zero), so left and right are uncorrelated.
const LEFT_TAPS: [f32; ROOM_LINES] = [1.0, -1.0, 1.0, -1.0, 1.0, -1.0, 1.0, -1.0];
const RIGHT_TAPS: [f32; ROOM_LINES] = [1.0, 1.0, -1.0, -1.0, 1.0, 1.0, -1.0, -1.0];

/// One delay line and the absorption filter that follows it.
#[derive(Debug, Clone)]
struct RoomLine {
    delay: DelayLine,
    length: usize,
    dc_gain: f32,
    pole: f32,
    state: f32,
}

impl RoomLine {
    fn new(milliseconds: f32, sample_rate: f32) -> Self {
        let length = math::clamp_or_low(math::round(milliseconds * 1e-3 * sample_rate), 1.0, 1e6);
        let seconds = length / sample_rate;
        let dc_gain = decay_gain(seconds, LOW_REVERB_SECONDS);
        let ratio = decay_gain(seconds, HIGH_REVERB_SECONDS) / dc_gain;
        Self {
            delay: DelayLine::with_capacity((LONGEST_LINE_SECONDS * sample_rate) as usize),
            length: length as usize,
            dc_gain,
            pole: (1.0 - ratio) / (1.0 + ratio),
            state: 0.0,
        }
    }

    /// The line's output this sample, after absorption.
    #[inline]
    fn read_absorbed(&mut self) -> f32 {
        let raw = self.delay.read(self.length.saturating_sub(1));
        let filtered = (1.0 - self.pole) * raw + self.pole * self.state;
        self.state = math::flush_denormal(filtered);
        self.dc_gain * self.state
    }
}

/// The gain that, applied once every `seconds`, decays 60 dB in
/// `reverb_seconds`.
fn decay_gain(seconds: f32, reverb_seconds: f32) -> f32 {
    math::powf(10.0, -3.0 * seconds / reverb_seconds)
}

/// The room — see the module docs.
#[derive(Debug, Clone)]
pub struct Room {
    lines: [RoomLine; ROOM_LINES],
    predelay: DelayLine,
    predelay_samples: usize,
    mix: f32,
}

impl Room {
    /// A silent room for a stream at `sample_rate_hz`, at wet level `0`.
    /// Allocates its delay lines; must not be called while processing.
    #[must_use]
    pub fn new(sample_rate_hz: f32) -> Self {
        let rate = math::clamp_or_low(sample_rate_hz, 1_000.0, 1.0e6);
        let predelay_samples = math::round(PREDELAY_MILLISECONDS * 1e-3 * rate) as usize;
        Self {
            lines: LINE_MILLISECONDS.map(|milliseconds| RoomLine::new(milliseconds, rate)),
            predelay: DelayLine::with_capacity(predelay_samples + 1),
            predelay_samples,
            mix: 0.0,
        }
    }

    /// Sets the wet level, clamped into `[0, MAX_ROOM_MIX]`, `NaN` to `0`.
    pub fn set_mix(&mut self, mix: f32) {
        self.mix = math::clamp_or_low(mix, 0.0, MAX_ROOM_MIX);
    }

    /// The room's `(left, right)` answer to `input`, already scaled by the
    /// wet level; `(0, 0)` at mix `0`, without advancing the network.
    #[inline]
    pub fn process(&mut self, input: f32) -> (f32, f32) {
        if self.mix <= 0.0 {
            return (0.0, 0.0);
        }
        self.predelay
            .write(math::clamp_or_low(input, -1.0e6, 1.0e6));
        let delayed = self.predelay.read(self.predelay_samples);
        let outputs = self.lines.each_mut().map(RoomLine::read_absorbed);
        let feedback = hadamard(outputs);
        for (line, returned) in self.lines.iter_mut().zip(feedback) {
            line.delay.write(returned + delayed);
        }
        let tap = |taps: [f32; ROOM_LINES]| -> f32 {
            outputs
                .iter()
                .zip(taps)
                .map(|(output, sign)| output * sign)
                .sum()
        };
        (
            self.mix * HADAMARD_SCALE * tap(LEFT_TAPS),
            self.mix * HADAMARD_SCALE * tap(RIGHT_TAPS),
        )
    }
}

/// The orthonormal 8-point Hadamard transform, as three butterfly stages.
#[inline]
fn hadamard(values: [f32; ROOM_LINES]) -> [f32; ROOM_LINES] {
    let transformed = butterfly(butterfly(butterfly(values, 1), 2), 4);
    transformed.map(|value| HADAMARD_SCALE * value)
}

/// One radix-2 stage: every pair `stride` apart becomes `(sum, difference)`.
#[inline]
fn butterfly(values: [f32; ROOM_LINES], stride: usize) -> [f32; ROOM_LINES] {
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

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec::Vec;
    use proptest::prelude::*;

    const RATE: f32 = 48_000.0;

    fn impulse_response(seconds: f32) -> (Vec<f32>, Vec<f32>) {
        let mut room = Room::new(RATE);
        room.set_mix(1.0);
        let count = (seconds * RATE) as usize;
        (0..count)
            .map(|n| room.process(if n == 0 { 1.0 } else { 0.0 }))
            .unzip()
    }

    fn energy(samples: &[f32]) -> f32 {
        samples.iter().map(|s| s * s).sum()
    }

    #[test]
    fn the_hadamard_matrix_preserves_energy() {
        let input = [0.3, -1.2, 0.5, 2.0, -0.7, 0.1, 0.9, -0.4];
        let output = hadamard(input);
        assert!((energy(&input) - energy(&output)).abs() < 1e-4);
    }

    #[test]
    fn the_room_stays_silent_through_the_predelay() {
        let (left, right) = impulse_response(0.011);
        assert!(left.iter().chain(&right).all(|&sample| sample == 0.0));
    }

    #[test]
    fn the_tail_decays_sixty_decibels_in_about_the_reverberation_time() {
        let (left, _) = impulse_response(2.5);
        let window = |start: f32| {
            let first = (start * RATE) as usize;
            energy(left.get(first..first + 4_800).unwrap_or(&[]))
        };
        let drop_db = 10.0 * (window(0.2) / window(1.2)).log10();
        let per_second = drop_db / 1.0;
        assert!((20.0..60.0).contains(&per_second), "{per_second:.1} dB/s");
    }

    #[test]
    fn left_and_right_are_nearly_uncorrelated() {
        let (left, right) = impulse_response(1.0);
        let cross: f32 = left.iter().zip(&right).map(|(l, r)| l * r).sum();
        let correlation = cross / (energy(&left) * energy(&right)).sqrt();
        assert!(correlation.abs() < 0.3, "{correlation}");
    }

    #[test]
    fn zero_mix_is_silent() {
        let mut room = Room::new(RATE);
        assert_eq!(room.process(1.0), (0.0, 0.0));
    }

    proptest! {
        #[test]
        fn any_input_stays_finite(
            inputs in proptest::collection::vec(proptest::num::f32::ANY, 0..256),
            rate in proptest::num::f32::ANY,
            mix in proptest::num::f32::ANY,
        ) {
            let mut room = Room::new(rate);
            room.set_mix(mix);
            for input in inputs {
                let (left, right) = room.process(input);
                prop_assert!(left.is_finite() && right.is_finite());
            }
        }
    }
}
