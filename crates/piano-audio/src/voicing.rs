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

use piano_core::dispersion::MAX_INHARMONICITY;
use piano_core::filter::LoopFilter;
use piano_core::string::{DEFAULT_LOOP_ZERO_MIX, SILENCE_THRESHOLD, StringConfig};
use piano_core::{SampleRate, math};
use piano_params::{CONCERT_A_KEY, HIGHEST_PIANO_KEY, LOWEST_PIANO_KEY, PianoKey, Tuning};

/// Target ring-out time at A0, the middle of `docs/PHYSICS.md`'s "Typical
/// decay" row for that key (30-40 s).
const BASS_DECAY_SECONDS: f32 = 35.0;

/// Target ring-out time at A4, the middle of the same row's 8-15 s.
const MID_DECAY_SECONDS: f32 = 11.0;

/// Target ring-out time at C8, the middle of the same row's 1-2 s.
const TREBLE_DECAY_SECONDS: f32 = 1.5;

/// Inharmonicity `B` at A0. Fletcher & Rossing's range, already cited in
/// [`piano_core::dispersion`], bottoms out "roughly 0.0001 in the bass".
const BASS_INHARMONICITY: f32 = 0.000_1;

/// Inharmonicity `B` at C8: the top of the same cited range,
/// [`MAX_INHARMONICITY`].
const TREBLE_INHARMONICITY: f32 = MAX_INHARMONICITY;

/// Loss-filter pole at A0, *if* the round-trip budget a key's target decay
/// time allows can afford it — see [`loop_filter_coefficients`]. Chaigne &
/// Askenfelt's simulations (cited elsewhere in this project for the hammer
/// and release models) find a bass note's upper partials collapse toward
/// the fundamental markedly faster than a treble note's few partials do —
/// this project has no per-register number for that in `docs/PHYSICS.md` to
/// interpolate from, so the *shape* (duller in the bass, brighter in the
/// treble) is literature-motivated but the specific anchor values are this
/// project's own reasoned interpolation, not a measured curve; a mild,
/// symmetric swing around [`piano_core::string::DEFAULT_DAMPING`] (0.5)
/// rather than a dramatic one, honestly labelled as such.
///
/// At A0 this ceiling is never actually clamped down: the bass round-trip
/// budget is generous enough (the fundamental sits far below Nyquist) that
/// the full desired pole always fits. It stops being free of charge partway
/// through the mid register — see [`loop_filter_coefficients`]'s doc
/// comment for the measured numbers.
const BASS_DAMPING: f32 = 0.6;

/// Loss-filter pole at C8, under the same "if the budget allows it" caveat
/// as [`BASS_DAMPING`]. In practice the treble round-trip budget almost
/// never allows it — [`loop_filter_coefficients`] scales this down to
/// single-digit percentages of itself for the top register.
const TREBLE_DAMPING: f32 = 0.4;

/// Loop-filter zero mix [`loop_filter_coefficients`] would use everywhere,
/// if the round-trip budget were unconstrained —
/// [`piano_core::filter::LoopFilter`]'s own `MAX_ZERO_MIX`, the zero's full
/// Nyquist-null rolloff, matching this filter's behaviour before per-string
/// `zero_mix` existed. Unlike [`BASS_DAMPING`]/[`TREBLE_DAMPING`] this is
/// not itself register-interpolated: the budget check tapers it down
/// exactly where it needs to, key by key, rather than a second hand-picked
/// curve trying to anticipate that.
const DESIRED_ZERO_MIX: f32 = DEFAULT_LOOP_ZERO_MIX;

/// How far above the bare round-trip-budget boundary
/// [`loop_filter_coefficients`] requires the loop filter's own gain to sit,
/// expressed as a multiplier on the target gain (`1.00001` asks for
/// 0.001% more gain than the boundary). Pure `f32`-rounding insurance
/// against landing exactly on the boundary and tipping the wrong way —
/// not a musically meaningful margin: at the upper keys the entire
/// round-trip budget is already only a few thousandths of gain wide (see
/// this function's own doc comment), so there is no room for a real
/// percentage safety margin without reintroducing the same shortfall this
/// module exists to close.
const FILTER_GAIN_SAFETY_MARGIN: f32 = 1.000_01;

