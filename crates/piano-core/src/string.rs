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
    filter::{DcBlocker, LoopFilter, OnePoleLowpass},
    hammer, math,
    noise::Xorshift32,
    units::{Hz, SampleRate},
};

/// Shortest usable loop delay, in samples.
///
/// Below two samples the delay line no longer represents a travelling wave and
/// the pitch is meaningless.
const MIN_LOOP_DELAY: f32 = 2.0;

/// How many identical one-pole sections shape the hammer excitation's
/// spectrum in [`PluckedString::write_excitation`].
///
/// Two, for 12 dB/octave. One is measurably too gentle (see that method);
/// three and beyond start eating the attack's audible transient along with
/// the click, and cost another multiply-add per burst sample for it.
const EXCITATION_POLES: usize = 2;

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

/// Where along the string the hammer lands, as a fraction of the loop
/// length, at [`StringConfig::new`].
///
/// ~1/8 is the canonical piano strike ratio: it puts the strike-position
/// comb's first notch on the 8th partial, the dissonant one builders design
/// the strike point to suppress (Fletcher & Rossing, *The Physics of Musical
/// Instruments*, 2nd ed., §12.3). `piano_audio::voicing` sets a per-key value
/// that drifts across the keyboard; a caller building a [`StringConfig`]
/// directly gets this classic single value.
pub const DEFAULT_STRIKE_POSITION: f32 = 0.125;

/// Widest [`StringConfig::strike_position`] the string honours — a strike at
/// the loop's midpoint, which notches every even partial. Beyond it the
/// geometry only mirrors, and [`PluckedString::new`] reserves delay-line
/// headroom for exactly this much comb reach, so the strike-position comb
/// (`DelayLine::apply_strike_comb`) can never wrap the ring.
const MAX_STRIKE_POSITION: f32 = 0.5;

/// Loop-filter zero mix [`StringConfig::new`] uses when the caller does not
/// choose one — the loop filter's original, pre-per-register fixed value
/// (`filter::LoopFilter`'s own `MAX_ZERO_MIX`, full Nyquist-null rolloff).
/// `piano_audio::voicing::solve_loop_losses` is what chooses a per-key
/// value instead, from that key's own per-partial decay targets; a caller
/// building a [`StringConfig`] directly (as
/// `piano-render`'s tests do, deliberately bypassing that per-register
/// voicing) keeps today's original filter character unchanged.
pub const DEFAULT_LOOP_ZERO_MIX: f32 = 0.5;

