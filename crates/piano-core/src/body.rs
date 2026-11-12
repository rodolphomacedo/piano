//! The soundboard as a diffuse field: what the piano's body does to a note
//! above the range where its resonances can be told apart.
//!
//! A string does not radiate. It shakes the bridge, and the soundboard — a
//! ribbed spruce plate more than a metre across — turns that into sound.
//! Up to a few hundred hertz the board answers with separate resonances,
//! which [`crate::soundboard`] models one at a time. Above that its modes
//! overlap: bending waves leave the bridge, reflect off the rim and the
//! ribs, and within a few milliseconds the plate is full of them. A force
//! at the bridge is therefore not heard as the sharp-edged wave the string
//! delivered but smeared over tens of milliseconds, and that smear is the
//! difference between a piano's attack and a plucked string's. Measured on
//! a concert grand, the band around 1 kHz takes about 12 ms to rise to
//! full level; the bridge force alone takes under 2 ms
//! (`docs/CALIBRATION.md`).
//!
//! [`BoardBody`] is the feedback delay network B. Bank proposed for this
//! job ("Physics-based sound synthesis of the piano", Helsinki University
//! of Technology, Laboratory of Acoustics and Audio Signal Processing,
//! Report 54, 2000; B. Bank, F. Avanzini, G. Borin, G. De Poli, F. Fontana
//! and D. Rocchesso, "Physically informed signal processing methods for
//! piano sound synthesis: a research overview", EURASIP Journal on Applied
//! Signal Processing 2003:10): the network of [`crate::fdn`], with lines a
//! few milliseconds long — the time a bending wave takes to cross the board
//! and come back — where a room's are tens of milliseconds. Two things
//! about it are physical rather than chosen by ear:
//!
//! - **How many resonances it has.** A delay network has as many modes per
//!   hertz as its lines hold seconds of delay (D. Rocchesso and J. O.
//!   Smith, "Circulant and elliptic feedback delay networks for artificial
//!   reverberation", IEEE Transactions on Speech and Audio Processing 5(1),
//!   1997). Each row of [`LINE_MILLISECONDS`] adds up to 43.7 ms, a mode
//!   every 23 Hz: the few modes per hundred hertz that modal analyses of
//!   piano soundboards count below 1 kHz (K. Ege, X. Boutillon and
//!   M. Rébillat, "Vibroacoustics of the piano soundboard: (non)linearity
//!   and modal properties in the low- and mid-frequency ranges", Journal of
//!   Sound and Vibration 332, 2013).
//! - **How fast it dies.** Ege et al. found the board's modal damping
//!   close to that of its wood. A mode at `f` hertz with loss factor `η`
//!   falls 60 dB in `2.2/(η·f)` seconds, and the absorption filters, fitted
//!   to the reference's attack, stay near that curve for a loss factor of
//!   one to two per cent, spruce's: about 100 ms at 1 kHz, 40 ms at 4 kHz,
//!   26 ms at 8 kHz on the middle lines, the shorter lines ringing a little
//!   longer and the longer ones a little less.
//!
//! What the network cannot give is the board's uneven loudness across the
//! spectrum; that is [`crate::radiation`]'s part. No impulse response is
//! measured, stored or convolved anywhere: the field is synthesised, like
//! everything else in this crate.

use core::f32::consts::{PI, TAU};

use crate::fdn::{self, AbsorbingLine};
use crate::math;
use crate::units::SampleRate;

/// Line lengths in milliseconds, one row per channel: the times bending
/// waves take to cross a grand's soundboard and return, with no small
/// common ratios so their echoes never line up into a pitch. The rows
/// differ by a few per cent so the two channels' fields share no resonance,
/// as two microphones over different parts of the board would not.
/// Their scale is fitted, together with the decay times below, to how the
/// reference's attack builds up octave by octave (`docs/CALIBRATION.md`).
const LINE_MILLISECONDS: [[f32; fdn::LINES]; 2] = [
    [2.47, 2.99, 4.03, 4.81, 5.59, 6.89, 7.93, 8.97],
    [2.60, 2.86, 4.16, 4.68, 5.85, 6.63, 8.19, 8.71],
];

/// Seconds the field takes to fall 60 dB at the bottom of the spectrum.
/// Short on purpose: down there the board's separate resonances, which ring
/// several times longer, are [`crate::soundboard`]'s.
const BASS_DECAY_SECONDS: f32 = 0.12;

/// Seconds the field takes to fall 60 dB at [`TREBLE_DECAY_HZ`].
const TREBLE_DECAY_SECONDS: f32 = 0.018;

/// Where [`TREBLE_DECAY_SECONDS`] is pinned. Above hearing on purpose: a
/// one-pole absorption filter pinned there and at DC passes through the
/// decay times the module docs give in between.
const TREBLE_DECAY_HZ: f32 = 24_000.0;

/// Sample rate the output level is referred to, so the body is as loud at
/// any other.
const REFERENCE_SAMPLE_RATE_HZ: f32 = 48_000.0;

/// How long the impulse response is run when its energy is measured: twice
/// the longest decay time, by when it is 120 dB down.
const CALIBRATION_SECONDS: f32 = 0.25;