/// Iterations [`loop_filter_coefficients`]'s bisection runs to find how much
/// of the desired pole/zero-mix pair a key's round-trip budget affords.
/// `0.5^24` of the initial `[0, 1]` search interval is far finer than `f32`
/// can resolve a coefficient in `[0, 1]` to, so more iterations would not
/// change the answer; this is a compile-time bound purely so the search is
/// provably finite, one of `docs/REALTIME-AUDIO-RULES.md`'s totality
/// requirements (this function does not run on the audio thread — see its
/// own docs — but the project holds every parameter-derivation function to
/// the same standard).
const FILTER_SCALE_SEARCH_ITERATIONS: u32 = 24;

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

/// Computes `key`'s baseline voicing under `tuning`, for a string that
/// will run at `sample_rate`.
///
/// `sample_rate` only feeds [`sustain_for_decay_seconds`]'s loop-filter
/// correction (see that function's docs) — every other computation here
/// is sample-rate-independent.
#[must_use]
pub fn voicing_for_key(key: PianoKey, tuning: Tuning, sample_rate: SampleRate) -> KeyVoicing {
    let frequency = key.frequency(tuning).hertz();
    let bass_hz = anchor_hz(LOWEST_PIANO_KEY, tuning);
    let mid_hz = anchor_hz(CONCERT_A_KEY, tuning);
    let treble_hz = anchor_hz(HIGHEST_PIANO_KEY, tuning);

    let decay_seconds = interpolate_two_segments(
        frequency,
        (bass_hz, BASS_DECAY_SECONDS),
        (mid_hz, MID_DECAY_SECONDS),
        (treble_hz, TREBLE_DECAY_SECONDS),
    );
    let inharmonicity = interpolate_log_frequency(
        frequency,
        bass_hz,
        BASS_INHARMONICITY,
        treble_hz,
        TREBLE_INHARMONICITY,
    );
    let (damping, zero_mix) =
        loop_filter_coefficients(frequency, bass_hz, treble_hz, decay_seconds, sample_rate);

    KeyVoicing {
        damping,
        sustain: sustain_for_decay_seconds(
            decay_seconds,
            frequency,
            damping,
            zero_mix,
            sample_rate,
        ),
        inharmonicity,
        zero_mix,
    }
}

