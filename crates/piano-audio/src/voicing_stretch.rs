//! Stretch tuning: how far each key is tuned from equal temperament.
//!
//! A piano tuned so that every fundamental sat on its equal-tempered
//! frequency would sound out of tune with itself. Its strings are stiff, so
//! their partials run sharp of whole multiples, and an octave whose two
//! fundamentals are exactly 2:1 apart has the upper note beating against
//! the lower note's second partial. A tuner removes that beat instead, and
//! in doing so tunes the treble progressively sharp and the bass
//! progressively flat: the curve O. L. Railsback measured ("Scale
//! temperament as applied to piano tuning", JASA 9(3), 1938) and
//! O. H. Schuck and R. W. Young explained by inharmonicity ("Observations
//! on the vibrations of piano strings", JASA 15(1), 1943).
//!
//! The stretch is therefore not a table but a consequence of the scale's
//! own `B` ([`super::inharmonicity`]), which is how it is computed here: a
//! middle octave is left in equal temperament, and every other key is tuned
//! in octaves from it, each octave made beatless between partial `2ρ` of
//! its lower note and partial `ρ` of its upper. `ρ` is the octave type of
//! F. Rigaud, B. David and L. Daudet's tuning model ("A parametric model
//! and estimation techniques for the inharmonicity and tuning of the
//! piano", JASA 133(5), 2013): `1` is the 2:1 octave tuners use in the
//! treble, `2` the 4:2 octave of the middle, `3` the 6:3 octave of the
//! bass, and it varies smoothly in between.
//!
//! Where the octave types change is fitted to the reference instrument
//! (`docs/CALIBRATION.md`), whose measured offsets these rules, applied to
//! its measured `B`, reproduce to about 2 cents: 15 cents flat at A0, flat
//! by a cent or two through the tenor, 10 cents sharp at C7 and 30 at C8.

use piano_core::{Hz, math};
use piano_params::{CONCERT_A_KEY, HIGHEST_PIANO_KEY, LOWEST_PIANO_KEY, PianoKey, Tuning};

use super::anchor_hz;
use super::inharmonicity::scale_inharmonicity;

/// Lowest key of the octave left in equal temperament, the one a tuner
/// sets first; concert A is its highest.
const TEMPERAMENT_LOWEST_MIDI: u8 = CONCERT_A_KEY - 11;

const SEMITONES_PER_OCTAVE: u8 = 12;

/// Cents in an octave.
const CENTS_PER_OCTAVE: f32 = 1_200.0;

/// Octaves between the temperament octave and the farthest key, which
/// bounds the walk from one to the other.
const MAX_OCTAVES_FROM_TEMPERAMENT: usize = 4;

/// One corner of the octave-type curve: octaves whose lower note is
/// `lower_midi` are tuned as type `octave_type`.
#[derive(Debug, Clone, Copy)]
struct OctaveTypeCorner {
    lower_midi: f32,
    octave_type: f32,
}

/// The octave type along the keyboard, by the octave's lower note: 6:3 up
/// to A1, easing to 4:2 by A2, 4:2 through the tenor, easing to 2:1 from A3
/// to A4, 2:1 above. Linear between corners, constant beyond the ends.
const OCTAVE_TYPES: [OctaveTypeCorner; 4] = [
    OctaveTypeCorner {
        lower_midi: 33.0,
        octave_type: 3.0,
    },
    OctaveTypeCorner {
        lower_midi: 45.0,
        octave_type: 2.0,
    },
    OctaveTypeCorner {
        lower_midi: 57.0,
        octave_type: 2.0,
    },
    OctaveTypeCorner {
        lower_midi: 69.0,
        octave_type: 1.0,
    },
];

/// How many cents sharp of equal temperament `key`'s fundamental is tuned
/// under `tuning`; negative is flat. See the module docs. Zero from A#3 to
/// A4, so concert pitch stays where `tuning` puts it.
#[must_use]
pub fn stretch_cents_for_key(key: PianoKey, tuning: Tuning) -> f32 {
    let mut midi = key.midi_number();
    let mut cents = 0.0;
    for _ in 0..MAX_OCTAVES_FROM_TEMPERAMENT {
        if midi > CONCERT_A_KEY {
            midi = midi.saturating_sub(SEMITONES_PER_OCTAVE);
            cents += octave_stretch_cents(midi, tuning);
        } else if midi < TEMPERAMENT_LOWEST_MIDI {
            cents -= octave_stretch_cents(midi, tuning);
            midi = midi.saturating_add(SEMITONES_PER_OCTAVE);
        }
    }
    cents
}

/// The frequency `key`'s strings are tuned to under `tuning`: its
/// equal-tempered pitch, stretched.
#[must_use]
pub fn tuned_frequency(key: PianoKey, tuning: Tuning) -> Hz {
    let nominal = key.frequency(tuning);
    let hertz = stretched_hz(nominal.hertz(), stretch_cents_for_key(key, tuning));
    Hz::new(hertz).unwrap_or(nominal)
}

/// `frequency_hz` moved by `cents`.
fn stretched_hz(frequency_hz: f32, cents: f32) -> f32 {
    frequency_hz * math::powf(2.0, cents / CENTS_PER_OCTAVE)
}