/// How much of the excitation is broadband noise rather than the hammer's
/// own force pulse, in `[0, 1]`, at [`StringConfig::new`].
///
/// `0` is a purely deterministic strike — the contact force's own first
/// difference (its `dF/dt`, the waveguide velocity-source form; see
/// [`PluckedString::contact_force_diff_inv_peak`]) injected as the
/// excitation signal itself, so two strikes at one velocity and seed
/// produce spectrally identical attacks rather than the ragged,
/// unrepeatable profile a fresh noise draw gives every strike (issue #77,
/// `docs/TIMBRE-PLAN.md` D4). `1` is the original behaviour exactly: white
/// noise shaped by the contact envelope.
///
/// The default is a roughly even split. #77's aspiration was to make the
/// pulse the *clear* majority and keep only a trace of noise, and the two
/// audible wins it names are already fully banked at any mix below 1: the
/// strike is repeatable (the generator is rewound to the string's seed
/// before every strike — the same "seeded noise for reproducible renders"
/// the `noise` module documents, now applied per strike), and the bulk of
/// the low/mid spectrum is the deterministic `dF/dt` pulse, not noise.
///
/// It does not go lower than this because the *stochastic* component still
/// does two jobs the `dF/dt` pulse alone cannot cover across the compass.
/// One is the velocity → brightness cue: the differentiated pulse is
/// broadband and its slope steepens with strike force, so on a note whose
/// period holds the whole contact that cue survives from the pulse alone,
/// but in the treble the contact is truncated to one loop length
/// (`PluckedString::pluck` writes no more), which flattens the pulse's
/// velocity dependence there, and
/// `m4_spectral::a_harder_strike_is_brighter_in_the_attack_itself` — the
/// gate #77 names — needs the noise term shaped by
/// `hammer::excitation_cutoff_hz` to make up the difference. The other is
/// upper-partial energy in the bass: the pulse's spectrum rolls off well
/// below the 8th partial of a low note, and the all-88 `engine_timbre`
/// sweep (#87) checks that the bright partial still rings a measurable
/// fraction as long as the fundamental rather than dying inside one
/// analysis window. That check is the binding one, and it binds low in the
/// bass — at `0.6` F2 (MIDI 41) failed it, its H8 decaying in ~0.17 s
/// against a 6 s fundamental (a ratio of 35, past the sweep's 25 ceiling) —
/// so the floor is `0.7`, the lowest even-tenth that clears every key of
/// the sweep. The treble brightness gate is satisfied by less; the bass
/// check wants more, and wins. Once the treble truncation is fixed the
/// pulse can carry more of the load and this can drop toward the trace #77
/// wanted; `voicing`'s F6 re-tune (#80) is where to move it by ear, per
/// register rather than globally.
pub const DEFAULT_EXCITATION_NOISE_MIX: f32 = 0.7;

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
    /// This string's own felt-contact physics. See [`hammer::HammerConfig`].
    pub hammer: hammer::HammerConfig,
    /// The loop filter's zero mix, in `[0, filter::MAX_ZERO_MIX]`. Higher
    /// gives upper partials extra rolloff beyond what `damping` alone
    /// provides, at the cost of amplitude at the fundamental itself — see
    /// [`crate::filter::LoopFilter`]'s docs for why this had to become a
    /// per-string value rather than a fixed constant.
    pub loop_zero_mix: f32,
    /// Where the hammer strikes, as a fraction of the loop length, in
    /// `[0, MAX_STRIKE_POSITION]`. `write_excitation` sums the excitation
    /// with its inverted reflection this far back to notch the partials with
    /// a node at the strike point — the `1/strike_position` harmonic and its
    /// multiples. `0` disables the comb. See [`DEFAULT_STRIKE_POSITION`].
    pub strike_position: f32,
    /// Fraction of the excitation that is broadband noise rather than the
    /// deterministic hammer force pulse, in `[0, 1]`. See
    /// [`DEFAULT_EXCITATION_NOISE_MIX`].
    pub excitation_noise_mix: f32,
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
            hammer: hammer::DEFAULT_HAMMER,
            loop_zero_mix: DEFAULT_LOOP_ZERO_MIX,
            strike_position: DEFAULT_STRIKE_POSITION,
            excitation_noise_mix: DEFAULT_EXCITATION_NOISE_MIX,
        }
    }
}

