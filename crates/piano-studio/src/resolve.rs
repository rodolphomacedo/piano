//! Turns a sparse [`PianoFile`] into the flat per-string table
//! `piano_audio::AudioSession`'s live setters actually consume —
//! `docs/PARAMETER-STUDIO.md`'s cascade, resolved once.

use piano_audio::voicing::{unison_count_for_key, voicing_for_key};
use piano_core::dispersion::DEFAULT_INHARMONICITY;
use piano_core::hammer::{DEFAULT_HAMMER, HammerConfig};
use piano_core::soundboard::SoundboardMode;
use piano_core::string::{DEFAULT_DAMPING, DEFAULT_SUSTAIN};
use piano_params::{HIGHEST_PIANO_KEY, LOWEST_PIANO_KEY, PianoKey, Tuning};

use crate::format::{HammerOverrides, ParameterOverrides, PianoFile};

/// Default detune a string resolves to when nothing overrides it — "no
/// detune" in the *absolute* cents-from-base-frequency sense
/// [`piano_core::UnisonGroup::set_string_detune`] uses, not `unison.rs`'s
/// separate construction-time per-position offset.
const DEFAULT_DETUNE_CENTS: f32 = 0.0;

/// Default excitation seed a string resolves to when nothing overrides it.
const DEFAULT_SEED: u32 = 0;

/// Mirrors `piano_core::unison`'s private `DEFAULT_LOCAL_COUPLING_GAIN` —
/// duplicated, not imported, because that constant is deliberately not
/// part of `piano-core`'s public API. Matches
/// `docs/PARAMETER-STUDIO.md`'s own JSON example value.
const DEFAULT_LOCAL_COUPLING_GAIN: f32 = 0.15;

/// Mirrors `piano_core::unison`'s private `DEFAULT_GLOBAL_COUPLING_GAIN`.
/// See [`DEFAULT_LOCAL_COUPLING_GAIN`].
const DEFAULT_GLOBAL_COUPLING_GAIN: f32 = 0.08;

/// One string's fully resolved parameters, ready to drive
/// `piano_audio::AudioSession`'s per-string setters.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResolvedString {
    /// MIDI note number of the key this string belongs to.
    pub midi: u8,
    /// Which string within that key's unison, `0`-based.
    pub string_index: u8,
    /// See [`piano_core::string::PluckedString::set_damping`].
    pub damping: f32,
    /// See [`piano_core::string::PluckedString::set_sustain`].
    pub sustain: f32,
    /// See [`piano_core::string::PluckedString::set_inharmonicity`].
    pub inharmonicity: f32,
    /// See [`piano_core::UnisonGroup::set_string_detune`].
    pub detune_cents: f32,
    /// See [`piano_core::string::PluckedString::set_seed`].
    pub seed: u32,
    /// See [`piano_core::string::PluckedString::set_hammer`].
    pub hammer: HammerConfig,
}

/// A whole piano's fully resolved live state: every string plus the
/// instrument-wide parameters.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedPiano {
    /// Every string on the instrument, resolved.
    pub strings: Vec<ResolvedString>,
    /// Soundboard modes to apply, in mode-index order. Shorter than
    /// [`piano_core::soundboard::MODE_COUNT`] leaves the remaining modes
    /// at whatever the running engine already has.
    pub soundboard_modes: Vec<SoundboardMode>,
    /// See [`piano_core::UnisonGroup::set_local_coupling_gain`].
    pub local_coupling_gain: f32,
    /// See [`piano_core::UnisonGroup::set_global_coupling_gain`].
    pub global_coupling_gain: f32,
}

/// A cascade tier's contribution, applied over whatever came before it —
/// `f32`/`u32`/[`HammerConfig`] rather than [`ParameterOverrides`]'s
/// `Option`s, since by the time this exists every field already has a
/// concrete value from a less specific tier.
#[derive(Debug, Clone, Copy)]
struct Resolved {
    damping: f32,
    sustain: f32,
    inharmonicity: f32,
    detune_cents: f32,
    seed: u32,
    hammer: HammerConfig,
}

