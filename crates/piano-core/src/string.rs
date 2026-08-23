//! A struck string, modelled as an extended digital waveguide.
//!
//! # The physics, in one paragraph
//!
//! A transverse wave travels along the string, reflects at both ends, and comes
//! back inverted and slightly quieter. One trip around the string takes
//! `sample_rate / frequency` samples — that is the delay line, read back with
//! an allpass-interpolated fractional tap (`PERF-004`) so the loop's tuning
//! does not drift with pitch. The reflection is neither perfect nor flat:
//! high partials lose energy faster than low ones (the loop filter), and
//! because the string is stiff rather than ideally flexible, its partials
//! sit progressively sharp of an exact harmonic series (the dispersion
//! cascade, `PERF-005`, implementing Fletcher's stiff-string formula). The
//! excitation is a nonlinear felt-hammer contact pulse (`PERF-007`) rather
//! than a flat noise burst, so how hard the key is struck changes the
//! *shape* of what excites the string, not just its level. That is the
//! whole of milestone M4.

use crate::{
    delay::DelayLine,
    dispersion::DispersionCascade,
    error::ParamError,
    filter::{DcBlocker, LoopFilter},
    hammer, math,
    noise::Xorshift32,
    units::{Hz, SampleRate},
};

/// Shortest usable loop delay, in samples.
///
/// Below two samples the delay line no longer represents a travelling wave and
/// the pitch is meaningless.
const MIN_LOOP_DELAY: f32 = 2.0;

/// How fast the envelope follower forgets, per sample, for a held note.
const ENVELOPE_DECAY: f32 = 0.999_5;

/// How fast the envelope follower forgets once the damper is engaged
/// ([`PluckedString::release`]).
///
/// [`ENVELOPE_DECAY`] alone would make [`PluckedString::is_silent`] — and
/// so the engine's energy-gated voice skipping, `PERF-006` — take about
/// 18 000 samples (~380 ms at 48 kHz) to notice a released note has died,
/// no matter how fast [`RELEASE_LOSS_MULTIPLIER`] makes the *signal*
/// itself decay: once the true signal magnitude collapses below the
/// follower's own forgetting curve, the follower can only fall as fast as
/// its own release-rate constant, not as fast as reality. A released
/// string needs its own, much faster, constant so `is_silent` reflects the
/// released signal's real (order-of-10-round-trip) decay instead of a
/// held note's much slower one.
const RELEASED_ENVELOPE_DECAY: f32 = 0.995;

/// Below this envelope level a voice is inaudible and can be reclaimed.
pub const SILENCE_THRESHOLD: f32 = 1e-4;

/// Extra broadband loop-gain loss applied on every round trip once
/// [`PluckedString::release`] has engaged the damper, on top of whatever
/// `sustain` is currently set to.
///
/// A felt damper does not merely stop new energy going in — pressed against
/// the string, it adds a large, sudden friction loss to every reflection,
/// which is why a released piano note reaches silence far sooner than the
/// seconds (or tens of seconds, in the bass) a held note rings for (Chaigne
/// & Askenfelt 1994 model piano string loss the same way: as a
/// per-round-trip energy loss coefficient, sharply increased once the
/// damper makes contact). Multiplying `sustain` rather than replacing it
/// with a fixed value guarantees a released string always decays *at least
/// as fast* as whatever it was doing before, for any starting `sustain` —
/// including one a live `set_sustain` call had already set unusually low,
/// where a fixed replacement value could accidentally slow the string down
/// instead. At the default `sustain` (0.996) this reaches
/// [`SILENCE_THRESHOLD`] from full amplitude in about 10 round trips —
/// roughly 20 ms at A4, longer in the bass simply because a low string's
/// round trip itself takes longer, the same register-dependent spread a
/// real damped piano note shows.
const RELEASE_LOSS_MULTIPLIER: f32 = 0.4;

/// Damping [`StringConfig::new`] uses when the caller does not choose one.
pub const DEFAULT_DAMPING: f32 = 0.5;

/// Sustain [`StringConfig::new`] uses when the caller does not choose one.
pub const DEFAULT_SUSTAIN: f32 = 0.996;

