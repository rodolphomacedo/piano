//! Realtime audio output for the piano synthesiser.
//!
//! `piano-core` knows nothing about threads, devices or wall-clock time; this
//! crate is where that boundary is crossed. It owns the `cpal` output
//! stream, the lock-free command queue that reaches it ([ADR-0005]), and the
//! platform-specific denormal control (`PERF-002`) — the one place in the
//! project with `unsafe`, narrowly scoped and documented at its use.
//!
//! [ADR-0005]: https://github.com/rodolphomacedo/piano/blob/main/docs/adr/0005-lock-free-spsc-command-queue.md
//!
//! # Realtime contract
//!
//! Everything inside the audio callback allocates nothing, locks nothing,
//! panics nowhere and has no unbounded loop. See
//! `docs/REALTIME-AUDIO-RULES.md`. `AudioSession::start` is the one
//! allocating, blocking call in this crate's public API — everything after
//! it is safe to call from any thread at any rate.

mod commands;
mod denormals;
mod engine;
mod error;
#[doc(hidden)]
pub mod fuzzing;
mod limiter;
pub mod offline;
#[path = "session_recovery.rs"]
mod recovery;
mod settings_log;
mod stream;
#[cfg(test)]
mod tests_no_allocation;
mod timing;
mod velocity_curve;
pub mod voicing;

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use piano_core::SampleRate;
use piano_params::Tuning;
use rtrb::Producer;

pub use engine::{DEFAULT_MASTER_GAIN, DEFAULT_SOUNDBOARD_MIX_GAIN};
pub use error::AudioError;
pub use recovery::Recovery;
pub use timing::TimingReport;
pub use velocity_curve::DEFAULT_VELOCITY_CURVE_EXPONENT;

use commands::Command;
use timing::CallbackTimer;

/// A live playback session: an open output stream plus the means to talk to
/// it.
///
/// Dropping this stops playback — the `cpal` stream inside is closed on
/// drop, like any other `cpal` stream.
pub struct AudioSession {
    // Never read directly: kept alive purely for its `Drop` impl, which
    // stops playback when the session is dropped.
    #[allow(dead_code)]
    stream: cpal::Stream,
    producer: Producer<Command>,
    timer: Arc<CallbackTimer>,
    sample_rate: SampleRate,
    /// What [`AudioSession::recover_if_needed`] needs to rebuild the
    /// stream after a device change (issue #20).
    recovery: recovery::RecoveryState,
}

impl AudioSession {
    /// Opens the default output device and starts playback immediately.
    ///
    /// # Errors
    ///
    /// Returns [`AudioError`] if no output device exists, its configuration
    /// cannot be read, or the stream cannot be built or started.
    pub fn start(tuning: Tuning) -> Result<Self, AudioError> {
        let timer = Arc::new(CallbackTimer::new());
        let failed = Arc::new(AtomicBool::new(false));
        let started = stream::start(tuning, Arc::clone(&timer), Arc::clone(&failed))?;
        Ok(Self {
            stream: started.stream,
            producer: started.producer,
            timer,
            sample_rate: started.sample_rate,
            recovery: recovery::RecoveryState::new(tuning, failed, started.device_name),
        })
    }

    /// Records `command` as the newest value of its setting, if it is one,
    /// and queues it for the engine. `false` when the queue was full and
    /// the engine will not see it — though a recorded setting still
    /// reaches a rebuilt engine.
    fn send(&mut self, command: Command) -> bool {
        self.recovery.settings.record(command);
        self.producer.push(command).is_ok()
    }

    /// Queues a note strike.
    ///
    /// Returns `false` if the command queue was full and the note was
    /// dropped rather than blocking the audio thread — see [ADR-0005].
    ///
    /// [ADR-0005]: https://github.com/rodolphomacedo/piano/blob/main/docs/adr/0005-lock-free-spsc-command-queue.md
    pub fn note_on(&mut self, midi: u8, velocity: f32) -> bool {
        self.send(Command::NoteOn { midi, velocity })
    }

    /// Queues silencing every ringing voice. Same drop-not-block behaviour as
    /// [`AudioSession::note_on`].
    pub fn all_notes_off(&mut self) -> bool {
        self.send(Command::AllNotesOff)
    }

    /// Queues releasing one key early — a MIDI note-off, or a
    /// computer-keyboard key-up on a terminal that reports one. While the
    /// sustain pedal ([`AudioSession::set_sustain_pedal`]) is held down,
    /// the voice is held rather than released immediately, same as a real
    /// piano. Same drop-not-block behaviour as [`AudioSession::note_on`].
    pub fn note_off(&mut self, midi: u8) -> bool {
        self.send(Command::NoteOff { midi })
    }