/// Response energy below which the level is left alone rather than
/// divided by: only a field with no lines to speak of gets there.
const MIN_RESPONSE_ENERGY: f32 = 1.0e-12;

/// Sample rates the lines are sized for.
const MIN_SAMPLE_RATE_HZ: f32 = 1_000.0;
/// See [`MIN_SAMPLE_RATE_HZ`].
const MAX_SAMPLE_RATE_HZ: f32 = 1.0e6;

/// One channel's field: a network driven by the bridge force and read
/// through one sign pattern.
#[derive(Debug, Clone)]
struct DiffuseField {
    lines: [AbsorbingLine; fdn::LINES],
    taps: [f32; fdn::LINES],
    output_gain: f32,
}

impl DiffuseField {
    fn new(milliseconds: [f32; fdn::LINES], taps: [f32; fdn::LINES], rate: f32) -> Self {
        let mut field = Self {
            lines: milliseconds.map(|length| tuned_line(length, rate)),
            taps,
            output_gain: 1.0,
        };
        field.output_gain = field.unit_energy_gain(rate);
        field.clear();
        field
    }

    #[inline]
    fn process(&mut self, drive: f32) -> f32 {
        let drive = math::bounded_signal(drive);
        let outputs = self.lines.each_mut().map(AbsorbingLine::read_absorbed);
        let feedback = fdn::hadamard(outputs);
        for (line, returned) in self.lines.iter_mut().zip(feedback) {
            line.write(returned + drive);
        }
        self.output_gain * fdn::tap(&outputs, self.taps)
    }

    fn clear(&mut self) {
        for line in &mut self.lines {
            line.clear();
        }
    }

    /// The output gain that gives the impulse response unit energy at
    /// [`REFERENCE_SAMPLE_RATE_HZ`]: a broadband force then comes out as
    /// loud as it went in, whatever the lines and decay times are set to.
    /// The same response sampled faster has proportionally less energy per
    /// impulse, hence the correction for other rates.
    fn unit_energy_gain(&mut self, rate: f32) -> f32 {
        let frames = (CALIBRATION_SECONDS * rate) as usize;
        let mut energy = 0.0_f32;
        for frame in 0..frames {
            let heard = self.process(if frame == 0 { 1.0 } else { 0.0 });
            energy += heard * heard;
        }
        let referred = energy * rate / REFERENCE_SAMPLE_RATE_HZ;
        if referred > MIN_RESPONSE_ENERGY {
            1.0 / math::sqrt(referred)
        } else {
            1.0
        }
    }
}

/// One line of a field, `milliseconds` long at `rate`, absorbing at the
/// body's two decay times.
fn tuned_line(milliseconds: f32, rate: f32) -> AbsorbingLine {
    let samples = milliseconds * 1e-3 * rate;
    let mut line = AbsorbingLine::with_capacity(samples as usize + 2);
    let seconds = line.set_length(nearest_odd(samples)) / rate;
    let bass = fdn::decay_gain(seconds, BASS_DECAY_SECONDS);
    let treble = fdn::decay_gain(seconds, TREBLE_DECAY_SECONDS);
    let omega = (TAU * TREBLE_DECAY_HZ / rate).min(PI);
    line.set_absorption(bass, fdn::pole_for_gain_at(treble / bass, omega));
    line
}

/// The odd whole number nearest `samples`. Odd lengths share no factor of
/// two, which keeps the lines' echoes from coinciding every other pass.
fn nearest_odd(samples: f32) -> f32 {
    2.0 * math::round((samples - 1.0) / 2.0) + 1.0
}

/// The soundboard's diffuse field — see the module docs.
#[derive(Debug, Clone)]
pub struct BoardBody {
    left: DiffuseField,
    right: DiffuseField,
}

impl BoardBody {
    /// A silent body for a stream at `sample_rate`. Allocates its delay
    /// lines and runs its own impulse response once to set its level; must
    /// not be called while processing.
    #[must_use]
    pub fn new(sample_rate: SampleRate) -> Self {
        let rate = math::clamp_or_low(sample_rate.hertz(), MIN_SAMPLE_RATE_HZ, MAX_SAMPLE_RATE_HZ);
        let [left, right] = LINE_MILLISECONDS;
        Self {
            left: DiffuseField::new(left, fdn::LEFT_TAPS, rate),
            right: DiffuseField::new(right, fdn::RIGHT_TAPS, rate),
        }
    }

    /// The field's `(left, right)` answer to the bridge force reaching each
    /// channel. Each side has its own field, so a note keeps its place
    /// across the keyboard while the two sides stay uncorrelated.
    #[inline]
    pub fn process(&mut self, left: f32, right: f32) -> (f32, f32) {
        (self.left.process(left), self.right.process(right))
    }

