//! The studio's authoritative in-memory picture of the running piano.
//!
//! [`LiveState`] holds what every string and the instrument as a whole are
//! *currently* set to, applies incoming [`Edit`]s to that picture, and
//! emits the [`StudioCommand`]s that make the running engine agree. It is
//! the only writer of that picture, and it lives entirely on the control
//! side of `docs/ARCHITECTURE.md`'s split — nothing here runs on, or
//! blocks, the audio thread.
//!
//! Per `docs/PARAMETER-STUDIO.md`'s "Persistence" section, editing never
//! touches the disk: [`LiveState::to_piano_file`] is called only when
//! something explicitly asks to save, and it writes the fully resolved
//! table rather than a diff against whatever was loaded.

use std::path::{Path, PathBuf};

use piano_core::SampleRate;
use piano_core::soundboard::{DEFAULT_MODES, MODE_COUNT, SoundboardMode};
use piano_params::{PianoKey, Tuning};

use crate::command::StudioCommand;
use crate::edit::{BridgeParameter, Edit, ModeParameter, StringParameter, clamped_velocity};
use crate::format::{
    BridgeOverrides, Group, HammerOverrides, Instrument, ParameterOverrides, PianoFile, Registers,
    SoundboardModeOverride, StringOverride, StringRef,
};
use crate::resolve::{ResolvedString, resolve};
use crate::snapshot::{
    BridgeSnapshot, KeySnapshot, ModeSnapshot, PianoSnapshot, Ranges, StringSnapshot,
};

/// Semitones within an octave that are black keys, so the page can draw a
/// keyboard without reimplementing the pattern.
const SHARP_SEMITONES: [u8; 5] = [1, 3, 6, 8, 10];

/// The whole instrument's current live state.
#[derive(Debug, Clone, PartialEq)]
pub struct LiveState {
    name: Option<String>,
    path: Option<PathBuf>,
    strings: Vec<ResolvedString>,
    modes: [SoundboardMode; MODE_COUNT],
    local_coupling_gain: f32,
    global_coupling_gain: f32,
    groups: Vec<Group>,
}

impl LiveState {
    /// Resolves `file` and takes the result as the current state.
    ///
    /// `sample_rate` must be the rate the engine actually opened at, not a
    /// hardcoded 48 kHz — see [`crate::resolve`] for why a wrong rate
    /// silently mistunes every string's decay.
    #[must_use]
    pub fn from_file(
        file: &PianoFile,
        path: Option<&Path>,
        tuning: Tuning,
        sample_rate: SampleRate,
    ) -> Self {
        let resolved = resolve(file, tuning, sample_rate);
        let mut modes = DEFAULT_MODES;
        for (slot, mode) in modes.iter_mut().zip(&resolved.soundboard_modes) {
            *slot = *mode;
        }
        Self {
            name: file.name.clone(),
            path: path.map(Path::to_path_buf),
            strings: resolved.strings,
            modes,
            local_coupling_gain: resolved.local_coupling_gain,
            global_coupling_gain: resolved.global_coupling_gain,
            groups: file.groups.clone(),
        }
    }

    /// Where a save goes when nothing names a path.
    #[must_use]
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Remembers `path` as where subsequent saves go — what "Save As"
    /// means once it has succeeded once.
    pub fn set_path(&mut self, path: PathBuf) {
        self.path = Some(path);
    }

    /// Every command needed to bring a freshly started engine to this
    /// state: the whole instrument, in one list.
    ///
    /// The caller is responsible for pacing these onto the command ring.
    /// There are well over a thousand for a full instrument and the ring
    /// holds far fewer at once, so pushing them in a tight loop drops most
    /// of them on the floor.
    #[must_use]
    pub fn commands(&self) -> Vec<StudioCommand> {
        let mut commands = Vec::with_capacity(self.strings.len() * 6 + MODE_COUNT + 2);
        for string in &self.strings {
            commands.extend(commands_for_string(string));
        }
        for (index, mode) in self.modes.iter().enumerate() {
            commands.push(StudioCommand::SetSoundboardMode { index, mode: *mode });
        }
        commands.push(StudioCommand::SetLocalCouplingGain {
            gain: self.local_coupling_gain,
        });
        commands.push(StudioCommand::SetGlobalCouplingGain {
            gain: self.global_coupling_gain,
        });
        commands
    }

