//! Per-key baseline voicing — M5's "per-key parameter tables"
//! (`docs/ROADMAP.md`).
//!
//! Before this module, [`crate::engine::Engine`] built every one of the 88
//! voices from the same global [`StringConfig::new`] default and varied
//! only `frequency`. Real piano strings are not uniform across the
//! keyboard: bass strings are wound, ring for tens of seconds and carry
//! little inharmonicity; treble strings are thin plain wire, decay in a
//! couple of seconds, and carry much more (Fletcher & Rossing, already
//! cited in [`piano_core::dispersion`] for exactly this bass-to-treble
//! range). [`voicing_for_key`] computes a `damping`/`sustain`/
//! `inharmonicity` baseline for each key by interpolating between three
//! anchors — A0, A4 and C8 — taken from numbers this project's own docs
//! already state, rather than inventing new ones (`docs/PRIOR-ART.md`'s
//! rule that parameter values come from a published source or a documented
//! fit, never an unexplained literal).
//!
//! # Why a key is voiced against three decay times, not one
//!
//! An earlier version of this module asked for one thing per key: the
//! *fundamental's* ring-out time. It then solved that single equation with
//! two free parameters — the loop filter's `pole` and `zero_mix` — leaving
//! the filter's slope, which is what sets every *upper* partial's decay, an
//! uncontrolled side effect. Nobody had ever said how fast the 8th partial
//! should die, so nothing did: measured, A0's 8th partial decayed only
//! 1.18x faster than its fundamental (a real bass string is nearer 6, and
//! partials decaying together is the spectral signature of an organ or a
//! struck bar), while A4 lost every partial above the first inside a single
//! analysis window and became a bare 440 Hz sine. `docs/TIMBRE-PLAN.md`
//! section D1 carries the full measurement.
//!
//! [`voicing_for_key`] now asks for three decay times per key — the
//! fundamental's, a mid partial's and a bright partial's — and solves for
//! three unknowns: `pole`, `zero_mix` and `sustain`. Adding `sustain` to
//! the solve is what makes it converge at all: the loop filter's DC gain is
//! exactly 1 by construction, so it *cannot* attenuate a fundamental, and a
//! calibration that drives `sustain` toward 1.0 and asks the filter to
//! carry the whole loss has to drag the filter's corner down far enough to
//! take the harmonics with it. See [`solve_loop_losses`].

use piano_core::dispersion::MAX_INHARMONICITY;
use piano_core::filter::LoopFilter;
use piano_core::string::{SILENCE_THRESHOLD, StringConfig};
use piano_core::{SampleRate, math};
use piano_params::{CONCERT_A_KEY, HIGHEST_PIANO_KEY, LOWEST_PIANO_KEY, PianoKey, Tuning};

/// Target ring-out time for the fundamental at A0, the middle of
/// `docs/PHYSICS.md`'s "Typical decay" row for that key (30-40 s).
const BASS_DECAY_SECONDS: f32 = 35.0;

/// Target ring-out time for the fundamental at A4, the middle of the same
/// row's 8-15 s.
const MID_DECAY_SECONDS: f32 = 11.0;

/// Target ring-out time for the fundamental at C8, the middle of the same
/// row's 1-2 s.
const TREBLE_DECAY_SECONDS: f32 = 1.5;

/// Which partial the `*_MID_PARTIAL_DECAY_SECONDS` anchors describe.
const MID_PARTIAL: f32 = 3.0;

/// Which partial the `*_BRIGHTNESS_DECAY_SECONDS` anchors describe.
const BRIGHTNESS_PARTIAL: f32 = 8.0;

/// Target ring-out time for [`MID_PARTIAL`] at A0.
///
/// A real string does not lose every partial at the same rate: both air
/// damping and the wire's own internal friction grow with frequency, so
/// partial `n` dies roughly `n` times faster than the fundamental (Fletcher
/// & Rossing, *The Physics of Musical Instruments*, the piano-string
/// damping section — the same source this project already cites in
/// [`piano_core::dispersion`] for inharmonicity). The anchors here follow
/// that `1/n` law loosely, flattened across the low partials where measured
/// pianos hold their first few closer together than `1/n` predicts: at A0,
/// `35 / 18 / 6 s` is a `1 : 1.9 : 5.8` spread against `1/n`'s `1 : 3 : 8`.
///
/// The specific values are this project's own reasoned anchors, first
/// solved against in `docs/TIMBRE-PLAN.md`'s D2 table — literature-shaped,
/// but not a measured curve, and labelled as such there and in
/// `docs/PHYSICS.md`.
const BASS_MID_PARTIAL_DECAY_SECONDS: f32 = 18.0;

