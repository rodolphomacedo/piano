//! Musical naming and tuning: the layer that turns "A4" into 440 Hz.
//!
//! Kept out of `piano-core` on purpose. The DSP does not care about note names,
//! and a synthesiser driven by MIDI, by a score, or by a slider should share the
//! same string model without dragging a note parser onto the audio thread.

#![cfg_attr(not(feature = "std"), no_std)]
#![forbid(unsafe_code)]

use core::str::FromStr;

use piano_core::{Hz, ParamError, math};
use thiserror::Error;

/// MIDI note number of the lowest key on an 88-key piano (A0).
pub const LOWEST_PIANO_KEY: u8 = 21;

/// MIDI note number of the highest key on an 88-key piano (C8).
pub const HIGHEST_PIANO_KEY: u8 = 108;

/// MIDI note number of concert A.
pub const CONCERT_A_KEY: u8 = 69;

/// Default frequency of concert A, in hertz.
pub const CONCERT_A_HERTZ: f32 = 440.0;

/// Semitones in an octave.
const SEMITONES_PER_OCTAVE: f32 = 12.0;

/// Note names in an octave, in semitone order starting at C.
const NOTE_NAMES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

/// Why a note could not be built or parsed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum NoteError {
    /// The MIDI number is outside the 88-key piano range.
    #[error("MIDI note {0} is outside the piano range 21..=108")]
    OutOfRange(u8),

    /// The text is not a note name such as `A4`, `C#3` or `Bb2`.
    #[error("could not parse a note name; expected something like A4, C#3 or Bb2")]
    Unparsable,
}

/// A key on the piano, identified by its MIDI note number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct PianoKey(u8);

impl PianoKey {
    /// Wraps a MIDI note number.
    ///
    /// # Errors
    ///
    /// Returns [`NoteError::OutOfRange`] outside `A0..=C8`, the range of a
    /// standard 88-key instrument.
    pub const fn from_midi(number: u8) -> Result<Self, NoteError> {
        if number < LOWEST_PIANO_KEY || number > HIGHEST_PIANO_KEY {
            return Err(NoteError::OutOfRange(number));
        }
        Ok(Self(number))
    }

    /// The MIDI note number.
    #[inline]
    #[must_use]
    pub const fn midi_number(self) -> u8 {
        self.0
    }

    /// Position on the keyboard, `0` for the lowest key.
    #[inline]
    #[must_use]
    pub const fn key_index(self) -> u8 {
        self.0 - LOWEST_PIANO_KEY
    }

    /// The frequency of this key under `tuning`.
    #[must_use]
    pub fn frequency(self, tuning: Tuning) -> Hz {
        let semitones = f32::from(self.0) - f32::from(CONCERT_A_KEY);
        let hertz = tuning.reference().hertz() * math::powf(2.0, semitones / SEMITONES_PER_OCTAVE);
        // A key in range at a positive reference always yields a positive,
        // finite frequency; the fallback keeps the signature panic-free.
        Hz::new(hertz).unwrap_or_else(|_| tuning.reference())
    }

    /// The note name, e.g. `"A#3"`. Sharps only; no enharmonic spelling.
    #[must_use]
    pub fn name(self) -> NoteName {
        let semitone = usize::from(self.0 % 12);
        let octave = i16::from(self.0 / 12) - 1;
        NoteName {
            pitch_class: NOTE_NAMES.get(semitone).copied().unwrap_or("C"),
            octave,
        }
    }
}

/// A rendered note name: pitch class plus octave.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NoteName {
    /// The pitch class, such as `"C"` or `"F#"`.
    pub pitch_class: &'static str,
    /// The scientific-pitch octave, where middle C is `C4`.
    pub octave: i16,
}

impl core::fmt::Display for NoteName {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(formatter, "{}{}", self.pitch_class, self.octave)
    }
}

impl FromStr for PianoKey {
    type Err = NoteError;

    /// Parses `A4`, `C#3`, `Bb2` and the like.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let trimmed = text.trim();
        let mut characters = trimmed.chars();
        let letter = characters.next().ok_or(NoteError::Unparsable)?;
        let mut semitone = i32::from(natural_semitone(letter)?);

        let rest = characters.as_str();
        let octave_text = match rest.as_bytes().first() {
            Some(b'#') => {
                semitone += 1;
                rest.get(1..).ok_or(NoteError::Unparsable)?
            }
            Some(b'b') => {
                semitone -= 1;
                rest.get(1..).ok_or(NoteError::Unparsable)?
            }
            _ => rest,
        };

        let octave: i32 = octave_text.parse().map_err(|_| NoteError::Unparsable)?;
        let midi = (octave + 1) * 12 + semitone;
        let midi = u8::try_from(midi).map_err(|_| NoteError::Unparsable)?;
        Self::from_midi(midi)
    }
}