/// Resolves `file` into a flat per-string table under `tuning` — the
/// cascade `docs/PARAMETER-STUDIO.md` describes: `defaults` < `registers`
/// < `groups` < `strings`, most specific wins.
#[must_use]
pub fn resolve(file: &PianoFile, tuning: Tuning) -> ResolvedPiano {
    let mut strings = Vec::new();
    for midi in LOWEST_PIANO_KEY..=HIGHEST_PIANO_KEY {
        let Ok(key) = PianoKey::from_midi(midi) else {
            continue;
        };
        for string_index in 0..unison_count_for_key(key) {
            strings.push(resolve_string(file, key, tuning, string_index as u8));
        }
    }
    ResolvedPiano {
        strings,
        soundboard_modes: file
            .instrument
            .soundboard_modes
            .iter()
            .map(|mode| SoundboardMode {
                frequency_hz: mode.frequency_hz,
                decay_seconds: mode.decay_seconds,
                gain: mode.gain,
            })
            .collect(),
        local_coupling_gain: file
            .instrument
            .bridge
            .local_coupling_gain
            .unwrap_or(DEFAULT_LOCAL_COUPLING_GAIN),
        global_coupling_gain: file
            .instrument
            .bridge
            .global_coupling_gain
            .unwrap_or(DEFAULT_GLOBAL_COUPLING_GAIN),
    }
}

/// Resolves one string through all four cascade tiers, in
/// least-to-most-specific order.
fn resolve_string(
    file: &PianoFile,
    key: PianoKey,
    tuning: Tuning,
    string_index: u8,
) -> ResolvedString {
    let midi = key.midi_number();
    let mut resolved = Resolved {
        damping: file.defaults.damping.unwrap_or(DEFAULT_DAMPING),
        sustain: file.defaults.sustain.unwrap_or(DEFAULT_SUSTAIN),
        inharmonicity: file.defaults.inharmonicity.unwrap_or(DEFAULT_INHARMONICITY),
        detune_cents: file.defaults.detune_cents.unwrap_or(DEFAULT_DETUNE_CENTS),
        seed: file.defaults.seed.unwrap_or(DEFAULT_SEED),
        hammer: resolve_hammer(DEFAULT_HAMMER, &file.defaults.hammer),
    };

    // The `registers` tier always wins over `defaults` for the three
    // fields `voicing_for_key` computes — see the module docs' honesty
    // note on why this reuses that fixed interpolation rather than the
    // file's own (currently unused) `registers` block.
    let voicing = voicing_for_key(key, tuning);
    resolved.damping = voicing.damping;
    resolved.sustain = voicing.sustain;
    resolved.inharmonicity = voicing.inharmonicity;

    for group in &file.groups {
        if group
            .strings
            .iter()
            .any(|string| string.midi == midi && string.string_index == string_index)
        {
            apply_overrides(&mut resolved, &group.overrides);
        }
    }

    if let Some(entry) = file
        .strings
        .iter()
        .find(|entry| entry.midi == midi && entry.string_index == string_index)
    {
        apply_overrides(&mut resolved, &entry.overrides);
    }

    ResolvedString {
        midi,
        string_index,
        damping: resolved.damping,
        sustain: resolved.sustain,
        inharmonicity: resolved.inharmonicity,
        detune_cents: resolved.detune_cents,
        seed: resolved.seed,
        hammer: resolved.hammer,
    }
}

/// Overwrites every field `overrides` sets (`Some`), leaving the rest of
/// `resolved` exactly as the previous, less specific tier left it.
fn apply_overrides(resolved: &mut Resolved, overrides: &ParameterOverrides) {
    if let Some(damping) = overrides.damping {
        resolved.damping = damping;
    }
    if let Some(sustain) = overrides.sustain {
        resolved.sustain = sustain;
    }
    if let Some(inharmonicity) = overrides.inharmonicity {
        resolved.inharmonicity = inharmonicity;
    }
    if let Some(detune_cents) = overrides.detune_cents {
        resolved.detune_cents = detune_cents;
    }
    if let Some(seed) = overrides.seed {
        resolved.seed = seed;
    }
    resolved.hammer = resolve_hammer(resolved.hammer, &overrides.hammer);
}

/// Overwrites whichever of `base`'s fields `overrides` sets.
fn resolve_hammer(base: HammerConfig, overrides: &HammerOverrides) -> HammerConfig {
    HammerConfig {
        contact_exponent: overrides.contact_exponent.unwrap_or(base.contact_exponent),
        stiffness: overrides.stiffness.unwrap_or(base.stiffness),
        mass: overrides.mass.unwrap_or(base.mass),
    }
}
