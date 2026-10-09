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

/// Silence before the room answers, in milliseconds, by default: the extra
/// path length of the first wall reflection over the direct sound.
pub const DEFAULT_ROOM_PREDELAY_MILLISECONDS: f32 = 12.0;

/// Longest predelay [`Room::set_predelay_milliseconds`] accepts: 100 ms is
/// a first wall some 17 m further away than the piano, a large hall.
pub const MAX_ROOM_PREDELAY_MILLISECONDS: f32 = 100.0;

/// Reverberation time at low frequencies, in seconds, by default — a small
/// recital hall or a large studio, the rooms concert pianos are usually
/// recorded in.
pub const DEFAULT_ROOM_REVERB_SECONDS: f32 = 1.8;

/// Reverberation time at Nyquist, in seconds, by default: air and soft
/// surfaces absorb the top octaves several times faster than the bass.
pub const DEFAULT_ROOM_TREBLE_REVERB_SECONDS: f32 = 0.5;

/// Shortest reverberation time either band accepts: a nearly dead booth.
pub const MIN_ROOM_REVERB_SECONDS: f32 = 0.1;

/// Longest reverberation time either band accepts: a cathedral.
pub const MAX_ROOM_REVERB_SECONDS: f32 = 8.0;

/// Default room size: the line lengths of [`LINE_MILLISECONDS`] as written.
pub const DEFAULT_ROOM_SIZE: f32 = 1.0;

/// Smallest room size: half the spacing of first reflections, a living room.
pub const MIN_ROOM_SIZE: f32 = 0.5;

/// Largest room size: twice the spacing of first reflections, a concert
/// hall. The lines are allocated for this, so resizing never allocates.
pub const MAX_ROOM_SIZE: f32 = 2.0;

/// Largest wet level [`Room::set_mix`] accepts.
pub const MAX_ROOM_MIX: f32 = 2.0;

/// The wet level a player hears by default through the live engine: the
/// room clearly present behind the instrument without washing it out.
pub const DEFAULT_ROOM_MIX: f32 = 0.25;

