//! The latest value of every setting an [`crate::AudioSession`] has sent,
//! kept on the control thread so a rebuilt engine can be brought back to
//! exactly the instrument the player had (issue #20).
//!
//! A device change builds a fresh engine at the new device's sample rate,
//! and a fresh engine knows nothing of the studio's edits, the pedals or the
//! room. Rather than make every caller remember and resend its own state,
//! the session records each setting command as it goes out — the newest per
//! setting, superseding older ones — and replays them in the order they
//! were sent, so a global setting sent after a per-string one still
//! overrides it, as it did the first time.
//!
//! Notes are deliberately not recorded: a rebuilt engine starts silent,
//! which is what guarantees no note is left stuck across a device change.

use std::collections::BTreeMap;

use crate::commands::Command;

/// What one setting command overwrites: the command's kind, plus which
/// mode or string it addresses when it addresses one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct SettingKey {
    kind: u8,
    target: (u8, u8, usize),
}

/// The latest command for every setting, each stamped with the order it
/// was sent in.
#[derive(Debug, Default)]
pub(crate) struct SettingsLog {
    latest: BTreeMap<SettingKey, (u64, Command)>,
    sent: u64,
}

impl SettingsLog {
    /// Records `command` if it is a setting; notes are ignored.
    pub(crate) fn record(&mut self, command: Command) {
        let Some(key) = setting_key(&command) else {
            return;
        };
        self.sent = self.sent.wrapping_add(1);
        self.latest.insert(key, (self.sent, command));
    }

    /// Every recorded setting, oldest first.
    pub(crate) fn replay(&self) -> Vec<Command> {
        let mut stamped: Vec<(u64, Command)> = self.latest.values().copied().collect();
        stamped.sort_by_key(|(sent, _)| *sent);
        stamped.into_iter().map(|(_, command)| command).collect()
    }
}

/// The setting `command` overwrites, or `None` for a note event.
fn setting_key(command: &Command) -> Option<SettingKey> {
    let global = |kind: u8| {
        Some(SettingKey {
            kind,
            target: (0, 0, 0),
        })
    };
    let string = |kind: u8, midi: u8, index: u8| {
        Some(SettingKey {
            kind,
            target: (midi, index, 0),
        })
    };
    match *command {
        Command::NoteOn { .. } | Command::NoteOff { .. } | Command::AllNotesOff => None,
        Command::SustainPedal { .. } | Command::SustainPedalPosition { .. } => global(1),
        Command::SostenutoPedal { .. } => global(2),
        Command::SoftPedal { .. } => global(3),
        Command::SetDamping { .. } => global(4),
        Command::SetSustain { .. } => global(5),
        Command::SetActionNoiseGain { .. } => global(6),
        Command::SetPhantomGain { .. } => global(7),
        Command::SetRoomMix { .. } => global(8),
        Command::SetDuplexGain { .. } => global(21),
        Command::SetDamperStrength { .. } => global(25),
        Command::SetRoomSize { .. } => global(26),
        Command::SetRoomReverbSeconds { .. } => global(27),
        Command::SetRoomTrebleReverbSeconds { .. } => global(28),
        Command::SetRoomPredelay { .. } => global(29),
        Command::SetSoundboardMixGain { .. } => global(9),
        Command::SetMasterGain { .. } => global(10),
        Command::SetLimiterThreshold { .. } => global(30),
        Command::SetVelocityCurve { .. } => global(11),
        Command::SetLocalCouplingGain { .. } => global(12),
        Command::SetGlobalCouplingGain { .. } => global(13),
        Command::SetSoundboardMode { index, .. } => Some(SettingKey {
            kind: 14,
            target: (0, 0, index),
        }),
        Command::SetStringDamping {
            midi, string_index, ..
        } => string(15, midi, string_index),
        Command::SetStringSustain {
            midi, string_index, ..
        } => string(16, midi, string_index),
        Command::SetStringInharmonicity {
            midi, string_index, ..
        } => string(17, midi, string_index),
        Command::SetStringDetune {
            midi, string_index, ..
        } => string(18, midi, string_index),
        Command::SetStringSeed {
            midi, string_index, ..
        } => string(19, midi, string_index),
        Command::SetStringHammer {
            midi, string_index, ..
        } => string(20, midi, string_index),
        Command::SetStringLoopZeroMix {
            midi, string_index, ..
        } => string(22, midi, string_index),
        Command::SetStringStrikePosition {
            midi, string_index, ..
        } => string(23, midi, string_index),
        Command::SetStringExcitationNoiseMix {
            midi, string_index, ..
        } => string(24, midi, string_index),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notes_are_never_replayed() {
        let mut log = SettingsLog::default();
        log.record(Command::NoteOn {
            midi: 60,
            velocity: 0.5,
        });
        log.record(Command::NoteOff { midi: 60 });
        assert_eq!(log.replay(), Vec::new());
    }

    #[test]
    fn only_the_newest_value_of_a_setting_is_replayed() {
        let mut log = SettingsLog::default();
        log.record(Command::SetRoomMix { mix: 0.1 });
        log.record(Command::SetRoomMix { mix: 0.4 });
        assert_eq!(log.replay(), vec![Command::SetRoomMix { mix: 0.4 }]);
    }

    #[test]
    fn settings_replay_in_the_order_they_were_last_sent() {
        let mut log = SettingsLog::default();
        let string = Command::SetStringDamping {
            midi: 60,
            string_index: 0,
            damping: 0.2,
        };
        log.record(string);
        log.record(Command::SetDamping { damping: 0.7 });
        assert_eq!(
            log.replay(),
            vec![string, Command::SetDamping { damping: 0.7 }]
        );
    }

    #[test]
    fn different_strings_keep_their_own_values() {
        let mut log = SettingsLog::default();
        for string_index in 0..3 {
            log.record(Command::SetStringSeed {
                midi: 60,
                string_index,
                seed: u32::from(string_index),
            });
        }
        assert_eq!(log.replay().len(), 3);
    }
}
