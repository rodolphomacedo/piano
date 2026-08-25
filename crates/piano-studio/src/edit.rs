//! The one-change-at-a-time messages the browser sends the studio.
//!
//! `docs/PARAMETER-STUDIO.md` specifies `/api/live` as carrying "one
//! parameter change at a time"; [`Edit`] is that change, and
//! [`StringParameter`]/[`ModeParameter`]/[`BridgeParameter`] name what a
//! change can touch. Each parameter also carries the [`ParameterRange`] a
//! slider for it should span — served to the page rather than duplicated
//! in JavaScript, so the browser cannot drift from what the engine
//! accepts.
//!
//! Nothing here clamps *for safety*: `piano-core` already treats every
//! live setter's argument as hostile and is total for `NaN`, `±∞` and
//! `usize::MAX` whatever arrives. These ranges exist so a slider has
//! sensible ends and so the file that gets saved holds musically
//! meaningful numbers.

use serde::{Deserialize, Serialize};

use crate::format::StringRef;

/// The inclusive span a parameter's slider covers, and which a value
/// arriving from a client is clamped into before being stored or sent on.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct ParameterRange {
    /// The lowest accepted value.
    pub low: f64,
    /// The highest accepted value.
    pub high: f64,
    /// A step fine enough to reach anything musically distinct in this
    /// span, for the page's `<input type="range">`.
    pub step: f64,
}

impl ParameterRange {
    const fn new(low: f64, high: f64, step: f64) -> Self {
        Self { low, high, step }
    }

    /// Clamps `value` into this range.
    ///
    /// Total for every `f64`. `NaN` has no ordering and so cannot be
    /// clamped meaningfully; it resolves to [`ParameterRange::low`], the
    /// same convention `piano_core::math::clamp_or_low` already uses for
    /// exactly this case.
    #[must_use]
    pub fn clamp(self, value: f64) -> f64 {
        if value.is_nan() || value < self.low {
            return self.low;
        }
        if value > self.high {
            return self.high;
        }
        value
    }
}

/// Which of a string's parameters an [`Edit::SetString`] changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StringParameter {
    /// See [`piano_core::string::PluckedString::set_damping`].
    Damping,
    /// See [`piano_core::string::PluckedString::set_sustain`].
    Sustain,
    /// See [`piano_core::string::PluckedString::set_inharmonicity`].
    Inharmonicity,
    /// See [`piano_core::UnisonGroup::set_string_detune`].
    DetuneCents,
    /// See [`piano_core::string::PluckedString::set_seed`].
    Seed,
    /// See [`piano_core::hammer::HammerConfig::contact_exponent`].
    HammerContactExponent,
    /// See [`piano_core::hammer::HammerConfig::stiffness`].
    HammerStiffness,
    /// See [`piano_core::hammer::HammerConfig::mass`].
    HammerMass,
}

/// Every [`StringParameter`], for callers that need to walk them all —
/// the snapshot's range table and the tests that check none was forgotten.
pub const STRING_PARAMETERS: [StringParameter; 8] = [
    StringParameter::Damping,
    StringParameter::Sustain,
    StringParameter::Inharmonicity,
    StringParameter::DetuneCents,
    StringParameter::Seed,
    StringParameter::HammerContactExponent,
    StringParameter::HammerStiffness,
    StringParameter::HammerMass,
];

/// Highest excitation seed a slider offers. `u32::MAX` would make every
/// pixel of the slider jump millions of seeds; a seed's only job is to
/// pick a different noise sequence, and this many distinct ones is already
/// far more than anyone auditions by hand.
const MAX_SEED: f64 = 65_535.0;

/// Mirrors `piano_core::hammer`'s private `MIN_CONTACT_EXPONENT` and
/// `MAX_CONTACT_EXPONENT` — duplicated rather than imported because those
/// bounds are deliberately not part of `piano-core`'s public API, the same
/// reason `crate::resolve` mirrors the two coupling-gain defaults.
const CONTACT_EXPONENT_RANGE: ParameterRange = ParameterRange::new(0.5, 8.0, 0.01);

/// Mirrors `piano_core::hammer`'s private `MIN_STIFFNESS`/`MAX_STIFFNESS`.
/// See [`CONTACT_EXPONENT_RANGE`].
const STIFFNESS_RANGE: ParameterRange = ParameterRange::new(1.7e7, 1.7e11, 1.7e7);

/// Mirrors `piano_core::hammer`'s private `MIN_MASS`/`MAX_MASS`. See
/// [`CONTACT_EXPONENT_RANGE`].
const MASS_RANGE: ParameterRange = ParameterRange::new(0.01, 100.0, 0.01);

