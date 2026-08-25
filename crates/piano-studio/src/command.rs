//! What the studio's control surface asks the running instrument to do.
//!
//! [`StudioCommand`] is deliberately *not* `piano_audio`'s own ring
//! command: that type is private to `piano-audio` and travels a lock-free
//! SPSC ring with exactly one producer (ADR-0005). The web server runs on
//! its own threads and must never be that producer, so it emits these
//! instead, and whoever owns the [`piano_audio::AudioSession`] —
//! `piano-cli`'s `studio` subcommand — translates each into the matching
//! setter call from its own single thread.
//!
//! Every variant is `Copy` plain data, for the same reason every ring
//! command is: nothing here can be the last owner of something that would
//! then have to be dropped downstream.

use piano_core::hammer::HammerConfig;
use piano_core::soundboard::SoundboardMode;

/// One instruction for the running instrument.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum StudioCommand {
    /// Strike a key. See [`piano_audio::AudioSession::note_on`].
    NoteOn {
        /// MIDI note number of the key struck.
        midi: u8,
        /// How hard, `0.0` to `1.0`.
        velocity: f32,
    },
    /// Release a key. See [`piano_audio::AudioSession::note_off`].
    NoteOff {
        /// MIDI note number of the key released.
        midi: u8,
    },
    /// Silence every ringing voice. See
    /// [`piano_audio::AudioSession::all_notes_off`].
    AllNotesOff,
    /// Hold or release the sustain pedal. See
    /// [`piano_audio::AudioSession::set_sustain_pedal`].
    SustainPedal {
        /// Whether the pedal is down.
        down: bool,
    },
    /// See [`piano_audio::AudioSession::set_string_damping`].
    SetStringDamping {
        /// MIDI note number of the key the string belongs to.
        midi: u8,
        /// Which string within that key's unison, `0`-based.
        string_index: u8,
        /// The new damping.
        damping: f32,
    },
    /// See [`piano_audio::AudioSession::set_string_sustain`].
    SetStringSustain {
        /// MIDI note number of the key the string belongs to.
        midi: u8,
        /// Which string within that key's unison, `0`-based.
        string_index: u8,
        /// The new sustain.
        sustain: f32,
    },
    /// See [`piano_audio::AudioSession::set_string_inharmonicity`].
    SetStringInharmonicity {
        /// MIDI note number of the key the string belongs to.
        midi: u8,
        /// Which string within that key's unison, `0`-based.
        string_index: u8,
        /// The new inharmonicity coefficient.
        inharmonicity: f32,
    },
    /// See [`piano_audio::AudioSession::set_string_detune`].
    SetStringDetune {
        /// MIDI note number of the key the string belongs to.
        midi: u8,
        /// Which string within that key's unison, `0`-based.
        string_index: u8,
        /// The new offset from the unison's base frequency, in cents.
        cents: f32,
    },
    /// See [`piano_audio::AudioSession::set_string_seed`].
    SetStringSeed {
        /// MIDI note number of the key the string belongs to.
        midi: u8,
        /// Which string within that key's unison, `0`-based.
        string_index: u8,
        /// The new excitation seed, taking effect on the next strike.
        seed: u32,
    },
    /// See [`piano_audio::AudioSession::set_string_hammer`].
    SetStringHammer {
        /// MIDI note number of the key the string belongs to.
        midi: u8,
        /// Which string within that key's unison, `0`-based.
        string_index: u8,
        /// The new felt-contact physics, taking effect on the next strike.
        hammer: HammerConfig,
    },
    /// See [`piano_audio::AudioSession::set_soundboard_mode`].
    SetSoundboardMode {
        /// Which mode, `0` to [`piano_core::soundboard::MODE_COUNT`] - 1.
        index: usize,
        /// The mode's new frequency, decay time and gain.
        mode: SoundboardMode,
    },
    /// See [`piano_audio::AudioSession::set_local_coupling_gain`].
    SetLocalCouplingGain {
        /// The new within-unison coupling gain.
        gain: f32,
    },
    /// See [`piano_audio::AudioSession::set_global_coupling_gain`].
    SetGlobalCouplingGain {
        /// The new cross-key coupling gain.
        gain: f32,
    },
}