/// How many cents wider than 2:1 the octave above `lower_midi` is tuned,
/// fundamental to fundamental.
///
/// Partial `n` of a string whose fundamental is `F` sits at
/// `n·F·√((1 + B·n²)/(1 + B))`. Setting partial `2ρ` of the lower note
/// equal to partial `ρ` of the upper gives the ratio below.
fn octave_stretch_cents(lower_midi: u8, tuning: Tuning) -> f32 {
    let upper_midi = lower_midi.saturating_add(SEMITONES_PER_OCTAVE);
    let lower = scale_inharmonicity(anchor_hz(clamped(lower_midi), tuning), tuning);
    let upper = scale_inharmonicity(anchor_hz(clamped(upper_midi), tuning), tuning);
    let type_squared = octave_type(f32::from(lower_midi)).powi(2);
    let widened = (1.0 + 4.0 * type_squared * lower) * (1.0 + upper);
    let narrowed = (1.0 + lower) * (1.0 + type_squared * upper);
    0.5 * CENTS_PER_OCTAVE * math::ln(widened / narrowed) / core::f32::consts::LN_2
}

/// `midi` held on the keyboard.
fn clamped(midi: u8) -> u8 {
    midi.clamp(LOWEST_PIANO_KEY, HIGHEST_PIANO_KEY)
}

/// The type of the octave whose lower note is `lower_midi`; see
/// [`OCTAVE_TYPES`].
fn octave_type(lower_midi: f32) -> f32 {
    let [first, .., last] = OCTAVE_TYPES;
    if lower_midi <= first.lower_midi {
        return first.octave_type;
    }
    for pair in OCTAVE_TYPES.windows(2) {
        let [low, high] = pair else { continue };
        if lower_midi <= high.lower_midi {
            let along = (lower_midi - low.lower_midi) / (high.lower_midi - low.lower_midi);
            return low.octave_type + along * (high.octave_type - low.octave_type);
        }
    }
    last.octave_type
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::float_cmp)]

    use proptest::prelude::*;

    use super::*;

    fn stretch(midi: u8) -> f32 {
        stretch_cents_for_key(PianoKey::from_midi(midi).unwrap(), Tuning::default())
    }

    #[test]
    fn concert_pitch_and_the_temperament_octave_are_left_alone() {
        assert!((TEMPERAMENT_LOWEST_MIDI..=CONCERT_A_KEY).all(|midi| stretch(midi) == 0.0));
    }

    #[test]
    fn the_treble_is_tuned_sharp_and_ever_sharper() {
        let sharp: [f32; 4] = [72, 84, 96, 108].map(stretch);
        assert!(
            sharp.is_sorted_by(|lower, higher| lower < higher),
            "{sharp:?}"
        );
        assert!((5.0..16.0).contains(&sharp[2]), "C7 {}", sharp[2]);
        assert!((20.0..40.0).contains(&sharp[3]), "C8 {}", sharp[3]);
    }

    #[test]
    fn the_bass_is_tuned_flat_and_ever_flatter() {
        let flat: [f32; 4] = [57, 45, 33, 21].map(stretch);
        assert!(
            flat.is_sorted_by(|nearer, farther| nearer > farther),
            "{flat:?}"
        );
        assert!((-22.0..-9.0).contains(&flat[3]), "A0 {}", flat[3]);
    }

    #[test]
    fn the_middle_of_the_keyboard_is_within_a_few_cents_of_equal_temperament() {
        assert!((36..=84).all(|midi| stretch(midi).abs() < 7.0));
    }

    #[test]
    fn an_octave_is_beatless_between_the_partials_its_type_names() {
        let partial = |fundamental: f32, b: f32, n: f32| {
            n * fundamental * ((1.0 + b * n * n) / (1.0 + b)).sqrt()
        };
        let tuning = Tuning::default();
        for (lower_midi, rho) in [(24_u8, 3.0_f32), (50, 2.0), (84, 1.0)] {
            let (low_hz, high_hz) = (100.0, 200.0);
            let cents = octave_stretch_cents(lower_midi, tuning);
            let lower_b = scale_inharmonicity(anchor_hz(lower_midi, tuning), tuning);
            let upper_b = scale_inharmonicity(anchor_hz(lower_midi + 12, tuning), tuning);
            let from_below = partial(low_hz, lower_b, 2.0 * rho);
            let from_above = partial(stretched_hz(high_hz, cents), upper_b, rho);
            assert!((from_below / from_above - 1.0).abs() < 1e-5, "{lower_midi}");
        }
    }

    #[test]
    fn the_octave_type_runs_from_six_three_to_two_one() {
        assert_eq!(octave_type(21.0), 3.0);
        assert_eq!(octave_type(50.0), 2.0);
        assert!((octave_type(63.0) - 1.5).abs() < 1e-6);
        assert_eq!(octave_type(100.0), 1.0);
    }

    proptest! {
        #[test]
        fn every_key_and_concert_pitch_give_a_small_finite_stretch(
            midi in LOWEST_PIANO_KEY..=HIGHEST_PIANO_KEY,
            concert_a in 400.0_f32..480.0,
        ) {
            let tuning = Tuning::with_concert_a(concert_a).unwrap();
            let cents = stretch_cents_for_key(PianoKey::from_midi(midi).unwrap(), tuning);
            prop_assert!(cents.is_finite() && cents.abs() < 80.0, "{cents}");
        }

        #[test]
        fn any_frequency_and_offset_give_a_finite_frequency(
            frequency_hz in 1.0_f32..20_000.0,
            cents in -200.0_f32..200.0,
        ) {
            prop_assert!(stretched_hz(frequency_hz, cents).is_finite());
        }
    }
}