impl StringParameter {
    /// The span a slider for this parameter should cover.
    #[must_use]
    pub fn range(self) -> ParameterRange {
        match self {
            Self::Damping | Self::Sustain => ParameterRange::new(0.0, 1.0, 0.001),
            Self::Inharmonicity => inharmonicity_range(),
            Self::DetuneCents => detune_range(),
            Self::Seed => ParameterRange::new(0.0, MAX_SEED, 1.0),
            Self::HammerContactExponent => CONTACT_EXPONENT_RANGE,
            Self::HammerStiffness => STIFFNESS_RANGE,
            Self::HammerMass => MASS_RANGE,
        }
    }
}

/// Stops at `piano-core`'s own published ceiling, so a slider cannot ask
/// for a stretch the dispersion cascade will silently refuse.
fn inharmonicity_range() -> ParameterRange {
    let high = f64::from(piano_core::dispersion::MAX_INHARMONICITY);
    ParameterRange::new(0.0, high, high / 1_000.0)
}

/// Symmetric around zero at `piano-core`'s own live-detune limit, for the
/// same reason [`inharmonicity_range`] stops where it does.
fn detune_range() -> ParameterRange {
    let limit = f64::from(piano_core::string::MAX_LIVE_DETUNE_CENTS);
    ParameterRange::new(-limit, limit, 0.1)
}

/// Which of a soundboard mode's parameters an [`Edit::SetMode`] changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ModeParameter {
    /// See [`piano_core::soundboard::SoundboardMode::frequency_hz`].
    FrequencyHz,
    /// See [`piano_core::soundboard::SoundboardMode::decay_seconds`].
    DecaySeconds,
    /// See [`piano_core::soundboard::SoundboardMode::gain`].
    Gain,
}

impl ModeParameter {
    /// The span a slider for this parameter should cover. Mirrors
    /// `piano_core::soundboard`'s private `MIN_MODE_FREQUENCY_HZ`,
    /// `MAX_MODE_FREQUENCY_HZ`, `MIN_DECAY_SECONDS` and `MAX_MODE_GAIN`,
    /// for the reason [`CONTACT_EXPONENT_RANGE`] gives.
    #[must_use]
    pub fn range(self) -> ParameterRange {
        match self {
            Self::FrequencyHz => ParameterRange::new(1.0, 20_000.0, 1.0),
            Self::DecaySeconds => ParameterRange::new(0.01, 10.0, 0.01),
            Self::Gain => ParameterRange::new(0.0, 4.0, 0.01),
        }
    }
}

/// Which of the bridge's two coupling gains an [`Edit::SetBridge`] changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BridgeParameter {
    /// See [`piano_core::UnisonGroup::set_local_coupling_gain`].
    LocalCouplingGain,
    /// See [`piano_core::UnisonGroup::set_global_coupling_gain`].
    GlobalCouplingGain,
}

impl BridgeParameter {
    /// The span a slider for this parameter should cover — the audio
    /// thread clamps both gains into `[0, 1]`.
    #[must_use]
    pub fn range(self) -> ParameterRange {
        ParameterRange::new(0.0, 1.0, 0.001)
    }
}

/// The velocity a strike from the page can ask for — the `0.0`-`1.0`
/// contract of [`piano_audio::AudioSession::note_on`].
const VELOCITY_RANGE: ParameterRange = ParameterRange::new(0.0, 1.0, 0.01);

/// Clamps a wire velocity into what the engine documents.
#[must_use]
pub(crate) fn clamped_velocity(velocity: f64) -> f32 {
    VELOCITY_RANGE.clamp(velocity) as f32
}