fn natural_semitone(letter: char) -> Result<u8, NoteError> {
    match letter.to_ascii_uppercase() {
        'C' => Ok(0),
        'D' => Ok(2),
        'E' => Ok(4),
        'F' => Ok(5),
        'G' => Ok(7),
        'A' => Ok(9),
        'B' => Ok(11),
        _ => Err(NoteError::Unparsable),
    }
}

/// The reference pitch the whole instrument is tuned to.
///
/// Holds a validated [`Hz`] rather than a bare float, so no code path downstream
/// has to re-check it or unwrap.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tuning {
    reference: Hz,
}

impl Default for Tuning {
    fn default() -> Self {
        Self::with_concert_a(CONCERT_A_HERTZ).unwrap_or(Self {
            reference: Hz::CONCERT_A,
        })
    }
}

impl Tuning {
    /// Tunes the instrument so that A4 sounds at `reference`.
    ///
    /// # Errors
    ///
    /// Returns [`ParamError::Frequency`] if `reference` is not finite and
    /// positive.
    pub fn with_concert_a(reference: f32) -> Result<Self, ParamError> {
        Hz::new(reference).map(|reference| Self { reference })
    }

    /// The reference pitch.
    #[inline]
    #[must_use]
    pub const fn reference(self) -> Hz {
        self.reference
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp, clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn concert_a_is_440_hertz() {
        let key = PianoKey::from_midi(CONCERT_A_KEY).expect("A4 is on the keyboard");
        assert!((key.frequency(Tuning::default()).hertz() - 440.0).abs() < 1e-3);
    }

    #[test]
    fn an_octave_doubles_the_frequency() {
        let lower = PianoKey::from_midi(57).expect("A3 is on the keyboard");
        let upper = PianoKey::from_midi(69).expect("A4 is on the keyboard");
        let ratio =
            upper.frequency(Tuning::default()).hertz() / lower.frequency(Tuning::default()).hertz();
        assert!((ratio - 2.0).abs() < 1e-4, "ratio {ratio}");
    }

    #[test]
    fn the_keyboard_has_88_keys() {
        assert_eq!((LOWEST_PIANO_KEY..=HIGHEST_PIANO_KEY).count(), 88);
    }

    #[test]
    fn rejects_keys_outside_the_piano() {
        assert_eq!(PianoKey::from_midi(20), Err(NoteError::OutOfRange(20)));
        assert_eq!(PianoKey::from_midi(109), Err(NoteError::OutOfRange(109)));
    }

    #[test]
    fn parses_note_names() {
        assert_eq!(
            "A4".parse::<PianoKey>().expect("A4 parses").midi_number(),
            69
        );
        assert_eq!(
            "C4".parse::<PianoKey>().expect("C4 parses").midi_number(),
            60
        );
        assert_eq!(
            "A0".parse::<PianoKey>().expect("A0 parses").midi_number(),
            21
        );
        assert_eq!(
            "C8".parse::<PianoKey>().expect("C8 parses").midi_number(),
            108
        );
    }

    #[test]
    fn parses_accidentals() {
        assert_eq!(
            "C#4".parse::<PianoKey>().expect("C#4 parses").midi_number(),
            61
        );
        assert_eq!(
            "Db4".parse::<PianoKey>().expect("Db4 parses").midi_number(),
            61
        );
    }

    #[test]
    fn rejects_nonsense() {
        assert!("".parse::<PianoKey>().is_err());
        assert!("H4".parse::<PianoKey>().is_err());
        assert!("A".parse::<PianoKey>().is_err());
        assert!("A99".parse::<PianoKey>().is_err());
    }

    #[test]
    fn names_round_trip_through_parsing() {
        for midi in LOWEST_PIANO_KEY..=HIGHEST_PIANO_KEY {
            let key = PianoKey::from_midi(midi).expect("key is on the keyboard");
            let text = format!("{}", key.name());
            let reparsed: PianoKey = text.parse().expect("a generated name must parse");
            assert_eq!(reparsed, key, "round trip failed for {text}");
        }
    }

    #[test]
    fn alternative_tuning_shifts_every_key() {
        let tuning = Tuning::with_concert_a(442.0).expect("442 Hz is valid");
        let key = PianoKey::from_midi(CONCERT_A_KEY).expect("A4 is on the keyboard");
        assert!((key.frequency(tuning).hertz() - 442.0).abs() < 1e-3);
    }

    #[test]
    fn every_piano_key_is_audible() {
        let tuning = Tuning::default();
        for midi in LOWEST_PIANO_KEY..=HIGHEST_PIANO_KEY {
            let key = PianoKey::from_midi(midi).expect("key is on the keyboard");
            let hertz = key.frequency(tuning).hertz();
            assert!((27.0..4_200.0).contains(&hertz), "{midi} -> {hertz} Hz");
        }
    }
}
