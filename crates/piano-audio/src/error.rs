//! Why a realtime audio session could not be started.

use thiserror::Error;

/// Everything that can go wrong on the control side of starting playback.
///
/// None of these can occur once the stream is running — they are all
/// setup-time failures, reported before any audio thread exists.
#[derive(Debug, Error)]
pub enum AudioError {
    /// No output device was reported by the platform's default audio host.
    #[error("no output audio device is available")]
    NoOutputDevice,

    /// The device's default output configuration could not be read.
    #[error("could not read the default output configuration: {0}")]
    DefaultConfig(#[from] cpal::DefaultStreamConfigError),

    /// The device reported a sample rate that `piano-core` cannot represent
    /// (must be finite and positive).
    #[error("device sample rate {0} Hz is not usable")]
    InvalidSampleRate(f32),

    /// The device's default output sample format is not one this crate
    /// converts into. In practice this means an unusual driver; F32, I16 and
    /// U16 cover every device seen during development.
    #[error("unsupported output sample format: {0:?}")]
    UnsupportedSampleFormat(cpal::SampleFormat),

    /// `cpal` could not build the output stream.
    #[error("could not build the output stream: {0}")]
    BuildStream(#[from] cpal::BuildStreamError),

    /// `cpal` could not start playback on an already-built stream.
    #[error("could not start the output stream: {0}")]
    PlayStream(#[from] cpal::PlayStreamError),
}