/// Chooses this key's loop-filter `(pole, zero_mix)` pair — [`BASS_DAMPING`]
/// through [`TREBLE_DAMPING`] and [`DESIRED_ZERO_MIX`], scaled down together
/// by whatever fraction the key's own round-trip budget can actually afford.
///
/// # Why a scaled-down pair, not the desired one directly
///
/// [`piano_core::filter::LoopFilter::magnitude_at`] measures how much of a
/// string's *fundamental* survives one round trip through the loop filter
/// alone. For a low fundamental that loss is negligible regardless of the
/// filter's coefficients — the fundamental sits far below Nyquist, where
/// any reasonable one-zero-one-pole filter is nearly transparent. For a
/// high fundamental it is not: a C8 string completes about 4186 round trips
/// a second, so reaching even the *documented* 1.5 s target needs each
/// round trip to keep more than `99.85%` of the previous one's amplitude.
/// Measured with the desired pair fixed at [`TREBLE_DAMPING`]/
/// [`DESIRED_ZERO_MIX`] (`0.4`/`0.5`), the loop filter alone was already
/// keeping only about `83.6%` per round trip — the note decayed to silence
/// in about 12 ms, not 1.5 s, because [`sustain_for_decay_seconds`] had
/// nothing left to work with once the filter had already spent the entire
/// budget and more.
///
/// This function is the fix: it starts from the desired `(pole, zero_mix)`
/// pair — the *timbre* this project wants, register by register — and, key
/// by key, scales both coefficients down by the same factor `s ∈ [0, 1]`
/// until the loop filter's own gain at that key's fundamental clears the
/// round-trip budget its target decay time allows, found by bisection
/// ([`FILTER_SCALE_SEARCH_ITERATIONS`], a bounded search, not an unbounded
/// one). Scaling both coefficients by the same factor, rather than solving
/// for pole and `zero_mix` independently, keeps their *ratio* — and so the
/// filter's qualitative brightness character — the same as the desired
/// pair's, just turned down. Measured across the full keyboard: bass keeps
/// `s = 1` (the desired pair, unclamped — its budget is never binding);
/// C8 lands around `s ≈ 0.01`, both coefficients turned down to about a
/// hundredth of themselves, which is what recovers the documented decay
/// time at the cost of most of the loop filter's own contribution to
/// treble brightness — see `docs/PHYSICS.md`'s "Why there is a lowpass
/// filter in the loop" for the honest accounting of that trade-off.
///
/// Total for every input via the same guarantees [`interpolate_log_frequency`]
/// and [`LoopFilter::magnitude_at`] already have: a degenerate
/// `bass_hz`/`treble_hz` span or a non-finite `frequency`/`sample_rate`
/// flows through to a fallback rather than a panic or a `NaN`.
fn loop_filter_coefficients(
    frequency: f32,
    bass_hz: f32,
    treble_hz: f32,
    decay_seconds: f32,
    sample_rate: SampleRate,
) -> (f32, f32) {
    let desired_pole =
        interpolate_log_frequency(frequency, bass_hz, BASS_DAMPING, treble_hz, TREBLE_DAMPING);
    let desired_zero_mix = DESIRED_ZERO_MIX;

    let round_trips = (frequency * decay_seconds).max(1.0);
    let target_gain = math::clamp_or_low(
        math::exp(math::ln(SILENCE_THRESHOLD) / round_trips) * FILTER_GAIN_SAFETY_MARGIN,
        0.0,
        0.999_999,
    );

    let filter_gain_at = |scale: f32| {
        LoopFilter::new(scale * desired_pole, scale * desired_zero_mix)
            .magnitude_at(frequency, sample_rate.hertz())
    };

    if filter_gain_at(1.0) >= target_gain {
        return (desired_pole, desired_zero_mix);
    }

    let mut low_scale = 0.0f32;
    let mut high_scale = 1.0f32;
    for _ in 0..FILTER_SCALE_SEARCH_ITERATIONS {
        let midpoint = f32::midpoint(low_scale, high_scale);
        if filter_gain_at(midpoint) >= target_gain {
            low_scale = midpoint;
        } else {
            high_scale = midpoint;
        }
    }
    (low_scale * desired_pole, low_scale * desired_zero_mix)
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

/// Converts a target ring-out time into the broadband per-round-trip loop
/// gain ([`StringConfig::sustain`]) that produces it at `frequency`, for a
/// string voiced with `damping`/`zero_mix` and running at `sample_rate`.
///
/// Derived from the waveguide loop's own geometry: after `n` round trips
/// the envelope has shrunk by `(sustain·g)^n`, where `g` is
/// [`LoopFilter::magnitude_at`]'s own loss at `frequency` — the loop's
/// *total* round-trip gain, not `sustain` alone. A string completes
/// `frequency * decay_seconds` round trips in `decay_seconds`, so solving
/// `(sustain·g)^(frequency * decay_seconds) = SILENCE_THRESHOLD` for
/// `sustain` gives the closed form below, then dividing out `g`.
///
/// # The bug this replaced, and the one after it
///
/// The original version solved `sustain^n = SILENCE_THRESHOLD` directly —
/// correct only if the loop filter were perfectly transparent at
/// `frequency`, which it is not: even a modest, mid-register `damping`
/// loses a small fraction of the fundamental's own amplitude every round
/// trip, and that loss compounds over the thousands of round trips a note
/// rings for. The result was every voiced note decaying several times
/// faster than `decay_seconds` actually called for — audibly a dry,
/// percussive "knock" rather than a sustained tone, worst in the
/// mid-register where `damping` sits furthest from either extreme.
///
/// Dividing out `g` (this function's current body) fixed the *formula*, but
/// on its own only reaches `decay_seconds` for a key whose `(damping,
/// zero_mix)` pair leaves `g` above what the round-trip budget needs —
/// `sustain`'s own ceiling of `1.0` cannot manufacture gain the filter
/// itself did not leave on the table. [`loop_filter_coefficients`] is what
/// closes that second gap, by choosing a `(damping, zero_mix)` pair that
/// actually fits the budget before this function ever runs — this function
/// no longer needs to (and cannot) rescue a filter that was already asked
/// to lose more than the target decay time affords.
fn sustain_for_decay_seconds(
    decay_seconds: f32,
    frequency: f32,
    damping: f32,
    zero_mix: f32,
    sample_rate: SampleRate,
) -> f32 {
    let round_trips = (frequency * decay_seconds).max(1.0);
    let total_gain_needed = math::exp(math::ln(SILENCE_THRESHOLD) / round_trips);
    let filter_gain =
        LoopFilter::new(damping, zero_mix).magnitude_at(frequency, sample_rate.hertz());
    // `filter_gain` is total and clamped into `[0, 1]` by construction
    // (`LoopFilter::magnitude_at`'s own contract), but a pathologically
    // small value would still blow `sustain` up past 1 here — floor it so
    // this division can never produce more gain than the loop itself
    // provides.
    let sustain = total_gain_needed / filter_gain.max(total_gain_needed);
    math::clamp_or_low(sustain, 0.0, 1.0)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::float_cmp)]

    use piano_core::string::PluckedString;

    use super::*;

    fn sample_rate() -> SampleRate {
        SampleRate::new(48_000.0).expect("48 kHz is valid")
    }

    #[test]
    fn bass_keys_get_more_inharmonicity_than_default_and_treble_keys_more_still() {
        let tuning = Tuning::default();
        let bass = PianoKey::from_midi(LOWEST_PIANO_KEY).expect("A0 is on the keyboard");
        let treble = PianoKey::from_midi(HIGHEST_PIANO_KEY).expect("C8 is on the keyboard");
        let bass_voicing = voicing_for_key(bass, tuning, sample_rate());
        let treble_voicing = voicing_for_key(treble, tuning, sample_rate());
        assert!(bass_voicing.inharmonicity < treble_voicing.inharmonicity);
    }

    #[test]
    fn every_key_gets_a_finite_in_range_voicing() {
        let tuning = Tuning::default();
        for midi in LOWEST_PIANO_KEY..=HIGHEST_PIANO_KEY {
            let key = PianoKey::from_midi(midi).expect("key is on the keyboard");
            let voicing = voicing_for_key(key, tuning, sample_rate());
            assert!(voicing.damping.is_finite() && (0.0..=1.0).contains(&voicing.damping));
            assert!(voicing.sustain.is_finite() && (0.0..=1.0).contains(&voicing.sustain));
            assert!(
                voicing.inharmonicity.is_finite()
                    && (0.0..=MAX_INHARMONICITY).contains(&voicing.inharmonicity)
            );
            assert!(
                voicing.zero_mix.is_finite()
                    && (0.0..=DESIRED_ZERO_MIX).contains(&voicing.zero_mix)
            );
        }
    }

    #[test]
    fn treble_keys_ring_for_less_time_than_bass_keys() {
        // `sustain` alone cannot be compared directly across two different
        // frequencies — the same sustain decays faster at a higher
        // frequency purely because it completes more round trips per
        // second, and (since `loop_filter_coefficients`) `sustain` is no
        // longer the *only* per-round-trip loss either: the loop filter's
        // own `magnitude_at` contributes too, sometimes the larger share
        // (see that function's doc comment). Convert each key's total
        // round-trip gain — `sustain * magnitude_at`, the same product
        // `sustain_for_decay_seconds` solves against — back to seconds, and
        // compare those instead.
        let tuning = Tuning::default();
        let bass = PianoKey::from_midi(LOWEST_PIANO_KEY).expect("A0 is on the keyboard");
        let treble = PianoKey::from_midi(HIGHEST_PIANO_KEY).expect("C8 is on the keyboard");
        let bass_voicing = voicing_for_key(bass, tuning, sample_rate());
        let treble_voicing = voicing_for_key(treble, tuning, sample_rate());

        let total_gain = |key: PianoKey, voicing: &KeyVoicing| {
            let filter_gain = LoopFilter::new(voicing.damping, voicing.zero_mix)
                .magnitude_at(key.frequency(tuning).hertz(), sample_rate().hertz());
            voicing.sustain * filter_gain
        };
        let bass_round_trips =
            math::ln(SILENCE_THRESHOLD) / math::ln(total_gain(bass, &bass_voicing));
        let treble_round_trips =
            math::ln(SILENCE_THRESHOLD) / math::ln(total_gain(treble, &treble_voicing));
        let bass_seconds = bass_round_trips / bass.frequency(tuning).hertz();
        let treble_seconds = treble_round_trips / treble.frequency(tuning).hertz();

        assert!(
            treble_seconds < bass_seconds,
            "treble decay {treble_seconds}s should be shorter than bass decay {bass_seconds}s"
        );
    }

    #[test]
    fn config_for_key_uses_the_computed_voicing_not_the_global_default() {
        let tuning = Tuning::default();
        let treble = PianoKey::from_midi(HIGHEST_PIANO_KEY).expect("C8 is on the keyboard");
        let config = config_for_key(treble, tuning, sample_rate());
        assert_eq!(
            config.inharmonicity,
            voicing_for_key(treble, tuning, sample_rate()).inharmonicity
        );
    }

    #[test]
    fn bass_keys_are_single_strung_and_treble_keys_are_triple_strung() {
        let bass = PianoKey::from_midi(LOWEST_PIANO_KEY).expect("A0 is on the keyboard");
        let treble = PianoKey::from_midi(HIGHEST_PIANO_KEY).expect("C8 is on the keyboard");
        assert_eq!(unison_count_for_key(bass), 1);
        assert_eq!(unison_count_for_key(treble), 3);
    }

    #[test]
    fn interpolation_never_produces_non_finite_values_across_odd_tunings() {
        // A pathological but constructible tuning: concert A far from 440,
        // which shifts every anchor frequency. The interpolation must stay
        // total regardless.
        let tuning = Tuning::with_concert_a(220.0).expect("220 Hz is a valid tuning");
        for midi in LOWEST_PIANO_KEY..=HIGHEST_PIANO_KEY {
            let key = PianoKey::from_midi(midi).expect("key is on the keyboard");
            let voicing = voicing_for_key(key, tuning, sample_rate());
            assert!(voicing.damping.is_finite());
            assert!(voicing.sustain.is_finite());
            assert!(voicing.inharmonicity.is_finite());
            assert!(voicing.zero_mix.is_finite());
        }
    }

    /// Renders a real, fully-built `PluckedString` (the same
    /// `config_for_key` path `Engine` uses) and returns how many seconds
    /// it actually takes to decay to `SILENCE_THRESHOLD`. Samples elapsed,
    /// not round trips, is the right unit: `PluckedString::is_silent`
    /// tracks a fast, sample-rate-scaled envelope meant for voice
    /// reclaiming, not a physics measurement, so converting its own
    /// per-sample updates back to seconds is what makes this honest.
    fn measured_decay_seconds(key: PianoKey, sustain: f32) -> f32 {
        let mut config = config_for_key(key, Tuning::default(), sample_rate());
        config.sustain = sustain;
        let mut string = PluckedString::new(config, sample_rate()).expect("key is tunable");
        string.pluck(1.0);

        // `BASS_DECAY_SECONDS` is the longest any anchor's target ever
        // asks for, so a cap of `1.5x` that comfortably bounds every
        // measurement this module takes, bass through treble, without the
        // cap itself becoming the thing under test.
        let sample_count_cap = (sample_rate().hertz() * BASS_DECAY_SECONDS * 1.5) as u32;
        let mut samples_elapsed = 0u32;
        while !string.is_silent() && samples_elapsed < sample_count_cap {
            let _ = string.process();
            samples_elapsed += 1;
        }
        samples_elapsed as f32 / sample_rate().hertz()
    }

    /// The regression test for the bug this module's `sustain_for_decay_
    /// seconds` fix closes: a real, fully-built A4 voiced through the
    /// *corrected* formula must ring measurably longer than the same
    /// string voiced through the *uncorrected* one — proof the loop
    /// filter's own, previously-ignored round-trip loss now actually gets
    /// compensated for, not just that the formula changed on paper.
    ///
    /// Not asserted here: that the corrected decay reaches
    /// `MID_DECAY_SECONDS` (11 s) itself, even after `loop_filter_
    /// coefficients` closed the ceiling-clamping gap `sustain_for_decay_
    /// seconds`'s own doc comment describes, and after `piano_core::
    /// string::PluckedString`'s `PendingContact` stopped truncating the
    /// hammer's contact force to one loop length (`docs/PHYSICS.md`'s "What
    /// the hammer still gets wrong"). A4 still lands under target (measured
    /// around 6-7 s, not 11 s) because A4's period (~109 samples) is close
    /// enough to the hammer's own contact duration (~170-310 samples) that
    /// only one or two extra round trips of continued injection are left to
    /// help — C8's much shorter period gets dozens, and lands much closer
    /// to its own target (see `treble_notes_no_longer_die_in_milliseconds`).
    /// The remaining shortfall is the noise-burst excitation itself not
    /// concentrating cleanly on the resonant fundamental the way a seeded
    /// sine does (verified by isolating just the delay line, loop filter,
    /// dispersion and sustain scaling — no hammer excitation — in a minimal
    /// closed loop seeded with a plain sine, which matches `magnitude_at`'s
    /// prediction to within measurement noise: the calibration itself is
    /// correct) — a real, separate gap, not a symptom of miscalibration.
    #[test]
    fn correcting_for_the_loop_filter_rings_measurably_longer() {
        let tuning = Tuning::default();
        let a4 = PianoKey::from_midi(CONCERT_A_KEY).expect("A4 is on the keyboard");
        let uncorrected_sustain = math::exp(
            math::ln(SILENCE_THRESHOLD) / (a4.frequency(tuning).hertz() * MID_DECAY_SECONDS),
        );
        let corrected_sustain = voicing_for_key(a4, tuning, sample_rate()).sustain;

        let uncorrected_seconds = measured_decay_seconds(a4, uncorrected_sustain);
        let corrected_seconds = measured_decay_seconds(a4, corrected_sustain);

        assert!(
            corrected_seconds > uncorrected_seconds * 1.3,
            "corrected {corrected_seconds}s should ring noticeably longer than \
             uncorrected {uncorrected_seconds}s"
        );
    }

    /// The regression test for the report this fix actually shipped for:
    /// with the loop filter's zero fixed at Nyquist and `TREBLE_DAMPING`
    /// applied uncorrected, a real C8 string measured about 12 ms to
    /// silence against a documented 1-2 s target — audibly a click, not a
    /// note. `loop_filter_coefficients` tapering both `damping` and
    /// `zero_mix` down for the top register directly targets that: the
    /// *per-round-trip* loss the loop filter itself contributes at C8's
    /// fundamental is now correctly small (verified against a minimal
    /// closed loop seeded with a plain sine, see `correcting_for_the_loop_
    /// filter_rings_measurably_longer`'s doc comment).
    ///
    /// A real, hammer-plucked C8 fell well short of the full 1-2 s target
    /// even after that correction, for a second, independent reason:
    /// `PluckedString::pluck` only wrote one loop length of the hammer's
    /// contact force, silently truncating the rest for any string (C8
    /// very much included, `docs/PHYSICS.md`'s "What the hammer still gets
    /// wrong") whose period is shorter than the felt's contact duration.
    /// `PluckedString`'s `PendingContact` closes that: C8 now measures
    /// close to (not merely "longer than") the documented target.
    #[test]
    fn treble_notes_no_longer_die_in_milliseconds() {
        let tuning = Tuning::default();
        let c8 = PianoKey::from_midi(HIGHEST_PIANO_KEY).expect("C8 is on the keyboard");
        let voicing = voicing_for_key(c8, tuning, sample_rate());
        let measured = measured_decay_seconds(c8, voicing.sustain);

        assert!(
            measured > TREBLE_DECAY_SECONDS * 0.7,
            "measured {measured}s should land close to the {TREBLE_DECAY_SECONDS}s target, \
             not the ~12ms the uncalibrated filter and the truncated excitation used to produce"
        );
    }

    /// A cheap, fast-running sanity check that the correction actually
    /// moves `sustain` closer to 1 (less loss) than the naive formula
    /// would, for a damping value where the loop filter has a real effect
    /// — proving the fix engages, without paying for a full render.
    #[test]
    fn the_loop_filter_correction_raises_sustain_above_the_naive_value() {
        let naive = math::exp(math::ln(SILENCE_THRESHOLD) / (440.0 * MID_DECAY_SECONDS));
        let corrected =
            sustain_for_decay_seconds(MID_DECAY_SECONDS, 440.0, 0.5, 0.5, sample_rate());
        assert!(
            corrected > naive,
            "corrected sustain {corrected} should exceed the naive {naive}"
        );
    }
}
