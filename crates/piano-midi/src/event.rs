//! Decoding raw MIDI bytes into the events this synthesiser acts on.
//!
//! Deliberately pure: no I/O, no threads, so [`parse`] is tested the same
//! way as any DSP primitive in `piano-core` — with plain byte slices in,
//! plain values out.

/// A decoded MIDI channel-voice event, filtered down to what this
/// synthesiser understands.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MidiEvent {
    /// A key was struck. `velocity` is normalised into `[0, 1]`.
    NoteOn {
        /// MIDI note number, `0..=127`.
        note: u8,
        /// Strike strength, `0.0` to `1.0`.
        velocity: f32,
    },
    /// A key was released. The current instrument has no release envelope
    /// (see the crate-level docs), so this is decoded but not acted on yet
    /// — a real piano note keeps ringing after the key comes up, same as
    /// `piano keyboard` today.
    NoteOff {
        /// MIDI note number, `0..=127`.
        note: u8,
    },
    /// A control change message, e.g. a knob or pedal.
    ControlChange {
        /// The controller number, `0..=127`.
        controller: u8,
        /// The controller value, normalised into `[0, 1]`.
        value: f32,
    },
}

/// Scales a 7-bit MIDI data byte (`0..=127`) into `[0.0, 1.0]`.
#[inline]
fn normalize(data: u8) -> f32 {
    f32::from(data) / 127.0
}

/// Decodes one MIDI message.
///
/// Returns `None` for messages this synthesiser does not act on — system
/// messages, aftertouch, pitch bend, program change, and anything shorter
/// than a complete channel-voice message. An unhandled message is not a
/// malformed one, so this never errors.
#[must_use]
pub(crate) fn parse(message: &[u8]) -> Option<MidiEvent> {
    let &[status, note_or_controller, value, ..] = message else {
        return None;
    };
    match status & 0xF0 {
        // A note-on with velocity 0 is the running-status idiom for
        // note-off, used by some controllers to avoid sending a status
        // byte change mid-stream.
        0x90 if value > 0 => Some(MidiEvent::NoteOn {
            note: note_or_controller,
            velocity: normalize(value),
        }),
        0x90 | 0x80 => Some(MidiEvent::NoteOff {
            note: note_or_controller,
        }),
        0xB0 => Some(MidiEvent::ControlChange {
            controller: note_or_controller,
            value: normalize(value),
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn decodes_a_note_on() {
        assert_eq!(
            parse(&[0x90, 69, 100]),
            Some(MidiEvent::NoteOn {
                note: 69,
                velocity: 100.0 / 127.0
            })
        );
    }

    #[test]
    fn a_note_on_with_zero_velocity_is_a_note_off() {
        assert_eq!(parse(&[0x90, 69, 0]), Some(MidiEvent::NoteOff { note: 69 }));
    }

    #[test]
    fn decodes_an_explicit_note_off() {
        assert_eq!(
            parse(&[0x80, 69, 64]),
            Some(MidiEvent::NoteOff { note: 69 })
        );
    }

    #[test]
    fn decodes_a_control_change() {
        assert_eq!(
            parse(&[0xB0, 74, 127]),
            Some(MidiEvent::ControlChange {
                controller: 74,
                value: 1.0
            })
        );
    }

    #[test]
    fn respects_the_channel_nibble() {
        // Channel 3 (0-indexed) note-on: status high nibble is still 0x90.
        assert_eq!(
            parse(&[0x92, 60, 90]),
            Some(MidiEvent::NoteOn {
                note: 60,
                velocity: 90.0 / 127.0
            })
        );
    }

    #[test]
    fn ignores_messages_it_does_not_understand() {
        assert_eq!(parse(&[0xC0, 5]), None); // program change, 2 bytes
        assert_eq!(parse(&[0xE0, 0, 64]), None); // pitch bend
        assert_eq!(parse(&[0xF8]), None); // system realtime, 1 byte
        assert_eq!(parse(&[]), None);
    }
}
