//! How the soundboard's motion becomes sound in the air.
//!
//! The force the strings put on the bridge and the pressure a listener
//! hears differ by the board's ability to move air, which is anything but
//! even across the spectrum:
//!
//! - **Below its lowest resonances a soundboard hardly radiates.** The
//!   plate is small against the wavelength, so the air pushed by one face
//!   flows round the edge to the other instead of being compressed — an
//!   acoustic short circuit — and the plate's own stiffness holds its
//!   motion down as well (N. Suzuki, "Vibration and sound radiation of a
//!   piano soundboard", JASA 80, 1986; N. H. Fletcher and T. D. Rossing,
//!   *The Physics of Musical Instruments*, 2nd ed., ch. 12). It is why the
//!   fundamental of a grand's lowest notes is all but missing from its
//!   sound — 20 to 35 dB below partials three to six on the reference
//!   instrument — and the pitch is heard from the partials' spacing.
//! - **Just above, the lowest resonances radiate strongly.** The reference
//!   is loudest in the bass between 75 and 135 Hz, some 9 dB above what
//!   the bridge force alone would give, with a steep edge on either side:
//!   two resonances, near 85 and 128 Hz.
//! - **The treble is radiated more efficiently than the middle.** A plate
//!   radiates better as its bending waves approach the speed of sound in
//!   air, which a ribbed spruce board's do between one and two kilohertz
//!   (Suzuki 1986; N. Giordano, "Sound production by a vibrating piano
//!   soundboard: experiment", JASA 104, 1998).
//!
//! [`Radiation`] is those three facts as one chain of second-order
//! sections per channel. The papers establish why a curve of this shape is
//! there; its corners and gains are fitted to the reference instrument's
//! spectral envelope, measured partial by partial against this model's own
//! body ([`crate::body`]) driven by its bridge force
//! (`docs/CALIBRATION.md`). Two of the fitted numbers are not the
//! instrument's alone, and say so where they are defined: the slope below
//! the first resonance, and the cut in the middle.
//!
//! The partials scatter by about 5 dB around any smooth curve — the
//! reference's own resonances and the model's fall on different
//! frequencies — so only the broad shape is fitted, and a narrower section
//! than these would be fitting that scatter.

use core::f32::consts::FRAC_1_SQRT_2;

use crate::math;
use crate::svf::StateVariableFilter;
use crate::units::SampleRate;

/// Corner, in hertz, of the further 12 dB per octave taken from the very
/// bottom. With the first resonance's own highpass the response falls by
/// 24 dB per octave there, as the reference does; a free plate's short
/// circuit accounts for 18 of them, and the reference's recording chain
/// cannot be told apart from the instrument for the rest.
const SHORT_CIRCUIT_HZ: f32 = 62.0;

/// Frequency of the lowest strongly radiating resonance, in hertz: where
/// the radiated bass is loudest, and below which it falls away.
const FIRST_RESONANCE_HZ: f32 = 85.0;

/// How far that resonance stands above the level further up, as the
/// resonance of the corner: 4.5 is a peak of 13 dB.
const FIRST_RESONANCE_Q: f32 = 4.5;

/// The next resonance up, which keeps the bass strong as far as 135 Hz.
const SECOND_RESONANCE: Bell = Bell {
    centre_hz: 128.0,
    gain_db: 7.0,
    q: 4.0,
};

/// A shallow cut where [`crate::body`]'s diffuse field is strongest. Its
/// taps cancel the lowest frequencies and its absorption the highest, which
/// leaves the octave around 500 Hz some 3 dB proud; this takes it back so
/// that the sum follows the reference. A property of the model's field,
/// not of a soundboard.
const FIELD_HUMP: Bell = Bell {
    centre_hz: 560.0,
    gain_db: -3.5,
    q: 1.2,
};

/// Where the board turns into an efficient radiator, in hertz.
const EFFICIENCY_CORNER_HZ: f32 = 950.0;

/// How many decibels the treble then stands above the middle. About 3 dB
/// of it makes up for the field's own absorption of the treble.
const EFFICIENCY_GAIN_DB: f32 = 10.0;

/// How many sections one channel runs.
const SECTION_COUNT: usize = 5;

/// A bell's centre in hertz, gain in decibels and Q.
#[derive(Debug, Clone, Copy)]
struct Bell {
    centre_hz: f32,
    gain_db: f32,
    q: f32,
}

impl Bell {
    fn section(self, sample_rate: SampleRate) -> StateVariableFilter {
        StateVariableFilter::bell(self.centre_hz, self.gain_db, self.q, sample_rate)
    }
}

/// One channel's filter.
#[derive(Debug, Clone, Copy)]
struct RadiationChannel {
    sections: [StateVariableFilter; SECTION_COUNT],
}

impl RadiationChannel {
    fn new(sample_rate: SampleRate) -> Self {
        Self {
            sections: [
                StateVariableFilter::highpass(SHORT_CIRCUIT_HZ, FRAC_1_SQRT_2, sample_rate),
                StateVariableFilter::highpass(FIRST_RESONANCE_HZ, FIRST_RESONANCE_Q, sample_rate),
                SECOND_RESONANCE.section(sample_rate),
                FIELD_HUMP.section(sample_rate),
                StateVariableFilter::high_shelf(
                    EFFICIENCY_CORNER_HZ,
                    EFFICIENCY_GAIN_DB,
                    sample_rate,
                ),
            ],
        }
    }