/// Target ring-out time for [`MID_PARTIAL`] at A4 — see
/// [`BASS_MID_PARTIAL_DECAY_SECONDS`] for where these come from.
const MID_MID_PARTIAL_DECAY_SECONDS: f32 = 5.0;

/// Target ring-out time for [`MID_PARTIAL`] at C8 — see
/// [`BASS_MID_PARTIAL_DECAY_SECONDS`].
const TREBLE_MID_PARTIAL_DECAY_SECONDS: f32 = 0.8;

/// Target ring-out time for [`BRIGHTNESS_PARTIAL`] at A0 — see
/// [`BASS_MID_PARTIAL_DECAY_SECONDS`].
const BASS_BRIGHTNESS_DECAY_SECONDS: f32 = 6.0;

/// Target ring-out time for [`BRIGHTNESS_PARTIAL`] at A4 — see
/// [`BASS_MID_PARTIAL_DECAY_SECONDS`].
const MID_BRIGHTNESS_DECAY_SECONDS: f32 = 1.5;

/// Target ring-out time for [`BRIGHTNESS_PARTIAL`] at C8 — see
/// [`BASS_MID_PARTIAL_DECAY_SECONDS`].
const TREBLE_BRIGHTNESS_DECAY_SECONDS: f32 = 0.3;

/// Inharmonicity `B` at A0. Fletcher & Rossing's range, already cited in
/// [`piano_core::dispersion`], bottoms out "roughly 0.0001 in the bass".
const BASS_INHARMONICITY: f32 = 0.000_1;

/// Inharmonicity `B` at C8: the top of the same cited range,
/// [`MAX_INHARMONICITY`].
const TREBLE_INHARMONICITY: f32 = MAX_INHARMONICITY;

/// How the three fitted partials weigh against each other in
/// [`fit_broadband_loss`]'s least squares, fundamental first.
///
/// The fundamental carries double weight because it is what "the note
/// sustains" means to a listener, and because the three targets cannot all
/// be met exactly: near DC a pole and a zero both attenuate as `f²`, so the
/// filter has less independent control over the mid partial than two free
/// coefficients suggest, and something has to give. `docs/TIMBRE-PLAN.md`'s
/// D2 note — "weighting H1 higher tightens it" — is where that trade-off
/// was first measured.
const PARTIAL_WEIGHTS: [f32; 3] = [2.0, 1.0, 1.0];

/// Highest fraction of the sample rate a partial may sit at and still be
/// worth fitting. Above it a partial is either aliased or inaudible, so
/// [`solved_partials`] fits a lower one instead — at C8 the 8th partial
/// would land at 33 kHz, well past Nyquist.
const HIGHEST_SOLVED_PARTIAL_FRACTION: f32 = 0.45;

/// Lowest partial [`solved_partials`] will fall back to for the brightness
/// target. Below the second there is no "upper partial" left to shape and
/// the solve would be fitting the fundamental three times over.
const MIN_SOLVED_PARTIAL: f32 = 2.0;

/// Floor under [`LoopFilter::magnitude_at`] before it is turned into a loss
/// in nepers. Purely so `ln` can never be handed a zero; a filter this
/// opaque is far outside anything the search selects.
const MIN_SOLVED_MAGNITUDE: f32 = 1e-9;

/// Shortest decay time [`required_losses`] will honour, in seconds. Guards
/// the reciprocal against a degenerate target rather than encoding a
/// musical minimum.
const MIN_TARGET_SECONDS: f32 = 1e-3;

/// Largest broadband loss, in nepers per round trip, [`fit_broadband_loss`]
/// may return. One neper per round trip silences a string in about ten
/// round trips — far past any musical setting; the cap exists only to keep
/// `sustain` strictly positive and the fit bounded.
const MAX_BROADBAND_LOSS: f32 = 1.0;

/// Smallest pole loss-shape coefficient the search covers — see
/// [`pole_for_axis`] for what that coefficient is. At `1e-7` the pole is
/// transparent to more digits than `f32` carries.
const MIN_POLE_SHAPE: f32 = 1e-7;

/// Largest pole loss-shape coefficient the search covers, corresponding to
/// a pole of about `0.998`. The bass, which needs the darkest filter on the
/// keyboard, solves to roughly `5e2`.
const MAX_POLE_SHAPE: f32 = 1e6;

/// Points per axis in [`solve_loop_losses`]'s opening sweep of the
/// normalised `(pole, zero-mix)` plane.
const COARSE_SEARCH_POINTS: u32 = 20;

/// How many times [`solve_loop_losses`] halves the search box around the
/// best point found so far. A compile-time bound, so the search is provably
/// finite — one of `docs/REALTIME-AUDIO-RULES.md`'s totality requirements.
/// This function does not run on the audio thread (see
/// [`solve_loop_losses`]), but the project holds every parameter-derivation
/// function to the same standard. Ten halvings shrink the opening grid's
/// spacing by `1024`, finer than the residual can be told apart in `f32`.
const SEARCH_REFINEMENTS: u32 = 10;

