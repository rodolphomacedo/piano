//! Measured evidence for M15's "done when" criterion (issue #69): setting a
//! soundboard mode or a unison coupling gain live must produce a
//! measurable *spectral* change, checked with an FFT — the same discipline
//! `m4_spectral.rs` established for M4's tuning and brightness claims,
//! reused here rather than reinvented.

#![allow(clippy::indexing_slicing, clippy::unwrap_used, clippy::expect_used)]

use piano_core::soundboard::SoundboardMode;
use piano_core::string::StringConfig;
use piano_core::{Hz, SampleRate, Soundboard, UnisonGroup};
use rustfft::{FftPlanner, num_complex::Complex32};

const SAMPLE_RATE_HZ: f32 = 48_000.0;

fn sample_rate() -> SampleRate {
    SampleRate::new(SAMPLE_RATE_HZ).expect("48 kHz is valid")
}

/// A Hann-windowed magnitude spectrum of `samples`, zero-padded to the next
/// power of two — the same construction `m4_spectral.rs::magnitude_spectrum`
/// uses.
fn magnitude_spectrum(samples: &[f32]) -> (Vec<f32>, usize) {
    let fft_len = samples.len().next_power_of_two();
    let window_span = (samples.len().max(2) - 1) as f32;
    let mut buffer: Vec<Complex32> = (0..fft_len)
        .map(|index| {
            let Some(&sample) = samples.get(index) else {
                return Complex32::new(0.0, 0.0);
            };
            let phase = core::f32::consts::TAU * index as f32 / window_span;
            let window = 0.5 - 0.5 * phase.cos();
            Complex32::new(sample * window, 0.0)
        })
        .collect();
    let mut planner = FftPlanner::new();
    let fft = planner.plan_fft_forward(fft_len);
    fft.process(&mut buffer);
    (buffer.iter().map(|value| value.norm()).collect(), fft_len)
}

/// The spectrum's energy-weighted mean frequency — same construction as
/// `m4_spectral.rs::spectral_centroid`.
fn spectral_centroid(samples: &[f32]) -> f32 {
    let (spectrum, fft_len) = magnitude_spectrum(samples);
    let bin_hz = SAMPLE_RATE_HZ / fft_len as f32;
    let half = fft_len / 2;

    let mut weighted = 0.0f64;
    let mut total = 0.0f64;
    for (bin, magnitude) in spectrum.iter().take(half).enumerate() {
        let frequency = bin as f32 * bin_hz;
        weighted += f64::from(frequency) * f64::from(*magnitude);
        total += f64::from(*magnitude);
    }
    if total <= 0.0 {
        0.0
    } else {
        (weighted / total) as f32
    }
}

#[test]
fn setting_a_soundboard_mode_live_measurably_shifts_the_spectral_centroid() {
    let mut before = Soundboard::new(sample_rate());
    let mut after = Soundboard::new(sample_rate());
    after.set_mode(
        0,
        SoundboardMode {
            frequency_hz: 8_000.0,
            decay_seconds: 0.3,
            gain: 2.5,
        },
    );

    // A broadband click, same excitation shape `m4_spectral.rs` uses to
    // "light up" every mode at once, then the board's own ring.
    let render = |board: &mut Soundboard| -> Vec<f32> {
        let mut samples = vec![0.0f32; 2_048];
        samples[0] = 1.0;
        for sample in &mut samples {
            *sample = board.process(*sample);
        }
        samples
    };
    let before_centroid = spectral_centroid(&render(&mut before));
    let after_centroid = spectral_centroid(&render(&mut after));

    assert!(
        (after_centroid - before_centroid).abs() > 50.0,
        "retuning mode 0 to 8 kHz did not measurably shift the centroid: \
         before {before_centroid} Hz, after {after_centroid} Hz"
    );
}

fn trichord() -> UnisonGroup {
    let config = StringConfig::new(Hz::new(440.0).expect("440 Hz is valid"));
    UnisonGroup::new(config, 3, sample_rate()).expect("A4 trichord is tunable")
}

#[test]
fn zeroing_local_coupling_gain_live_measurably_changes_the_spectral_centroid() {
    let mut coupled = trichord();
    let mut uncoupled = trichord();
    uncoupled.set_local_coupling_gain(0.0);

    coupled.pluck(0.9);
    uncoupled.pluck(0.9);
    let coupled_samples: Vec<f32> = (0..4_096).map(|_| coupled.process()).collect();
    let uncoupled_samples: Vec<f32> = (0..4_096).map(|_| uncoupled.process()).collect();

    let coupled_centroid = spectral_centroid(&coupled_samples);
    let uncoupled_centroid = spectral_centroid(&uncoupled_samples);
    assert!(
        (coupled_centroid - uncoupled_centroid).abs() > 1.0,
        "zeroing local_coupling_gain did not measurably shift the centroid: \
         coupled {coupled_centroid} Hz, uncoupled {uncoupled_centroid} Hz"
    );
}