    /// Applies `edit` to this picture and returns the commands that make
    /// the engine agree.
    ///
    /// An edit naming a string, key or mode that does not exist changes
    /// nothing and produces no commands — the same "a caller cannot crash
    /// this with a bad index" contract `piano-core`'s own setters keep,
    /// and the reason this returns a list rather than a `Result`.
    #[must_use]
    pub fn apply(&mut self, edit: &Edit) -> Vec<StudioCommand> {
        match edit {
            Edit::NoteOn { midi, velocity } => vec![StudioCommand::NoteOn {
                midi: *midi,
                velocity: clamped_velocity(*velocity),
            }],
            Edit::NoteOff { midi } => vec![StudioCommand::NoteOff { midi: *midi }],
            Edit::AllNotesOff => vec![StudioCommand::AllNotesOff],
            Edit::SustainPedal { down } => vec![StudioCommand::SustainPedal { down: *down }],
            Edit::SetString {
                midi,
                string_index,
                parameter,
                value,
            } => self.set_string(*midi, *string_index, *parameter, *value),
            Edit::SetStrings {
                strings,
                parameter,
                value,
            } => self.set_strings(strings, *parameter, *value),
            Edit::SetMode {
                index,
                parameter,
                value,
            } => self.set_mode(*index, *parameter, *value),
            Edit::SetBridge { parameter, value } => self.set_bridge(*parameter, *value),
        }
    }

    /// Writes one parameter on one string.
    fn set_string(
        &mut self,
        midi: u8,
        string_index: u8,
        parameter: StringParameter,
        value: f64,
    ) -> Vec<StudioCommand> {
        let Some(string) = self
            .strings
            .iter_mut()
            .find(|string| string.midi == midi && string.string_index == string_index)
        else {
            return Vec::new();
        };
        write_string_field(string, parameter, value);
        vec![command_for_field(string, parameter)]
    }

    /// Writes one parameter across a selection — a group applied, which
    /// `docs/PARAMETER-STUDIO.md` defines as N individual per-string
    /// writes rather than a new kind of entity.
    fn set_strings(
        &mut self,
        selection: &[StringRef],
        parameter: StringParameter,
        value: f64,
    ) -> Vec<StudioCommand> {
        let mut commands = Vec::with_capacity(selection.len());
        for target in selection {
            commands.extend(self.set_string(target.midi, target.string_index, parameter, value));
        }
        commands
    }

    /// Writes one parameter of one soundboard mode.
    fn set_mode(
        &mut self,
        index: usize,
        parameter: ModeParameter,
        value: f64,
    ) -> Vec<StudioCommand> {
        let Some(mode) = self.modes.get_mut(index) else {
            return Vec::new();
        };
        let value = parameter.range().clamp(value) as f32;
        match parameter {
            ModeParameter::FrequencyHz => mode.frequency_hz = value,
            ModeParameter::DecaySeconds => mode.decay_seconds = value,
            ModeParameter::Gain => mode.gain = value,
        }
        vec![StudioCommand::SetSoundboardMode { index, mode: *mode }]
    }

    /// Writes one of the bridge's two coupling gains.
    fn set_bridge(&mut self, parameter: BridgeParameter, value: f64) -> Vec<StudioCommand> {
        let gain = parameter.range().clamp(value) as f32;
        match parameter {
            BridgeParameter::LocalCouplingGain => {
                self.local_coupling_gain = gain;
                vec![StudioCommand::SetLocalCouplingGain { gain }]
            }
            BridgeParameter::GlobalCouplingGain => {
                self.global_coupling_gain = gain;
                vec![StudioCommand::SetGlobalCouplingGain { gain }]
            }
        }
    }

