//! Turns a sparse [`PianoFile`] into the flat per-string table
//! `piano_audio::AudioSession`'s live setters actually consume —
//! `docs/PARAMETER-STUDIO.md`'s cascade, resolved once.

use piano_audio::voicing::{
    RegisterAnchorOverride, RegisterOverrides, unison_count_for_key, voicing_for_key_with_registers,
};
use piano_core::SampleRate;
use piano_core::dispersion::DEFAULT_INHARMONICITY;
use piano_core::hammer::{DEFAULT_HAMMER, HammerConfig};
use piano_core::soundboard::SoundboardMode;
use piano_core::string::{DEFAULT_DAMPING, DEFAULT_SUSTAIN};
use piano_params::{HIGHEST_PIANO_KEY, LOWEST_PIANO_KEY, PianoKey, Tuning};

use crate::format::{HammerOverrides, ParameterOverrides, PianoFile, RegisterAnchor, Registers};

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

/// Resolves `file` into a flat per-string table under `tuning`, for
/// strings that will run at `sample_rate` — the cascade
/// `docs/PARAMETER-STUDIO.md` describes: `defaults` < `registers` <
/// `groups` < `strings`, most specific wins.
///
/// `sample_rate` matters here because the register tier
/// (`piano_audio::voicing::voicing_for_key`) computes `sustain` from a
/// target decay time *and* the sample rate — see that function's docs.
/// Passing the wrong sample rate (e.g. a hardcoded `48_000.0` when the
/// engine actually opened at `44_100.0`) would silently mistune every
/// string's decay, the same class of bug this function's sibling fix in
/// `piano-audio` closes.
#[must_use]
pub fn resolve(file: &PianoFile, tuning: Tuning, sample_rate: SampleRate) -> ResolvedPiano {
    let registers = register_overrides_from(&file.registers);
    let mut strings = Vec::new();
    for midi in LOWEST_PIANO_KEY..=HIGHEST_PIANO_KEY {
        let Ok(key) = PianoKey::from_midi(midi) else {
            continue;
        };
        for string_index in 0..unison_count_for_key(key) {
            strings.push(resolve_string(
                file,
                registers,
                key,
                tuning,
                sample_rate,
                string_index as u8,
            ));
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

/// Turns a file's `registers` block into what
/// `piano_audio::voicing::voicing_for_key_with_registers` expects — the
/// wire format's `RegisterAnchor` maps field-for-field onto
/// `RegisterAnchorOverride`, an absent register (`None`) becoming that
/// anchor's all-`None` default, which changes nothing (see
/// `registers_default_matches_voicing_for_key_on_every_key` in
/// `piano-audio`'s own tests).
fn register_overrides_from(registers: &Registers) -> RegisterOverrides {
    let anchor = |entry: Option<RegisterAnchor>| {
        entry.map_or(RegisterAnchorOverride::default(), |anchor| {
            RegisterAnchorOverride {
                anchor_midi: Some(anchor.anchor_midi),
                decay_seconds: anchor.decay_seconds,
                damping: anchor.damping,
                inharmonicity: anchor.inharmonicity,
            }
        })
    };
    RegisterOverrides {
        bass: anchor(registers.bass),
        mid: anchor(registers.mid),
        treble: anchor(registers.treble),
    }
}

/// Resolves one string through all four cascade tiers, in
/// least-to-most-specific order.
fn resolve_string(
    file: &PianoFile,
    registers: RegisterOverrides,
    key: PianoKey,
    tuning: Tuning,
    sample_rate: SampleRate,
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
    // fields `voicing_for_key_with_registers` computes. `registers` here
    // carries the file's own bass/mid/treble anchor overrides — see
    // `register_overrides_from` — rather than always reproducing the
    // built-in three-anchor curve regardless of what the file said, which
    // is what this line did until `docs/TIMBRE-PLAN.md` D5/P1 was fixed.
    let voicing = voicing_for_key_with_registers(key, tuning, sample_rate, registers);
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::float_cmp)]

    use super::*;
    use crate::format::RegisterAnchor;

    fn sample_rate() -> SampleRate {
        SampleRate::new(48_000.0).expect("48 kHz is valid")
    }

    fn find(piano: &ResolvedPiano, midi: u8, string_index: u8) -> &ResolvedString {
        piano
            .strings
            .iter()
            .find(|string| string.midi == midi && string.string_index == string_index)
            .expect("midi/string_index exists on an 88-key instrument")
    }

    /// The literal report behind `docs/TIMBRE-PLAN.md`'s D5/P1: "a user who
    /// edits `decay_seconds`... in their piano file gets silence — no
    /// error, no effect." A bass register override must now change the
    /// resolved bass string, and must not touch the treble string.
    #[test]
    fn a_bass_register_decay_override_reaches_the_resolved_bass_string() {
        let mut file = PianoFile::default();
        let baseline = resolve(&file, Tuning::default(), sample_rate());

        file.registers.bass = Some(RegisterAnchor {
            anchor_midi: LOWEST_PIANO_KEY,
            decay_seconds: Some(1.0), // far shorter than the built-in bass target
            damping: None,
            inharmonicity: None,
        });
        let overridden = resolve(&file, Tuning::default(), sample_rate());

        let baseline_bass = find(&baseline, LOWEST_PIANO_KEY, 0);
        let overridden_bass = find(&overridden, LOWEST_PIANO_KEY, 0);
        assert_ne!(
            baseline_bass.sustain, overridden_bass.sustain,
            "the registers.bass.decay_seconds override never reached resolution"
        );

        let baseline_treble = find(&baseline, HIGHEST_PIANO_KEY, 0);
        let overridden_treble = find(&overridden, HIGHEST_PIANO_KEY, 0);
        assert_eq!(
            baseline_treble.sustain, overridden_treble.sustain,
            "a bass-only register override changed the treble string too"
        );
    }

    /// The same report, for `inharmonicity`.
    #[test]
    fn a_treble_register_inharmonicity_override_reaches_the_resolved_treble_string() {
        let mut file = PianoFile::default();
        file.registers.treble = Some(RegisterAnchor {
            anchor_midi: HIGHEST_PIANO_KEY,
            decay_seconds: None,
            damping: None,
            inharmonicity: Some(0.02),
        });
        let piano = resolve(&file, Tuning::default(), sample_rate());
        let treble = find(&piano, HIGHEST_PIANO_KEY, 0);
        assert!(
            (treble.inharmonicity - 0.02).abs() < 1e-4,
            "resolved treble inharmonicity was {}, not the overridden 0.02",
            treble.inharmonicity
        );
    }

    /// The same report, for `damping` — pinned at the exact anchor key, per
    /// `piano_audio::voicing::RegisterAnchorOverride::damping`.
    #[test]
    fn a_register_damping_override_pins_the_resolved_anchor_string() {
        let mut file = PianoFile::default();
        file.registers.mid = Some(RegisterAnchor {
            anchor_midi: piano_params::CONCERT_A_KEY,
            decay_seconds: None,
            damping: Some(0.222_222),
            inharmonicity: None,
        });
        let piano = resolve(&file, Tuning::default(), sample_rate());
        let a4 = find(&piano, piano_params::CONCERT_A_KEY, 0);
        assert_eq!(a4.damping, 0.222_222);
    }

    /// The cascade order (`docs/PARAMETER-STUDIO.md`: `defaults` <
    /// `registers` < `groups` < `strings`) must still hold now that
    /// `registers` is no longer a no-op: a `strings[]` entry on the same
    /// key as a register override still wins.
    #[test]
    fn a_string_override_still_wins_over_a_register_override_on_the_same_key() {
        let mut file = PianoFile::default();
        file.registers.bass = Some(RegisterAnchor {
            anchor_midi: LOWEST_PIANO_KEY,
            decay_seconds: None,
            damping: Some(0.111_111),
            inharmonicity: None,
        });
        file.strings.push(crate::format::StringOverride {
            midi: LOWEST_PIANO_KEY,
            string_index: 0,
            overrides: ParameterOverrides {
                damping: Some(0.9),
                ..ParameterOverrides::default()
            },
        });
        let piano = resolve(&file, Tuning::default(), sample_rate());
        let bass = find(&piano, LOWEST_PIANO_KEY, 0);
        assert_eq!(
            bass.damping, 0.9,
            "the register tier's damping pin outranked an explicit string override"
        );
    }

    /// An empty `registers` block (the default, absent from a hand-written
    /// file) must resolve identically to the built-in three-anchor curve —
    /// the same backward-compatibility guarantee
    /// `registers_default_matches_voicing_for_key_on_every_key` proves one
    /// layer down, checked here through the whole file-load path.
    #[test]
    fn an_empty_registers_block_resolves_exactly_like_the_built_in_anchors() {
        let file = PianoFile::default();
        let piano = resolve(&file, Tuning::default(), sample_rate());
        let a4 = find(&piano, piano_params::CONCERT_A_KEY, 0);
        let expected = piano_audio::voicing::voicing_for_key(
            PianoKey::from_midi(piano_params::CONCERT_A_KEY).expect("A4 is a real key"),
            Tuning::default(),
            sample_rate(),
        );
        assert_eq!(a4.damping, expected.damping);
        assert_eq!(a4.sustain, expected.sustain);
        assert_eq!(a4.inharmonicity, expected.inharmonicity);
    }
}