/// Points per axis in each of [`SEARCH_REFINEMENTS`]' sweeps.
const REFINEMENT_POINTS: u32 = 5;

/// A key's computed baseline voicing, ready to drop into
/// [`StringConfig`]'s matching fields.
///
/// `pub`, not `pub(crate)`: `piano-studio`'s cascade resolver
/// (`docs/PARAMETER-STUDIO.md`) reuses this as its "registers" tier rather
/// than reimplementing the same three-anchor interpolation.
#[derive(Debug, Clone, Copy)]
pub struct KeyVoicing {
    /// See [`StringConfig::damping`].
    pub damping: f32,
    /// See [`StringConfig::sustain`].
    pub sustain: f32,
    /// See [`StringConfig::inharmonicity`].
    pub inharmonicity: f32,
    /// See [`StringConfig::loop_zero_mix`].
    pub zero_mix: f32,
}

/// The three ring-out times one key's loop is solved against — its
/// fundamental's, [`MID_PARTIAL`]'s and [`BRIGHTNESS_PARTIAL`]'s — all in
/// seconds.
#[derive(Debug, Clone, Copy)]
struct DecayTargets {
    fundamental: f32,
    mid_partial: f32,
    brightness: f32,
}

impl DecayTargets {
    /// The target ring-out time for `partial`, interpolated between the
    /// three anchors in log-partial / log-time space — the space the `1/n`
    /// damping law [`BASS_MID_PARTIAL_DECAY_SECONDS`] cites is a straight
    /// line in, so anchors that sit on that law stay on it between
    /// themselves instead of bulging away from it.
    fn seconds_for_partial(self, partial: f32) -> f32 {
        let (low, high) = if partial <= MID_PARTIAL {
            ((1.0, self.fundamental), (MID_PARTIAL, self.mid_partial))
        } else {
            (
                (MID_PARTIAL, self.mid_partial),
                (BRIGHTNESS_PARTIAL, self.brightness),
            )
        };
        math::exp(interpolate_log_frequency(
            partial,
            low.0,
            math::ln(low.1),
            high.0,
            math::ln(high.1),
        ))
    }
}

/// A key's solved per-round-trip losses: the loop filter's two
/// coefficients, plus the broadband `sustain` that completes them.
#[derive(Debug, Clone, Copy)]
struct LoopLosses {
    pole: f32,
    zero_mix: f32,
    sustain: f32,
}

/// One axis-aligned square of the normalised `(pole, zero-mix)` search
/// plane, held as a centre and a half-width so [`sweep`] can shrink it
/// without ever leaving `[0, 1]²`.
#[derive(Debug, Clone, Copy)]
struct SearchBox {
    pole_axis: f32,
    zero_axis: f32,
    half_width: f32,
}

impl SearchBox {
    /// The whole plane: both axes span their full `[0, 1]`.
    fn whole_plane() -> Self {
        Self {
            pole_axis: 0.5,
            zero_axis: 0.5,
            half_width: 0.5,
        }
    }

    /// The coordinate `step` steps of `points` along this box's span about
    /// `centre`, clamped back into the plane at the edges.
    fn sample(self, centre: f32, step: u32, points: u32) -> f32 {
        let position = if points <= 1 {
            0.5
        } else {
            f32::from(u16::try_from(step).unwrap_or(u16::MAX))
                / f32::from(u16::try_from(points - 1).unwrap_or(u16::MAX))
        };
        math::clamp_or_low(centre + (2.0 * position - 1.0) * self.half_width, 0.0, 1.0)
    }
}

/// One register anchor's file-supplied override, as read from a
/// `.piano.json` file's `registers` block (`docs/PARAMETER-STUDIO.md`) —
/// every field independently optional, each falling back to this crate's
/// own built-in value for that anchor when unset. See [`RegisterOverrides`]
/// for what leaving every field unset guarantees.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct RegisterAnchorOverride {
    /// Which key this anchor sits at. `None`, or a value that is not a
    /// valid piano key (including `0`, the wire format's default for an
    /// absent field), falls back to this anchor's built-in position.
    pub anchor_midi: Option<u8>,
    /// Target ring-out time for the fundamental at this anchor. `None`
    /// falls back to this anchor's built-in target
    /// ([`BASS_DECAY_SECONDS`]/[`MID_DECAY_SECONDS`]/[`TREBLE_DECAY_SECONDS`]).
    pub decay_seconds: Option<f32>,
    /// Direct override of [`KeyVoicing::damping`], applied only to the one
    /// key exactly at this anchor's resolved position — not blended into a
    /// curve, the way [`RegisterAnchorOverride::inharmonicity`] is. Unlike
    /// `decay_seconds`/`inharmonicity`, damping is normally a *solved
    /// output* (see [`solve_loop_losses`]) rather than an independent
    /// anchor value, so there is no principled "default damping curve" to
    /// blend a partial override into; pinning the exact anchor key is the
    /// honest reading of "the same value `ParameterOverrides::damping`
    /// sets" without inventing one. `None` leaves that key's damping to the
    /// solve, as every key's already is.
    pub damping: Option<f32>,
    /// Inharmonicity `B` at this anchor. `None` falls back to this
    /// anchor's built-in value — for the middle anchor, that built-in
    /// value is whatever [`voicing_for_key`]'s own bass-to-treble curve
    /// already produces there, so an unset middle anchor changes nothing.
    pub inharmonicity: Option<f32>,
}

