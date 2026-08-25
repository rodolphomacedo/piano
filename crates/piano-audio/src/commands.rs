//! Plain-data commands that cross from a control thread to the audio thread.
//!
//! See [ADR-0005](../../../docs/adr/0005-lock-free-spsc-command-queue.md): the
//! audio thread never locks, so every command here is `Copy` plain data with no
//! pointer whose last clone could be dropped, and therefore freed, on the audio
//! thread.

/// A command the audio thread drains at the top of every callback.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Command {
    /// Strike a key. `midi` is a MIDI note number; `velocity` is clamped into
    /// `[0, 1]` by [`piano_core::PluckedString::pluck`].
    NoteOn {
        /// MIDI note number of the key struck.
        midi: u8,
        /// Strike strength, `0.0` to `1.0`.
        velocity: f32,
    },
    /// Silence every ringing voice immediately.
    AllNotesOff,
    /// Sets the high-frequency loss on every voice, ringing or not. See
    /// [`piano_core::string::PluckedString::set_damping`].
    SetDamping {
        /// New damping, `0.0` to `1.0`.
        damping: f32,
    },
    /// Sets the broadband loop gain on every voice, ringing or not. See
    /// [`piano_core::string::PluckedString::set_sustain`].
    SetSustain {
        /// New sustain, `0.0` to `1.0`.
        sustain: f32,
    },
    /// Releases one key early — MIDI note-off or, on a terminal that
    /// reports it, a computer-keyboard key-up. See
    /// [`piano_core::string::PluckedString::release`]. If the sustain
    /// pedal ([`Command::SustainPedal`]) is currently down, the voice is
    /// held rather than released immediately, same as a real piano.
    NoteOff {
        /// MIDI note number of the key released.
        midi: u8,
    },
    /// Sets the CC64 sustain-*pedal* hold state. This is **not** the same
    /// control as [`Command::SetSustain`] — that adjusts
    /// [`piano_core::string::PluckedString`]'s broadband decay-rate
    /// parameter, a voicing knob. This is the physical hold pedal: while
    /// `down`, a [`Command::NoteOff`] does not release its voice; the
    /// moment the pedal comes back up, everything it was holding is
    /// released for real.
    SustainPedal {
        /// `true` while the pedal is held down.
        down: bool,
    },
    /// Rebuilds one of the soundboard's resonant modes live. See
    /// [`piano_core::soundboard::Soundboard::set_mode`].
    SetSoundboardMode {
        /// Which mode, `0` to [`piano_core::soundboard::MODE_COUNT`] - 1;
        /// any other index is silently ignored.
        index: usize,
        /// The mode's new frequency, decay time and gain.
        mode: piano_core::soundboard::SoundboardMode,
    },
    /// Sets how strongly each voice's own unison strings couple to each
    /// other, live, on every voice. See
    /// [`piano_core::UnisonGroup::set_local_coupling_gain`].
    SetLocalCouplingGain {
        /// New local coupling gain, `0.0` to `1.0`.
        gain: f32,
    },
    /// Sets how strongly each voice couples to the shared, cross-key
    /// bridge bus, live, on every voice. See
    /// [`piano_core::UnisonGroup::set_global_coupling_gain`].
    SetGlobalCouplingGain {
        /// New global coupling gain, `0.0` to `1.0`.
        gain: f32,
    },
}