    /// The whole instrument, in the shape `GET /api/piano` serves.
    #[must_use]
    pub fn snapshot(&self) -> PianoSnapshot {
        PianoSnapshot {
            name: self.name.clone(),
            path: self.path.as_ref().map(|path| path.display().to_string()),
            keys: self.key_snapshots(),
            modes: self.mode_snapshots(),
            bridge: BridgeSnapshot {
                local_coupling_gain: self.local_coupling_gain,
                global_coupling_gain: self.global_coupling_gain,
            },
            groups: self.groups.clone(),
            ranges: Ranges::default(),
        }
    }

    /// Groups this state's flat string table back under its keys, which is
    /// how the page draws it.
    fn key_snapshots(&self) -> Vec<KeySnapshot> {
        let mut keys: Vec<KeySnapshot> = Vec::new();
        for string in &self.strings {
            if keys.last().map(|key| key.midi) != Some(string.midi) {
                keys.push(new_key_snapshot(string.midi));
            }
            if let Some(key) = keys.last_mut() {
                key.strings.push(string_snapshot(string));
            }
        }
        keys
    }

    fn mode_snapshots(&self) -> Vec<ModeSnapshot> {
        self.modes
            .iter()
            .enumerate()
            .map(|(index, mode)| ModeSnapshot {
                index,
                frequency_hz: mode.frequency_hz,
                decay_seconds: mode.decay_seconds,
                gain: mode.gain,
            })
            .collect()
    }

    /// This state, as a `.piano.json` — every string written out
    /// explicitly, per `docs/PARAMETER-STUDIO.md`'s "Persistence" section.
    ///
    /// Groups are carried through unchanged, as the named selections they
    /// are. They cannot change what anything resolves to here, because the
    /// `strings[]` tier this writes is more specific than any of them.
    #[must_use]
    pub fn to_piano_file(&self) -> PianoFile {
        PianoFile {
            name: self.name.clone(),
            defaults: ParameterOverrides::default(),
            registers: Registers::default(),
            groups: self.groups.clone(),
            strings: self.strings.iter().map(string_override).collect(),
            instrument: Instrument {
                soundboard_modes: self.modes.iter().map(mode_override).collect(),
                bridge: BridgeOverrides {
                    local_coupling_gain: Some(self.local_coupling_gain),
                    global_coupling_gain: Some(self.global_coupling_gain),
                },
            },
        }
    }
}

/// The six commands that put one string's whole state into the engine.
fn commands_for_string(string: &ResolvedString) -> [StudioCommand; 6] {
    let (midi, string_index) = (string.midi, string.string_index);
    [
        StudioCommand::SetStringDamping {
            midi,
            string_index,
            damping: string.damping,
        },
        StudioCommand::SetStringSustain {
            midi,
            string_index,
            sustain: string.sustain,
        },
        StudioCommand::SetStringInharmonicity {
            midi,
            string_index,
            inharmonicity: string.inharmonicity,
        },
        StudioCommand::SetStringDetune {
            midi,
            string_index,
            cents: string.detune_cents,
        },
        StudioCommand::SetStringSeed {
            midi,
            string_index,
            seed: string.seed,
        },
        StudioCommand::SetStringHammer {
            midi,
            string_index,
            hammer: string.hammer,
        },
    ]
}

/// Overwrites the one field `parameter` names, clamped into its range.
fn write_string_field(string: &mut ResolvedString, parameter: StringParameter, value: f64) {
    let value = parameter.range().clamp(value);
    match parameter {
        StringParameter::Damping => string.damping = value as f32,
        StringParameter::Sustain => string.sustain = value as f32,
        StringParameter::Inharmonicity => string.inharmonicity = value as f32,
        StringParameter::DetuneCents => string.detune_cents = value as f32,
        StringParameter::Seed => string.seed = value as u32,
        StringParameter::HammerContactExponent => string.hammer.contact_exponent = value as f32,
        StringParameter::HammerStiffness => string.hammer.stiffness = value as f32,
        StringParameter::HammerMass => string.hammer.mass = value as f32,
    }
}