    /// Queues a new CC64 sustain-*pedal* hold state.
    ///
    /// **Not** the same control as [`AudioSession::set_sustain`] — that
    /// adjusts [`piano_core::PluckedString`]'s broadband decay-rate voicing
    /// parameter. This is the physical hold pedal: while `down`,
    /// [`AudioSession::note_off`] does not release its voice; the moment
    /// the pedal comes back up, everything it was holding is released for
    /// real. Same drop-not-block behaviour as [`AudioSession::note_on`].
    pub fn set_sustain_pedal(&mut self, down: bool) -> bool {
        self.send(Command::SustainPedal { down })
    }

    /// Queues the sustain pedal's continuous position, `0` (up) to `1`
    /// (down), for controllers that report half-pedalling (issue #61).
    /// Same drop-not-block behaviour as [`AudioSession::note_on`].
    pub fn set_sustain_pedal_position(&mut self, position: f32) -> bool {
        self.send(Command::SustainPedalPosition { position })
    }

    /// Queues the sostenuto (middle) pedal's state (issue #60). Same
    /// drop-not-block behaviour as [`AudioSession::note_on`].
    pub fn set_sostenuto_pedal(&mut self, down: bool) -> bool {
        self.send(Command::SostenutoPedal { down })
    }

    /// Queues the keybed thump's level at full velocity (issue #64); `0`
    /// silences the action noise. Same drop-not-block behaviour as
    /// [`AudioSession::note_on`].
    pub fn set_action_noise_gain(&mut self, gain: f32) -> bool {
        self.send(Command::SetActionNoiseGain { gain })
    }

    /// Queues the bass's phantom-partial gain (issue #54); `0` turns
    /// phantoms off. Same drop-not-block behaviour as
    /// [`AudioSession::note_on`].
    pub fn set_phantom_gain(&mut self, gain: f32) -> bool {
        self.send(Command::SetPhantomGain { gain })
    }

    /// Queues the room's wet level; `0` is a dry instrument. A new session
    /// starts at [`piano_core::room::DEFAULT_ROOM_MIX`]. Same
    /// drop-not-block behaviour as [`AudioSession::note_on`].
    pub fn set_room_mix(&mut self, mix: f32) -> bool {
        self.send(Command::SetRoomMix { mix })
    }

    /// Queues the room's shape change: scales the room: how far apart its first reflections are. Same
    /// drop-not-block behaviour as [`AudioSession::note_on`].
    pub fn set_room_size(&mut self, size: f32) -> bool {
        self.send(Command::SetRoomSize { size })
    }

    /// Queues the room's shape change: sets how long the room rings in the bass. Same
    /// drop-not-block behaviour as [`AudioSession::note_on`].
    pub fn set_room_reverb_seconds(&mut self, seconds: f32) -> bool {
        self.send(Command::SetRoomReverbSeconds { seconds })
    }

    /// Queues the room's shape change: sets how long the room rings at the top of the spectrum. Same
    /// drop-not-block behaviour as [`AudioSession::note_on`].
    pub fn set_room_treble_reverb_seconds(&mut self, seconds: f32) -> bool {
        self.send(Command::SetRoomTrebleReverbSeconds { seconds })
    }

    /// Queues the room's shape change: sets the silence before the room answers. Same
    /// drop-not-block behaviour as [`AudioSession::note_on`].
    pub fn set_room_predelay(&mut self, milliseconds: f32) -> bool {
        self.send(Command::SetRoomPredelay { milliseconds })
    }

    /// Queues the treble's duplex-segment gain; `0` mutes them. Same
    /// drop-not-block behaviour as [`AudioSession::note_on`].
    pub fn set_duplex_gain(&mut self, gain: f32) -> bool {
        self.send(Command::SetDuplexGain { gain })
    }

    /// Queues how hard every damper grips its string. Same drop-not-block
    /// behaviour as [`AudioSession::note_on`].
    pub fn set_damper_strength(&mut self, strength: f32) -> bool {
        self.send(Command::SetDamperStrength { strength })
    }

    /// Queues the soft (una corda) pedal's state (issue #59). Same
    /// drop-not-block behaviour as [`AudioSession::note_on`].
    pub fn set_soft_pedal(&mut self, down: bool) -> bool {
        self.send(Command::SoftPedal { down })
    }