/// Longest delay any line needs at [`MAX_ROOM_SIZE`], in seconds, used to
/// size them.
const LONGEST_LINE_SECONDS: f32 = 0.16;

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
    fn new(sample_rate: f32) -> Self {
        Self {
            delay: DelayLine::with_capacity((LONGEST_LINE_SECONDS * sample_rate) as usize),
            length: 1,
            dc_gain: 0.0,
            pole: 0.0,
            state: 0.0,
        }
    }

    /// Sets the line to `milliseconds` long and its absorption to the
    /// shape's two reverberation times. Never allocates.
    fn retune(&mut self, milliseconds: f32, sample_rate: f32, shape: &RoomShape) {
        let longest = (self.delay.max_delay() as f32).max(1.0);
        let length =
            math::clamp_or_low(math::round(milliseconds * 1e-3 * sample_rate), 1.0, longest);
        let seconds = length / sample_rate;
        self.dc_gain = decay_gain(seconds, shape.reverb_seconds);
        let ratio = decay_gain(seconds, shape.treble_reverb_seconds) / self.dc_gain;
        self.pole = (1.0 - ratio) / (1.0 + ratio);
        self.length = length as usize;
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

/// The room's acoustic shape: everything about it except how loud it is.
#[derive(Debug, Clone, Copy, PartialEq)]
struct RoomShape {
    size: f32,
    reverb_seconds: f32,
    treble_reverb_seconds: f32,
}

/// The room — see the module docs.
#[derive(Debug, Clone)]
pub struct Room {
    lines: [RoomLine; ROOM_LINES],
    predelay: DelayLine,
    predelay_samples: usize,
    sample_rate: f32,
    shape: RoomShape,
    mix: f32,
}

impl Room {
    /// A silent room for a stream at `sample_rate_hz`, at wet level `0` and
    /// the default shape. Allocates its delay lines for the largest shape
    /// and the longest predelay; must not be called while processing.
    #[must_use]
    pub fn new(sample_rate_hz: f32) -> Self {
        let rate = math::clamp_or_low(sample_rate_hz, 1_000.0, 1.0e6);
        let longest_predelay = MAX_ROOM_PREDELAY_MILLISECONDS * 1e-3 * rate;
        let mut room = Self {
            lines: core::array::from_fn(|_| RoomLine::new(rate)),
            predelay: DelayLine::with_capacity(longest_predelay as usize + 1),
            predelay_samples: 0,
            sample_rate: rate,
            shape: RoomShape {
                size: DEFAULT_ROOM_SIZE,
                reverb_seconds: DEFAULT_ROOM_REVERB_SECONDS,
                treble_reverb_seconds: DEFAULT_ROOM_TREBLE_REVERB_SECONDS,
            },
            mix: 0.0,
        };
        room.set_predelay_milliseconds(DEFAULT_ROOM_PREDELAY_MILLISECONDS);
        room.retune_lines();
        room
    }

    /// Sets the wet level, clamped into `[0, MAX_ROOM_MIX]`, `NaN` to `0`.
    pub fn set_mix(&mut self, mix: f32) {
        self.mix = math::clamp_or_low(mix, 0.0, MAX_ROOM_MIX);
    }

    /// Scales every line's length, clamped into `[MIN_ROOM_SIZE,
    /// MAX_ROOM_SIZE]`, `NaN` to the minimum. Never allocates.
    pub fn set_size(&mut self, size: f32) {
        self.shape.size = math::clamp_or_low(size, MIN_ROOM_SIZE, MAX_ROOM_SIZE);
        self.retune_lines();
    }

    /// Sets the low-frequency reverberation time, clamped into
    /// `[MIN_ROOM_REVERB_SECONDS, MAX_ROOM_REVERB_SECONDS]`, `NaN` to the
    /// minimum.
    pub fn set_reverb_seconds(&mut self, seconds: f32) {
        self.shape.reverb_seconds = clamp_reverb_seconds(seconds);
        self.retune_lines();
    }

    /// Sets the reverberation time at Nyquist, clamped like
    /// [`Room::set_reverb_seconds`].
    pub fn set_treble_reverb_seconds(&mut self, seconds: f32) {
        self.shape.treble_reverb_seconds = clamp_reverb_seconds(seconds);
        self.retune_lines();
    }

    /// Sets the silence before the room answers, clamped into `[0,
    /// MAX_ROOM_PREDELAY_MILLISECONDS]`, `NaN` to `0`.
    pub fn set_predelay_milliseconds(&mut self, milliseconds: f32) {
        let clamped = math::clamp_or_low(milliseconds, 0.0, MAX_ROOM_PREDELAY_MILLISECONDS);
        let samples = math::round(clamped * 1e-3 * self.sample_rate) as usize;
        self.predelay_samples = samples.min(self.predelay.max_delay());
    }

    fn retune_lines(&mut self) {
        let shape = self.shape;
        for (line, milliseconds) in self.lines.iter_mut().zip(LINE_MILLISECONDS) {
            line.retune(milliseconds * shape.size, self.sample_rate, &shape);
        }
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

fn clamp_reverb_seconds(seconds: f32) -> f32 {
    math::clamp_or_low(seconds, MIN_ROOM_REVERB_SECONDS, MAX_ROOM_REVERB_SECONDS)
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
        shaped_impulse_response(seconds, |_| {})
    }

    fn shaped_impulse_response(seconds: f32, shape: impl Fn(&mut Room)) -> (Vec<f32>, Vec<f32>) {
        let mut room = Room::new(RATE);
        room.set_mix(1.0);
        shape(&mut room);
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

    fn tail_energy_after(seconds: f32, shape: impl Fn(&mut Room)) -> f32 {
        let (left, _) = shaped_impulse_response(seconds + 0.1, shape);
        energy(left.get((seconds * RATE) as usize..).unwrap_or(&[]))
    }

    #[test]
    fn a_longer_reverberation_time_leaves_a_louder_late_tail() {
        let dry = tail_energy_after(1.5, |room| room.set_reverb_seconds(0.8));
        let wet = tail_energy_after(1.5, |room| room.set_reverb_seconds(4.0));
        assert!(wet > dry * 100.0, "short {dry:e}, long {wet:e}");
    }

    #[test]
    fn a_shorter_treble_time_darkens_the_tail() {
        let bright = tail_energy_after(0.8, |room| room.set_treble_reverb_seconds(1.8));
        let dark = tail_energy_after(0.8, |room| room.set_treble_reverb_seconds(0.2));
        assert!(bright > dark, "bright {bright:e}, dark {dark:e}");
    }

    #[test]
    fn the_predelay_setting_moves_the_first_answer() {
        let (left, right) =
            shaped_impulse_response(0.05, |room| room.set_predelay_milliseconds(40.0));
        let first = left
            .iter()
            .zip(&right)
            .position(|(l, r)| *l != 0.0 || *r != 0.0)
            .unwrap_or(usize::MAX);
        assert!(
            first >= (0.040 * RATE) as usize,
            "first answer at sample {first}"
        );
    }

    #[test]
    fn a_larger_room_answers_later() {
        let first_answer = |size: f32| {
            let (left, _) = shaped_impulse_response(0.4, |room| room.set_size(size));
            left.iter().position(|s| *s != 0.0).unwrap_or(usize::MAX)
        };
        assert!(first_answer(MAX_ROOM_SIZE) > first_answer(MIN_ROOM_SIZE));
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
            shape in proptest::array::uniform4(proptest::num::f32::ANY),
        ) {
            let mut room = Room::new(rate);
            room.set_mix(mix);
            room.set_size(shape[0]);
            room.set_reverb_seconds(shape[1]);
            room.set_treble_reverb_seconds(shape[2]);
            room.set_predelay_milliseconds(shape[3]);
            for input in inputs {
                let (left, right) = room.process(input);
                prop_assert!(left.is_finite() && right.is_finite());
            }
        }
    }
}
