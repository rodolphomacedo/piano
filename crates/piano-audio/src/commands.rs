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
}