    /// Queues a new damping (high-frequency loss) for every voice, applied
    /// live to voices already ringing as well as future strikes. `damping`
    /// is clamped into `[0, 1]` on the audio thread. Same drop-not-block
    /// behaviour as [`AudioSession::note_on`].
    pub fn set_damping(&mut self, damping: f32) -> bool {
        self.send(Command::SetDamping { damping })
    }

    /// Queues a new sustain (broadband loop gain) for every voice, applied
    /// live to voices already ringing as well as future strikes. `sustain`
    /// is clamped into `[0, 1]` on the audio thread. Same drop-not-block
    /// behaviour as [`AudioSession::note_on`].
    pub fn set_sustain(&mut self, sustain: f32) -> bool {
        self.send(Command::SetSustain { sustain })
    }

    /// Queues rebuilding one of the soundboard's resonant modes live. See
    /// [`piano_core::soundboard::Soundboard::set_mode`]. `index` outside
    /// `0..piano_core::soundboard::MODE_COUNT` is silently ignored on the
    /// audio thread. Same drop-not-block behaviour as
    /// [`AudioSession::note_on`].
    pub fn set_soundboard_mode(
        &mut self,
        index: usize,
        mode: piano_core::soundboard::SoundboardMode,
    ) -> bool {
        self.send(Command::SetSoundboardMode { index, mode })
    }

    /// Queues a new soundboard mix gain, live (issue #78): how much of the
    /// modal soundboard's radiated signal is added back to the direct
    /// output. `gain` is clamped into `[0, 2]` on the audio thread — `NaN`
    /// or a negative mutes the board. Same drop-not-block behaviour as
    /// [`AudioSession::note_on`].
    pub fn set_soundboard_mix_gain(&mut self, gain: f32) -> bool {
        self.send(Command::SetSoundboardMixGain { gain })
    }

    /// Queues a new master output gain, live (issue #79): a linear gain
    /// applied to the mixed, soundboard-coloured signal just before the
    /// output limiter, so turning down actually reduces limiting rather than
    /// feeding an already-engaged limiter. `gain` is clamped into `[0, 4]`
    /// on the audio thread — `NaN` or a negative silences the output. Same
    /// drop-not-block behaviour as [`AudioSession::note_on`].
    pub fn set_master_gain(&mut self, gain: f32) -> bool {
        self.send(Command::SetMasterGain { gain })
    }

    /// Queues a new velocity-curve exponent, live (issue #79): every strike
    /// arriving through [`AudioSession::note_on`] afterwards is warped
    /// through `pluck_velocity = velocity.powf(exponent)` before it reaches
    /// the string, rather than the previous straight pass-through. `1.0` is
    /// the old linear behaviour; `exponent` is clamped on the audio thread —
    /// `NaN` or a non-positive value falls back to a documented low bound.
    /// Same drop-not-block behaviour as [`AudioSession::note_on`].
    pub fn set_velocity_curve(&mut self, exponent: f32) -> bool {
        self.send(Command::SetVelocityCurve { exponent })
    }

    /// Queues a new local (within-group) unison coupling gain for every
    /// voice, live. See
    /// [`piano_core::UnisonGroup::set_local_coupling_gain`]. `gain` is
    /// clamped into `[0, 1]` on the audio thread. Same drop-not-block
    /// behaviour as [`AudioSession::note_on`].
    pub fn set_local_coupling_gain(&mut self, gain: f32) -> bool {
        self.send(Command::SetLocalCouplingGain { gain })
    }

    /// Queues a new global (cross-key, [`piano_core::BridgeBus`]) coupling
    /// gain for every voice, live. See
    /// [`piano_core::UnisonGroup::set_global_coupling_gain`]. `gain` is
    /// clamped into `[0, 1]` on the audio thread. Same drop-not-block
    /// behaviour as [`AudioSession::note_on`].
    pub fn set_global_coupling_gain(&mut self, gain: f32) -> bool {
        self.send(Command::SetGlobalCouplingGain { gain })
    }

    /// Queues a new damping for one string within `midi`'s unison, live.
    /// See [`piano_core::UnisonGroup::set_string_damping`]. An
    /// unrecognised `midi` or an out-of-range `string_index` is silently
    /// ignored on the audio thread. Same drop-not-block behaviour as
    /// [`AudioSession::note_on`].
    pub fn set_string_damping(&mut self, midi: u8, string_index: u8, damping: f32) -> bool {
        self.send(Command::SetStringDamping {
            midi,
            string_index,
            damping,
        })
    }