/// The `bass`/`mid`/`treble` register overrides a `.piano.json` file's
/// `registers` block can carry (`docs/PARAMETER-STUDIO.md`).
/// [`RegisterOverrides::default`] is every field unset, which makes
/// [`voicing_for_key_with_registers`] compute exactly what
/// [`voicing_for_key`] always has — see
/// `registers_default_matches_voicing_for_key_on_every_key` in this
/// module's tests.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct RegisterOverrides {
    /// Overrides the bass anchor, built-in at [`LOWEST_PIANO_KEY`].
    pub bass: RegisterAnchorOverride,
    /// Overrides the middle anchor, built-in at [`CONCERT_A_KEY`].
    pub mid: RegisterAnchorOverride,
    /// Overrides the treble anchor, built-in at [`HIGHEST_PIANO_KEY`].
    pub treble: RegisterAnchorOverride,
}

/// How many physical strings `key` is struck by (M6, `docs/ROADMAP.md`).
///
/// Delegates the register boundaries and per-register counts to
/// [`piano_core::unison::unison_count_for_key_index`] — see that
/// function's module docs (`piano_core::unison`) for the citation and the
/// honesty note on where the specific boundaries come from. This wrapper
/// exists only to convert this crate's [`PianoKey`] into the plain
/// zero-based index that function expects, the same shape every other
/// function in this module already uses `key.key_index()` for.
#[must_use]
pub fn unison_count_for_key(key: PianoKey) -> usize {
    piano_core::unison::unison_count_for_key_index(key.key_index())
}

/// Computes `key`'s baseline voicing under `tuning`, for a string that will
/// run at `sample_rate`.
///
/// `sample_rate` feeds [`solve_loop_losses`] — both the loop filter's
/// magnitude response and which partials still sit below Nyquist depend on
/// it. Only `inharmonicity` is sample-rate-independent.
#[must_use]
pub fn voicing_for_key(key: PianoKey, tuning: Tuning, sample_rate: SampleRate) -> KeyVoicing {
    voicing_for_key_with_registers(key, tuning, sample_rate, RegisterOverrides::default())
}

/// [`voicing_for_key`], with the `bass`/`mid`/`treble` register anchors a
/// `.piano.json` file's `registers` block can override —
/// `docs/PARAMETER-STUDIO.md`'s register tier, previously parsed by
/// `piano-studio` and silently discarded (`docs/TIMBRE-PLAN.md`, D5/P1).
/// `registers.default()` makes this compute exactly what [`voicing_for_key`]
/// always has; see this module's
/// `registers_default_matches_voicing_for_key_on_every_key` test.
#[must_use]
pub fn voicing_for_key_with_registers(
    key: PianoKey,
    tuning: Tuning,
    sample_rate: SampleRate,
    registers: RegisterOverrides,
) -> KeyVoicing {
    let frequency = key.frequency(tuning).hertz();
    let bass_midi = resolved_anchor_midi(LOWEST_PIANO_KEY, registers.bass.anchor_midi);
    let mid_midi = resolved_anchor_midi(CONCERT_A_KEY, registers.mid.anchor_midi);
    let treble_midi = resolved_anchor_midi(HIGHEST_PIANO_KEY, registers.treble.anchor_midi);
    let bass_hz = anchor_hz(bass_midi, tuning);
    let mid_hz = anchor_hz(mid_midi, tuning);
    let treble_hz = anchor_hz(treble_midi, tuning);

    let fundamental_targets = (
        registers.bass.decay_seconds.unwrap_or(BASS_DECAY_SECONDS),
        registers.mid.decay_seconds.unwrap_or(MID_DECAY_SECONDS),
        registers
            .treble
            .decay_seconds
            .unwrap_or(TREBLE_DECAY_SECONDS),
    );
    let targets = decay_targets_for(frequency, bass_hz, mid_hz, treble_hz, fundamental_targets);
    let losses = solve_loop_losses(frequency, targets, sample_rate);

    let damping = anchor_damping_pin(key, bass_midi, registers.bass.damping)
        .or_else(|| anchor_damping_pin(key, mid_midi, registers.mid.damping))
        .or_else(|| anchor_damping_pin(key, treble_midi, registers.treble.damping))
        .map_or(losses.pole, |pin| math::clamp_or_low(pin, 0.0, 1.0));

    KeyVoicing {
        damping,
        sustain: losses.sustain,
        inharmonicity: inharmonicity_for(frequency, bass_hz, mid_hz, treble_hz, registers),
        zero_mix: losses.zero_mix,
    }
}