/// The single command carrying `parameter`'s new value to the engine. All
/// three hammer fields ride one [`StudioCommand::SetStringHammer`],
/// because a whole hammer is the granularity `piano-core` offers.
fn command_for_field(string: &ResolvedString, parameter: StringParameter) -> StudioCommand {
    let (midi, string_index) = (string.midi, string.string_index);
    match parameter {
        StringParameter::Damping => StudioCommand::SetStringDamping {
            midi,
            string_index,
            damping: string.damping,
        },
        StringParameter::Sustain => StudioCommand::SetStringSustain {
            midi,
            string_index,
            sustain: string.sustain,
        },
        StringParameter::Inharmonicity => StudioCommand::SetStringInharmonicity {
            midi,
            string_index,
            inharmonicity: string.inharmonicity,
        },
        StringParameter::DetuneCents => StudioCommand::SetStringDetune {
            midi,
            string_index,
            cents: string.detune_cents,
        },
        StringParameter::Seed => StudioCommand::SetStringSeed {
            midi,
            string_index,
            seed: string.seed,
        },
        StringParameter::HammerContactExponent
        | StringParameter::HammerStiffness
        | StringParameter::HammerMass => StudioCommand::SetStringHammer {
            midi,
            string_index,
            hammer: string.hammer,
        },
    }
}

/// An empty key entry, named and classified for the page's keyboard strip.
fn new_key_snapshot(midi: u8) -> KeySnapshot {
    let name =
        PianoKey::from_midi(midi).map_or_else(|_| midi.to_string(), |key| key.name().to_string());
    KeySnapshot {
        midi,
        name,
        sharp: SHARP_SEMITONES.contains(&(midi % 12)),
        strings: Vec::new(),
    }
}

fn string_snapshot(string: &ResolvedString) -> StringSnapshot {
    StringSnapshot {
        string_index: string.string_index,
        damping: string.damping,
        sustain: string.sustain,
        inharmonicity: string.inharmonicity,
        detune_cents: string.detune_cents,
        seed: string.seed,
        hammer_contact_exponent: string.hammer.contact_exponent,
        hammer_stiffness: string.hammer.stiffness,
        hammer_mass: string.hammer.mass,
    }
}

fn string_override(string: &ResolvedString) -> StringOverride {
    StringOverride {
        midi: string.midi,
        string_index: string.string_index,
        overrides: ParameterOverrides {
            damping: Some(string.damping),
            sustain: Some(string.sustain),
            inharmonicity: Some(string.inharmonicity),
            detune_cents: Some(string.detune_cents),
            seed: Some(string.seed),
            hammer: HammerOverrides {
                contact_exponent: Some(string.hammer.contact_exponent),
                stiffness: Some(string.hammer.stiffness),
                mass: Some(string.hammer.mass),
            },
        },
    }
}

