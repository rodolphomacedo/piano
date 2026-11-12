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
//! string is set in motion by a felt hammer whose contact with it is solved
//! every sample ([`crate::hammer`], `PERF-007`): nothing is written into the
//! delay line in advance, so how hard the key is struck changes the *shape*
//! of what excites the string, not just its level, and a string struck
//! again is struck while still moving.
//!
//! What the string reports is the force it exerts on the bridge.

use crate::{
    delay::DelayLine,
    dispersion::DispersionCascade,
    error::ParamError,
    filter::{DcBlocker, LoopFilter},
    hammer, math,
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

/// The default for [`PluckedString::set_damper_strength`]: the fraction of
/// a wave's amplitude a fully seated damper takes per round trip,
/// `1 − RELEASE_LOSS_MULTIPLIER`.
pub const DEFAULT_DAMPER_STRENGTH: f32 = 1.0 - RELEASE_LOSS_MULTIPLIER;

/// Weakest damper [`PluckedString::set_damper_strength`] accepts. At zero a
/// released note would never stop, the stuck-damper fault a technician
/// fixes, so it is not a voicing choice.
pub const MIN_DAMPER_STRENGTH: f32 = 0.05;

/// Strongest damper accepted. Taking the whole wave in one round trip
/// would cut a note off within a period, a click rather than a damper.
pub const MAX_DAMPER_STRENGTH: f32 = 0.95;

/// How a damper's pressure becomes loss: the per-round-trip loss in nepers
/// is `−ln(RELEASE_LOSS_MULTIPLIER)·pressureⁿ`. A fully seated damper stops
/// a string in a few dozen round trips, so a loss merely proportional to
/// pressure would still kill a half-damped A3 inside 50 ms; felt that only
/// grazes the string touches it over a sliver of its width and damps a
/// small fraction as hard. With `n = 5` half pressure lets A3 ring for
/// about a second and a quarter-pressure damper costs it little — the
/// graded sustain half-pedalling is for (issue #61).
const DAMPER_CONTACT_EXPONENT: f32 = 5.0;

/// How far below its construction-time frequency [`PluckedString::set_frequency`]
/// can retune without reallocating.
///
/// [`PluckedString::new`] reserves delay-line capacity for this much downward
/// detune (a lower frequency means a longer period means more delay-line
/// capacity needed); [`PluckedString::retune_loop_delay`] then caps
/// `loop_delay` at whatever that reservation actually holds, so a request
/// beyond this range degrades pitch accuracy rather than reading outside the
/// delay line. 100 cents is a full semitone either way — far past
/// `crate::unison`'s widest unison spread (±4 cents) — so headroom for the
/// live parameter studio's per-string detune control never actually binds in
/// practice; it exists so that is provable rather than assumed.
pub const MAX_LIVE_DETUNE_CENTS: f32 = 100.0;

/// Damping [`StringConfig::new`] uses when the caller does not choose one.
pub const DEFAULT_DAMPING: f32 = 0.5;

/// Sustain [`StringConfig::new`] uses when the caller does not choose one.
pub const DEFAULT_SUSTAIN: f32 = 0.996;

/// Where along the string the hammer lands, as a fraction of its length
/// from the agraffe, at [`StringConfig::new`].
///
/// ~1/8 is the canonical piano strike ratio: it leaves the 8th partial, the
/// dissonant one builders place the strike point to suppress, with a node
/// under the hammer (Fletcher & Rossing, *The Physics of Musical
/// Instruments*, 2nd ed., §12.3). `piano_audio::voicing` sets a per-key value
/// that drifts across the keyboard; a caller building a [`StringConfig`]
/// directly gets this classic single value.
pub const DEFAULT_STRIKE_POSITION: f32 = 0.125;

/// Widest [`StringConfig::strike_position`] the string honours — a strike at
/// the midpoint, which excites no even partial. Beyond it the geometry only
/// mirrors.
pub const MAX_STRIKE_POSITION: f32 = 0.5;

/// Loop-filter zero mix [`StringConfig::new`] uses when the caller does not
/// choose one — the loop filter's original, pre-per-register fixed value
/// (`filter::LoopFilter`'s own `MAX_ZERO_MIX`, full Nyquist-null rolloff).
/// `piano_audio::voicing::solve_loop_losses` is what chooses a per-key
/// value instead, from that key's own per-partial decay targets; a caller
/// building a [`StringConfig`] directly (as
/// `piano-render`'s tests do, deliberately bypassing that per-register
/// voicing) keeps today's original filter character unchanged.
pub const DEFAULT_LOOP_ZERO_MIX: f32 = 0.5;

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
    /// This string's own felt-contact physics. See [`hammer::HammerConfig`];
    /// [`StringConfig::new`] picks the register's own hammer.
    pub hammer: hammer::HammerConfig,
    /// The loop filter's zero mix, in `[0, filter::MAX_ZERO_MIX]`. Higher
    /// gives upper partials extra rolloff beyond what `damping` alone
    /// provides, at the cost of amplitude at the fundamental itself — see
    /// [`crate::filter::LoopFilter`]'s docs for why this had to become a
    /// per-string value rather than a fixed constant.
    pub loop_zero_mix: f32,
    /// Where the hammer strikes, as a fraction of the string's length
    /// measured from the agraffe, in `[0, MAX_STRIKE_POSITION]`. See
    /// [`DEFAULT_STRIKE_POSITION`].
    pub strike_position: f32,
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
            hammer: hammer::hammer_for_frequency(frequency.hertz()),
            loop_zero_mix: DEFAULT_LOOP_ZERO_MIX,
            strike_position: DEFAULT_STRIKE_POSITION,
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
    /// The hammer, at rest except during a strike.
    junction: hammer::HammerJunction,
    /// This string's hammer and impedance, prepared for the sample rate.
    contact: hammer::ContactSolver,
    /// How far behind the write head the wave returning from the agraffe
    /// sits — where in the loop the hammer acts. See
    /// [`PluckedString::place_strike_tap`].
    strike_tap: usize,
    /// How far past `strike_tap` towards the next tap the hammer sits,
    /// `0..1`.
    strike_blend: f32,
    /// What the hammer added at `strike_tap` one sample ago, which now
    /// sits under the next tap and is not part of the wave arriving there.
    strike_carry: f32,
    /// Samples per cycle at this string's fixed frequency. `frequency`
    /// itself is only live-adjustable within [`MAX_LIVE_DETUNE_CENTS`] of the
    /// frequency `PluckedString::new` reserved delay-line headroom for (see
    /// [`PluckedString::set_frequency`]) — but `period` is kept so
    /// [`PluckedString::set_damping`] can retune `loop_delay` after the loss
    /// filter's phase delay changes, without needing the caller to pass the
    /// frequency back in.
    period: f32,
    loop_delay: f32,
    sustain: f32,
    /// How hard the felt damper presses on the string, in `[0, 1]`: `1` is
    /// fully resting ([`PluckedString::release`]), `0` fully lifted (a held
    /// key, or [`PluckedString::lift_damper`]), anything between a damper
    /// grazing the string under a half-pressed pedal
    /// ([`PluckedString::set_damper_pressure`]). Named for the physical
    /// damper rather than anything to do with [`PluckedString::set_sustain`]
    /// — that is a decay-*rate* voicing parameter a player never triggers.
    damper_pressure: f32,
    /// The extra per-round-trip gain [`PluckedString::damper_pressure`]
    /// costs: `seated_damper_gain^pressure`, kept so the per-sample
    /// loop multiplies instead of calling `powf`.
    damper_gain: f32,
    /// The per-round-trip gain of a fully seated damper,
    /// `1 − damper strength` ([`PluckedString::set_damper_strength`]).
    seated_damper_gain: f32,
    envelope: f32,
    sample_rate: f32,
    /// Where the hammer strikes, as a fraction of the string's length,
    /// clamped into `[0, MAX_STRIKE_POSITION]`.
    strike_position: f32,
}