/// One change arriving from a connected page.
///
/// Playing — `NoteOn`, `NoteOff`, `SustainPedal`, `AllNotesOff` — travels
/// the same route as editing, so a browser with no MIDI keyboard attached
/// can still audition what a slider just did. The page *is* a controller.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Edit {
    /// Strike a key.
    NoteOn {
        /// MIDI note number of the key struck.
        midi: u8,
        /// How hard, `0.0` to `1.0`.
        velocity: f64,
    },
    /// Release a key.
    NoteOff {
        /// MIDI note number of the key released.
        midi: u8,
    },
    /// Silence every ringing voice.
    AllNotesOff,
    /// Hold or release the sustain pedal.
    SustainPedal {
        /// Whether the pedal is down.
        down: bool,
    },
    /// Change one parameter on one string.
    SetString {
        /// MIDI note number of the key the string belongs to.
        midi: u8,
        /// Which string within that key's unison, `0`-based.
        string_index: u8,
        /// Which parameter to change.
        parameter: StringParameter,
        /// Its new value, clamped into [`StringParameter::range`].
        value: f64,
    },
    /// Change one parameter on every string in a selection — a group
    /// applied, which `docs/PARAMETER-STUDIO.md` defines as "N individual
    /// per-string writes", never a new kind of entity.
    SetStrings {
        /// Which strings to write.
        strings: Vec<StringRef>,
        /// Which parameter to change on each.
        parameter: StringParameter,
        /// Its new value, clamped into [`StringParameter::range`].
        value: f64,
    },
    /// Change one parameter of one soundboard mode.
    SetMode {
        /// Which mode, `0` to [`piano_core::soundboard::MODE_COUNT`] - 1.
        index: usize,
        /// Which parameter to change.
        parameter: ModeParameter,
        /// Its new value, clamped into [`ModeParameter::range`].
        value: f64,
    },
    /// Change one of the bridge's coupling gains.
    SetBridge {
        /// Which gain to change.
        parameter: BridgeParameter,
        /// Its new value, clamped into [`BridgeParameter::range`].
        value: f64,
    },
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp, clippy::expect_used)]

    use super::*;

    #[test]
    fn clamping_is_total_for_nan_and_infinities() {
        let range = ParameterRange::new(0.0, 1.0, 0.01);
        assert_eq!(range.clamp(f64::NAN), 0.0);
        assert_eq!(range.clamp(f64::NEG_INFINITY), 0.0);
        assert_eq!(range.clamp(f64::INFINITY), 1.0);
        assert_eq!(range.clamp(0.5), 0.5);
    }

    #[test]
    fn every_string_parameter_has_a_usable_range() {
        for parameter in STRING_PARAMETERS {
            let range = parameter.range();
            assert!(range.high > range.low, "empty range for {parameter:?}");
            assert!(range.step > 0.0, "unusable step for {parameter:?}");
        }
    }

    #[test]
    fn every_mode_and_bridge_parameter_has_a_usable_range() {
        let modes = [
            ModeParameter::FrequencyHz,
            ModeParameter::DecaySeconds,
            ModeParameter::Gain,
        ];
        for parameter in modes {
            assert!(parameter.range().high > parameter.range().low);
        }
        for parameter in [
            BridgeParameter::LocalCouplingGain,
            BridgeParameter::GlobalCouplingGain,
        ] {
            assert!(parameter.range().high > parameter.range().low);
        }
    }

    #[test]
    fn detune_spans_piano_core_s_own_live_limit_symmetrically() {
        let range = StringParameter::DetuneCents.range();
        assert_eq!(range.low, -range.high);
        assert_eq!(
            range.high,
            f64::from(piano_core::string::MAX_LIVE_DETUNE_CENTS)
        );
    }

    #[test]
    fn inharmonicity_stops_at_piano_core_s_published_ceiling() {
        let range = StringParameter::Inharmonicity.range();
        assert_eq!(
            range.high,
            f64::from(piano_core::dispersion::MAX_INHARMONICITY)
        );
    }

    #[test]
    fn an_edit_round_trips_through_its_tagged_json_form() {
        let edit = Edit::SetString {
            midi: 69,
            string_index: 1,
            parameter: StringParameter::Damping,
            value: 0.42,
        };
        let json = serde_json::to_string(&edit).expect("serialises");
        assert!(json.contains("\"type\":\"set_string\""), "{json}");
        let parsed: Edit = serde_json::from_str(&json).expect("parses its own output");
        assert_eq!(parsed, edit);
    }

    #[test]
    fn a_note_on_parses_from_the_shape_the_page_sends() {
        let parsed: Edit = serde_json::from_str(r#"{"type":"note_on","midi":69,"velocity":0.8}"#)
            .expect("the page's own note_on shape parses");
        assert_eq!(
            parsed,
            Edit::NoteOn {
                midi: 69,
                velocity: 0.8,
            }
        );
    }

    #[test]
    fn velocity_outside_the_documented_range_is_clamped_not_rejected() {
        assert_eq!(clamped_velocity(9.0), 1.0);
        assert_eq!(clamped_velocity(-1.0), 0.0);
        assert_eq!(clamped_velocity(f64::NAN), 0.0);
    }

    #[test]
    fn a_parameter_name_on_the_wire_matches_the_snapshot_s_field_name() {
        // The page indexes a string's values by the same key it sends back
        // as `parameter`, so these two spellings must not drift.
        let json =
            serde_json::to_string(&StringParameter::HammerContactExponent).expect("serialises");
        assert_eq!(json, "\"hammer_contact_exponent\"");
    }
}