fn mode_override(mode: &SoundboardMode) -> SoundboardModeOverride {
    SoundboardModeOverride {
        frequency_hz: mode.frequency_hz,
        decay_seconds: mode.decay_seconds,
        gain: mode.gain,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::float_cmp)]

    use super::*;

    fn sample_rate() -> SampleRate {
        SampleRate::new(48_000.0).expect("48 kHz is valid")
    }

    fn state() -> LiveState {
        LiveState::from_file(
            &PianoFile::default(),
            None,
            Tuning::default(),
            sample_rate(),
        )
    }

    fn find(state: &LiveState, midi: u8, string_index: u8) -> ResolvedString {
        *state
            .strings
            .iter()
            .find(|string| string.midi == midi && string.string_index == string_index)
            .expect("midi/string_index exists on an 88-key instrument")
    }

    #[test]
    fn a_fresh_state_starts_from_the_engine_s_own_soundboard_table() {
        // A file with no `instrument.soundboard_modes` must show what the
        // running board actually has, not eight zeroes.
        assert_eq!(state().modes, DEFAULT_MODES);
    }

    #[test]
    fn setting_a_string_parameter_changes_that_string_and_emits_one_command() {
        let mut state = state();
        let commands = state.apply(&Edit::SetString {
            midi: 69,
            string_index: 0,
            parameter: StringParameter::Damping,
            value: 0.25,
        });
        assert_eq!(find(&state, 69, 0).damping, 0.25);
        assert_eq!(
            commands,
            vec![StudioCommand::SetStringDamping {
                midi: 69,
                string_index: 0,
                damping: 0.25,
            }]
        );
    }

    #[test]
    fn a_value_past_a_slider_s_end_is_clamped_not_rejected() {
        let mut state = state();
        let _ = state.apply(&Edit::SetString {
            midi: 69,
            string_index: 0,
            parameter: StringParameter::Damping,
            value: 40.0,
        });
        assert_eq!(find(&state, 69, 0).damping, 1.0);
    }

    #[test]
    fn an_edit_naming_a_string_that_does_not_exist_changes_nothing() {
        let mut state = state();
        let before = state.clone();
        // MIDI 21 is a monochord: there is no string index 2 to write.
        let commands = state.apply(&Edit::SetString {
            midi: 21,
            string_index: 2,
            parameter: StringParameter::Damping,
            value: 0.25,
        });
        assert!(commands.is_empty());
        assert_eq!(state, before);
    }

    #[test]
    fn any_hammer_field_emits_the_whole_hammer_because_that_is_the_setter_s_granularity() {
        let mut state = state();
        let commands = state.apply(&Edit::SetString {
            midi: 69,
            string_index: 0,
            parameter: StringParameter::HammerStiffness,
            value: 3.4e9,
        });
        let string = find(&state, 69, 0);
        assert_eq!(string.hammer.stiffness, 3.4e9);
        assert_eq!(
            commands,
            vec![StudioCommand::SetStringHammer {
                midi: 69,
                string_index: 0,
                hammer: string.hammer,
            }]
        );
    }

    #[test]
    fn applying_a_selection_writes_every_string_it_names_and_no_others() {
        let mut state = state();
        let commands = state.apply(&Edit::SetStrings {
            strings: vec![
                StringRef {
                    midi: 60,
                    string_index: 0,
                },
                StringRef {
                    midi: 61,
                    string_index: 1,
                },
            ],
            parameter: StringParameter::Sustain,
            value: 0.9,
        });
        assert_eq!(commands.len(), 2);
        assert_eq!(find(&state, 60, 0).sustain, 0.9);
        assert_eq!(find(&state, 61, 1).sustain, 0.9);
        assert_ne!(find(&state, 61, 0).sustain, 0.9);
    }

    #[test]
    fn setting_a_mode_parameter_keeps_the_mode_s_other_two_fields() {
        let mut state = state();
        let before = state.modes[0];
        let commands = state.apply(&Edit::SetMode {
            index: 0,
            parameter: ModeParameter::Gain,
            value: 0.5,
        });
        assert_eq!(state.modes[0].gain, 0.5);
        assert_eq!(state.modes[0].frequency_hz, before.frequency_hz);
        assert_eq!(state.modes[0].decay_seconds, before.decay_seconds);
        assert_eq!(commands.len(), 1);
    }

    #[test]
    fn a_mode_index_past_the_bank_changes_nothing() {
        let mut state = state();
        let before = state.clone();
        let commands = state.apply(&Edit::SetMode {
            index: MODE_COUNT,
            parameter: ModeParameter::Gain,
            value: 0.5,
        });
        assert!(commands.is_empty());
        assert_eq!(state, before);
    }

    #[test]
    fn both_bridge_gains_are_separately_settable() {
        let mut state = state();
        let _ = state.apply(&Edit::SetBridge {
            parameter: BridgeParameter::LocalCouplingGain,
            value: 0.4,
        });
        let _ = state.apply(&Edit::SetBridge {
            parameter: BridgeParameter::GlobalCouplingGain,
            value: 0.2,
        });
        assert_eq!(state.local_coupling_gain, 0.4);
        assert_eq!(state.global_coupling_gain, 0.2);
    }

    #[test]
    fn playing_never_touches_the_stored_state() {
        let mut state = state();
        let before = state.clone();
        let commands = state.apply(&Edit::NoteOn {
            midi: 69,
            velocity: 0.8,
        });
        assert_eq!(
            commands,
            vec![StudioCommand::NoteOn {
                midi: 69,
                velocity: 0.8,
            }]
        );
        assert_eq!(state, before);
    }

    #[test]
    fn the_full_command_list_covers_every_string_mode_and_gain() {
        let state = state();
        let expected = state.strings.len() * 6 + MODE_COUNT + 2;
        assert_eq!(state.commands().len(), expected);
    }

    #[test]
    fn a_saved_file_reloads_to_exactly_the_state_it_was_saved_from() {
        // The round trip that matters: edit, save, reload, and the
        // instrument must sound identical. This is what makes
        // `to_piano_file`'s "write everything, diff nothing" honest.
        let mut original = state();
        let _ = original.apply(&Edit::SetString {
            midi: 69,
            string_index: 0,
            parameter: StringParameter::Damping,
            value: 0.25,
        });
        let _ = original.apply(&Edit::SetMode {
            index: 3,
            parameter: ModeParameter::FrequencyHz,
            value: 333.0,
        });
        let _ = original.apply(&Edit::SetBridge {
            parameter: BridgeParameter::GlobalCouplingGain,
            value: 0.33,
        });

        let file = original.to_piano_file();
        let reloaded = LiveState::from_file(&file, None, Tuning::default(), sample_rate());
        assert_eq!(reloaded.strings, original.strings);
        assert_eq!(reloaded.modes, original.modes);
        assert_eq!(reloaded.global_coupling_gain, 0.33);
    }

    #[test]
    fn a_saved_file_survives_a_trip_through_json() {
        let state = state();
        let json = serde_json::to_string(&state.to_piano_file()).expect("serialises");
        let parsed: PianoFile = serde_json::from_str(&json).expect("parses its own output");
        assert_eq!(parsed, state.to_piano_file());
    }

    #[test]
    fn the_snapshot_groups_every_string_under_its_own_key() {
        let state = state();
        let snapshot = state.snapshot();
        assert_eq!(snapshot.keys.len(), 88);
        let total: usize = snapshot.keys.iter().map(|key| key.strings.len()).sum();
        assert_eq!(total, state.strings.len());
        let a4 = snapshot
            .keys
            .iter()
            .find(|key| key.midi == 69)
            .expect("A4 is on the keyboard");
        assert_eq!(a4.name, "A4");
        assert!(!a4.sharp);
    }

    #[test]
    fn the_snapshot_marks_black_keys_so_the_page_need_not_know_the_pattern() {
        let snapshot = state().snapshot();
        let sharp = snapshot
            .keys
            .iter()
            .find(|key| key.midi == 70)
            .expect("A#4 is on the keyboard");
        assert!(sharp.sharp);
    }

    #[test]
    fn a_group_in_the_loaded_file_is_carried_through_a_save() {
        let mut file = PianoFile::default();
        file.groups.push(Group {
            name: "darker overtones".to_string(),
            strings: vec![StringRef {
                midi: 60,
                string_index: 0,
            }],
            overrides: ParameterOverrides::default(),
        });
        let state = LiveState::from_file(&file, None, Tuning::default(), sample_rate());
        assert_eq!(state.to_piano_file().groups, file.groups);
    }
}