/// A felt hammer's contact force still being injected past the first loop
/// length.
///
/// [`PluckedString::write_excitation`] can only fill one loop length's worth
/// of delay line before [`PluckedString::pluck`] returns; a real hammer's
/// contact can outlast that for any string whose period is shorter than the
/// contact duration (`docs/PHYSICS.md`'s "What the hammer still gets
/// wrong"), so this carries the rest of the contact — now a live,
/// [`hammer::couple_contact_step`] simulation rather than an index into a
/// precomputed curve — and the same shaping filters' state, to be advanced
/// sample by sample as [`PluckedString::write_mixed_feedback`] runs.
#[derive(Debug, Clone, Copy)]
struct PendingContact {
    /// The hammer's own live state — compression, hammer velocity, and
    /// whether it has already separated from the string
    /// ([`hammer::ContactState`]). Replaces a plain array index now that
    /// each sample's force comes from [`hammer::couple_contact_step`]
    /// rather than a precomputed, uncoupled curve.
    contact: hammer::ContactState,
    /// This strike's own [`hammer::HammerConfig`], carried so
    /// [`PluckedString::next_contact_sample`] can call
    /// [`hammer::couple_contact_step`] without re-reading
    /// [`PluckedString::hammer`], which a live [`PluckedString::set_hammer`]
    /// call could have changed mid-strike.
    hammer: hammer::HammerConfig,
    /// The reference (uncoupled) curve's peak force for *this* strike —
    /// [`hammer::couple_contact_step`]'s raw output is divided by this, the
    /// same scale [`PluckedString::write_excitation`] normalised the first
    /// `burst_length` samples by, so [`PluckedString::contact_force_diff_inv_peak`]
    /// stays meaningful across the whole strike, not just its first loop
    /// length.
    peak: f32,
    /// This strike's velocity, same role as in
    /// [`PluckedString::write_excitation`]'s own `* velocity` scaling:
    /// without it, a strike whose contact outlasts one loop length would
    /// stop getting louder for the tail injected by
    /// [`PluckedString::next_contact_sample`], even though harder strikes
    /// are still louder for the first loop length's worth.
    velocity: f32,
    /// The peak-normalised force the burst's *last* sample carried, so the
    /// tail's first sample differentiates against the force that really
    /// preceded it rather than against silence. The excitation is `dF/dt`
    /// scaled by [`PluckedString::contact_force_diff_inv_peak`], which is
    /// calibrated to the largest step the contact curve ever takes between
    /// two adjacent samples; restarting `prev` at `0.0` mid-contact fakes a
    /// step the size of the whole pulse — measured at 56× a real one for A4
    /// at velocity 0.8 — and injects it as a single full-scale spike one
    /// round trip into the note.
    prev_shape: f32,
    /// Continuation of the same shaping filter chain
    /// [`PluckedString::write_excitation`] started, so the spectrum does not
    /// discontinuously change where the first loop length's worth left off.
    felt: [OnePoleLowpass; EXCITATION_POLES],
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
    /// See [`PendingContact`]. `None` once a strike's contact has fully
    /// injected (or for a strike whose contact fit in one loop length to
    /// begin with — the common case for bass and mid-register keys).
    pending_contact: Option<PendingContact>,
    /// `1 / max|Δcontact_force|` for the current strike, set by
    /// [`PluckedString::write_excitation`]; `0.0` for a contact too short to
    /// have a slope. The deterministic part of the excitation is the force
    /// pulse's first difference scaled by this, so it peaks near unit
    /// amplitude whatever the contact's duration — a bass strike's
    /// 400-sample pulse and a treble strike's 50-sample one differentiate to
    /// very different raw slopes. Differentiating is also what makes the
    /// deterministic excitation broadband (a smooth pulse's derivative
    /// carries the high-frequency energy the velocity-dependent lowpass then
    /// shapes) and exactly DC-free (a telescoping sum), so a unipolar pulse
    /// never builds a standing offset in the near-lossless treble. It is the
    /// waveguide velocity-source form: a string is driven by `dF/dt` at the
    /// contact, not by `F` (Van Duyne & Smith, "Developments for the
    /// Commuted Piano", ICMC 1995; Bank, "Physically Informed Sound
    /// Synthesis of the Piano", 2000).
    contact_force_diff_inv_peak: f32,
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
    /// This string's own felt-contact physics, set from
    /// [`StringConfig::hammer`] and live-adjustable via
    /// [`PluckedString::set_hammer`].
    hammer: hammer::HammerConfig,
    /// Where the hammer strikes, as a fraction of the loop length, clamped
    /// into `[0, MAX_STRIKE_POSITION]`. Read by [`PluckedString::write_excitation`]
    /// to size the strike-position comb; live-adjustable for the *next*
    /// strike via [`PluckedString::set_strike_position`].
    strike_position: f32,
    /// This string's excitation noise seed. Kept alongside `rng` so
    /// [`PluckedString::write_excitation`] can rewind the generator to it on
    /// every strike — a strike at a given velocity and seed is then
    /// byte-identical however many times the voice has been re-plucked,
    /// which is what makes the deterministic-pulse claim of issue #77
    /// testable. [`PluckedString::set_seed`] moves both together.
    seed: u32,
    /// Fraction of the excitation that is broadband noise rather than the
    /// deterministic hammer force pulse, clamped into `[0, 1]`. Read by
    /// [`PluckedString::write_excitation`] and [`PluckedString::next_contact_sample`];
    /// live-adjustable for the *next* strike via
    /// [`PluckedString::set_excitation_noise_mix`]. See
    /// [`DEFAULT_EXCITATION_NOISE_MIX`].
    excitation_noise_mix: f32,
}

