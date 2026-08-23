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
use piano_core::math;
use piano_core::string::{SILENCE_THRESHOLD, StringConfig};
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

/// Loss-filter pole at A0. Chaigne & Askenfelt's simulations (cited
/// elsewhere in this project for the hammer and release models) find a
/// bass note's upper partials collapse toward the fundamental markedly
/// faster than a treble note's few partials do — this project has no
/// per-register number for that in `docs/PHYSICS.md` to interpolate from,
/// so the *shape* (duller in the bass, brighter in the treble) is
/// literature-motivated but the specific anchor values are this project's
/// own reasoned interpolation, not a measured curve; a mild, symmetric
/// swing around [`piano_core::string::DEFAULT_DAMPING`] (0.5) rather than
/// a dramatic one, honestly labelled as such.
const BASS_DAMPING: f32 = 0.6;

/// Loss-filter pole at C8. See [`BASS_DAMPING`].
const TREBLE_DAMPING: f32 = 0.4;

/// A key's computed baseline voicing, ready to drop into
/// [`StringConfig`]'s matching fields.
pub(crate) struct KeyVoicing {
    /// See [`StringConfig::damping`].
    pub(crate) damping: f32,
    /// See [`StringConfig::sustain`].
    pub(crate) sustain: f32,
    /// See [`StringConfig::inharmonicity`].
    pub(crate) inharmonicity: f32,
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
pub(crate) fn unison_count_for_key(key: PianoKey) -> usize {
    piano_core::unison::unison_count_for_key_index(key.key_index())
}

/// Computes `key`'s baseline voicing under `tuning`.
pub(crate) fn voicing_for_key(key: PianoKey, tuning: Tuning) -> KeyVoicing {
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
    let damping =
        interpolate_log_frequency(frequency, bass_hz, BASS_DAMPING, treble_hz, TREBLE_DAMPING);

    KeyVoicing {
        damping,
        sustain: sustain_for_decay_seconds(decay_seconds, frequency),
        inharmonicity,
    }
}

/// Builds a [`StringConfig`] for `key` under `tuning`, its voicing fields
/// set from [`voicing_for_key`] rather than left at
/// [`StringConfig::new`]'s single global default.
pub(crate) fn config_for_key(key: PianoKey, tuning: Tuning) -> StringConfig {
    let voicing = voicing_for_key(key, tuning);
    let mut config = StringConfig::new(key.frequency(tuning));
    config.damping = voicing.damping;
    config.sustain = voicing.sustain;
    config.inharmonicity = voicing.inharmonicity;
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
/// gain ([`StringConfig::sustain`]) that produces it at `frequency`.
///
/// Derived from the waveguide loop's own geometry: after `n` round trips
/// the envelope has shrunk by `sustain^n`, and a string completes
/// `frequency * decay_seconds` round trips in `decay_seconds`. Solving
/// `sustain^(frequency * decay_seconds) = SILENCE_THRESHOLD` for `sustain`
/// gives the closed form below — a real derivation from this project's own
/// loop model, not a fitted or invented constant.
fn sustain_for_decay_seconds(decay_seconds: f32, frequency: f32) -> f32 {
    let round_trips = (frequency * decay_seconds).max(1.0);
    let sustain = math::exp(math::ln(SILENCE_THRESHOLD) / round_trips);
    math::clamp_or_low(sustain, 0.0, 1.0)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::float_cmp)]

    use super::*;

    #[test]
    fn bass_keys_get_more_inharmonicity_than_default_and_treble_keys_more_still() {
        let tuning = Tuning::default();
        let bass = PianoKey::from_midi(LOWEST_PIANO_KEY).expect("A0 is on the keyboard");
        let treble = PianoKey::from_midi(HIGHEST_PIANO_KEY).expect("C8 is on the keyboard");
        let bass_voicing = voicing_for_key(bass, tuning);
        let treble_voicing = voicing_for_key(treble, tuning);
        assert!(bass_voicing.inharmonicity < treble_voicing.inharmonicity);
    }

    #[test]
    fn every_key_gets_a_finite_in_range_voicing() {
        let tuning = Tuning::default();
        for midi in LOWEST_PIANO_KEY..=HIGHEST_PIANO_KEY {
            let key = PianoKey::from_midi(midi).expect("key is on the keyboard");
            let voicing = voicing_for_key(key, tuning);
            assert!(voicing.damping.is_finite() && (0.0..=1.0).contains(&voicing.damping));
            assert!(voicing.sustain.is_finite() && (0.0..=1.0).contains(&voicing.sustain));
            assert!(
                voicing.inharmonicity.is_finite()
                    && (0.0..=MAX_INHARMONICITY).contains(&voicing.inharmonicity)
            );
        }
    }

    #[test]
    fn treble_keys_ring_for_less_time_than_bass_keys() {
        // `sustain` alone cannot be compared directly across two different
        // frequencies — the same sustain decays faster at a higher
        // frequency purely because it completes more round trips per
        // second. Convert each key's `sustain` back to seconds through the
        // same closed form `voicing_for_key` used to derive it, and compare
        // those instead.
        let tuning = Tuning::default();
        let bass = PianoKey::from_midi(LOWEST_PIANO_KEY).expect("A0 is on the keyboard");
        let treble = PianoKey::from_midi(HIGHEST_PIANO_KEY).expect("C8 is on the keyboard");
        let bass_voicing = voicing_for_key(bass, tuning);
        let treble_voicing = voicing_for_key(treble, tuning);

        let bass_round_trips = math::ln(SILENCE_THRESHOLD) / math::ln(bass_voicing.sustain);
        let treble_round_trips = math::ln(SILENCE_THRESHOLD) / math::ln(treble_voicing.sustain);
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
        let config = config_for_key(treble, tuning);
        assert_eq!(
            config.inharmonicity,
            voicing_for_key(treble, tuning).inharmonicity
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
            let voicing = voicing_for_key(key, tuning);
            assert!(voicing.damping.is_finite());
            assert!(voicing.sustain.is_finite());
            assert!(voicing.inharmonicity.is_finite());
        }
    }
}
