//! Serde types for the `.piano.json` cascade format
//! (`docs/PARAMETER-STUDIO.md`'s "Piano file format" section — every
//! field name and nesting here matches that document's JSON example
//! exactly).
//!
//! These types only describe the file on disk; [`crate::resolve`] is what
//! turns a sparse [`PianoFile`] into the flat per-string table the engine
//! consumes.

use serde::{Deserialize, Serialize};

/// One string's felt-hammer contact physics, all fields optional so a
/// cascade tier can override only some of them. See
/// [`piano_core::hammer::HammerConfig`] for what each field means.
#[derive(Debug, Clone, Copy, Default, PartialEq, Deserialize, Serialize)]
pub struct HammerOverrides {
    /// See [`piano_core::hammer::HammerConfig::contact_exponent`].
    pub contact_exponent: Option<f32>,
    /// See [`piano_core::hammer::HammerConfig::stiffness`].
    pub stiffness: Option<f32>,
    /// See [`piano_core::hammer::HammerConfig::mass`].
    pub mass: Option<f32>,
}

/// The parameters one cascade tier (`defaults`, a group's `overrides`, or
/// one `strings[]` entry) can set — every field optional, since a tier
/// only needs to state what it changes; [`crate::resolve`] fills in
/// whatever a tier leaves `None` from the next-least-specific tier.
#[derive(Debug, Clone, Copy, Default, PartialEq, Deserialize, Serialize)]
pub struct ParameterOverrides {
    /// See [`piano_core::string::PluckedString::set_damping`].
    pub damping: Option<f32>,
    /// See [`piano_core::string::PluckedString::set_sustain`].
    pub sustain: Option<f32>,
    /// See [`piano_core::string::PluckedString::set_inharmonicity`].
    pub inharmonicity: Option<f32>,
    /// See [`piano_core::UnisonGroup::set_string_detune`].
    pub detune_cents: Option<f32>,
    /// See [`piano_core::string::PluckedString::set_seed`].
    pub seed: Option<u32>,
    /// See [`HammerOverrides`].
    #[serde(default)]
    pub hammer: HammerOverrides,
}

/// One `registers.bass`/`mid`/`treble` entry.
///
/// Honesty note: [`crate::resolve`] currently reuses
/// `piano_audio::voicing::voicing_for_key`'s existing, fixed three-anchor
/// interpolation for the `damping`/`sustain`/`inharmonicity` a key falls
/// back to — the same reuse-not-reimplement scope `docs/PARAMETER-STUDIO.md`
/// calls for. This type round-trips a file's `registers` block faithfully
/// (so saving what was loaded is lossless), but its fields do not yet
/// feed back into that interpolation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Deserialize, Serialize)]
pub struct RegisterAnchor {
    /// The MIDI note this register is anchored at.
    #[serde(default)]
    pub anchor_midi: u8,
    /// Target ring-out time at the anchor, in seconds.
    pub decay_seconds: Option<f32>,
    /// See [`ParameterOverrides::damping`].
    pub damping: Option<f32>,
    /// See [`ParameterOverrides::inharmonicity`].
    pub inharmonicity: Option<f32>,
}

/// The `registers` block: up to three named anchors.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
pub struct Registers {
    /// The bass anchor, nominally A0.
    pub bass: Option<RegisterAnchor>,
    /// The middle anchor, nominally A4.
    pub mid: Option<RegisterAnchor>,
    /// The treble anchor, nominally C8.
    pub treble: Option<RegisterAnchor>,
}

/// One string, addressed by the key it belongs to and its position within
/// that key's unison.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
pub struct StringRef {
    /// MIDI note number of the key this string belongs to.
    pub midi: u8,
    /// Which string within that key's unison, `0`-based.
    pub string_index: u8,
}

/// A named, saved selection of individual strings plus a set of values to
/// write into each of them — never a new kind of entity the engine has to
/// know about (`docs/PARAMETER-STUDIO.md`'s "per-string is the atomic
/// unit" scope decision).
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct Group {
    /// The group's display name.
    pub name: String,
    /// Which strings this group applies to.
    pub strings: Vec<StringRef>,
    /// The values this group writes into each of its strings.
    #[serde(default)]
    pub overrides: ParameterOverrides,
}

/// One explicit `strings[]` entry: the most specific cascade tier, always
/// winning over a matching group or register.
#[derive(Debug, Clone, Copy, Default, PartialEq, Deserialize, Serialize)]
pub struct StringOverride {
    /// MIDI note number of the key this string belongs to.
    pub midi: u8,
    /// Which string within that key's unison, `0`-based.
    pub string_index: u8,
    /// The values this entry sets on the string.
    #[serde(flatten)]
    pub overrides: ParameterOverrides,
}

/// One `instrument.soundboard_modes[]` entry. See
/// [`piano_core::soundboard::SoundboardMode`] for what each field means.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize)]
pub struct SoundboardModeOverride {
    /// Mode frequency, in hertz.
    pub frequency_hz: f32,
    /// Time for this mode's contribution to decay to `1/e`, in seconds.
    pub decay_seconds: f32,
    /// Relative gain of this mode in the mixed output.
    pub gain: f32,
}

/// The `instrument.bridge` block. See
/// [`piano_core::UnisonGroup::set_local_coupling_gain`] and
/// [`piano_core::UnisonGroup::set_global_coupling_gain`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Deserialize, Serialize)]
pub struct BridgeOverrides {
    /// See [`piano_core::UnisonGroup::set_local_coupling_gain`].
    pub local_coupling_gain: Option<f32>,
    /// See [`piano_core::UnisonGroup::set_global_coupling_gain`].
    pub global_coupling_gain: Option<f32>,
}

/// The `instrument` block: the parameters shared by the whole piano rather
/// than owned by one string.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
pub struct Instrument {
    /// Overrides for [`piano_core::soundboard::MODE_COUNT`] resonant
    /// modes, in mode-index order. Fewer entries than
    /// [`piano_core::soundboard::MODE_COUNT`] leaves the remaining modes
    /// at their built-in defaults; entries are matched to mode index by
    /// their position in this list.
    #[serde(default)]
    pub soundboard_modes: Vec<SoundboardModeOverride>,
    /// See [`BridgeOverrides`].
    #[serde(default)]
    pub bridge: BridgeOverrides,
}

/// A whole `.piano.json` file — see the module docs and
/// `docs/PARAMETER-STUDIO.md` for the cascade this resolves through.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
pub struct PianoFile {
    /// The piano's display name.
    pub name: Option<String>,
    /// The least-specific tier: instrument-wide fallback values.
    #[serde(default)]
    pub defaults: ParameterOverrides,
    /// The bass/mid/treble register anchors. See [`Registers`].
    #[serde(default)]
    pub registers: Registers,
    /// Named string selections with their own overrides, most specific
    /// after `strings`.
    #[serde(default)]
    pub groups: Vec<Group>,
    /// Explicit per-string overrides — the most specific tier.
    #[serde(default)]
    pub strings: Vec<StringOverride>,
    /// Instrument-wide (not per-string) parameters.
    #[serde(default)]
    pub instrument: Instrument,
}