    /// Silences the field.
    pub fn reset(&mut self) {
        self.left.clear();
        self.right.clear();
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use alloc::vec::Vec;

    use proptest::prelude::*;

    use super::*;

    const RATE_HZ: f32 = 48_000.0;

    fn body_at(rate_hz: f32) -> BoardBody {
        BoardBody::new(SampleRate::new(rate_hz).unwrap())
    }

    /// Both channels' answers to a unit impulse on both.
    fn impulse_response(rate_hz: f32, seconds: f32) -> (Vec<f32>, Vec<f32>) {
        let mut body = body_at(rate_hz);
        (0..(seconds * rate_hz) as usize)
            .map(|n| {
                let drive = if n == 0 { 1.0 } else { 0.0 };
                body.process(drive, drive)
            })
            .unzip()
    }

    fn energy(samples: &[f32]) -> f32 {
        samples.iter().map(|s| s * s).sum()
    }

    fn after(samples: &[f32], seconds: f32) -> &[f32] {
        samples.get((seconds * RATE_HZ) as usize..).unwrap_or(&[])
    }

    /// Energy of the sample-to-sample difference over the energy: near 0
    /// for a dark signal, towards 4 for one at the top of the spectrum.
    fn brightness(samples: &[f32]) -> f32 {
        let pairs = samples.iter().zip(samples.iter().skip(1));
        let steps: f32 = pairs.map(|(now, next)| (next - now).powi(2)).sum();
        steps / energy(samples).max(f32::MIN_POSITIVE)
    }

    #[test]
    fn nothing_is_heard_before_the_shortest_line_returns() {
        let (left, right) = impulse_response(RATE_HZ, 0.0018);
        assert!(left.iter().chain(&right).all(|sample| *sample == 0.0));
    }

    #[test]
    fn each_channel_answers_an_impulse_with_unit_energy() {
        let (left, right) = impulse_response(RATE_HZ, 0.25);
        assert!((energy(&left) - 1.0).abs() < 1e-3, "{}", energy(&left));
        assert!((energy(&right) - 1.0).abs() < 1e-3, "{}", energy(&right));
    }

    #[test]
    fn the_level_is_the_same_at_another_sample_rate() {
        for rate_hz in [44_100.0, 96_000.0] {
            let (left, _) = impulse_response(rate_hz, 0.25);
            let referred = energy(&left) * rate_hz / REFERENCE_SAMPLE_RATE_HZ;
            assert!((referred - 1.0).abs() < 1e-3, "{rate_hz}: {referred}");
        }
    }

    #[test]
    fn a_force_is_smeared_over_milliseconds_not_delivered_at_once() {
        let (left, _) = impulse_response(RATE_HZ, 0.25);
        let late = energy(after(&left, 0.005));
        assert!((0.2..0.9).contains(&late), "after 5 ms: {late}");
    }

    #[test]
    fn the_field_is_gone_within_a_fifth_of_a_second() {
        let (left, _) = impulse_response(RATE_HZ, 0.25);
        let tail = energy(after(&left, 0.2));
        assert!(tail < 1e-8, "after 200 ms: {tail:e}");
    }

    #[test]
    fn the_treble_of_the_field_dies_before_its_bass() {
        let (left, _) = impulse_response(RATE_HZ, 0.1);
        let early = brightness(left.get(..480).unwrap_or(&[]));
        let late = brightness(after(&left, 0.04));
        assert!(late < 0.25 * early, "early {early}, late {late}");
    }

    #[test]
    fn the_two_channels_are_nearly_uncorrelated() {
        let (left, right) = impulse_response(RATE_HZ, 0.25);
        let cross: f32 = left.iter().zip(&right).map(|(l, r)| l * r).sum();
        let correlation = cross / (energy(&left) * energy(&right)).sqrt();
        assert!(correlation.abs() < 0.3, "{correlation}");
    }

    #[test]
    fn each_channel_hears_only_its_own_side() {
        let mut body = body_at(RATE_HZ);
        let heard: Vec<(f32, f32)> = (0..2_400)
            .map(|n| body.process(if n == 0 { 1.0 } else { 0.0 }, 0.0))
            .collect();
        assert!(heard.iter().any(|(left, _)| *left != 0.0));
        assert!(heard.iter().all(|(_, right)| *right == 0.0));
    }

    #[test]
    fn reset_silences_the_field() {
        let mut body = body_at(RATE_HZ);
        for _ in 0..512 {
            body.process(1.0, -1.0);
        }
        body.reset();
        let silent = (0..512).all(|_| body.process(0.0, 0.0) == (0.0, 0.0));
        assert!(silent);
    }

    #[test]
    fn odd_lengths_are_the_nearest_ones() {
        assert!((nearest_odd(91.2) - 91.0).abs() < f32::EPSILON);
        assert!((nearest_odd(110.4) - 111.0).abs() < f32::EPSILON);
        assert!((nearest_odd(0.2) - 1.0).abs() < f32::EPSILON);
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(32))]

        #[test]
        fn any_rate_and_input_stay_finite(
            rate_hz in 1.0_f32..300_000.0,
            inputs in proptest::collection::vec(proptest::num::f32::ANY, 0..256),
        ) {
            let mut body = body_at(rate_hz);
            for input in inputs {
                let (left, right) = body.process(input, -input);
                prop_assert!(left.is_finite() && right.is_finite());
            }
        }
    }
}
