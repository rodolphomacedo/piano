//! A plucked string, modelled as an extended Karplus–Strong loop.
//!
//! # The physics, in one paragraph
//!
//! A transverse wave travels along the string, reflects at both ends, and comes
//! back inverted and slightly quieter. One trip around the string takes
//! `sample_rate / frequency` samples — that is the delay line. The reflection is
//! neither perfect nor flat: high partials lose energy faster than low ones,
//! which is the lowpass filter in the feedback path. That is the entire model.
//! Its limits (no inharmonicity, no hammer, one string per note) are what
//! milestone M4 replaces with a full digital waveguide.

use crate::{
    delay::DelayLine,
    error::ParamError,
    filter::{DcBlocker, OnePoleLowpass},
    math,
    noise::Xorshift32,
    units::{Hz, SampleRate},
};

/// Shortest usable loop delay, in samples.
///
/// Below two samples the delay line no longer represents a travelling wave and
/// the pitch is meaningless.
const MIN_LOOP_DELAY: f32 = 2.0;

/// How fast the envelope follower forgets, per sample.
const ENVELOPE_DECAY: f32 = 0.999_5;

/// Below this envelope level a voice is inaudible and can be reclaimed.
const SILENCE_THRESHOLD: f32 = 1e-4;

/// Tunable properties of a plucked string.
#[derive(Debug, Clone, Copy)]
pub struct StringConfig {
    /// Fundamental frequency of the note.
    pub frequency: Hz,
    /// High-frequency loss per round trip, in `[0, 1]`. Higher is duller.
    pub damping: f32,
    /// Broadband loop gain in `[0, 1]`. Higher sustains longer.
    pub sustain: f32,
    /// Seed for the excitation noise, so renders are reproducible.
    pub seed: u32,
}

impl StringConfig {
    /// A reasonable default voicing for `frequency`.
    #[must_use]
    pub fn new(frequency: Hz) -> Self {
        Self {
            frequency,
            damping: 0.5,
            sustain: 0.996,
            seed: 0x2545_F491,
        }
    }
}

/// A single plucked string voice.
///
/// Allocates once, in [`PluckedString::new`]. Every other method is
/// allocation-free, lock-free and panic-free, and is therefore safe to call from
/// an audio callback.
#[derive(Debug, Clone)]
pub struct PluckedString {
    delay: DelayLine,
    loop_filter: OnePoleLowpass,
    dc_blocker: DcBlocker,
    rng: Xorshift32,
    loop_delay: f32,
    sustain: f32,
    envelope: f32,
}

impl PluckedString {
    /// Builds a string tuned to `config.frequency` at `sample_rate`.
    ///
    /// # Errors
    ///
    /// Returns [`ParamError::FrequencyOutOfRange`] when the requested
    /// fundamental is too high to be represented — the loop would need fewer
    /// than two samples of delay.
    pub fn new(config: StringConfig, sample_rate: SampleRate) -> Result<Self, ParamError> {
        let period = sample_rate.hertz() / config.frequency.hertz();
        let loop_filter = OnePoleLowpass::new(math::clamp_or_low(config.damping, 0.0, 1.0));

        // The loop is the delay line plus the loss filter plus the one-sample
        // delay of the feedback path itself. Tuning must account for all three.
        let loop_delay = period - loop_filter.phase_delay_at_dc() - 1.0;
        if loop_delay < MIN_LOOP_DELAY {
            return Err(ParamError::FrequencyOutOfRange {
                frequency: config.frequency.hertz(),
                sample_rate: sample_rate.hertz(),
                maximum: sample_rate.hertz() / (MIN_LOOP_DELAY + 1.0),
            });
        }

        Ok(Self {
            delay: DelayLine::with_capacity(period as usize + 4),
            loop_filter,
            dc_blocker: DcBlocker::default(),
            rng: Xorshift32::new(config.seed),
            loop_delay,
            sustain: math::clamp_or_low(config.sustain, 0.0, 1.0),
            envelope: 0.0,
        })
    }

    /// Excites the string with a noise burst of the given velocity.
    ///
    /// `velocity` is clamped into `[0, 1]`. Cost is proportional to the loop
    /// length — a few hundred writes even for the lowest note — which is a
    /// bounded spike on the audio thread, not an unbounded one.
    pub fn pluck(&mut self, velocity: f32) {
        let velocity = math::clamp_or_low(velocity, 0.0, 1.0);
        self.delay.clear();
        self.loop_filter.reset();
        self.dc_blocker.reset();
        self.envelope = velocity;

        let burst_length = self.loop_delay as usize + 1;
        for _ in 0..burst_length {
            let sample = self.rng.next_bipolar() * velocity;
            self.delay.write(sample);
        }
    }

