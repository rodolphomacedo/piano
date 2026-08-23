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

/// Damping [`StringConfig::new`] uses when the caller does not choose one.
pub const DEFAULT_DAMPING: f32 = 0.5;

/// Sustain [`StringConfig::new`] uses when the caller does not choose one.
pub const DEFAULT_SUSTAIN: f32 = 0.996;

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
            damping: DEFAULT_DAMPING,
            sustain: DEFAULT_SUSTAIN,
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
    /// Samples per cycle at this string's fixed frequency. `frequency`
    /// itself is not live-adjustable — that would need the delay line to
    /// grow — but `period` is kept so [`PluckedString::set_damping`] can
    /// retune `loop_delay` after the loss filter's phase delay changes,
    /// without needing the caller to pass the frequency back in.
    period: f32,
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
            period,
            loop_delay,
            sustain: math::clamp_or_low(config.sustain, 0.0, 1.0),
            envelope: 0.0,
        })
    }

    /// Adjusts the high-frequency loss for every future round trip.
    ///
    /// `damping` is clamped into `[0, 1]`, same as [`StringConfig::damping`].
    /// Because the loss filter's phase delay is part of what tunes the loop
    /// (see the module docs), changing it also retunes `loop_delay` to keep
    /// pitch accurate — capped at the minimum representable loop delay
    /// rather than going negative, in the unreachable-in-practice case
    /// where the new damping would need more phase delay than the string's
    /// period has to spare.
    /// The delay line's capacity was sized for the *original* damping at
    /// construction and never shrinks, so this never reads out of bounds.
    pub fn set_damping(&mut self, damping: f32) {
        self.loop_filter
            .set_pole(math::clamp_or_low(damping, 0.0, 1.0));
        let retuned = self.period - self.loop_filter.phase_delay_at_dc() - 1.0;
        self.loop_delay = if retuned < MIN_LOOP_DELAY {
            MIN_LOOP_DELAY
        } else {
            retuned
        };
    }

    /// Adjusts the broadband loop gain for every future round trip.
    ///
    /// `sustain` is clamped into `[0, 1]`, same as [`StringConfig::sustain`].
    /// Unlike damping, sustain does not affect tuning, so this never touches
    /// `loop_delay`.
    pub fn set_sustain(&mut self, sustain: f32) {
        self.sustain = math::clamp_or_low(sustain, 0.0, 1.0);
    }

    /// Reseeds the excitation noise used by the *next* [`PluckedString::pluck`].
    /// A string already ringing is unaffected, since its noise burst was
    /// already drawn.
    pub fn set_seed(&mut self, seed: u32) {
        self.rng = Xorshift32::new(seed);
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

    use proptest::prelude::*;

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
    fn set_damping_keeps_the_total_loop_length_anchored_to_the_period() {
        // set_damping's whole point: loop_delay plus the loss filter's own
        // phase delay plus the feedback path's one-sample delay must sum
        // back to the period, so the fundamental frequency does not drift
        // when damping changes — only loop_delay's *share* of that total
        // moves.
        let rate = SampleRate::new(48_000.0).expect("48 kHz is valid");
        let frequency = Hz::new(440.0).expect("440 Hz is valid");
        let period = 48_000.0 / 440.0;
        // 0.9 is a bright-to-dull swing any real voicing knob would use.
        // 0.999 is deliberately excluded: at that pole the filter's own
        // phase delay (999 samples) exceeds the whole period of a 440 Hz
        // string, so loop_delay floors at MIN_LOOP_DELAY instead of
        // preserving pitch — a real, documented degradation at extreme
        // settings, covered separately by
        // `live_damping_changes_never_break_the_string` (which asserts the
        // floor and finiteness hold for every damping, including that one).
        for damping in [0.0, 0.3, 0.6, 0.9] {
            let mut string =
                PluckedString::new(StringConfig::new(frequency), rate).expect("440 Hz is tunable");
            string.set_damping(damping);
            let total_delay = string.loop_delay() + string.loop_filter.phase_delay_at_dc() + 1.0;
            assert!(
                (total_delay - period).abs() < 1e-3,
                "damping {damping}: total delay {total_delay} drifted from period {period}"
            );
        }
    }

    #[test]
    fn set_damping_never_produces_a_non_finite_or_unbounded_signal() {
        let mut string = string_at(440.0);
        string.pluck(1.0);
        string.set_damping(1.0);
        for index in 0..48_000 {
            let sample = string.process();
            assert!(sample.is_finite(), "sample {index} was not finite");
            assert!(sample.abs() < 4.0, "sample {index} = {sample} escaped");
        }
    }

    #[test]
    fn set_damping_is_audible_immediately_on_an_already_ringing_string() {
        let mut string = string_at(220.0);
        string.pluck(1.0);
        for _ in 0..2_000 {
            string.process();
        }
        let before = (0..200).map(|_| string.process()).collect::<Vec<_>>();

        let mut identical_twin = string_at(220.0);
        identical_twin.pluck(1.0);
        for _ in 0..2_000 {
            identical_twin.process();
        }
        identical_twin.set_damping(0.99);
        let after = (0..200)
            .map(|_| identical_twin.process())
            .collect::<Vec<_>>();

        assert_ne!(before, after, "changing damping had no audible effect");
    }

    #[test]
    fn set_sustain_does_not_change_loop_delay() {
        let mut string = string_at(440.0);
        let before = string.loop_delay();
        string.set_sustain(0.5);
        assert_eq!(string.loop_delay(), before);
    }

    #[test]
    fn set_seed_changes_the_next_pluck_without_affecting_the_current_one() {
        let mut left = string_at(440.0);
        let mut right = string_at(440.0);
        left.pluck(0.8);
        right.pluck(0.8);
        for _ in 0..64 {
            assert_eq!(left.process(), right.process());
        }
        right.set_seed(0xDEAD_BEEF);
        left.pluck(0.8);
        right.pluck(0.8);
        let mut differed = false;
        for _ in 0..64 {
            if left.process() != right.process() {
                differed = true;
            }
        }
        assert!(differed, "reseeding did not change the next pluck");
    }

    proptest! {
        /// Whatever damping is requested, live retuning never produces a
        /// loop shorter than the minimum representable delay, and the
        /// string never blows up.
        #[test]
        fn live_damping_changes_never_break_the_string(damping in proptest::num::f32::ANY) {
            let mut string = string_at(220.0);
            string.pluck(1.0);
            string.set_damping(damping);
            prop_assert!(string.loop_delay() >= MIN_LOOP_DELAY);
            for _ in 0..1_000 {
                let sample = string.process();
                prop_assert!(sample.is_finite());
            }
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
