//! The measurements themselves, on a plain buffer of samples: the level of
//! one partial, the time it takes to fall away, and the spectral centroid.

use rustfft::FftPlanner;
use rustfft::num_complex::Complex32;

/// One analysis window, in samples. Long enough to resolve a bass
/// fundamental's harmonics, short enough that a treble note's decay is
/// sampled several times over its life.
pub const WINDOW: usize = 8192;

/// A partial has died away, for [`seconds_to_fall`], once it is this far
/// below its own peak: 20 dB.
const FALL_RATIO: f32 = 0.1;

/// Where partial `n` of a string actually sits: stretched by the string's
/// inharmonicity `B`, `f_n = n·f0·√((1 + B·n²)/(1 + B))`, the curve the
/// dispersion cascade is fitted to. Measuring at exact `n·f0` puts the probe
/// off the real peak once partials are stretched (issue #96).
#[must_use]
pub fn partial_hz(fundamental_hz: f32, inharmonicity: f32, harmonic: u8) -> f32 {
    let n = f32::from(harmonic);
    let b = inharmonicity.max(0.0);
    n * fundamental_hz * ((1.0 + b * n * n) / (1.0 + b)).sqrt()
}

/// The Hann window's weight at `index` of `length`.
fn hann(index: usize, length: usize) -> f32 {
    0.5 - 0.5 * (std::f32::consts::TAU * index as f32 / length.max(1) as f32).cos()
}

/// Magnitude of `samples` at `frequency_hz`, by direct evaluation of the DFT
/// at that one frequency under a Hann window. No error from snapping a
/// partial to the nearest FFT bin, which matters because piano partials sit
/// deliberately sharp of exact multiples. `0` for an empty buffer.
#[must_use]
pub fn magnitude_at(samples: &[f32], frequency_hz: f32, sample_rate_hz: f32) -> f32 {
    let omega = std::f32::consts::TAU * frequency_hz / sample_rate_hz;
    let (real, imag) =
        samples
            .iter()
            .enumerate()
            .fold((0.0f32, 0.0f32), |(real, imag), (index, &sample)| {
                let weighted = sample * hann(index, samples.len());
                let phase = omega * index as f32;
                (real + weighted * phase.cos(), imag - weighted * phase.sin())
            });
    (real * real + imag * imag).sqrt() / samples.len().max(1) as f32
}

/// Seconds for the partial at `frequency_hz` to fall 20 dB from its own
/// peak, measured one [`WINDOW`] at a time; `None` when it never does
/// within `samples`.
#[must_use]
pub fn seconds_to_fall(samples: &[f32], frequency_hz: f32, sample_rate_hz: f32) -> Option<f32> {
    let levels: Vec<f32> = samples
        .chunks_exact(WINDOW)
        .map(|chunk| magnitude_at(chunk, frequency_hz, sample_rate_hz))
        .collect();
    let (peak_index, peak) =
        levels
            .iter()
            .copied()
            .enumerate()
            .fold((0, 0.0f32), |best, (index, level)| {
                if level > best.1 { (index, level) } else { best }
            });
    if peak <= 0.0 {
        return None;
    }
    levels
        .iter()
        .skip(peak_index)
        .position(|&level| level < peak * FALL_RATIO)
        .map(|offset| (offset * WINDOW) as f32 / sample_rate_hz)
}

/// The magnitude-weighted mean frequency of the first [`WINDOW`] samples,
/// in hertz, zero-padded if shorter; `0` for silence.
#[must_use]
pub fn spectral_centroid(samples: &[f32], sample_rate_hz: f32) -> f32 {
    let mut buffer: Vec<Complex32> = samples
        .iter()
        .take(WINDOW)
        .enumerate()
        .map(|(index, &sample)| Complex32::new(sample * hann(index, WINDOW), 0.0))
        .collect();
    buffer.resize(WINDOW, Complex32::new(0.0, 0.0));
    FftPlanner::<f32>::new()
        .plan_fft_forward(WINDOW)
        .process(&mut buffer);
    let bin_hz = f64::from(sample_rate_hz) / WINDOW as f64;
    let (weighted, total) = buffer.iter().take(WINDOW / 2).enumerate().fold(
        (0.0f64, 0.0f64),
        |(weighted, total), (bin, value)| {
            let magnitude = f64::from(value.norm());
            (
                weighted + magnitude * bin as f64 * bin_hz,
                total + magnitude,
            )
        },
    );
    if total <= 0.0 {
        0.0
    } else {
        (weighted / total) as f32
    }
}

/// Root-mean-square level of `samples`; `0` for an empty buffer.
#[must_use]
pub fn rms(samples: &[f32]) -> f32 {
    let energy: f32 = samples.iter().map(|sample| sample * sample).sum();
    (energy / samples.len().max(1) as f32).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const RATE: f32 = 48_000.0;

    fn sine(frequency: f32, amplitude: f32, samples: usize) -> Vec<f32> {
        (0..samples)
            .map(|n| amplitude * (std::f32::consts::TAU * frequency * n as f32 / RATE).sin())
            .collect()
    }

    #[test]
    fn a_sine_is_measured_at_its_own_frequency_and_not_an_octave_away() {
        let tone = sine(440.0, 1.0, WINDOW);
        assert!(magnitude_at(&tone, 440.0, RATE) > 100.0 * magnitude_at(&tone, 880.0, RATE));
    }

    #[test]
    fn a_sine_has_its_centroid_at_its_frequency() {
        let centroid = spectral_centroid(&sine(1_000.0, 1.0, WINDOW), RATE);
        assert!((centroid - 1_000.0).abs() < 60.0, "{centroid}");
    }

    #[test]
    fn a_decaying_sine_falls_20_db_when_its_envelope_says_it_should() {
        let tau = 0.5;
        let tone: Vec<f32> = sine(440.0, 1.0, WINDOW * 24)
            .iter()
            .enumerate()
            .map(|(n, &x)| x * (-(n as f32) / RATE / tau).exp())
            .collect();
        let fall = seconds_to_fall(&tone, 440.0, RATE).unwrap_or(f32::INFINITY);
        let expected = tau * 10.0f32.ln();
        assert!((fall - expected).abs() < 0.2, "{fall} vs {expected}");
    }

    #[test]
    fn a_steady_sine_never_falls() {
        assert_eq!(
            seconds_to_fall(&sine(440.0, 1.0, WINDOW * 4), 440.0, RATE),
            None
        );
    }

    #[test]
    fn a_stiff_string_stretches_its_upper_partials_sharp() {
        assert!(partial_hz(100.0, 1e-3, 10) > 1_000.0);
        assert!((partial_hz(100.0, 0.0, 10) - 1_000.0).abs() < 1e-3);
    }

    proptest! {
        #[test]
        fn every_measurement_is_finite_for_finite_audio(
            samples in proptest::collection::vec(-4.0f32..4.0, 0..512),
            frequency in 1.0f32..20_000.0,
        ) {
            prop_assert!(magnitude_at(&samples, frequency, RATE).is_finite());
            prop_assert!(spectral_centroid(&samples, RATE).is_finite());
            prop_assert!(rms(&samples).is_finite());
        }
    }
}
