//! The instrument-wide settings: one value each for the whole piano rather
//! than one per string — how much body, room and mechanism the player hears,
//! and how the keyboard responds.
//!
//! Each [`InstrumentParameter`] carries its own slider range and the one
//! engine command it maps to, so the page, the piano file and the engine
//! cannot fall out of step: adding a variant fails to compile until every
//! `match` here knows its range, its command and its file field (#84).

use serde::{Deserialize, Serialize};

use crate::command::StudioCommand;
use crate::edit::ParameterRange;
use crate::format::Instrument;
use crate::resolve::ResolvedPiano;

/// One instrument-wide setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InstrumentParameter {
    /// How much of the soundboard is mixed back into the direct sound.
    SoundboardMixGain,
    /// The output level, before the limiter.
    MasterGain,
    /// How strongly a strike's velocity is curved before it reaches the
    /// hammer.
    VelocityCurveExponent,
    /// How much of the room the piano is heard in.
    RoomMix,
    /// How loud the keybed knock and the dampers' landing are.
    ActionNoiseGain,
    /// How strongly the bass mixes phantom partials.
    PhantomGain,
    /// How strongly the treble's duplex segments ring.
    DuplexGain,
}

/// Every [`InstrumentParameter`], in the order the page lists them.
pub const INSTRUMENT_PARAMETERS: [InstrumentParameter; 7] = [
    InstrumentParameter::RoomMix,
    InstrumentParameter::SoundboardMixGain,
    InstrumentParameter::ActionNoiseGain,
    InstrumentParameter::PhantomGain,
    InstrumentParameter::DuplexGain,
    InstrumentParameter::VelocityCurveExponent,
    InstrumentParameter::MasterGain,
];

impl InstrumentParameter {
    /// The span a slider for this setting covers — inside what the engine
    /// clamps to, and wide enough to hear each effect clearly.
    #[must_use]
    pub fn range(self) -> ParameterRange {
        match self {
            Self::SoundboardMixGain => ParameterRange::new(0.0, 2.0, 0.01),
            Self::MasterGain => ParameterRange::new(0.0, 4.0, 0.01),
            Self::VelocityCurveExponent => ParameterRange::new(0.25, 6.0, 0.05),
            Self::RoomMix => ParameterRange::new(0.0, 1.0, 0.01),
            Self::ActionNoiseGain => ParameterRange::new(0.0, 4.0, 0.05),
            Self::PhantomGain => ParameterRange::new(0.0, 0.5, 0.005),
            Self::DuplexGain => ParameterRange::new(0.0, 1.0, 0.005),
        }
    }

    /// The engine command that sets this parameter to `value`.
    fn command(self, value: f32) -> StudioCommand {
        match self {
            Self::SoundboardMixGain => StudioCommand::SetSoundboardMixGain { gain: value },
            Self::MasterGain => StudioCommand::SetMasterGain { gain: value },
            Self::VelocityCurveExponent => StudioCommand::SetVelocityCurve { exponent: value },
            Self::RoomMix => StudioCommand::SetRoomMix { mix: value },
            Self::ActionNoiseGain => StudioCommand::SetActionNoiseGain { gain: value },
            Self::PhantomGain => StudioCommand::SetPhantomGain { gain: value },
            Self::DuplexGain => StudioCommand::SetDuplexGain { gain: value },
        }
    }
}

/// The current value of every [`InstrumentParameter`], as the page shows it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct InstrumentSettings {
    /// See [`InstrumentParameter::SoundboardMixGain`].
    pub soundboard_mix_gain: f32,
    /// See [`InstrumentParameter::MasterGain`].
    pub master_gain: f32,
    /// See [`InstrumentParameter::VelocityCurveExponent`].
    pub velocity_curve_exponent: f32,
    /// See [`InstrumentParameter::RoomMix`].
    pub room_mix: f32,
    /// See [`InstrumentParameter::ActionNoiseGain`].
    pub action_noise_gain: f32,
    /// See [`InstrumentParameter::PhantomGain`].
    pub phantom_gain: f32,
    /// See [`InstrumentParameter::DuplexGain`].
    pub duplex_gain: f32,
}

impl InstrumentSettings {
    /// The settings a resolved piano file starts with.
    #[must_use]
    pub(crate) fn from_resolved(resolved: &ResolvedPiano) -> Self {
        Self {
            soundboard_mix_gain: resolved.soundboard_mix_gain,
            master_gain: resolved.master_gain,
            velocity_curve_exponent: resolved.velocity_curve_exponent,
            room_mix: resolved.room_mix,
            action_noise_gain: resolved.action_noise_gain,
            phantom_gain: resolved.phantom_gain,
            duplex_gain: resolved.duplex_gain,
        }
    }