/// How many fixed-point steps [`tuned_loop_delay`] takes. The delay line's
/// phase delay moves almost one-for-one with its length, so each step cuts
/// the residual by orders of magnitude; three leave it far below a
/// hundredth of a cent at every key.
const TUNING_ITERATIONS: usize = 3;

/// The delay-line length that makes the whole loop — delay line with its
/// fractional allpass, loss filter, dispersion cascade, and the feedback
/// path's one sample — exactly `period` samples long *at the fundamental*.
///
/// Tuning by each element's delay at DC instead put the mid and treble up
/// to 22 cents sharp (issue #96): every element in the loop delays the
/// fundamental less than it delays DC.
fn tuned_loop_delay(period: f32, loop_filter: &LoopFilter, dispersion: &DispersionCascade) -> f32 {
    let omega = core::f32::consts::TAU / period;
    let wanted =
        period - loop_filter.phase_delay_at(omega) - dispersion.phase_delay_at(omega) - 1.0;
    let mut length = wanted;
    for _ in 0..TUNING_ITERATIONS {
        length += wanted - DelayLine::allpass_read_phase_delay(length, omega);
    }
    length
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
        let loop_filter = LoopFilter::new(
            math::clamp_or_low(config.damping, 0.0, 1.0),
            config.loop_zero_mix,
        );
        let dispersion = DispersionCascade::new(period, config.inharmonicity);

        // The loop is the delay line plus the loss filter plus the
        // dispersion cascade plus the one-sample delay of the feedback path
        // itself. Tuning must account for all four — at the fundamental.
        // Checked on the plain period first: below it the fundamental sits
        // above Nyquist and no phase delay evaluated there means anything.
        let representable = period > MIN_LOOP_DELAY + 1.0
            && tuned_loop_delay(period, &loop_filter, &dispersion) >= MIN_LOOP_DELAY;
        if !representable {
            return Err(ParamError::FrequencyOutOfRange {
                frequency: config.frequency.hertz(),
                sample_rate: sample_rate.hertz(),
                maximum: sample_rate.hertz() / (MIN_LOOP_DELAY + 1.0),
            });
        }

        // Sized for the lowest frequency a live `set_frequency` call is
        // allowed to reach, not just the construction-time frequency —
        // otherwise a downward live retune would need to grow the delay
        // line, which the audio thread cannot do.
        let lowest_live_frequency =
            config.frequency.hertz() * math::powf(2.0, -MAX_LIVE_DETUNE_CENTS / 1200.0);
        let max_live_period = sample_rate.hertz() / lowest_live_frequency;

        let mut string = Self {
            delay: DelayLine::with_capacity(max_live_period as usize + 4),
            loop_filter,
            dispersion,
            dc_blocker: DcBlocker::default(),
            junction: hammer::HammerJunction::resting(),
            contact: hammer::ContactSolver::new(config.hammer, sample_rate.hertz()),
            strike_tap: 0,
            strike_blend: 0.0,
            strike_carry: 0.0,
            period,
            loop_delay: 0.0,
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
            damper_pressure: 1.0,
            damper_gain: RELEASE_LOSS_MULTIPLIER,
            seated_damper_gain: RELEASE_LOSS_MULTIPLIER,
            envelope: 0.0,
            sample_rate: sample_rate.hertz(),
            strike_position: math::clamp_or_low(config.strike_position, 0.0, MAX_STRIKE_POSITION),
        };
        string.retune_loop_delay();
        Ok(string)
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

    /// Adjusts the loss filter's averaging zero for every future round trip
    /// — how much of the loss falls on the highest partials rather than
    /// across the band. Clamped like [`StringConfig::loop_zero_mix`], and,
    /// like [`PluckedString::set_damping`], retunes `loop_delay`, since the
    /// zero's phase delay is part of the loop.
    pub fn set_loop_zero_mix(&mut self, zero_mix: f32) {
        self.loop_filter.set_zero_mix(zero_mix);
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

    /// The string's current round-trip broadband loss factor, as last set
    /// by [`StringConfig::sustain`] or [`PluckedString::set_sustain`].
    #[inline]
    #[must_use]
    pub fn sustain(&self) -> f32 {
        self.sustain
    }

    /// Retunes the string to `frequency`, without reallocating the delay
    /// line.
    ///
    /// `frequency` is used exactly as given — [`Hz`] already guarantees a
    /// finite, positive value, so unlike `damping`/`sustain` this has nothing
    /// of its own to clamp. What *is* bounded is how far the resulting
    /// `loop_delay` can move: [`PluckedString::new`] only reserved delay-line
    /// headroom for [`MAX_LIVE_DETUNE_CENTS`] of downward detune, and
    /// [`PluckedString::retune_loop_delay`]'s clamp caps `loop_delay` at
    /// whatever that reservation actually holds, so a request beyond that
    /// range degrades pitch accuracy rather than reading outside the delay
    /// line.
    pub fn set_frequency(&mut self, frequency: Hz) {
        self.period = self.sample_rate / frequency.hertz();
        self.retune_loop_delay();
    }

    /// Changes the hammer this string is struck with, and the impedance it
    /// meets. A strike already in progress finishes with the new values.
    pub fn set_hammer(&mut self, hammer: hammer::HammerConfig) {
        self.contact = hammer::ContactSolver::new(hammer, self.sample_rate);
    }

    /// Moves where the hammer strikes, as a fraction of the string's
    /// length. `strike_position` is clamped into `[0, MAX_STRIKE_POSITION]`,
    /// `NaN` mapping to `0`, the same convention every other live setter
    /// here uses. Partials with a node under the hammer — the
    /// `1/strike_position` harmonic and its multiples — are the ones a
    /// strike there cannot excite.
    pub fn set_strike_position(&mut self, strike_position: f32) {
        self.strike_position = math::clamp_or_low(strike_position, 0.0, MAX_STRIKE_POSITION);
        self.place_strike_tap();
    }

    /// Throws the hammer at the string.
    ///
    /// `velocity` is clamped into `[0, 1]` and sets the hammer's speed
    /// ([`hammer::strike_mps_for`]); everything else about the blow — how
    /// long the felt stays on the string, how bright the note is — follows
    /// from the contact being solved against the string sample by sample
    /// (see [`hammer`]). The string is not cleared first: a key struck
    /// again meets a string that is still moving, as on the instrument.
    /// Costs nothing here; the contact is paid for a sample at a time in
    /// [`PluckedString::write_mixed_feedback`].
    pub fn pluck(&mut self, velocity: f32) {
        let strike_mps = hammer::strike_mps_for(velocity);
        // A hammer strike lifts the damper off the string, same as a real
        // key press.
        self.set_damper_pressure(0.0);
        self.junction.strike(strike_mps, &self.contact);
        // Keeps the voice awake until the first wave reaches the bridge.
        self.envelope = self.envelope.max(strike_mps * self.contact.bridge_gain());
    }

    /// Stops the string dead: no travelling wave, no hammer, filters at
    /// rest. For "all notes off", where a release would still ring.
    pub fn silence(&mut self) {
        self.delay.clear();
        self.loop_filter.reset();
        self.dispersion.reset();
        self.dc_blocker.reset();
        self.junction = hammer::HammerJunction::resting();
        self.envelope = 0.0;
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
        self.set_damper_pressure(1.0);
    }

    /// Rests the damper on the string with `pressure` in `[0, 1]` — the
    /// continuous form of [`PluckedString::release`] (`1`) and
    /// [`PluckedString::lift_damper`] (`0`) a half-pressed sustain pedal
    /// needs (issue #61). See [`DAMPER_CONTACT_EXPONENT`] for how pressure
    /// becomes loss. `NaN` maps to `0`, the same convention every other
    /// live setter uses; idempotent.
    pub fn set_damper_pressure(&mut self, pressure: f32) {
        self.damper_pressure = math::clamp_or_low(pressure, 0.0, 1.0);
        let contact = math::powf(self.damper_pressure, DAMPER_CONTACT_EXPONENT);
        self.damper_gain = math::powf(self.seated_damper_gain, contact);
    }

    /// Sets how hard the felt grips the string: the fraction of a wave's
    /// amplitude a fully seated damper takes per round trip, clamped into
    /// `[MIN_DAMPER_STRENGTH, MAX_DAMPER_STRENGTH]`, `NaN` to the minimum.
    /// Applies at once to a damper already resting on the string.
    pub fn set_damper_strength(&mut self, strength: f32) {
        let strength = math::clamp_or_low(strength, MIN_DAMPER_STRENGTH, MAX_DAMPER_STRENGTH);
        self.seated_damper_gain = 1.0 - strength;
        self.set_damper_pressure(self.damper_pressure);
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
        self.set_damper_pressure(0.0);
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
        self.damper_pressure >= 1.0
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
        self.write_mixed_feedback(tap, dispersed, 0.0)
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
    /// round-trip loss to `mixed` plus whatever `coupling` a shared bridge
    /// is driving into it, and writes the result back, returning the
    /// DC-blocked output sample read from `tap`.
    ///
    /// `mixed` is normally exactly [`PluckedString::disperse`]'s own
    /// result (as [`PluckedString::process`] passes), optionally blended
    /// with this note's *own* unison strings' dispersed signal first by a
    /// **convex** combination — `crate::unison::UnisonGroup` is what builds
    /// that blend, gated so a damped string (`damper_engaged`) is never
    /// blended with anything, only ever writing back its own signal, since
    /// a real felt damper blocks vibration regardless of where energy is
    /// trying to come from.
    ///
    /// # Why `mixed` is convex but `coupling` is additive
    ///
    /// These are two different physical situations needing two different
    /// mixing laws. This project got it wrong in both directions before
    /// measuring its way to the split.
    ///
    /// **The convex half.** An early version took the cross-string term
    /// *additively*, scaled by `loop_gain`. That is only stable while
    /// `loop_gain` is comfortably below 1: a group of `N` coupled,
    /// near-lossless strings (`sustain` close to 1, the common case for a
    /// freshly-struck note) has a "common mode" — every string moving
    /// together — whose effective gain is `loop_gain·(1 + coupling·(N−1))`,
    /// which exceeds 1, and so grows without bound, for any positive
    /// additive coupling once `loop_gain` is close enough to 1.
    /// `unison::tests::output_stays_bounded_for_a_full_second_with_local_
    /// coupling` caught that diverging. A convex combination cannot have
    /// that failure mode: its result always lies between the smallest and
    /// largest of its inputs, for any weight and any `loop_gain`.
    ///
    /// **Why convex is nonetheless wrong for the shared bus.** A convex
    /// combination `own + w·(other − own)` gives the string's own signal a
    /// coefficient of `1 − w` instead of `1`. That is harmless exactly when
    /// `other ≈ own`, which is what makes it right for a note's own unison
    /// strings: same note, coupled with no latency, so the difference term
    /// is genuinely small and what survives of it *is* the beating this
    /// project wants. It is wrong for [`crate::bridge::BridgeBus`], whose
    /// readback is neither. It is a whole block old (`PERF-008`), so it is
    /// decorrelated from `own` at audio rates; and it is a *mean over every
    /// contributing voice*, so it shrinks towards `own/N` as polyphony
    /// rises. The string's own amplitude was therefore multiplied by about
    /// `1 − w·(1 − 1/N)` every round trip — an unmodelled,
    /// polyphony-dependent loss sitting in the one place `sustain` is meant
    /// to be the only authority. Measured at the default weight: a note
    /// sounded *alone* lost 11× its level by 0.5 s against the same note
    /// with the bus disabled, and a second key already decayed to
    /// inaudibility still cost a freshly-struck note a further 40×. A
    /// silent string resting on a bridge does not drain its neighbour, so
    /// that was never physics — it was the mixing law.
    ///
    /// **The additive half, and why it is stable.** `coupling` is added on
    /// top instead, leaving the string's own signal at coefficient `1`, so
    /// a bus driving nothing costs nothing — exactly the property that
    /// failed above. Stability comes from scaling it by `1 − loop_gain`
    /// rather than by `loop_gain`, which is what the earlier additive
    /// version did. That is not a fudge factor: `1 − loop_gain` *is* this
    /// string's round-trip energy loss, and Weinreich (1977) has
    /// cross-string coupling strength set by the bridge admittance, which
    /// is the very thing that loss represents — a string that barely gives
    /// energy to the bridge barely receives any back through it. It also
    /// makes the loop unconditionally bounded: the worst case is a bus
    /// perfectly correlated with the string itself, giving a loop gain of
    /// `loop_gain·(1 + 1 − loop_gain) = 1 − (1 − loop_gain)²`, which is
    /// `≤ 1` for every `loop_gain` in `[0, 1]` and every coupling weight in
    /// `[0, 1]` — checked by `string_tests::any_coupling_and_sustain_stays_
    /// bounded`, not argued.
    ///
    /// Both halves together are still the digital passive scattering
    /// junction this method has always modelled: mixing that never creates
    /// energy, with losses applied strictly after scattering (J. O. Smith
    /// III, *Physical Audio Signal Processing*, "Scattering at an Impedance
    /// Discontinuity").
    #[inline]
    pub fn write_mixed_feedback(&mut self, tap: f32, mixed: f32, coupling: f32) -> f32 {
        let loop_gain = self.sustain * self.damper_gain;
        let bridge_admittance = 1.0 - loop_gain;
        let driven = mixed + coupling * bridge_admittance;
        let returning = driven * loop_gain;
        let struck = self.hammer_injection(returning);
        self.delay.write(math::flush_denormal(returning - struck));

        let output = self.dc_blocker.process(tap * self.contact.bridge_gain());
        self.track_envelope(output);
        output
    }

    /// Lets the hammer, if one is on the string, act for this sample, and
    /// returns the velocity it adds to the wave leaving for the agraffe.
    ///
    /// The delay line holds one loop, bridge end at the read tap and the
    /// wave that just left the strike point at the write head. `returning`
    /// is the wave arriving back from the bridge; `strike_tap` samples
    /// behind the write head is the wave arriving back from the agraffe.
    /// Their difference is the string's velocity under the felt. The hammer
    /// then adds `F / 2Z` to both waves leaving the point: to the one bound
    /// for the bridge in place, and — with the sign its reflection at the
    /// agraffe will undo — to the one written now.
    #[inline]
    fn hammer_injection(&mut self, returning: f32) -> f32 {
        if !self.junction.is_striking() {
            self.strike_carry = 0.0;
            return 0.0;
        }
        let near = self.delay.read(self.strike_tap);
        let far = self.delay.read(self.strike_tap + 1) - self.strike_carry;
        let from_agraffe = near + self.strike_blend * (far - near);
        let struck = self.junction.step(&self.contact, from_agraffe - returning);
        self.strike_carry = (1.0 - self.strike_blend) * struck;
        self.delay.add_at(self.strike_tap, self.strike_carry);
        self.delay
            .add_at(self.strike_tap + 1, self.strike_blend * struck);
        struck
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

    /// Recomputes `loop_delay` from `period` minus every loop element's
    /// current phase delay at the fundamental ([`tuned_loop_delay`]), clamped into `[MIN_LOOP_DELAY, max_delay - 1]`
    /// rather than going negative or reading outside the delay line. Shared
    /// by [`PluckedString::set_damping`], [`PluckedString::set_inharmonicity`]
    /// and [`PluckedString::set_frequency`] — every live control whose
    /// change can move where in the tuned loop `loop_delay` needs to sit.
    /// The upper bound only ever binds for `set_frequency`: damping and
    /// inharmonicity move a filter's phase delay, which only ever shrinks
    /// `loop_delay` below `period`, never grows it past what the delay line
    /// was sized for.
    fn retune_loop_delay(&mut self) {
        let retuned = tuned_loop_delay(self.period, &self.loop_filter, &self.dispersion);
        let max_loop_delay = self.delay.max_delay() as f32 - 1.0;
        self.loop_delay = math::clamp_or_low(retuned, MIN_LOOP_DELAY, max_loop_delay);
        self.place_strike_tap();
    }

    /// Recomputes where in the delay line the hammer acts.
    ///
    /// A wave leaving the strike point towards the agraffe is back
    /// `strike_position` of a period later, which is seldom a whole number
    /// of samples: the hammer reads and writes between two taps, weighted
    /// by how near each is. Rounded to one tap, the top octave's strike
    /// point — its whole loop is a dozen samples — jumped by half its
    /// distance from one key to the next, and the notes' levels with it.
    /// The position is clamped so the allpass bridge read never sees a
    /// sample the hammer has yet to touch.
    fn place_strike_tap(&mut self) {
        let longest = (self.loop_delay as usize).saturating_sub(2);
        let round_trip = self.strike_position * self.period;
        let position = math::clamp_or_low(round_trip - 1.0, 0.0, longest as f32);
        self.strike_tap = position as usize;
        self.strike_blend = position - self.strike_tap as f32;
    }

    #[inline]
    fn track_envelope(&mut self, output: f32) {
        let magnitude = math::abs(output);
        let decay_rate = if self.damper_engaged() {
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
