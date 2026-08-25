//! The JSON shape `GET /api/piano` serves: the whole live instrument, as
//! the page needs to draw it.
//!
//! Two deliberate shaping decisions, both to keep the browser code dumb:
//!
//! 1. **Strings are nested under keys**, not a flat list of 222. The page
//!    draws 88 keys and expands one at a time, so a flat list would force
//!    it to regroup them on every render.
//! 2. **A string's hammer is flattened** into `hammer_contact_exponent`,
//!    `hammer_stiffness` and `hammer_mass`, so every
//!    [`StringParameter`]'s wire name is also the key its current value
//!    lives under. The page can then read and write any parameter as
//!    `string[parameter]`, with no per-parameter special cases.
//!
//! [`Ranges`] travels with the state for the same reason: slider ends come
//! from the Rust definitions, so the page has nothing to get wrong.

use serde::Serialize;

use crate::edit::{BridgeParameter, ModeParameter, ParameterRange, StringParameter};
use crate::format::Group;

/// One string's live values, keyed the way the page addresses them.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct StringSnapshot {
    /// Which string within its key's unison, `0`-based.
    pub string_index: u8,
    /// See [`StringParameter::Damping`].
    pub damping: f32,
    /// See [`StringParameter::Sustain`].
    pub sustain: f32,
    /// See [`StringParameter::Inharmonicity`].
    pub inharmonicity: f32,
    /// See [`StringParameter::DetuneCents`].
    pub detune_cents: f32,
    /// See [`StringParameter::Seed`].
    pub seed: u32,
    /// See [`StringParameter::HammerContactExponent`].
    pub hammer_contact_exponent: f32,
    /// See [`StringParameter::HammerStiffness`].
    pub hammer_stiffness: f32,
    /// See [`StringParameter::HammerMass`].
    pub hammer_mass: f32,
}

/// One key and the one to three strings under it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct KeySnapshot {
    /// MIDI note number.
    pub midi: u8,
    /// The key's name, e.g. `A4`.
    pub name: String,
    /// Whether this is a black key, so the page can draw a keyboard
    /// without reimplementing the semitone pattern.
    pub sharp: bool,
    /// This key's strings, in unison order.
    pub strings: Vec<StringSnapshot>,
}

/// One soundboard mode's live values.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct ModeSnapshot {
    /// Which mode, `0` to [`piano_core::soundboard::MODE_COUNT`] - 1.
    pub index: usize,
    /// See [`ModeParameter::FrequencyHz`].
    pub frequency_hz: f32,
    /// See [`ModeParameter::DecaySeconds`].
    pub decay_seconds: f32,
    /// See [`ModeParameter::Gain`].
    pub gain: f32,
}

/// The bridge's two live coupling gains.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct BridgeSnapshot {
    /// See [`BridgeParameter::LocalCouplingGain`].
    pub local_coupling_gain: f32,
    /// See [`BridgeParameter::GlobalCouplingGain`].
    pub global_coupling_gain: f32,
}

/// Every per-string slider's ends, keyed by the parameter's wire name.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct StringRanges {
    /// See [`StringParameter::Damping`].
    pub damping: ParameterRange,
    /// See [`StringParameter::Sustain`].
    pub sustain: ParameterRange,
    /// See [`StringParameter::Inharmonicity`].
    pub inharmonicity: ParameterRange,
    /// See [`StringParameter::DetuneCents`].
    pub detune_cents: ParameterRange,
    /// See [`StringParameter::Seed`].
    pub seed: ParameterRange,
    /// See [`StringParameter::HammerContactExponent`].
    pub hammer_contact_exponent: ParameterRange,
    /// See [`StringParameter::HammerStiffness`].
    pub hammer_stiffness: ParameterRange,
    /// See [`StringParameter::HammerMass`].
    pub hammer_mass: ParameterRange,
}

/// Every soundboard-mode slider's ends. See [`ModeParameter`].
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct ModeRanges {
    /// See [`ModeParameter::FrequencyHz`].
    pub frequency_hz: ParameterRange,
    /// See [`ModeParameter::DecaySeconds`].
    pub decay_seconds: ParameterRange,
    /// See [`ModeParameter::Gain`].
    pub gain: ParameterRange,
}

