//! The seam between a decoded MIDI message and the instrument it plays.
//!
//! `piano_audio::AudioSession` can only exist where there is a real output
//! device, which no CI machine has and which no unit test should need. Every
//! MIDI test this project had therefore stopped at
//! `piano_midi::MidiEvent` — the mapping from a decoded event to the engine
//! call it is supposed to make was covered by nothing at all, which is how
//! `crate::studio` came to silently discard every control change (the
//! sustain pedal included) while `crate::midi` handled them.
//!
//! [`NoteSink`] is that mapping's boundary: `AudioSession` implements it in
//! production, and the tests implement it with a recorder, so "CC64 above the
//! threshold presses the sustain pedal" is a statement about code that can be
//! asserted rather than a statement about a comment.

use piano_audio::AudioSession;

/// What playing a MIDI message needs from an instrument.
///
/// Deliberately smaller than [`AudioSession`]'s own surface: only the calls a
/// MIDI message can produce belong here, so a new method on `AudioSession`
/// does not silently widen what a controller is able to reach.
///
/// Every method drops [`AudioSession`]'s "was the command queued?" boolean.
/// A dropped command is already the documented behaviour under queue
/// pressure (ADR-0005) and neither caller has ever acted on it; carrying it
/// through this trait would imply a recovery path that does not exist.
pub(crate) trait NoteSink {
    /// Strikes `midi` at `velocity`, in `[0, 1]`.
    fn note_on(&mut self, midi: u8, velocity: f32);
    /// Releases `midi` — damped now, or when the sustain pedal comes up.
    fn note_off(&mut self, midi: u8);
    /// Sets the CC64 hold-pedal state.
    fn set_sustain_pedal(&mut self, down: bool);
    /// Sets the global high-frequency loss ("brightness") knob.
    fn set_damping(&mut self, damping: f32);
    /// Sets the global broadband decay-rate voicing knob. Not the pedal —
    /// see [`NoteSink::set_sustain_pedal`].
    fn set_sustain(&mut self, sustain: f32);
}

impl NoteSink for AudioSession {
    fn note_on(&mut self, midi: u8, velocity: f32) {
        AudioSession::note_on(self, midi, velocity);
    }

    fn note_off(&mut self, midi: u8) {
        AudioSession::note_off(self, midi);
    }

    fn set_sustain_pedal(&mut self, down: bool) {
        AudioSession::set_sustain_pedal(self, down);
    }

    fn set_damping(&mut self, damping: f32) {
        AudioSession::set_damping(self, damping);
    }

    fn set_sustain(&mut self, sustain: f32) {
        AudioSession::set_sustain(self, sustain);
    }
}

#[cfg(test)]
pub(crate) mod recorder {
    //! A [`NoteSink`](super::NoteSink) that writes down what it was asked to
    //! do, so a test can assert on the calls a MIDI message produces.

    use super::NoteSink;

    /// One call made on a [`Recorder`], in the order it arrived. Ordering
    /// matters as much as content: a note-off that reaches the engine before
    /// its note-on is as broken as one that never arrives.
    #[derive(Debug, Clone, Copy, PartialEq)]
    pub(crate) enum Played {
        NoteOn { midi: u8, velocity: f32 },
        NoteOff { midi: u8 },
        SustainPedal { down: bool },
        Damping { damping: f32 },
        Sustain { sustain: f32 },
    }

    /// Records every [`NoteSink`] call for a test to assert against.
    #[derive(Debug, Default)]
    pub(crate) struct Recorder {
        pub(crate) played: Vec<Played>,
    }

    impl NoteSink for Recorder {
        fn note_on(&mut self, midi: u8, velocity: f32) {
            self.played.push(Played::NoteOn { midi, velocity });
        }

        fn note_off(&mut self, midi: u8) {
            self.played.push(Played::NoteOff { midi });
        }

        fn set_sustain_pedal(&mut self, down: bool) {
            self.played.push(Played::SustainPedal { down });
        }

        fn set_damping(&mut self, damping: f32) {
            self.played.push(Played::Damping { damping });
        }

        fn set_sustain(&mut self, sustain: f32) {
            self.played.push(Played::Sustain { sustain });
        }
    }
}