    fn slot(&mut self, parameter: InstrumentParameter) -> &mut f32 {
        match parameter {
            InstrumentParameter::SoundboardMixGain => &mut self.soundboard_mix_gain,
            InstrumentParameter::MasterGain => &mut self.master_gain,
            InstrumentParameter::VelocityCurveExponent => &mut self.velocity_curve_exponent,
            InstrumentParameter::RoomMix => &mut self.room_mix,
            InstrumentParameter::ActionNoiseGain => &mut self.action_noise_gain,
            InstrumentParameter::PhantomGain => &mut self.phantom_gain,
            InstrumentParameter::DuplexGain => &mut self.duplex_gain,
        }
    }

    /// Sets `parameter` to `value`, clamped into its range, and returns the
    /// command that makes the engine follow.
    pub(crate) fn set(&mut self, parameter: InstrumentParameter, value: f64) -> StudioCommand {
        let clamped = parameter.range().clamp(value) as f32;
        *self.slot(parameter) = clamped;
        parameter.command(clamped)
    }

    /// One command per setting, bringing a fresh engine to these values.
    pub(crate) fn commands(&self) -> Vec<StudioCommand> {
        let mut settings = *self;
        INSTRUMENT_PARAMETERS
            .iter()
            .map(|&parameter| parameter.command(*settings.slot(parameter)))
            .collect()
    }

    /// Writes every setting into a piano file's `instrument` block.
    pub(crate) fn write_into(&self, instrument: &mut Instrument) {
        instrument.soundboard_mix_gain = Some(self.soundboard_mix_gain);
        instrument.master_gain = Some(self.master_gain);
        instrument.velocity_curve_exponent = Some(self.velocity_curve_exponent);
        instrument.room_mix = Some(self.room_mix);
        instrument.action_noise_gain = Some(self.action_noise_gain);
        instrument.phantom_gain = Some(self.phantom_gain);
        instrument.duplex_gain = Some(self.duplex_gain);
    }
}

/// Every instrument slider's ends, keyed like [`InstrumentSettings`].
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct InstrumentRanges {
    /// See [`InstrumentParameter::SoundboardMixGain`].
    pub soundboard_mix_gain: ParameterRange,
    /// See [`InstrumentParameter::MasterGain`].
    pub master_gain: ParameterRange,
    /// See [`InstrumentParameter::VelocityCurveExponent`].
    pub velocity_curve_exponent: ParameterRange,
    /// See [`InstrumentParameter::RoomMix`].
    pub room_mix: ParameterRange,
    /// See [`InstrumentParameter::ActionNoiseGain`].
    pub action_noise_gain: ParameterRange,
    /// See [`InstrumentParameter::PhantomGain`].
    pub phantom_gain: ParameterRange,
    /// See [`InstrumentParameter::DuplexGain`].
    pub duplex_gain: ParameterRange,
}

impl Default for InstrumentRanges {
    fn default() -> Self {
        Self {
            soundboard_mix_gain: InstrumentParameter::SoundboardMixGain.range(),
            master_gain: InstrumentParameter::MasterGain.range(),
            velocity_curve_exponent: InstrumentParameter::VelocityCurveExponent.range(),
            room_mix: InstrumentParameter::RoomMix.range(),
            action_noise_gain: InstrumentParameter::ActionNoiseGain.range(),
            phantom_gain: InstrumentParameter::PhantomGain.range(),
            duplex_gain: InstrumentParameter::DuplexGain.range(),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp)]

    use super::*;

    fn settings() -> InstrumentSettings {
        InstrumentSettings {
            soundboard_mix_gain: 0.5,
            master_gain: 1.0,
            velocity_curve_exponent: 1.0,
            room_mix: 0.25,
            action_noise_gain: 0.8,
            phantom_gain: 0.1,
            duplex_gain: 0.2,
        }
    }

    #[test]
    fn every_parameter_has_a_usable_range() {
        for parameter in INSTRUMENT_PARAMETERS {
            let range = parameter.range();
            assert!(range.high > range.low && range.step > 0.0, "{parameter:?}");
        }
    }

    #[test]
    fn setting_a_parameter_clamps_it_and_sends_exactly_its_command() {
        let mut settings = settings();
        let command = settings.set(InstrumentParameter::DuplexGain, 9.0);
        assert_eq!(command, StudioCommand::SetDuplexGain { gain: 1.0 });
        assert_eq!(settings.duplex_gain, 1.0);
    }

    #[test]
    fn a_fresh_engine_gets_one_command_per_parameter() {
        assert_eq!(settings().commands().len(), INSTRUMENT_PARAMETERS.len());
    }

    #[test]
    fn every_setting_reaches_the_piano_file() {
        let mut instrument = Instrument::default();
        settings().write_into(&mut instrument);
        assert_eq!(instrument.phantom_gain, Some(0.1));
        assert_eq!(instrument.action_noise_gain, Some(0.8));
        assert_eq!(instrument.room_mix, Some(0.25));
    }
}
