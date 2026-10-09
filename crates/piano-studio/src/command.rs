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
    /// See [`piano_audio::AudioSession::set_sostenuto_pedal`].
    SostenutoPedal {
        /// Whether the pedal is down.
        down: bool,
    },
    /// See [`piano_audio::AudioSession::set_soft_pedal`].
    SoftPedal {
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
    /// See [`piano_audio::AudioSession::set_string_loop_zero_mix`].
    SetStringLoopZeroMix {
        /// MIDI note number of the key the string belongs to.
        midi: u8,
        /// Which string within that key's unison, `0`-based.
        string_index: u8,
        /// The loss filter's new zero weight.
        zero_mix: f32,
    },
    /// See [`piano_audio::AudioSession::set_string_strike_position`].
    SetStringStrikePosition {
        /// MIDI note number of the key the string belongs to.
        midi: u8,
        /// Which string within that key's unison, `0`-based.
        string_index: u8,
        /// The strike point, as a fraction of the loop length.
        position: f32,
    },
    /// See [`piano_audio::AudioSession::set_string_excitation_noise_mix`].
    SetStringExcitationNoiseMix {
        /// MIDI note number of the key the string belongs to.
        midi: u8,
        /// Which string within that key's unison, `0`-based.
        string_index: u8,
        /// The noise share of the next strike, `0..=1`.
        mix: f32,
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
    /// See [`piano_audio::AudioSession::set_soundboard_mix_gain`].
    SetSoundboardMixGain {
        /// The new soundboard mix gain: how much radiated soundboard
        /// signal is added back to the direct output.
        gain: f32,
    },
    /// See [`piano_audio::AudioSession::set_action_noise_gain`].
    SetActionNoiseGain {
        /// The keybed knock's level at full velocity.
        gain: f32,
    },
    /// See [`piano_audio::AudioSession::set_phantom_gain`].
    SetPhantomGain {
        /// The bass's phantom-partial gain.
        gain: f32,
    },
    /// See [`piano_audio::AudioSession::set_duplex_gain`].
    SetDuplexGain {
        /// The treble's duplex-segment gain.
        gain: f32,
    },
    /// See [`piano_audio::AudioSession::set_damper_strength`].
    SetDamperStrength {
        /// Fraction of a wave a seated damper takes per round trip.
        strength: f32,
    },
    /// See [`piano_audio::AudioSession::set_room_size`].
    SetRoomSize {
        /// How big the room is: how far apart its first reflections arrive.
        size: f32,
    },
    /// See [`piano_audio::AudioSession::set_room_reverb_seconds`].
    SetRoomReverbSeconds {
        /// How long the room rings in the bass, in seconds.
        seconds: f32,
    },
    /// See [`piano_audio::AudioSession::set_room_treble_reverb_seconds`].
    SetRoomTrebleReverbSeconds {
        /// How long the room rings in the treble, in seconds: lower is a darker room.
        seconds: f32,
    },
    /// See [`piano_audio::AudioSession::set_room_predelay`].
    SetRoomPredelay {
        /// Silence before the room answers, in milliseconds: longer sounds like a farther wall.
        milliseconds: f32,
    },
    /// See [`piano_audio::AudioSession::set_room_mix`].
    SetRoomMix {
        /// The room's new wet level; `0` is a dry instrument.
        mix: f32,
    },
    /// See [`piano_audio::AudioSession::set_limiter_threshold`].
    SetLimiterThreshold {
        /// Where the limiter starts compressing, as a fraction of full scale.
        threshold: f32,
    },
    /// See [`piano_audio::AudioSession::set_board_load_gain`].
    SetBoardLoadGain {
        /// How strongly the soundboard's modes take energy from the strings.
        gain: f32,
    },
    /// See [`piano_audio::AudioSession::set_master_gain`].
    SetMasterGain {
        /// The new master output gain, applied just before the limiter.
        gain: f32,
    },
    /// See [`piano_audio::AudioSession::set_velocity_curve`].
    SetVelocityCurve {
        /// The new exponent a strike velocity is warped through before it
        /// reaches the string.
        exponent: f32,
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
