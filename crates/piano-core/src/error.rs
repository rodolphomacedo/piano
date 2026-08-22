//! Parameter validation errors.

use thiserror::Error;

/// Rejected at construction time so that the audio thread never has to.
///
/// Every variant carries the offending value, because a validation error that
/// does not tell you what was wrong costs a debugging session.
#[derive(Debug, Clone, Copy, PartialEq, Error)]
pub enum ParamError {
    /// A sample rate must be finite and strictly positive.
    #[error("sample rate must be finite and > 0, got {0}")]
    SampleRate(f32),

    /// A frequency must be finite and strictly positive.
    #[error("frequency must be finite and > 0, got {0}")]
    Frequency(f32),

    /// The requested fundamental cannot be represented at this sample rate.
    ///
    /// A digital waveguide needs at least two samples of loop delay, which caps
    /// the fundamental at half the sample rate.
    #[error("frequency {frequency} Hz is not representable at {sample_rate} Hz (max {maximum} Hz)")]
    FrequencyOutOfRange {
        /// The requested fundamental, in hertz.
        frequency: f32,
        /// The sample rate the string was built for, in hertz.
        sample_rate: f32,
        /// The highest fundamental this sample rate supports, in hertz.
        maximum: f32,
    },

    /// A normalised gain, damping or feedback coefficient left `[0, 1]`.
    #[error("{name} must be finite and within [0, 1], got {value}")]
    NormalisedRange {
        /// The name of the parameter, for the error message.
        name: &'static str,
        /// The offending value.
        value: f32,
    },
}