    /// Queues a new sustain for one string within `midi`'s unison, live.
    /// See [`piano_core::UnisonGroup::set_string_sustain`]. Same
    /// out-of-range and drop-not-block behaviour as
    /// [`AudioSession::set_string_damping`].
    pub fn set_string_sustain(&mut self, midi: u8, string_index: u8, sustain: f32) -> bool {
        self.send(Command::SetStringSustain {
            midi,
            string_index,
            sustain,
        })
    }

    /// Queues a new inharmonicity coefficient for one string within
    /// `midi`'s unison, live. See
    /// [`piano_core::UnisonGroup::set_string_inharmonicity`]. Same
    /// out-of-range and drop-not-block behaviour as
    /// [`AudioSession::set_string_damping`].
    pub fn set_string_inharmonicity(
        &mut self,
        midi: u8,
        string_index: u8,
        inharmonicity: f32,
    ) -> bool {
        self.send(Command::SetStringInharmonicity {
            midi,
            string_index,
            inharmonicity,
        })
    }

    /// Queues retuning one string within `midi`'s unison to `cents` away
    /// from that unison's base frequency, live. See
    /// [`piano_core::UnisonGroup::set_string_detune`]. Same out-of-range
    /// and drop-not-block behaviour as [`AudioSession::set_string_damping`].
    pub fn set_string_detune(&mut self, midi: u8, string_index: u8, cents: f32) -> bool {
        self.send(Command::SetStringDetune {
            midi,
            string_index,
            cents,
        })
    }

    /// Queues reseeding one string's excitation noise for its *next*
    /// strike. See [`piano_core::UnisonGroup::set_string_seed`]. Same
    /// out-of-range and drop-not-block behaviour as
    /// [`AudioSession::set_string_damping`].
    pub fn set_string_seed(&mut self, midi: u8, string_index: u8, seed: u32) -> bool {
        self.send(Command::SetStringSeed {
            midi,
            string_index,
            seed,
        })
    }

    /// Queues moving one string's loss-filter zero, live. See
    /// [`piano_core::UnisonGroup::set_string_loop_zero_mix`]. Same
    /// out-of-range and drop-not-block behaviour as
    /// [`AudioSession::set_string_damping`].
    pub fn set_string_loop_zero_mix(&mut self, midi: u8, string_index: u8, zero_mix: f32) -> bool {
        self.send(Command::SetStringLoopZeroMix {
            midi,
            string_index,
            zero_mix,
        })
    }

    /// Queues moving where the hammer strikes one string, for its *next*
    /// strike. See [`piano_core::UnisonGroup::set_string_strike_position`].
    /// Same out-of-range and drop-not-block behaviour as
    /// [`AudioSession::set_string_damping`].
    pub fn set_string_strike_position(
        &mut self,
        midi: u8,
        string_index: u8,
        position: f32,
    ) -> bool {
        self.send(Command::SetStringStrikePosition {
            midi,
            string_index,
            position,
        })
    }

    /// Queues how noisy one string's *next* strike is. See
    /// [`piano_core::UnisonGroup::set_string_excitation_noise_mix`]. Same
    /// out-of-range and drop-not-block behaviour as
    /// [`AudioSession::set_string_damping`].
    pub fn set_string_excitation_noise_mix(
        &mut self,
        midi: u8,
        string_index: u8,
        mix: f32,
    ) -> bool {
        self.send(Command::SetStringExcitationNoiseMix {
            midi,
            string_index,
            mix,
        })
    }

    /// Queues changing one string's felt-contact physics for its *next*
    /// strike. See [`piano_core::UnisonGroup::set_string_hammer`]. Same
    /// out-of-range and drop-not-block behaviour as
    /// [`AudioSession::set_string_damping`].
    pub fn set_string_hammer(
        &mut self,
        midi: u8,
        string_index: u8,
        hammer: piano_core::hammer::HammerConfig,
    ) -> bool {
        self.send(Command::SetStringHammer {
            midi,
            string_index,
            hammer,
        })
    }

    /// The sample rate the engine is tuned for: the output device's own
    /// rate, not necessarily 48 kHz.
    #[inline]
    #[must_use]
    pub fn sample_rate(&self) -> SampleRate {
        self.sample_rate
    }

    /// A snapshot of callback timing so far: p50 through p99.9 and the max,
    /// in microseconds. See issue #18.
    #[must_use]
    pub fn timing_report(&self) -> TimingReport {
        self.timer.report()
    }
}

impl core::fmt::Debug for AudioSession {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("AudioSession")
            .field("sample_rate", &self.sample_rate)
            .finish_non_exhaustive()
    }
}