/// Both bridge sliders' ends. See [`BridgeParameter`].
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct BridgeRanges {
    /// See [`BridgeParameter::LocalCouplingGain`].
    pub local_coupling_gain: ParameterRange,
    /// See [`BridgeParameter::GlobalCouplingGain`].
    pub global_coupling_gain: ParameterRange,
}

/// Every slider's ends, in one block.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Ranges {
    /// Per-string parameter ranges.
    pub strings: StringRanges,
    /// Soundboard-mode parameter ranges.
    pub modes: ModeRanges,
    /// Bridge coupling-gain ranges.
    pub bridge: BridgeRanges,
}

impl Default for Ranges {
    fn default() -> Self {
        Self {
            strings: StringRanges::from_definitions(),
            modes: ModeRanges::from_definitions(),
            bridge: BridgeRanges::from_definitions(),
        }
    }
}

impl StringRanges {
    /// Reads every range straight off [`StringParameter`], so this table
    /// cannot fall out of step with what the server will accept.
    fn from_definitions() -> Self {
        Self {
            damping: StringParameter::Damping.range(),
            sustain: StringParameter::Sustain.range(),
            inharmonicity: StringParameter::Inharmonicity.range(),
            detune_cents: StringParameter::DetuneCents.range(),
            seed: StringParameter::Seed.range(),
            hammer_contact_exponent: StringParameter::HammerContactExponent.range(),
            hammer_stiffness: StringParameter::HammerStiffness.range(),
            hammer_mass: StringParameter::HammerMass.range(),
        }
    }
}

impl ModeRanges {
    /// See [`StringRanges::from_definitions`].
    fn from_definitions() -> Self {
        Self {
            frequency_hz: ModeParameter::FrequencyHz.range(),
            decay_seconds: ModeParameter::DecaySeconds.range(),
            gain: ModeParameter::Gain.range(),
        }
    }
}

impl BridgeRanges {
    /// See [`StringRanges::from_definitions`].
    fn from_definitions() -> Self {
        Self {
            local_coupling_gain: BridgeParameter::LocalCouplingGain.range(),
            global_coupling_gain: BridgeParameter::GlobalCouplingGain.range(),
        }
    }
}

/// The whole live instrument, as `GET /api/piano` serves it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PianoSnapshot {
    /// The piano's display name, if the loaded file gave one.
    pub name: Option<String>,
    /// Where a save goes by default, if the studio was started from a file.
    pub path: Option<String>,
    /// Every key on the instrument, low to high.
    pub keys: Vec<KeySnapshot>,
    /// Every soundboard mode, in mode order.
    pub modes: Vec<ModeSnapshot>,
    /// The bridge's two coupling gains.
    pub bridge: BridgeSnapshot,
    /// Named string selections carried over from the loaded file.
    pub groups: Vec<Group>,
    /// Every slider's ends. See [`Ranges`].
    pub ranges: Ranges,
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::*;
    use crate::edit::STRING_PARAMETERS;

    fn wire_name(parameter: StringParameter) -> String {
        let value = serde_json::to_value(parameter).expect("a parameter serialises");
        value
            .as_str()
            .expect("a parameter serialises as a string")
            .to_string()
    }

    #[test]
    fn the_range_table_has_an_entry_under_every_parameter_s_own_wire_name() {
        // The page looks a range up as `ranges.strings[parameter]`, with
        // the same string it will send back as `parameter`. A parameter
        // added to the enum but not here would leave the page with an
        // undefined slider, silently.
        let json = serde_json::to_value(Ranges::default().strings).expect("serialises");
        for parameter in STRING_PARAMETERS {
            let name = wire_name(parameter);
            assert!(json.get(&name).is_some(), "no range served for {name}");
        }
    }

    #[test]
    fn a_string_snapshot_carries_a_value_under_every_parameter_s_wire_name() {
        let snapshot = StringSnapshot {
            string_index: 0,
            damping: 0.5,
            sustain: 0.996,
            inharmonicity: 0.000_4,
            detune_cents: 0.0,
            seed: 0,
            hammer_contact_exponent: 2.5,
            hammer_stiffness: 1.7e9,
            hammer_mass: 1.0,
        };
        let json = serde_json::to_value(snapshot).expect("serialises");
        for parameter in STRING_PARAMETERS {
            let name = wire_name(parameter);
            assert!(json.get(&name).is_some(), "no value served for {name}");
        }
    }
}