/// Resolves which key an anchor sits at: `override_midi` when it names a
/// real piano key, `default_midi` otherwise — covering both "not given"
/// (`None`) and the wire format's `#[serde(default)]` zero, which is never
/// a valid piano key either.
fn resolved_anchor_midi(default_midi: u8, override_midi: Option<u8>) -> u8 {
    override_midi
        .filter(|&midi| PianoKey::from_midi(midi).is_ok())
        .unwrap_or(default_midi)
}

/// `damping_override` if `key` is exactly the key at `anchor_midi`,
/// `None` otherwise — see [`RegisterAnchorOverride::damping`] for why this
/// pins one key rather than blending a curve.
fn anchor_damping_pin(
    key: PianoKey,
    anchor_midi: u8,
    damping_override: Option<f32>,
) -> Option<f32> {
    if key.midi_number() == anchor_midi {
        damping_override
    } else {
        None
    }
}

/// Interpolates inharmonicity across the three register anchors. The
/// middle anchor's default (when [`RegisterAnchorOverride::inharmonicity`]
/// leaves it unset) is computed from the *resolved* bass/treble values at
/// `mid_hz`, which sit exactly on the straight line between them — so the
/// three-point curve this produces degenerates to the same bass-to-treble
/// line [`voicing_for_key`] always used, unless the file overrides the
/// middle anchor specifically.
fn inharmonicity_for(
    frequency: f32,
    bass_hz: f32,
    mid_hz: f32,
    treble_hz: f32,
    registers: RegisterOverrides,
) -> f32 {
    let clamp = |value: f32| math::clamp_or_low(value, 0.0, MAX_INHARMONICITY);
    let bass = clamp(registers.bass.inharmonicity.unwrap_or(BASS_INHARMONICITY));
    let treble = clamp(
        registers
            .treble
            .inharmonicity
            .unwrap_or(TREBLE_INHARMONICITY),
    );
    let mid_default = interpolate_log_frequency(mid_hz, bass_hz, bass, treble_hz, treble);
    let mid = clamp(registers.mid.inharmonicity.unwrap_or(mid_default));
    interpolate_two_segments(
        frequency,
        (bass_hz, bass),
        (mid_hz, mid),
        (treble_hz, treble),
    )
}

/// Interpolates all three of this key's decay targets across the register
/// anchors sitting at `bass_hz`/`mid_hz`/`treble_hz`. `fundamental_targets`
/// is the (bass, mid, treble) ring-out times for the fundamental only — the
/// one curve a `.piano.json` file's `registers` block can override
/// (`decay_seconds`); [`MID_PARTIAL`]/[`BRIGHTNESS_PARTIAL`]'s curves have
/// no file-exposed equivalent, so they always use this module's own
/// built-in anchors.
fn decay_targets_for(
    frequency: f32,
    bass_hz: f32,
    mid_hz: f32,
    treble_hz: f32,
    fundamental_targets: (f32, f32, f32),
) -> DecayTargets {
    let across_registers = |bass: f32, mid: f32, treble: f32| {
        interpolate_two_segments(
            frequency,
            (bass_hz, bass),
            (mid_hz, mid),
            (treble_hz, treble),
        )
    };
    let (fundamental_bass, fundamental_mid, fundamental_treble) = fundamental_targets;
    DecayTargets {
        fundamental: across_registers(fundamental_bass, fundamental_mid, fundamental_treble),
        mid_partial: across_registers(
            BASS_MID_PARTIAL_DECAY_SECONDS,
            MID_MID_PARTIAL_DECAY_SECONDS,
            TREBLE_MID_PARTIAL_DECAY_SECONDS,
        ),
        brightness: across_registers(
            BASS_BRIGHTNESS_DECAY_SECONDS,
            MID_BRIGHTNESS_DECAY_SECONDS,
            TREBLE_BRIGHTNESS_DECAY_SECONDS,
        ),
    }
}

