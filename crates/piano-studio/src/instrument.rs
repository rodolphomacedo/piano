//! The instrument-wide settings: one value each for the whole piano rather
//! than one per string — how much body, room and mechanism the player hears,
//! and how the keyboard responds.
//!
//! Each [`InstrumentParameter`] carries its own slider range and the one
//! engine command it maps to, so the page, the piano file and the engine
//! cannot fall out of step: adding a variant fails to compile until every
//! `match` here knows its range, its command and its file field (#84).

use piano_core::room::{
    MAX_ROOM_PREDELAY_MILLISECONDS, MAX_ROOM_REVERB_SECONDS, MAX_ROOM_SIZE,
    MIN_ROOM_REVERB_SECONDS, MIN_ROOM_SIZE,
};
use piano_core::string::{MAX_DAMPER_STRENGTH, MIN_DAMPER_STRENGTH};
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
    /// How hard the dampers grip a string once seated: how fast a note
    /// stops when its key comes up.
    DamperStrength,
    /// How big the room is: how far apart its first reflections arrive.
    RoomSize,
    /// How long the room rings in the bass, in seconds.
    RoomReverbSeconds,
    /// How long the room rings in the treble, in seconds: lower is a darker room.
    RoomTrebleReverbSeconds,
    /// Silence before the room answers, in milliseconds: longer sounds like a farther wall.
    RoomPredelay,
}

/// Every [`InstrumentParameter`], in the order the page lists them.
pub const INSTRUMENT_PARAMETERS: [InstrumentParameter; 12] = [
    InstrumentParameter::RoomMix,
    InstrumentParameter::RoomSize,
    InstrumentParameter::RoomReverbSeconds,
    InstrumentParameter::RoomTrebleReverbSeconds,
    InstrumentParameter::RoomPredelay,
    InstrumentParameter::SoundboardMixGain,
    InstrumentParameter::ActionNoiseGain,
    InstrumentParameter::PhantomGain,
    InstrumentParameter::DuplexGain,
    InstrumentParameter::DamperStrength,
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
            Self::DamperStrength => ParameterRange::new(
                f64::from(MIN_DAMPER_STRENGTH),
                f64::from(MAX_DAMPER_STRENGTH),
                0.01,
            ),
            Self::RoomSize => {
                ParameterRange::new(f64::from(MIN_ROOM_SIZE), f64::from(MAX_ROOM_SIZE), 0.01)
            }
            Self::RoomReverbSeconds | Self::RoomTrebleReverbSeconds => ParameterRange::new(
                f64::from(MIN_ROOM_REVERB_SECONDS),
                f64::from(MAX_ROOM_REVERB_SECONDS),
                0.05,
            ),
            Self::RoomPredelay => {
                ParameterRange::new(0.0, f64::from(MAX_ROOM_PREDELAY_MILLISECONDS), 0.5)
            }
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
            Self::DamperStrength => StudioCommand::SetDamperStrength { strength: value },
            Self::RoomSize => StudioCommand::SetRoomSize { size: value },
            Self::RoomReverbSeconds => StudioCommand::SetRoomReverbSeconds { seconds: value },
            Self::RoomTrebleReverbSeconds => {
                StudioCommand::SetRoomTrebleReverbSeconds { seconds: value }
            }
            Self::RoomPredelay => StudioCommand::SetRoomPredelay {
                milliseconds: value,
            },
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
    /// See [`InstrumentParameter::DamperStrength`].
    pub damper_strength: f32,
    /// See [`InstrumentParameter::RoomSize`].
    pub room_size: f32,
    /// See [`InstrumentParameter::RoomReverbSeconds`].
    pub room_reverb_seconds: f32,
    /// See [`InstrumentParameter::RoomTrebleReverbSeconds`].
    pub room_treble_reverb_seconds: f32,
    /// See [`InstrumentParameter::RoomPredelay`].
    pub room_predelay_milliseconds: f32,
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
            damper_strength: resolved.damper_strength,
            room_size: resolved.room_size,
            room_reverb_seconds: resolved.room_reverb_seconds,
            room_treble_reverb_seconds: resolved.room_treble_reverb_seconds,
            room_predelay_milliseconds: resolved.room_predelay_milliseconds,
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
            InstrumentParameter::DamperStrength => &mut self.damper_strength,
            InstrumentParameter::RoomSize => &mut self.room_size,
            InstrumentParameter::RoomReverbSeconds => &mut self.room_reverb_seconds,
            InstrumentParameter::RoomTrebleReverbSeconds => &mut self.room_treble_reverb_seconds,
            InstrumentParameter::RoomPredelay => &mut self.room_predelay_milliseconds,
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
        instrument.damper_strength = Some(self.damper_strength);
        instrument.room_size = Some(self.room_size);
        instrument.room_reverb_seconds = Some(self.room_reverb_seconds);
        instrument.room_treble_reverb_seconds = Some(self.room_treble_reverb_seconds);
        instrument.room_predelay_milliseconds = Some(self.room_predelay_milliseconds);
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
    /// See [`InstrumentParameter::DamperStrength`].
    pub damper_strength: ParameterRange,
    /// See [`InstrumentParameter::RoomSize`].
    pub room_size: ParameterRange,
    /// See [`InstrumentParameter::RoomReverbSeconds`].
    pub room_reverb_seconds: ParameterRange,
    /// See [`InstrumentParameter::RoomTrebleReverbSeconds`].
    pub room_treble_reverb_seconds: ParameterRange,
    /// See [`InstrumentParameter::RoomPredelay`].
    pub room_predelay_milliseconds: ParameterRange,
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
            damper_strength: InstrumentParameter::DamperStrength.range(),
            room_size: InstrumentParameter::RoomSize.range(),
            room_reverb_seconds: InstrumentParameter::RoomReverbSeconds.range(),
            room_treble_reverb_seconds: InstrumentParameter::RoomTrebleReverbSeconds.range(),
            room_predelay_milliseconds: InstrumentParameter::RoomPredelay.range(),
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
            damper_strength: 0.6,
            room_size: 1.0,
            room_reverb_seconds: 1.8,
            room_treble_reverb_seconds: 0.5,
            room_predelay_milliseconds: 12.0,
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
        assert_eq!(instrument.room_reverb_seconds, Some(1.8));
        assert_eq!(instrument.room_predelay_milliseconds, Some(12.0));
    }
}