    #[inline]
    fn process(&mut self, motion: f32) -> f32 {
        let mut heard = math::bounded_signal(motion);
        for section in &mut self.sections {
            heard = section.process(heard);
        }
        heard
    }

    fn reset(&mut self) {
        for section in &mut self.sections {
            section.reset();
        }
    }
}

/// The soundboard's radiation, for a stereo pair — see the module docs.
#[derive(Debug, Clone, Copy)]
pub struct Radiation {
    left: RadiationChannel,
    right: RadiationChannel,
}

impl Radiation {
    /// The filter for a stream at `sample_rate`. Allocates nothing.
    #[must_use]
    pub fn new(sample_rate: SampleRate) -> Self {
        let channel = RadiationChannel::new(sample_rate);
        Self {
            left: channel,
            right: channel,
        }
    }

    /// The sound radiated by a board moving as `left` and `right`.
    #[inline]
    pub fn process(&mut self, left: f32, right: f32) -> (f32, f32) {
        (self.left.process(left), self.right.process(right))
    }

    /// Clears the filters' memory.
    pub fn reset(&mut self) {
        self.left.reset();
        self.right.reset();
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use proptest::prelude::*;

    use super::*;
    use crate::test_signal::sine_gain_db;

    /// Where the response is level, between the bass resonances and the
    /// cut in the middle: the reference every other gain is read against.
    const LEVEL_HZ: f32 = 280.0;

    /// Steady-state gain of the left channel at `frequency_hz` for a
    /// stream at `rate_hz`, in decibels.
    fn gain_db_at(rate_hz: f32, frequency_hz: f32) -> f32 {
        let mut radiation = Radiation::new(SampleRate::new(rate_hz).unwrap());
        sine_gain_db(rate_hz, frequency_hz, |input| {
            radiation.process(input, 0.0).0
        })
    }

    fn gain_db(frequency_hz: f32) -> f32 {
        gain_db_at(48_000.0, frequency_hz)
    }

    #[test]
    fn the_lowest_fundamental_is_all_but_cancelled() {
        let below = gain_db(27.5) - gain_db(LEVEL_HZ);
        assert!(below < -25.0, "{below} dB");
    }

    #[test]
    fn the_bass_falls_away_steeply_below_the_first_resonance() {
        let edge = gain_db(FIRST_RESONANCE_HZ) - gain_db(62.0);
        assert!(edge > 12.0, "{edge} dB");
    }

    #[test]
    fn the_bass_is_loudest_at_the_first_resonance() {
        let hump = gain_db(FIRST_RESONANCE_HZ) - gain_db(LEVEL_HZ);
        assert!((9.0..15.0).contains(&hump), "{hump} dB");
    }

    #[test]
    fn the_second_resonance_keeps_the_bass_strong_above_the_first() {
        let second = gain_db(SECOND_RESONANCE.centre_hz) - gain_db(LEVEL_HZ);
        let above = gain_db(170.0) - gain_db(LEVEL_HZ);
        assert!((7.0..13.0).contains(&second), "{second} dB");
        assert!(above < 3.5, "{above} dB");
    }

    #[test]
    fn the_middle_is_eased_where_the_field_is_strongest() {
        let eased = gain_db(FIELD_HUMP.centre_hz) - gain_db(LEVEL_HZ);
        assert!((-4.0..-0.5).contains(&eased), "{eased} dB");
    }

    #[test]
    fn the_treble_stands_above_the_middle() {
        let lift = gain_db(8_000.0) - gain_db(LEVEL_HZ);
        assert!((8.0..12.0).contains(&lift), "{lift} dB");
    }

    #[test]
    fn the_response_does_not_depend_on_the_sample_rate() {
        for frequency_hz in [40.0, 100.0, 1_000.0, 5_000.0] {
            let low = gain_db_at(44_100.0, frequency_hz);
            let high = gain_db_at(96_000.0, frequency_hz);
            assert!((low - high).abs() < 0.3, "{frequency_hz}: {low} vs {high}");
        }
    }

    #[test]
    fn both_channels_are_filtered_alike() {
        let mut radiation = Radiation::new(SampleRate::new(48_000.0).unwrap());
        let alike = (0..256).all(|n| {
            let sample = (0.05 * n as f32).sin();
            let (left, right) = radiation.process(sample, sample);
            (left - right).abs() < f32::EPSILON
        });
        assert!(alike);
    }

    #[test]
    fn reset_silences_the_filter() {
        let mut radiation = Radiation::new(SampleRate::new(48_000.0).unwrap());
        for _ in 0..256 {
            radiation.process(1.0, -1.0);
        }
        radiation.reset();
        let (left, right) = radiation.process(0.0, 0.0);
        assert!(left.abs() < f32::EPSILON && right.abs() < f32::EPSILON);
    }

    proptest! {
        #[test]
        fn any_rate_and_input_stay_finite(
            rate_hz in 1.0_f32..1.0e6,
            inputs in proptest::collection::vec(proptest::num::f32::ANY, 0..256),
        ) {
            let mut radiation = Radiation::new(SampleRate::new(rate_hz).unwrap());
            for input in inputs {
                let (left, right) = radiation.process(input, -input);
                prop_assert!(left.is_finite() && right.is_finite());
            }
        }
    }
}
