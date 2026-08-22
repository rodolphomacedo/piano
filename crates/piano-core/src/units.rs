//! Newtypes for the two quantities that are catastrophic to mix up.
//!
//! Sample rate and frequency are both "some number of hertz". Passing one where
//! the other is expected compiles fine with bare `f32` and produces silence or
//! a scream. These newtypes make that a compile error.

use crate::error::ParamError;

/// A validated audio sample rate in hertz.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct SampleRate(f32);

impl SampleRate {
    /// Validates and wraps a sample rate.
    ///
    /// # Errors
    ///
    /// Returns [`ParamError::SampleRate`] if `hertz` is not finite and positive.
    pub fn new(hertz: f32) -> Result<Self, ParamError> {
        if !hertz.is_finite() || hertz <= 0.0 {
            return Err(ParamError::SampleRate(hertz));
        }
        Ok(Self(hertz))
    }

    /// The sample rate as a plain float.
    #[inline]
    #[must_use]
    pub const fn hertz(self) -> f32 {
        self.0
    }

    /// The highest frequency this sample rate can represent.
    #[inline]
    #[must_use]
    pub fn nyquist(self) -> f32 {
        self.0 * 0.5
    }
}

/// A validated frequency in hertz.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct Hz(f32);

impl Hz {
    /// Concert A, 440 Hz. Available as a `const` so callers have a valid
    /// fallback that needs no runtime validation.
    pub const CONCERT_A: Self = Self(440.0);

    /// Validates and wraps a frequency.
    ///
    /// # Errors
    ///
    /// Returns [`ParamError::Frequency`] if `hertz` is not finite and positive.
    pub fn new(hertz: f32) -> Result<Self, ParamError> {
        if !hertz.is_finite() || hertz <= 0.0 {
            return Err(ParamError::Frequency(hertz));
        }
        Ok(Self(hertz))
    }

    /// The frequency as a plain float.
    #[inline]
    #[must_use]
    pub const fn hertz(self) -> f32 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp, clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn rejects_non_positive_sample_rate() {
        assert_eq!(SampleRate::new(0.0), Err(ParamError::SampleRate(0.0)));
        assert_eq!(
            SampleRate::new(-48_000.0),
            Err(ParamError::SampleRate(-48_000.0))
        );
    }

    #[test]
    fn rejects_non_finite_sample_rate() {
        assert!(SampleRate::new(f32::NAN).is_err());
        assert!(SampleRate::new(f32::INFINITY).is_err());
    }

    #[test]
    fn nyquist_is_half_the_sample_rate() {
        let rate = SampleRate::new(48_000.0).expect("48 kHz is a valid sample rate");
        assert!((rate.nyquist() - 24_000.0).abs() < f32::EPSILON);
    }

    #[test]
    fn rejects_non_positive_frequency() {
        assert!(Hz::new(0.0).is_err());
        assert!(Hz::new(-1.0).is_err());
        assert!(Hz::new(f32::NAN).is_err());
    }

    #[test]
    fn accepts_audible_frequency() {
        let note = Hz::new(440.0).expect("440 Hz is a valid frequency");
        assert!((note.hertz() - 440.0).abs() < f32::EPSILON);
    }
}
