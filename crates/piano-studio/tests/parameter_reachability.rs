//! Gate: every live-settable parameter `piano-core` has is reachable from
//! the studio and survives a round trip through the piano file (#84).
//!
//! Parameters were added to `piano-core` over several milestones with
//! nothing checking that they ever reached a slider; `loop_zero_mix` sat
//! in `StringConfig`, solved per key, exposed nowhere. The mechanism here is
//! exhaustive destructuring: each core struct is taken apart field by field
//! with no `..`, so adding a field fails to compile until this file says how
//! the player reaches it — a studio parameter, or a stated reason it is
//! derived rather than set.

#![allow(clippy::expect_used)]

use piano_core::hammer::{DEFAULT_HAMMER, HammerConfig};
use piano_core::soundboard::{DEFAULT_MODES, SoundboardMode};
use piano_core::string::StringConfig;
use piano_core::{Hz, SampleRate};
use piano_params::Tuning;
use piano_studio::{Edit, LiveState, ModeParameter, PianoFile, STRING_PARAMETERS, StringParameter};

/// How a player reaches one `piano-core` field.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Reach {
    /// Through this per-string studio parameter.
    String(StringParameter),
    /// Through this soundboard-mode studio parameter.
    Mode(ModeParameter),
    /// Not set directly, for the reason given.
    Derived(&'static str),
}

fn string_config_reach() -> Vec<Reach> {
    let StringConfig {
        frequency: _,
        damping: _,
        sustain: _,
        inharmonicity: _,
        seed: _,
        hammer: _,
        loop_zero_mix: _,
        strike_position: _,
        excitation_noise_mix: _,
    } = StringConfig::new(Hz::new(440.0).expect("440 Hz is valid"));
    vec![
        Reach::Derived("frequency: the key and the tuning set it; detune_cents moves it"),
        Reach::String(StringParameter::Damping),
        Reach::String(StringParameter::Sustain),
        Reach::String(StringParameter::Inharmonicity),
        Reach::String(StringParameter::Seed),
        Reach::Derived("hammer: reached field by field, see hammer_reach"),
        Reach::String(StringParameter::LoopZeroMix),
        Reach::String(StringParameter::StrikePosition),
        Reach::String(StringParameter::ExcitationNoiseMix),
    ]
}

fn hammer_reach() -> Vec<Reach> {
    let HammerConfig {
        contact_exponent: _,
        stiffness: _,
        mass: _,
        string_impedance: _,
        felt_bandwidth: _,
    } = DEFAULT_HAMMER;
    vec![
        Reach::String(StringParameter::HammerContactExponent),
        Reach::String(StringParameter::HammerStiffness),
        Reach::String(StringParameter::HammerMass),
        Reach::String(StringParameter::HammerStringImpedance),
        Reach::String(StringParameter::HammerFeltBandwidth),
    ]
}

fn soundboard_mode_reach() -> Vec<Reach> {
    let [mode, ..] = DEFAULT_MODES;
    let SoundboardMode {
        frequency_hz: _,
        decay_seconds: _,
        gain: _,
        bridge_coupling: _,
    } = mode;
    vec![
        Reach::Mode(ModeParameter::FrequencyHz),
        Reach::Mode(ModeParameter::DecaySeconds),
        Reach::Mode(ModeParameter::Gain),
        Reach::Derived(
            "bridge_coupling: nothing reads it until the board loads the strings (#91); \
             the file carries it",
        ),
    ]
}

/// The per-string parameters the core structs above reach, plus
/// `DetuneCents`, the one with no core field of its own, since it moves
/// `frequency`.
fn reached_string_parameters() -> Vec<StringParameter> {
    let mut reached: Vec<StringParameter> = string_config_reach()
        .into_iter()
        .chain(hammer_reach())
        .filter_map(|reach| match reach {
            Reach::String(parameter) => Some(parameter),
            Reach::Mode(_) | Reach::Derived(_) => None,
        })
        .collect();
    reached.push(StringParameter::DetuneCents);
    reached
}

#[test]
fn every_core_string_field_has_a_studio_parameter_and_every_parameter_a_field() {
    let reached = reached_string_parameters();
    for parameter in STRING_PARAMETERS {
        assert!(
            reached.contains(&parameter),
            "{parameter:?} reaches no core field"
        );
    }
    for parameter in &reached {
        assert!(
            STRING_PARAMETERS.contains(parameter),
            "{parameter:?} has no slider"
        );
    }
}

#[test]
fn every_soundboard_mode_field_is_reached_or_explained() {
    let reach = soundboard_mode_reach();
    for parameter in [
        ModeParameter::FrequencyHz,
        ModeParameter::DecaySeconds,
        ModeParameter::Gain,
    ] {
        assert!(reach.contains(&Reach::Mode(parameter)), "{parameter:?}");
    }
}

fn state(file: &PianoFile) -> LiveState {
    let rate = SampleRate::new(48_000.0).expect("48 kHz is valid");
    LiveState::from_file(file, None, Tuning::default(), rate)
}

/// A value inside `parameter`'s range that no key is voiced with by
/// default, so a round trip that silently dropped it would show.
fn distinctive_value(parameter: StringParameter) -> f64 {
    let range = parameter.range();
    range.low + (range.high - range.low) * 0.37
}

#[test]
fn every_string_parameter_survives_a_save_and_a_reload() {
    for parameter in STRING_PARAMETERS {
        let mut live = state(&PianoFile::default());
        let _ = live.apply(&Edit::SetString {
            midi: 60,
            string_index: 1,
            parameter,
            value: distinctive_value(parameter),
        });
        let saved = live.snapshot();
        let reloaded = state(&live.to_piano_file()).snapshot();
        assert_eq!(saved, reloaded, "{parameter:?} did not survive the file");
    }
}