/// Solves a string at `frequency` for the `(pole, zero_mix, sustain)` that
/// comes closest to `targets` — three unknowns against three per-partial
/// decay times.
///
/// # The shape of the problem
///
/// After `n` round trips a partial's amplitude has shrunk by
/// `(sustain·g)^n`, where `g` is [`LoopFilter::magnitude_at`]'s own gain
/// *at that partial's* frequency, and a string completes
/// `frequency · seconds` round trips — the loop period is the
/// fundamental's, whatever partial is riding on it. Writing losses in
/// nepers (`L = −ln g`, `s = −ln sustain`) turns that into one linear
/// equation per partial, `s + Lₙ = Dₙ`, with `Dₙ` what [`required_losses`]
/// derives from the target time. `Lₙ` depends non-linearly on the two
/// filter coefficients; `s` enters every equation identically, which is
/// exactly why it had to join the solve — the loop filter's DC gain is 1 by
/// construction, so `L₁ ≈ 0` and without `s` there is nothing left to
/// attenuate a fundamental with.
///
/// # Why a search rather than a closed form
///
/// The three equations have no exact solution in general: near DC a pole
/// and a zero both attenuate as `f²`, so the two coefficients are close to
/// collinear there and the fit is a genuine least squares (see
/// [`PARTIAL_WEIGHTS`]). Given a candidate filter, though, the best `s`
/// does have a closed form ([`fit_broadband_loss`]), so only two dimensions
/// are actually searched: a coarse sweep of the normalised plane followed
/// by [`SEARCH_REFINEMENTS`] halvings around the best point. Bounded
/// iterations, no early exit, and a defined answer for every input —
/// including a `NaN` frequency, which flows through [`math::clamp_or_low`]'s
/// and [`LoopFilter::magnitude_at`]'s own fallbacks rather than panicking.
///
/// This runs at voice construction ([`crate::engine::Engine::new`]) and in
/// `piano-studio`'s resolver, never on the audio thread.
fn solve_loop_losses(frequency: f32, targets: DecayTargets, sample_rate: SampleRate) -> LoopLosses {
    let partials = solved_partials(frequency, sample_rate);
    let required = required_losses(frequency, targets, partials);
    let residual_at = |pole_axis: f32, zero_axis: f32| {
        let losses = filter_losses(pole_axis, zero_axis, frequency, partials, sample_rate);
        fit_broadband_loss(losses, required).1
    };

    let mut best = sweep(SearchBox::whole_plane(), COARSE_SEARCH_POINTS, &residual_at);
    for _ in 0..SEARCH_REFINEMENTS {
        best = sweep(best, REFINEMENT_POINTS, &residual_at);
    }

    let losses = filter_losses(
        best.pole_axis,
        best.zero_axis,
        frequency,
        partials,
        sample_rate,
    );
    let broadband_loss = fit_broadband_loss(losses, required).0;
    LoopLosses {
        pole: pole_for_axis(best.pole_axis),
        zero_mix: zero_mix_for_axis(best.zero_axis),
        sustain: math::clamp_or_low(math::exp(-broadband_loss), 0.0, 1.0),
    }
}

/// Evaluates `points`x`points` candidates inside `bounds` and returns the
/// best one as a box half the size, ready for the next sweep.
///
/// A `NaN` residual never wins: the comparison is `<`, which is false
/// against `NaN`, so a pathological candidate leaves the incumbent alone
/// and a sweep in which *every* candidate is `NaN` returns `bounds`' own
/// centre rather than an undefined coordinate.
fn sweep(bounds: SearchBox, points: u32, residual_at: &impl Fn(f32, f32) -> f32) -> SearchBox {
    let mut best = SearchBox {
        half_width: bounds.half_width * 0.5,
        ..bounds
    };
    let mut lowest = f32::INFINITY;
    for pole_step in 0..points {
        let pole_axis = bounds.sample(bounds.pole_axis, pole_step, points);
        for zero_step in 0..points {
            let zero_axis = bounds.sample(bounds.zero_axis, zero_step, points);
            let residual = residual_at(pole_axis, zero_axis);
            if residual < lowest {
                lowest = residual;
                best.pole_axis = pole_axis;
                best.zero_axis = zero_axis;
            }
        }
    }
    best
}

/// Which three partials this key's loop is fitted against.
///
/// The fundamental always, then [`BRIGHTNESS_PARTIAL`] and the geometric
/// middle between the two — except where [`BRIGHTNESS_PARTIAL`] would land
/// above [`HIGHEST_SOLVED_PARTIAL_FRACTION`] of the sample rate, in which
/// case the highest partial that still fits takes its place and the middle
/// follows it down. At 48 kHz that binds from about A6 upwards: C8 is
/// fitted at partials 1, 2.3 and 5.2, because its 8th would sit at 33 kHz.
/// The targets are a continuous function of the partial index
/// ([`DecayTargets::seconds_for_partial`]), so a fractional partial needs
/// no special case.
fn solved_partials(frequency: f32, sample_rate: SampleRate) -> [f32; 3] {
    let highest_hz = sample_rate.hertz() * HIGHEST_SOLVED_PARTIAL_FRACTION;
    let brightness = math::clamp_or_low(
        highest_hz / frequency,
        MIN_SOLVED_PARTIAL,
        BRIGHTNESS_PARTIAL,
    );
    [1.0, math::sqrt(brightness), brightness]
}

