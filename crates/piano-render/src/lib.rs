//! Offline rendering: turn a note into a buffer, and a buffer into a WAV file.
//!
//! This is the slow, allocating, file-touching half of the system, deliberately
//! separated from [`piano_core`] so that nothing here can ever be reached from
//! an audio callback.

#![forbid(unsafe_code)]

use std::path::Path;

use piano_core::{ParamError, PluckedString, SampleRate, string::StringConfig};
use piano_params::{PianoKey, Tuning};
use thiserror::Error;

/// Longest render this crate will produce, in seconds.
///
/// A bound, rather than trusting the caller, so a typo cannot ask for a
/// terabyte of samples.
pub const MAX_DURATION_SECONDS: f32 = 600.0;

/// Length of the fade applied to the end of every render, in seconds.
///
/// Cutting a decaying string mid-cycle produces a click; 20 ms of fade removes
/// it without being audible as a fade.
const FADE_OUT_SECONDS: f32 = 0.02;

/// Why a render or a file write failed.
#[derive(Debug, Error)]
pub enum RenderError {
    /// A synthesis parameter was rejected.
    #[error("invalid synthesis parameter")]
    Parameter(#[from] ParamError),

    /// The requested duration is not a usable number of seconds.
    #[error("duration must be finite and within (0, 600] seconds, got {0}")]
    Duration(f32),

    /// Writing the WAV file failed.
    #[error("could not write the WAV file")]
    Wav(#[from] hound::Error),
}

/// Everything needed to render one note offline.
#[derive(Debug, Clone, Copy)]
pub struct RenderRequest {
    /// Which key to strike.
    pub key: PianoKey,
    /// The instrument's reference pitch.
    pub tuning: Tuning,
    /// Output sample rate.
    pub sample_rate: SampleRate,
    /// How long to record, in seconds.
    pub seconds: f32,
    /// How hard the key is struck, in `[0, 1]`.
    pub velocity: f32,
}

/// Renders a single note into a freshly allocated mono buffer.
///
/// # Errors
///
/// Returns [`RenderError::Duration`] for a duration outside
/// `(0, MAX_DURATION_SECONDS]`, or [`RenderError::Parameter`] if the key cannot
/// be voiced at this sample rate.
pub fn render_note(request: RenderRequest) -> Result<Vec<f32>, RenderError> {
    let sample_count = duration_to_samples(request.seconds, request.sample_rate)?;

    let frequency = request.key.frequency(request.tuning);
    let config = StringConfig::new(frequency);
    let mut string = PluckedString::new(config, request.sample_rate)?;
    string.pluck(request.velocity);

    let mut samples = vec![0.0; sample_count];
    string.process_block_add(&mut samples);
    apply_fade_out(&mut samples, fade_length(request.sample_rate));
    Ok(samples)
}

/// Scales `samples` so that the loudest one sits at `target_peak`.
///
/// A silent buffer is left alone rather than amplified into noise.
pub fn normalise(samples: &mut [f32], target_peak: f32) {
    let peak = samples
        .iter()
        .copied()
        .fold(0.0f32, |worst, sample| worst.max(sample.abs()));
    if peak <= f32::EPSILON {
        return;
    }
    let gain = target_peak / peak;
    for sample in samples {
        *sample *= gain;
    }
}

/// Fades the last `length` samples linearly down to silence.
pub fn apply_fade_out(samples: &mut [f32], length: usize) {
    let length = length.min(samples.len());
    if length == 0 {
        return;
    }
    let start = samples.len() - length;
    let step = 1.0 / length as f32;
    // `offset + 1` so the ramp reaches exactly zero on the final sample; ending
    // at `1/length` instead would leave the click the fade exists to remove.
    for (offset, sample) in samples.iter_mut().skip(start).enumerate() {
        *sample *= 1.0 - (offset + 1) as f32 * step;
    }
}

/// Writes a mono buffer to a 32-bit float WAV file.
///
/// # Errors
///
/// Returns [`RenderError::Wav`] if the file cannot be created or written.
pub fn write_wav(path: &Path, samples: &[f32], sample_rate: SampleRate) -> Result<(), RenderError> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: sample_rate.hertz() as u32,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut writer = hound::WavWriter::create(path, spec)?;
    for sample in samples {
        writer.write_sample(*sample)?;
    }
    writer.finalize()?;
    Ok(())
}

fn duration_to_samples(seconds: f32, sample_rate: SampleRate) -> Result<usize, RenderError> {
    if !seconds.is_finite() || seconds <= 0.0 || seconds > MAX_DURATION_SECONDS {
        return Err(RenderError::Duration(seconds));
    }
    // Both operands are finite and bounded, and f32 -> usize saturates.
    Ok((seconds * sample_rate.hertz()) as usize)
}

fn fade_length(sample_rate: SampleRate) -> usize {
    (FADE_OUT_SECONDS * sample_rate.hertz()) as usize
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::float_cmp,
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::indexing_slicing
    )]

    use super::*;

    fn request_for(seconds: f32) -> RenderRequest {
        RenderRequest {
            key: PianoKey::from_midi(69).expect("A4 is on the keyboard"),
            tuning: Tuning::default(),
            sample_rate: SampleRate::new(48_000.0).expect("48 kHz is valid"),
            seconds,
            velocity: 0.8,
        }
    }

    #[test]
    fn renders_the_requested_number_of_samples() {
        let samples = render_note(request_for(0.5)).expect("half a second renders");
        assert_eq!(samples.len(), 24_000);
    }

    #[test]
    fn rejects_impossible_durations() {
        assert!(matches!(
            render_note(request_for(0.0)),
            Err(RenderError::Duration(_))
        ));
        assert!(matches!(
            render_note(request_for(-1.0)),
            Err(RenderError::Duration(_))
        ));
        assert!(matches!(
            render_note(request_for(f32::NAN)),
            Err(RenderError::Duration(_))
        ));
        assert!(matches!(
            render_note(request_for(1e9)),
            Err(RenderError::Duration(_))
        ));
    }

    #[test]
    fn rendered_audio_is_finite_and_audible() {
        let samples = render_note(request_for(1.0)).expect("one second renders");
        assert!(samples.iter().all(|sample| sample.is_finite()));
        let peak = samples
            .iter()
            .fold(0.0f32, |worst, sample| worst.max(sample.abs()));
        assert!(peak > 0.01, "peak {peak} is inaudible");
    }

    #[test]
    fn the_render_ends_in_silence() {
        let samples = render_note(request_for(1.0)).expect("one second renders");
        assert_eq!(samples.last().copied().unwrap_or(1.0), 0.0);
    }

    #[test]
    fn normalise_brings_the_peak_to_target() {
        let mut samples = vec![0.1, -0.2, 0.05];
        normalise(&mut samples, 1.0);
        let peak = samples
            .iter()
            .fold(0.0f32, |worst, sample| worst.max(sample.abs()));
        assert!((peak - 1.0).abs() < 1e-6, "peak {peak}");
    }

    #[test]
    fn normalise_leaves_silence_alone() {
        let mut samples = vec![0.0; 16];
        normalise(&mut samples, 1.0);
        assert!(samples.iter().all(|sample| *sample == 0.0));
    }

    #[test]
    fn fade_out_longer_than_the_buffer_is_clamped() {
        let mut samples = vec![1.0; 4];
        apply_fade_out(&mut samples, 1_000);
        assert!(
            samples.windows(2).all(|pair| pair[0] >= pair[1]),
            "ramp is not monotonic"
        );
        assert_eq!(samples.last().copied().unwrap_or(1.0), 0.0);
    }

    #[test]
    fn writes_a_readable_wav_file() {
        let directory = std::env::temp_dir().join("piano-render-tests");
        std::fs::create_dir_all(&directory).expect("temp dir is writable");
        let path = directory.join("a4.wav");

        let rate = SampleRate::new(48_000.0).expect("48 kHz is valid");
        let samples = render_note(request_for(0.25)).expect("a quarter second renders");
        write_wav(&path, &samples, rate).expect("the WAV file is written");

        let reader = hound::WavReader::open(&path).expect("the WAV file is readable");
        assert_eq!(reader.spec().sample_rate, 48_000);
        assert_eq!(reader.spec().channels, 1);
        assert_eq!(reader.len() as usize, samples.len());

        std::fs::remove_file(&path).expect("the temp file is removable");
    }
}