/// Tunable properties of a struck string.
#[derive(Debug, Clone, Copy)]
pub struct StringConfig {
    /// Fundamental frequency of the note.
    pub frequency: Hz,
    /// High-frequency loss per round trip, in `[0, 1]`. Higher is duller.
    pub damping: f32,
    /// Broadband loop gain in `[0, 1]`. Higher sustains longer.
    pub sustain: f32,
    /// Stiff-string inharmonicity coefficient `B` (Fletcher's
    /// `f_n ≈ n·f_1·sqrt(1 + B·n²)`), in
    /// `[0, dispersion::MAX_INHARMONICITY]`. Higher makes upper partials sit
    /// further above an exact harmonic series. See [`crate::dispersion`].
    pub inharmonicity: f32,
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
            inharmonicity: crate::dispersion::DEFAULT_INHARMONICITY,
            seed: 0x2545_F491,
        }
    }
}

/// A single struck string voice.
///
/// Allocates once, in [`PluckedString::new`]. Every other method is
/// allocation-free, lock-free and panic-free, and is therefore safe to call from
/// an audio callback.
#[derive(Debug, Clone)]
pub struct PluckedString {
    delay: DelayLine,
    loop_filter: LoopFilter,
    dispersion: DispersionCascade,
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
    /// Set by [`PluckedString::release`], cleared by the next
    /// [`PluckedString::pluck`]. Named for the physical damper rather than
    /// anything to do with [`PluckedString::set_sustain`] — that is a
    /// decay-*rate* voicing parameter a player never directly triggers;
    /// this is a hammer/damper *event*, on or off, the same shape as a
    /// piano key's own mechanism.
    damper_engaged: bool,
    envelope: f32,
    /// Kept only for [`hammer::simulate_contact`]'s integration step at the
    /// next [`PluckedString::pluck`]; the audio-thread `process` loop never
    /// reads it.
    sample_rate: f32,
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
        let loop_filter = LoopFilter::new(math::clamp_or_low(config.damping, 0.0, 1.0));
        let dispersion = DispersionCascade::new(config.frequency.hertz(), config.inharmonicity);