/// The per-round-trip loss, in nepers, each of `partials`' own target decay
/// time asks the *whole* loop for — filter and `sustain` together.
///
/// Solving `g^(frequency · seconds) = SILENCE_THRESHOLD` for `−ln g` is
/// where the closed form comes from; `frequency` rather than the partial's
/// own frequency, because round trips are counted in loop periods.
fn required_losses(frequency: f32, targets: DecayTargets, partials: [f32; 3]) -> [f32; 3] {
    partials.map(|partial| {
        let seconds = targets.seconds_for_partial(partial).max(MIN_TARGET_SECONDS);
        let round_trips = (frequency * seconds).max(1.0);
        -math::ln(SILENCE_THRESHOLD) / round_trips
    })
}

/// Each of `partials`' per-round-trip loss, in nepers, through the loop
/// filter that the search coordinates `(pole_axis, zero_axis)` describe.
fn filter_losses(
    pole_axis: f32,
    zero_axis: f32,
    frequency: f32,
    partials: [f32; 3],
    sample_rate: SampleRate,
) -> [f32; 3] {
    let filter = LoopFilter::new(pole_for_axis(pole_axis), zero_mix_for_axis(zero_axis));
    partials.map(|partial| {
        let magnitude = filter.magnitude_at(frequency * partial, sample_rate.hertz());
        -math::ln(magnitude.max(MIN_SOLVED_MAGNITUDE))
    })
}

/// The broadband loss `s` that best completes `losses` toward `required`,
/// and the weighted residual it leaves behind.
///
/// `s` is the only unknown that attenuates a fundamental, so for a fixed
/// filter the fit is a weighted least squares in one variable and has a
/// closed form: minimising `Σ wₙ·((s + Lₙ)/Dₙ − 1)²` — squared *relative*
/// error, so a bass target of 35 s and a treble one of 0.3 s carry
/// comparable pull — gives `s = Σ (wₙ/Dₙ²)(Dₙ − Lₙ) / Σ (wₙ/Dₙ²)`.
fn fit_broadband_loss(losses: [f32; 3], required: [f32; 3]) -> (f32, f32) {
    let mut weight_sum = 0.0f32;
    let mut weighted_deficit = 0.0f32;
    for ((&loss, &target), &weight) in losses
        .iter()
        .zip(required.iter())
        .zip(PARTIAL_WEIGHTS.iter())
    {
        let scale = weight / (target * target).max(f32::MIN_POSITIVE);
        weight_sum += scale;
        weighted_deficit += scale * (target - loss);
    }
    let broadband_loss = math::clamp_or_low(
        weighted_deficit / weight_sum.max(f32::MIN_POSITIVE),
        0.0,
        MAX_BROADBAND_LOSS,
    );
    (
        broadband_loss,
        weighted_residual(broadband_loss, losses, required),
    )
}

/// The weighted sum of squared relative errors `broadband_loss` leaves —
/// [`fit_broadband_loss`]'s own objective, evaluated.
fn weighted_residual(broadband_loss: f32, losses: [f32; 3], required: [f32; 3]) -> f32 {
    losses
        .iter()
        .zip(required.iter())
        .zip(PARTIAL_WEIGHTS.iter())
        .map(|((&loss, &target), &weight)| {
            let error = (broadband_loss + loss) / target.max(f32::MIN_POSITIVE) - 1.0;
            weight * error * error
        })
        .sum()
}

/// Maps a normalised search coordinate onto the loop filter's pole.
///
/// The search does not run on the pole directly. A one-pole section's loss
/// at angular frequency `ω` is `½·ln(1 + a·sin²(ω/2))` with
/// `a = 4·pole/(1 − pole)²`, so it is `a`, not `pole`, that the loss is
/// proportional to at low frequency — and useful values of `a` span ten
/// decades between C8 (about `4e-5`) and A0 (about `5e2`), which a linear
/// sweep of `pole ∈ [0, 1]` cannot resolve at both ends at once. The
/// coordinate is therefore geometric in `a`, between [`MIN_POLE_SHAPE`] and
/// [`MAX_POLE_SHAPE`].
///
/// Inverting `a = 4p/(1 − p)²` on the stable branch gives
/// `p = (√(a+1) − 1)/(√(a+1) + 1)`. That form cancels to exactly zero once
/// `a` drops below `f32`'s epsilon — the whole treble end of the search —
/// so it is rewritten here through `√(a+1) − 1 = a/(√(a+1) + 1)` into
/// `p = a/(√(a+1) + 1)²`, which stays accurate all the way down to
/// [`MIN_POLE_SHAPE`].
fn pole_for_axis(axis: f32) -> f32 {
    let low = math::ln(MIN_POLE_SHAPE);
    let high = math::ln(MAX_POLE_SHAPE);
    let shape = math::exp(low + math::clamp_or_low(axis, 0.0, 1.0) * (high - low));
    let root_plus_one = math::sqrt(shape + 1.0) + 1.0;
    math::clamp_or_low(shape / (root_plus_one * root_plus_one), 0.0, 1.0)
}