/// `1 / max|force[i] − force[i−1]|` over the first `active` entries of
/// `force`, or `0.0` when `active` is `0` or the contact is flat (no
/// slope). Used by [`PluckedString::write_excitation`] to normalise the
/// differentiated excitation to roughly unit peak whatever the contact's
/// duration — see [`PluckedString::contact_force_diff_inv_peak`].
fn diff_inv_peak_of(force: &[f32; hammer::MAX_CONTACT_SAMPLES], active: usize) -> f32 {
    let active = active.min(force.len());
    let earlier = force.iter().take(active);
    let later = force.iter().take(active).skip(1);
    let peak = earlier
        .zip(later)
        .map(|(a, b)| math::abs(b - a))
        .fold(0.0f32, f32::max);
    if peak > f32::EPSILON { 1.0 / peak } else { 0.0 }
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

        // Sized for the lowest frequency a live `set_frequency` call is
        // allowed to reach, not just the construction-time frequency —
        // otherwise a downward live retune would need to grow the delay
        // line, which the audio thread cannot do.
        let lowest_live_frequency =
            config.frequency.hertz() * math::powf(2.0, -MAX_LIVE_DETUNE_CENTS / 1200.0);
        let max_live_period = sample_rate.hertz() / lowest_live_frequency;

        // Extra headroom past one loop length so the strike-position comb
        // (`write_excitation`) can reach back up to `MAX_STRIKE_POSITION` of
        // the loop without the `x[i - delay]` read wrapping the ring onto the
        // burst's own far end. Sized for the lowest live frequency and the
        // widest strike position together, so the comb delay never has to be
        // clamped for any note or any live-set strike position.
        let comb_headroom = max_live_period * MAX_STRIKE_POSITION;

        Ok(Self {
            delay: DelayLine::with_capacity((max_live_period + comb_headroom) as usize + 4),
            loop_filter,
            dispersion,
            dc_blocker: DcBlocker::default(),
            rng: Xorshift32::new(config.seed),
            seed: config.seed,
            pending_contact: None,
            contact_force_diff_inv_peak: 0.0,
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
            hammer: config.hammer,
            strike_position: math::clamp_or_low(config.strike_position, 0.0, MAX_STRIKE_POSITION),
            excitation_noise_mix: math::clamp_or_low(config.excitation_noise_mix, 0.0, 1.0),
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

    /// Reseeds the excitation noise used by the *next* [`PluckedString::pluck`].
    /// A string already ringing is unaffected, since its excitation was
    /// already written. Stores the seed as well as reseeding, so
    /// [`PluckedString::write_excitation`]'s per-strike rewind uses the new
    /// value on every future strike, not just the next one.
    pub fn set_seed(&mut self, seed: u32) {
        self.seed = seed;
        self.rng = Xorshift32::new(seed);
    }

    /// Changes this string's felt-contact physics, used by the *next*
    /// [`PluckedString::pluck`]. A string already ringing is unaffected,
    /// since its excitation burst was already shaped and written into the
    /// delay line.
    pub fn set_hammer(&mut self, hammer: hammer::HammerConfig) {
        self.hammer = hammer;
    }

    /// Moves where the hammer strikes, as a fraction of the loop length,
    /// used by the *next* [`PluckedString::pluck`]. `strike_position` is
    /// clamped into `[0, MAX_STRIKE_POSITION]`, `NaN` mapping to `0` (no
    /// comb), the same convention every other live setter here uses. A
    /// string already ringing is unaffected, since its excitation burst —
    /// and so its strike-position comb — was already written into the delay
    /// line. Lower values push the comb notch onto a higher partial
    /// (`1/strike_position`), the audible direction issue #32 asks for.
    pub fn set_strike_position(&mut self, strike_position: f32) {
        self.strike_position = math::clamp_or_low(strike_position, 0.0, MAX_STRIKE_POSITION);
    }

    /// Sets how much of the *next* strike's excitation is broadband noise
    /// rather than the deterministic hammer force pulse. `mix` is clamped
    /// into `[0, 1]`, `NaN` mapping to `0` (a fully deterministic strike),
    /// the same convention every other live setter here uses. A string
    /// already ringing is unaffected, since its excitation was already
    /// written into the delay line. See [`DEFAULT_EXCITATION_NOISE_MIX`].
    pub fn set_excitation_noise_mix(&mut self, mix: f32) {
        self.excitation_noise_mix = math::clamp_or_low(mix, 0.0, 1.0);
    }

    /// Excites the string with a felt-shaped hammer force pulse at the given
    /// velocity, blended with a broadband noise component
    /// ([`PluckedString::excitation_noise_mix`]).
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

        self.write_excitation(velocity);
    }

    /// Fills the delay line with one strike's worth of excitation: the
    /// hammer contact force's first difference (`dF/dt`), shaped in *time* by
    /// its own felt-contact envelope, blended toward an envelope-shaped
    /// noise burst by [`PluckedString::excitation_noise_mix`] (issue #77),
    /// shaped in *frequency* by a lowpass whose corner that same contact
    /// duration sets, and finally in *space* by the strike-position comb —
    /// the inverted reflection from where the hammer lands, which notches
    /// the partials with a node at the strike point (see the comb step at
    /// the end of this method and `DelayLine::apply_strike_comb`).
    ///
    /// # Why the excitation is a differentiated force pulse, not a noise burst
    ///
    /// A real hammer delivers a deterministic force pulse, so two strikes of
    /// one key at one velocity are spectrally the same strike. The earlier
    /// model instead multiplied white noise by the contact envelope, giving
    /// every partial a random amplitude and phase on every strike — a
    /// ragged, unrepeatable attack profile (`docs/TIMBRE-PLAN.md` D4) rather
    /// than the fixed spectral envelope a struck string has. The rng is now
    /// rewound to [`PluckedString::seed`] before every strike, so a given
    /// velocity and seed is byte-identical however many times the voice has
    /// been re-plucked, and the bulk of the excitation is the force pulse
    /// itself rather than noise.
    ///
    /// It is the pulse's *derivative* that is injected, not `F` raw, for two
    /// reasons that happen to coincide. Physically, a waveguide is a
    /// velocity source driven by `dF/dt` at the contact point (Van Duyne &
    /// Smith, ICMC 1995; Bank 2000). Practically, `F` raw is a smooth
    /// unipolar bump whose spectrum rolls off by `~1/contact_duration` — a
    /// few hundred hertz — so it leaves the upper partials unexcited *and*
    /// its velocity → brightness behaviour is swamped by the bump's own
    /// shape; `dF/dt` is broadband, exactly DC-free, and its high-frequency
    /// content rises with strike velocity (harder is shorter is steeper),
    /// which is the "brighter, not merely louder" cue
    /// [`hammer::excitation_cutoff_hz`] then sharpens. Normalised to unit
    /// peak by [`PluckedString::contact_force_diff_inv_peak`] so its level
    /// does not swing three registers wide with the contact duration.
    ///
    /// [`PluckedString::excitation_noise_mix`] keeps a *small* broadband
    /// stochastic component: a real felt strike has one, from the felt's
    /// surface and the string's back-reaction during contact, and removing
    /// it entirely sounds synthetic the other way. Mix `1` reproduces the
    /// original noise excitation exactly; mix `0` is a purely deterministic
    /// strike.
    ///
    /// The *frequency* shaping still applies on top: both components pass
    /// the lowpass, whose corner [`hammer::excitation_cutoff_hz`] moves with
    /// strike velocity. See that function for the measurement.
    ///
    /// The lowpass is [`EXCITATION_POLES`] identical one-pole sections in
    /// series, not one: a single 6 dB/octave section leaves so much of the
    /// band above its corner intact that the strike still measures as
    /// broadband (8.5 kHz of spectral centroid against a 4 kHz corner, at
    /// 48 kHz — because a magnitude falling as `1/f` contributes equally to
    /// the centroid from every octave above the corner). Cascading identical
    /// sections is also the shape the physics suggests: the felt's finite
    /// contact width and the force pulse's own smoothness are two separate
    /// rolloffs, not one.
    ///
    /// Once `contact.separated` goes true, the coupled force has gone to
    /// `0.0`, so the burst goes silent on its own once the hammer has left
    /// the string; the rest of the delay line is filled with that silence,
    /// filtered, rather than left untouched.
    ///
    /// Fills exactly one loop length here — the most this call alone can
    /// write before [`PluckedString::pluck`] returns — and, when
    /// `contact_samples` outlasts that (any string whose period is shorter
    /// than the felt's contact, `docs/PHYSICS.md`'s "What the hammer still
    /// gets wrong"), leaves [`PendingContact`] set up with the live, not yet
    /// separated [`hammer::ContactState`] so
    /// [`PluckedString::write_mixed_feedback`] can keep coupling and
    /// injecting sample by sample as the string actually rings. A bass or
    /// mid-register strike, whose contact fits in one loop length, leaves
    /// `pending_contact` `None` and never sets one up at all.
    fn write_excitation(&mut self, velocity: f32) {
        let (mut reference_curve, contact_samples, peak) =
            hammer::uncoupled_contact_curve(velocity, self.sample_rate, self.hammer);
        hammer::normalize_by_peak(&mut reference_curve, contact_samples, peak);
        let cutoff_hz = hammer::excitation_cutoff_hz(contact_samples, self.sample_rate);
        let mut felt = [OnePoleLowpass::from_cutoff(cutoff_hz, self.sample_rate); EXCITATION_POLES];
        self.contact_force_diff_inv_peak = diff_inv_peak_of(&reference_curve, contact_samples);
        // A given velocity and seed is one strike, every time it is played.
        self.rng = Xorshift32::new(self.seed);
        let burst_length = self.loop_delay as usize + 1;
        let mut contact = hammer::ContactState::starting(velocity);
        let mut prev_shape = 0.0f32;
        for index in 0..burst_length {
            // The earliest self-reflection of this very strike, read raw
            // (not yet through `loop_filter`/`dispersion`, which only run
            // once real per-sample playback starts) — see the design doc's
            // note on why this approximation only matters for strings
            // whose `loop_delay` is shorter than the burst. `DelayLine::read`
            // counts back from the sample written *last*, so one loop length
            // ago is `loop_delay - 1` entries back, not `index - loop_delay`.
            let v_incoming = if index >= self.loop_delay as usize {
                self.delay
                    .read((self.loop_delay as usize).saturating_sub(1))
            } else {
                0.0
            };
            let (next_contact, raw_force) =
                hammer::couple_contact_step(contact, self.hammer, v_incoming, self.sample_rate);
            contact = next_contact;
            // Same peak the reference curve above was normalised by, so
            // `contact_force_diff_inv_peak` stays on the scale it was
            // calibrated against — see `PendingContact::peak`'s doc comment.
            let shape = if peak > f32::EPSILON {
                raw_force / peak
            } else {
                0.0
            };
            let excitation = self.blended_excitation(shape, prev_shape) * velocity;
            prev_shape = shape;
            let shaped = felt
                .iter_mut()
                .fold(excitation, |sample, stage| stage.process(sample));
            self.delay.write(shaped);
        }
        let comb_delay = (self.strike_position * self.loop_delay + 0.5) as usize;
        let comb_delay = comb_delay.min(self.delay.max_delay().saturating_sub(burst_length));
        self.delay.apply_strike_comb(burst_length, comb_delay);
        self.pending_contact =
            (contact_samples > burst_length && !contact.separated).then_some(PendingContact {
                contact,
                hammer: self.hammer,
                peak,
                velocity,
                prev_shape,
                felt,
            });
    }

    /// One excitation sample for contact-force envelope value `shape` and
    /// its predecessor `prev`: the pulse's first difference
    /// `(shape − prev)·contact_force_diff_inv_peak` — its `dF/dt`, broadband
    /// and exactly DC-free — blended toward an envelope-shaped noise sample
    /// by [`PluckedString::excitation_noise_mix`] (issue #77).
    ///
    /// `d + mix·(noisy − d)` is a convex blend: at `mix == 0` it is the
    /// differentiated pulse, at `mix == 1` it is `rng.next_bipolar() · shape`
    /// — the original noise excitation exactly, so a default of `1` is a
    /// bit-for-bit no-op against it. The generator is drawn unconditionally
    /// so its state advances by the same amount whatever the mix, keeping a
    /// live `set_excitation_noise_mix` from desynchronising a seed.
    ///
    /// Total: one finite multiply-add of finite inputs
    /// ([`Xorshift32::next_bipolar`] is bounded, `shape`/`prev` come from
    /// [`hammer::simulate_contact`]'s `[0, 1]` output,
    /// `contact_force_diff_inv_peak` is finite and non-negative by
    /// construction, `excitation_noise_mix` is clamped into `[0, 1]` at
    /// every entry point) — it cannot panic or return a non-finite value.
    #[inline]
    fn blended_excitation(&mut self, shape: f32, prev: f32) -> f32 {
        let deterministic = (shape - prev) * self.contact_force_diff_inv_peak;
        let noisy = self.rng.next_bipolar() * shape;
        deterministic + self.excitation_noise_mix * (noisy - deterministic)
    }

    /// Injects the next sample of an ongoing hammer contact
    /// ([`PendingContact`]) on top of the loop's own feedback, or `0.0` once
    /// contact has ended (or the string's last strike never needed one).
    /// `v_incoming` is the real, current bridge-tap value
    /// [`PluckedString::write_mixed_feedback`] just read, so the contact
    /// this advances is genuinely coupled to the string rather than
    /// approximated against silence.
    ///
    /// Differentiates against [`PendingContact::prev_shape`], not against
    /// `0.0`: the injected value is `dF/dt`, so the tail has to pick the
    /// force up exactly where the burst put it down or the hand-off itself
    /// reads as a step change in force — see that field's doc comment.
    ///
    /// Total: every branch returns a plain `f32`,
    /// [`hammer::couple_contact_step`] is itself total for any input, and
    /// [`OnePoleLowpass::process`] cannot return non-finite output for a
    /// finite input — this can never panic or leak a `NaN` into the
    /// feedback loop it feeds.
    #[inline]
    fn next_contact_sample(&mut self, v_incoming: f32) -> f32 {
        let Some(mut contact) = self.pending_contact.take() else {
            return 0.0;
        };
        let (next_state, raw_force) = hammer::couple_contact_step(
            contact.contact,
            contact.hammer,
            v_incoming,
            self.sample_rate,
        );
        contact.contact = next_state;
        // Same peak `write_excitation` normalised this strike's reference
        // curve by (`PendingContact::peak`'s doc comment).
        let shape = if contact.peak > f32::EPSILON {
            raw_force / contact.peak
        } else {
            0.0
        };
        let prev = contact.prev_shape;
        contact.prev_shape = shape;
        let excitation = self.blended_excitation(shape, prev) * contact.velocity;
        let shaped = contact
            .felt
            .iter_mut()
            .fold(excitation, |sample, stage| stage.process(sample));
        if !next_state.separated {
            self.pending_contact = Some(contact);
        }
        shaped
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
        let loop_gain = if self.damper_engaged {
            self.sustain * RELEASE_LOSS_MULTIPLIER
        } else {
            self.sustain
        };
        let bridge_admittance = 1.0 - loop_gain;
        let driven = mixed + coupling * bridge_admittance;
        // The hammer's continued push (`PendingContact`, when a strike's
        // contact outlasts one loop length) is an external force on top of
        // the loop's own scattering, so it is added after `loop_gain` is
        // applied to the recirculating signal, not folded into it.
        let reflected = driven * loop_gain + self.next_contact_sample(tap);
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
    /// phase delay at DC, clamped into `[MIN_LOOP_DELAY, max_delay - 1]`
    /// rather than going negative or reading outside the delay line. Shared
    /// by [`PluckedString::set_damping`], [`PluckedString::set_inharmonicity`]
    /// and [`PluckedString::set_frequency`] — every live control whose
    /// change can move where in the tuned loop `loop_delay` needs to sit.
    /// The upper bound only ever binds for `set_frequency`: damping and
    /// inharmonicity move a filter's phase delay, which only ever shrinks
    /// `loop_delay` below `period`, never grows it past what the delay line
    /// was sized for.
    fn retune_loop_delay(&mut self) {
        let retuned = self.period
            - self.loop_filter.phase_delay_at_dc()
            - self.dispersion.phase_delay_at_dc()
            - 1.0;
        let max_loop_delay = self.delay.max_delay() as f32 - 1.0;
        self.loop_delay = math::clamp_or_low(retuned, MIN_LOOP_DELAY, max_loop_delay);
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