        // The loop is the delay line plus the loss filter plus the
        // dispersion cascade plus the one-sample delay of the feedback path
        // itself. Tuning must account for all four.
        let loop_delay =
            period - loop_filter.phase_delay_at_dc() - dispersion.phase_delay_at_dc() - 1.0;
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
            dispersion,
            dc_blocker: DcBlocker::default(),
            rng: Xorshift32::new(config.seed),
            period,
            loop_delay,
            sustain: math::clamp_or_low(config.sustain, 0.0, 1.0),
            // A freshly built string is idle: the felt damper rests on it,
            // same as any un-struck key on a real piano. `pluck` is what
            // lifts it (M4, unchanged); M6 adds `lift_damper` so a
            // sustain-pedal press can lift it too, without a strike, so the
            // string becomes receptive to sympathetic energy from the
            // shared bridge bus (`PERF-008`, `crate::bridge`). Before M6
            // this field started `false`, which cost nothing because
            // nothing could inject energy into an un-struck string; M6's
            // bridge coupling means that default now matters, since a
            // wrongly-lifted idle string would be able to pick up
            // sympathetic energy it never should while the pedal is up.
            damper_engaged: true,
            envelope: 0.0,
            sample_rate: sample_rate.hertz(),
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
        self.retune_loop_delay();
    }

    /// Adjusts the broadband loop gain for every future round trip.
    ///
    /// `sustain` is clamped into `[0, 1]`, same as [`StringConfig::sustain`].
    /// Unlike damping, sustain does not affect tuning, so this never touches
    /// `loop_delay`.
    pub fn set_sustain(&mut self, sustain: f32) {
        self.sustain = math::clamp_or_low(sustain, 0.0, 1.0);
    }

    /// Adjusts the stiff-string inharmonicity coefficient `B` for every
    /// future round trip. `inharmonicity` is clamped into
    /// `[0, dispersion::MAX_INHARMONICITY]`, same as
    /// [`StringConfig::inharmonicity`]. Because the dispersion cascade's own
    /// phase delay is part of what tunes the loop (see the module docs),
    /// changing it also retunes `loop_delay`, the same reasoning
    /// [`PluckedString::set_damping`] uses for the loss filter.
    pub fn set_inharmonicity(&mut self, inharmonicity: f32) {
        self.dispersion.set_inharmonicity(inharmonicity);
        self.retune_loop_delay();
    }

    /// Reseeds the excitation noise used by the *next* [`PluckedString::pluck`].
    /// A string already ringing is unaffected, since its noise burst was
    /// already drawn.
    pub fn set_seed(&mut self, seed: u32) {
        self.rng = Xorshift32::new(seed);
    }

    /// Excites the string with a hammer-shaped noise burst at the given
    /// velocity.
    ///
    /// `velocity` is clamped into `[0, 1]` and, via
    /// [`hammer::simulate_contact`], shapes the burst's envelope: a harder
    /// strike produces a shorter, more sharply-peaked pulse with more
    /// high-frequency content, not merely a louder one. See the `hammer`
    /// module docs for the physical model and its simplifications. Cost is
    /// proportional to the loop length — a few hundred writes even for the
    /// lowest note — plus the bounded, compile-time-capped hammer contact
    /// simulation: a bounded spike on the audio thread, not an unbounded
    /// one.
    pub fn pluck(&mut self, velocity: f32) {
        let velocity = math::clamp_or_low(velocity, 0.0, 1.0);
        self.delay.clear();
        self.loop_filter.reset();
        self.dispersion.reset();
        self.dc_blocker.reset();
        self.envelope = velocity;
        // A hammer strike lifts the damper off the string, same as a real
        // piano key: any release the previous ringing of this voice had
        // engaged no longer applies to the new note.
        self.damper_engaged = false;

        let (contact_force, contact_samples) = hammer::simulate_contact(velocity, self.sample_rate);
        let burst_length = self.loop_delay as usize + 1;
        for index in 0..burst_length {
            let shape = if index < contact_samples {
                contact_force.get(index).copied().unwrap_or(0.0)
            } else {
                0.0
            };
            let sample = self.rng.next_bipolar() * velocity * shape;
            self.delay.write(sample);
        }
    }

    /// Engages the damper: from this call on, every round trip loses extra
    /// energy on top of `sustain` (see [`RELEASE_LOSS_MULTIPLIER`]), so a
    /// released note reaches silence in a fraction of the time a held one
    /// would take — a key coming up, or the sustain pedal releasing a note
    /// it was holding.
    ///
    /// Total and idempotent: calling this on a string that has already been
    /// released, or one that was never plucked and is already silent, only
    /// ever tightens the loop gain further, never loosens it, so it can be
    /// called freely without checking the string's current state first.
    #[inline]
    pub fn release(&mut self) {
        self.damper_engaged = true;
    }

    /// Lifts the damper without a fresh strike.
    ///
    /// [`PluckedString::pluck`] already lifts the damper for a struck
    /// note; this is for the other way a real piano's damper leaves the
    /// string — the sustain pedal, which lifts every damper regardless of
    /// which keys are held. Unlike `pluck`, this does not clear the delay
    /// line, reset the envelope or draw a new excitation: an idle string
    /// whose damper the pedal lifts has not been struck, it has only
    /// become free to keep ringing (if already ringing) or to pick up
    /// energy from a coupled group's shared bridge signal (see
    /// `crate::unison`) — sympathetic resonance, `PERF-008`.
    /// Idempotent, same as [`PluckedString::release`].
    #[inline]
    pub fn lift_damper(&mut self) {
        self.damper_engaged = false;
    }

    /// Whether the felt damper currently rests on the string.
    ///
    /// `true` means the string cannot radiate or receive sympathetic
    /// energy right now — used by [`crate::unison::UnisonGroup::is_receptive`]
    /// so a caller can tell a genuinely-damped voice (safe to skip
    /// entirely, `PERF-006`) from a silent-but-undamped one (must still be
    /// processed so it can wake up from the bridge bus, `PERF-008`).
    #[inline]
    #[must_use]
    pub fn damper_engaged(&self) -> bool {
        self.damper_engaged
    }

    /// Produces one output sample and advances the string by one sample.
    ///
    /// Equivalent to [`PluckedString::read_bridge_tap`],
    /// [`PluckedString::disperse`] and [`PluckedString::write_mixed_feedback`]
    /// in sequence, with no coupling mixed in — the single-string path
    /// M1-M5 already used, unaffected by M6's split.
    #[inline]
    pub fn process(&mut self) -> f32 {
        let tap = self.read_bridge_tap();
        let dispersed = self.disperse(tap);
        self.write_mixed_feedback(tap, dispersed)
    }

    /// Reads this sample's bridge-end travelling-wave value, without yet
    /// filtering it or writing the loop's feedback.
    ///
    /// Split out of [`PluckedString::process`] so a group of strings that
    /// share a bridge (`crate::unison`, `crate::bridge`, M6) can read every
    /// string's contribution to this sample *before* any of them mixes or
    /// writes back — the shared bridge signal has to be complete before
    /// anyone reads from it.
    #[inline]
    #[must_use]
    pub fn read_bridge_tap(&mut self) -> f32 {
        self.delay.read_allpass(self.loop_delay)
    }

    /// Runs the loop filter and dispersion cascade on `tap`, returning this
    /// string's own filtered signal — what a coupled group mixes with other
    /// strings' before finally writing back
    /// ([`PluckedString::write_mixed_feedback`]).
    ///
    /// Stateful, like [`PluckedString::read_bridge_tap`]: call this at most
    /// once per sample, after that call, before
    /// [`PluckedString::write_mixed_feedback`].
    #[inline]
    #[must_use]
    pub fn disperse(&mut self, tap: f32) -> f32 {
        let filtered = self.loop_filter.process(tap);
        self.dispersion.process(filtered)
    }

    /// Completes the sample [`PluckedString::read_bridge_tap`] and
    /// [`PluckedString::disperse`] started: applies this string's own
    /// round-trip loss to `mixed` and writes it back, returning the
    /// DC-blocked output sample read from `tap`.
    ///
    /// `mixed` is normally exactly [`PluckedString::disperse`]'s own
    /// result (as [`PluckedString::process`] passes), optionally blended
    /// with other strings' dispersed signal first by a **convex**
    /// combination — `crate::unison::UnisonGroup` is what builds that
    /// blend, gated so a damped string (`damper_engaged`) is never blended
    /// with anything, only ever writing back its own signal, since a real
    /// felt damper blocks vibration regardless of where energy is trying
    /// to come from.
    ///
    /// Convex, not additive, is a load-bearing choice: an earlier version
    /// of this method took an *additive* `external_input` term scaled by
    /// `loop_gain` alongside the string's own signal. That is only stable
    /// while `loop_gain` is comfortably below 1 — a group of `N` coupled,
    /// *near-lossless* strings (`sustain` close to 1, the common case for
    /// a freshly-struck note) has a "common mode" (every string moving
    /// together) whose effective gain is `loop_gain·(1 + coupling·(N-1))`,
    /// which exceeds 1, and so grows without bound, for any positive
    /// additive coupling once `loop_gain` is close enough to 1 — caught by
    /// `unison::tests::output_stays_bounded_for_a_full_second_with_local_
    /// coupling` diverging during development, not by inspection. A convex
    /// combination cannot have this failure mode: its result is always
    /// between the smallest and largest of its inputs, for *any* mixing
    /// weight and *any* `loop_gain`, so `write_mixed_feedback`'s own
    /// `loop_gain < 1` contraction is the only thing responsible for decay,
    /// exactly as it is for an uncoupled string. This is the digital
    /// equivalent of a passive scattering junction, whose mixing never
    /// creates energy and whose losses are applied strictly after
    /// scattering (J. O. Smith III, *Physical Audio Signal Processing*,
    /// "Scattering at an Impedance Discontinuity").
    #[inline]
    pub fn write_mixed_feedback(&mut self, tap: f32, mixed: f32) -> f32 {
        let loop_gain = if self.damper_engaged {
            self.sustain * RELEASE_LOSS_MULTIPLIER
        } else {
            self.sustain
        };
        let reflected = mixed * loop_gain;
        self.delay.write(math::flush_denormal(reflected));

        let output = self.dc_blocker.process(tap);
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

    /// Recomputes `loop_delay` from `period` minus every filter's current
    /// phase delay at DC, floored at [`MIN_LOOP_DELAY`] rather than going
    /// negative. Shared by [`PluckedString::set_damping`] and
    /// [`PluckedString::set_inharmonicity`], the two live controls whose
    /// filters sit inside the tuned loop.
    fn retune_loop_delay(&mut self) {
        let retuned = self.period
            - self.loop_filter.phase_delay_at_dc()
            - self.dispersion.phase_delay_at_dc()
            - 1.0;
        self.loop_delay = if retuned < MIN_LOOP_DELAY {
            MIN_LOOP_DELAY
        } else {
            retuned
        };
    }

    #[inline]
    fn track_envelope(&mut self, output: f32) {
        let magnitude = math::abs(output);
        let decay_rate = if self.damper_engaged {
            RELEASED_ENVELOPE_DECAY
        } else {
            ENVELOPE_DECAY
        };
        let decayed = self.envelope * decay_rate;
        self.envelope = math::flush_denormal(if magnitude > decayed {
            magnitude
        } else {
            decayed
        });
    }
}

// Split into `string_tests.rs` to keep this file under the project's
// 500-line limit (`CONTRIBUTING.md`) — still compiles as `string::tests`.
#[cfg(test)]
#[path = "string_tests.rs"]
mod tests;
