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
    /// Sets how much of the soundboard's radiated signal is mixed back into
    /// the direct output, live (issue #78). Clamped into
    /// `[0.0, 2.0]` engine-side; `NaN` or a negative mutes the soundboard.
    SetSoundboardMixGain {
        /// New soundboard mix gain. `0.0` mutes the board, `1.0` is unity,
        /// values above are a deliberate boost up to the engine's ceiling.
        gain: f32,
    },
    /// Sets the master output gain, applied just before the output limiter,
    /// live (issue #79). Clamped into `[0.0, 4.0]` engine-side; `NaN` or a
    /// negative silences the output.
    SetMasterGain {
        /// New master gain. `1.0` is unity (the level everything else was
        /// measured at); values above deliberately drive the limiter.
        gain: f32,
    },
    /// Sets the velocity-curve exponent a strike velocity is warped through
    /// on its way into [`Command::NoteOn`]'s handling, live (issue #79).
    /// Clamped engine-side; `NaN` or a non-positive value falls back to the
    /// low bound rather than producing a degenerate (constant or inverted)
    /// curve.
    SetVelocityCurve {
        /// New exponent. `1.0` is a linear map; above `1.0` spreads the
        /// bottom of the velocity range out (softer touches separate more)
        /// at the cost of compressing the top further — see
        /// `engine::DEFAULT_VELOCITY_CURVE_EXPONENT` for why that is the
        /// direction a felt hammer's own response needs.
        exponent: f32,
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
    /// Sets one string's damping, live. See
    /// [`piano_core::UnisonGroup::set_string_damping`].
    SetStringDamping {
        /// MIDI note number of the key whose unison this string belongs to.
        midi: u8,
        /// Which string within that key's unison, `0`-based.
        string_index: u8,
        /// New damping, `0.0` to `1.0`.
        damping: f32,
    },
    /// Sets one string's sustain, live. See
    /// [`piano_core::UnisonGroup::set_string_sustain`].
    SetStringSustain {
        /// MIDI note number of the key whose unison this string belongs to.
        midi: u8,
        /// Which string within that key's unison, `0`-based.
        string_index: u8,
        /// New sustain, `0.0` to `1.0`.
        sustain: f32,
    },
    /// Sets one string's inharmonicity coefficient, live. See
    /// [`piano_core::UnisonGroup::set_string_inharmonicity`].
    SetStringInharmonicity {
        /// MIDI note number of the key whose unison this string belongs to.
        midi: u8,
        /// Which string within that key's unison, `0`-based.
        string_index: u8,
        /// New inharmonicity coefficient `B`.
        inharmonicity: f32,
    },
    /// Retunes one string to `cents` away from its unison's base
    /// frequency, live. See [`piano_core::UnisonGroup::set_string_detune`].
    SetStringDetune {
        /// MIDI note number of the key whose unison this string belongs to.
        midi: u8,
        /// Which string within that key's unison, `0`-based.
        string_index: u8,
        /// Detune from the unison's base frequency, in cents.
        cents: f32,
    },
    /// Reseeds one string's excitation noise for its *next* strike. See
    /// [`piano_core::UnisonGroup::set_string_seed`].
    SetStringSeed {
        /// MIDI note number of the key whose unison this string belongs to.
        midi: u8,
        /// Which string within that key's unison, `0`-based.
        string_index: u8,
        /// New RNG seed.
        seed: u32,
    },
    /// Changes one string's felt-contact physics for its *next* strike.
    /// See [`piano_core::UnisonGroup::set_string_hammer`].
    SetStringHammer {
        /// MIDI note number of the key whose unison this string belongs to.
        midi: u8,
        /// Which string within that key's unison, `0`-based.
        string_index: u8,
        /// The string's new hammer contact physics.
        hammer: piano_core::hammer::HammerConfig,
    },
}