/// Maps a normalised search coordinate onto the loop filter's zero mix.
///
/// Same idea as [`pole_for_axis`]: the averaging zero's loss is
/// `−½·ln(1 − b·sin²(ω/2))` with `b = 4·zero_mix·(1 − zero_mix)`, so `b` is
/// what the loss is linear in. Unlike the pole's coefficient it is already
/// bounded — `b ∈ [0, 1]` covers `zero_mix ∈ [0, 0.5]`, the whole range
/// [`LoopFilter::new`] accepts — so the axis maps onto it directly.
/// Inverting gives `zero_mix = (1 − √(1 − b))/2`, written as
/// `b / (2·(1 + √(1 − b)))` to avoid the same cancellation near zero.
fn zero_mix_for_axis(axis: f32) -> f32 {
    let shape = math::clamp_or_low(axis, 0.0, 1.0);
    shape / (2.0 * (1.0 + math::sqrt((1.0 - shape).max(0.0))))
}

/// Builds a [`StringConfig`] for `key` under `tuning`, its voicing fields
/// set from [`voicing_for_key`] rather than left at
/// [`StringConfig::new`]'s single global default.
#[must_use]
pub fn config_for_key(key: PianoKey, tuning: Tuning, sample_rate: SampleRate) -> StringConfig {
    let voicing = voicing_for_key(key, tuning, sample_rate);
    let mut config = StringConfig::new(key.frequency(tuning));
    config.damping = voicing.damping;
    config.sustain = voicing.sustain;
    config.inharmonicity = voicing.inharmonicity;
    config.loop_zero_mix = voicing.zero_mix;
    config
}

/// `midi`'s frequency under `tuning`, falling back to the tuning's own
/// reference pitch in the unreachable case that `midi` is not a valid piano
/// key — keeps every caller here total without needing to plumb a
/// `Result` through a construction-time table lookup.
fn anchor_hz(midi: u8, tuning: Tuning) -> f32 {
    PianoKey::from_midi(midi).map_or_else(
        |_| tuning.reference().hertz(),
        |key| key.frequency(tuning).hertz(),
    )
}

/// Linear interpolation of `value` between `(low_hz, low_value)` and
/// `(high_hz, high_value)`, positioned by `frequency`'s place in
/// **log**-frequency space — pitch, and therefore register, is
/// logarithmic, so this is what makes the interpolation land on each
/// octave evenly rather than bunching every change into the bass.
///
/// [`DecayTargets::seconds_for_partial`] reuses it for a partial index
/// rather than a frequency, which is the same geometry: partials, like
/// octaves, are spaced multiplicatively.
fn interpolate_log_frequency(
    frequency: f32,
    low_hz: f32,
    low_value: f32,
    high_hz: f32,
    high_value: f32,
) -> f32 {
    let low_log = math::ln(low_hz);
    let high_log = math::ln(high_hz);
    let span = high_log - low_log;
    if span <= 0.0 {
        return low_value;
    }
    let position = math::clamp_or_low((math::ln(frequency) - low_log) / span, 0.0, 1.0);
    low_value + position * (high_value - low_value)
}

/// Piecewise version of [`interpolate_log_frequency`] across three anchors
/// — bass-to-mid below the middle anchor, mid-to-treble above it — since a
/// single straight line cannot fit three independently sourced points.
fn interpolate_two_segments(
    frequency: f32,
    low: (f32, f32),
    mid: (f32, f32),
    high: (f32, f32),
) -> f32 {
    if frequency <= mid.0 {
        interpolate_log_frequency(frequency, low.0, low.1, mid.0, mid.1)
    } else {
        interpolate_log_frequency(frequency, mid.0, mid.1, high.0, high.1)
    }
}

// Split into `voicing_tests.rs` to keep this file under the project's
// 500-line limit (`CONTRIBUTING.md`) — still compiles as `voicing::tests`.
#[cfg(test)]
#[path = "voicing_tests.rs"]
mod tests;
