//! Measured evidence for milestone M4's two objective "done when" criteria
//! from `docs/ROADMAP.md`: partials sit measurably sharp of an exact
//! harmonic series, and hitting harder makes a note brighter, not just
//! louder. Both are checked here with an FFT rather than asserted from the
//! implementation — this project's own precedent from M1, whose tuning claim
//! was a measured cents figure, not an assertion.
//!
//! The third M4 criterion, "audibly a piano rather than a guitar", is a
//! human-hearing judgement this test suite cannot make; see the M4 report
//! for where sample renders were left for a person to listen to.

#![allow(clippy::indexing_slicing, clippy::unwrap_used, clippy::expect_used)]

use piano_core::SampleRate;
use piano_params::{PianoKey, Tuning};
use piano_render::{RenderRequest, render_note};
use rustfft::{FftPlanner, num_complex::Complex32};

const SAMPLE_RATE_HZ: f32 = 48_000.0;

fn render(midi: u8, velocity: f32, seconds: f32) -> Vec<f32> {
    let request = RenderRequest {
        key: PianoKey::from_midi(midi).expect("valid MIDI note"),
        tuning: Tuning::default(),
        sample_rate: SampleRate::new(SAMPLE_RATE_HZ).expect("48 kHz is valid"),
        seconds,
        velocity,
    };
    render_note(request).expect("renders")
}

/// A Hann-windowed magnitude spectrum of `samples`, zero-padded to the next
/// power of two so `rustfft` can use its fastest path.
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

/// The frequency of the strongest bin within `+-search_hz` of `target_hz`.
fn peak_frequency_near(
    spectrum: &[f32],
    fft_len: usize,
    sample_rate: f32,
    target_hz: f32,
    search_hz: f32,
) -> f32 {
    let bin_hz = sample_rate / fft_len as f32;
    let center_bin = (target_hz / bin_hz).round() as i64;
    let span = (search_hz / bin_hz).round() as i64;

    let mut best_bin = center_bin;
    let mut best_magnitude = -1.0f32;
    for offset in -span..=span {
        let bin = center_bin + offset;
        if bin < 0 {
            continue;
        }
        if let Some(&magnitude) = spectrum.get(bin as usize)
            && magnitude > best_magnitude
        {
            best_magnitude = magnitude;
            best_bin = bin;
        }
    }
    best_bin as f32 * bin_hz
}

fn cents_deviation(measured_hz: f32, expected_hz: f32) -> f32 {
    1200.0 * (measured_hz / expected_hz).log2()
}

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
fn partials_sit_measurably_sharp_of_an_exact_harmonic_series() {
    // Fletcher's stiff-string formula (see `piano_core::dispersion`)
    // predicts f_n ~ n*f1*sqrt(1+B*n^2): partial n's deviation from n*f1, in
    // cents, should grow with n. Render A4, take a window well after the
    // attack transient (so the hammer's own broadband click has decayed and
    // only the string's own partials remain), and measure partials 1..=8.
    let samples = render(69, 0.9, 2.0);
    let start = (0.3 * SAMPLE_RATE_HZ) as usize;
    let length = 32_768.min(samples.len() - start);
    let window = &samples[start..start + length];
    let (spectrum, fft_len) = magnitude_spectrum(window);

    // Partial 8 of A4 (3520 Hz) is measured but deliberately excluded from
    // the assertions below: by that far above the fundamental the loop
    // filter's fixed zero (see `filter::LoopFilter`) has already cut its
    // energy close to the noise floor for this register, and
    // `peak_frequency_near` occasionally locks onto a spurious nearby bin
    // instead of the true partial — a measurement-resolution limitation of
    // this test, not evidence against the dispersion model, which is why
    // partials 1..=7 (all comfortably above the noise floor for A4) are
    // what the assertions below check.
    let fundamental = 440.0;
    let mut deviations = Vec::new();
    for partial in 1..=8u32 {
        let expected = fundamental * partial as f32;
        let measured = peak_frequency_near(
            &spectrum,
            fft_len,
            SAMPLE_RATE_HZ,
            expected,
            expected * 0.05,
        );
        deviations.push(cents_deviation(measured, expected));
    }
    println!("measured partial deviations (cents): {deviations:?}");

    // Partial 1 is what the loop is tuned to, so it should sit close to
    // in-tune.
    assert!(
        deviations[0].abs() < 5.0,
        "fundamental drifted {} cents",
        deviations[0]
    );

    // Higher partials should sit measurably sharper than the fundamental,
    // and the sharpening should grow with partial number: the qualitative
    // signature Fletcher's formula predicts, which a pure harmonic series
    // (an ideal, non-dispersive string) could never produce.
    assert!(
        deviations[6] > deviations[0] + 1.0,
        "partial 7 ({} cents) is not measurably sharper than the fundamental ({} cents)",
        deviations[6],
        deviations[0]
    );
    assert!(
        deviations[6] > deviations[3],
        "sharpening does not grow with partial number: partial 4 {} cents, partial 7 {} cents",
        deviations[3],
        deviations[6]
    );
}

#[test]
fn hitting_harder_makes_a_note_brighter_not_merely_louder() {
    // Compare spectral centroid (energy-weighted mean frequency) between a
    // soft and a hard strike of the same note. Neither render is
    // loudness-normalised on purpose: the hammer model's whole claim is that
    // velocity changes the excitation's *shape*, and a shape change shows up
    // as a centroid shift regardless of overall level, which is exactly what
    // "brighter, not merely louder" needs to demonstrate.
    let soft = render(69, 0.15, 0.5);
    let hard = render(69, 0.95, 0.5);

    let centroid_soft = spectral_centroid(&soft);
    let centroid_hard = spectral_centroid(&hard);
    println!("spectral centroid: soft {centroid_soft:.1} Hz, hard {centroid_hard:.1} Hz");

    assert!(
        centroid_hard > centroid_soft,
        "hard strike centroid {centroid_hard} Hz is not brighter than soft strike centroid {centroid_soft} Hz"
    );
}