    /// Produces one output sample and advances the string by one sample.
    #[inline]
    pub fn process(&mut self) -> f32 {
        let travelled = self.delay.read_interpolated(self.loop_delay);
        let reflected = self.loop_filter.process(travelled) * self.sustain;
        self.delay.write(math::flush_denormal(reflected));

        let output = self.dc_blocker.process(travelled);
        self.track_envelope(output);
        output
    }

    /// Renders `output.len()` samples and **adds** them into `output`.
    ///
    /// Additive so that a polyphonic engine can mix voices into a shared buffer
    /// without a per-voice scratch buffer.
    pub fn process_block_add(&mut self, output: &mut [f32]) {
        for slot in output {
            *slot += self.process();
        }
    }

    /// A cheap estimate of the current output level, used for voice reclaiming.
    #[inline]
    #[must_use]
    pub fn envelope(&self) -> f32 {
        self.envelope
    }

    /// Whether the string has decayed below audibility.
    #[inline]
    #[must_use]
    pub fn is_silent(&self) -> bool {
        self.envelope < SILENCE_THRESHOLD
    }

    /// The tuned loop length in samples, for tests and diagnostics.
    #[inline]
    #[must_use]
    pub fn loop_delay(&self) -> f32 {
        self.loop_delay
    }

    #[inline]
    fn track_envelope(&mut self, output: f32) {
        let magnitude = math::abs(output);
        let decayed = self.envelope * ENVELOPE_DECAY;
        self.envelope = math::flush_denormal(if magnitude > decayed {
            magnitude
        } else {
            decayed
        });
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp, clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn string_at(frequency: f32) -> PluckedString {
        let rate = SampleRate::new(48_000.0).expect("48 kHz is valid");
        let config = StringConfig::new(Hz::new(frequency).expect("frequency is valid"));
        PluckedString::new(config, rate).expect("frequency is representable at 48 kHz")
    }

    #[test]
    fn rejects_frequencies_above_the_representable_range() {
        let rate = SampleRate::new(48_000.0).expect("48 kHz is valid");
        let config = StringConfig::new(Hz::new(30_000.0).expect("frequency is valid"));
        assert!(matches!(
            PluckedString::new(config, rate),
            Err(ParamError::FrequencyOutOfRange { .. })
        ));
    }

    #[test]
    fn loop_delay_is_close_to_the_period() {
        let string = string_at(440.0);
        let period = 48_000.0 / 440.0;
        assert!(
            (string.loop_delay() - period).abs() < 3.0,
            "delay {}",
            string.loop_delay()
        );
    }

    #[test]
    fn is_silent_before_being_plucked() {
        let mut string = string_at(220.0);
        for _ in 0..1_000 {
            assert_eq!(string.process(), 0.0);
        }
        assert!(string.is_silent());
    }

    #[test]
    fn plucking_produces_signal() {
        let mut string = string_at(220.0);
        string.pluck(1.0);
        let peak = (0..4_800)
            .map(|_| math::abs(string.process()))
            .fold(0.0f32, f32::max);
        assert!(peak > 0.05, "peak {peak} is inaudible");
    }

    #[test]
    fn output_stays_bounded_for_a_full_second() {
        let mut string = string_at(27.5);
        string.pluck(1.0);
        for index in 0..48_000 {
            let sample = string.process();
            assert!(sample.is_finite(), "sample {index} was not finite");
            assert!(sample.abs() < 4.0, "sample {index} = {sample} escaped");
        }
    }

    #[test]
    fn energy_decays_after_the_attack() {
        let mut string = string_at(440.0);
        string.pluck(1.0);
        for _ in 0..4_800 {
            string.process();
        }
        let early = string.envelope();
        for _ in 0..48_000 {
            string.process();
        }
        assert!(string.envelope() < early, "envelope grew from {early}");
    }

    #[test]
    fn a_plucked_string_eventually_goes_quiet() {
        let mut string = string_at(440.0);
        string.pluck(1.0);
        for _ in 0..48_000 * 30 {
            string.process();
        }
        assert!(string.is_silent(), "envelope {}", string.envelope());
    }

    #[test]
    fn the_same_seed_renders_the_same_note() {
        let mut left = string_at(440.0);
        let mut right = string_at(440.0);
        left.pluck(0.8);
        right.pluck(0.8);
        for _ in 0..2_400 {
            assert_eq!(left.process(), right.process());
        }
    }

    #[test]
    fn block_processing_adds_into_the_buffer() {
        let mut string = string_at(440.0);
        string.pluck(1.0);
        let mut buffer = [1.0f32; 64];
        string.process_block_add(&mut buffer);
        assert!(buffer.iter().any(|sample| (sample - 1.0).abs() > 1e-6));
    }
}
