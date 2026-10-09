//! Measures every key's realised inharmonicity and fundamental tuning from
//! its rendered sound, against what `voicing` asked for (issue #96).
//!
//! Before #96 the dispersion cascade realised 1-13% of the requested `B`
//! below C6 and none above it, and the mid/treble sat up to 22 cents sharp —
//! while every existing test passed, because none of them located a
//! partial where it actually was. This one does: a long FFT per key, each
//! partial's peak searched for near where the previous one predicts it,
//! refined by parabolic interpolation, `B` solved from the highest partial
//! found.
//!
//! Run with `--nocapture` for the per-key table.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use piano_audio::voicing::config_for_key;
use piano_core::SampleRate;
use piano_core::string::PluckedString;
use piano_params::{PianoKey, Tuning};
use rustfft::{FftPlanner, num_complex::Complex32};

const SAMPLE_RATE_HZ: f32 = 48_000.0;
const FFT_LENGTH: usize = 1 << 15;
const HIGHEST_PARTIAL: usize = 8;
/// Matches the band the dispersion fit places partials in (about 10.8 kHz
/// at 48 kHz); above it the loss filter has already all but removed them.
const HIGHEST_MEASURED_HZ: f32 = 10_000.0;

/// A listener hears roughly 5 cents between two notes; the loop is tuned
/// analytically, so this is a regression bound with margin, not a target.
const FUNDAMENTAL_TOLERANCE_CENTS: f32 = 1.5;

/// The fit places partials to within a few cents; translated to `B` that
/// leaves this much relative slack on keys with enough stretch to measure.
const STIFFNESS_TOLERANCE: f32 = 0.35;

/// Below this much stretch on the highest measured partial, `B` cannot be
/// measured reliably from an FFT this long.
const MIN_MEASURABLE_STRETCH_CENTS: f32 = 4.0;

struct KeyMeasurement {
    midi: u8,
    requested_b: f32,
    realised_b: f32,
    fundamental_cents: f32,
    stretch_cents: f32,
}

fn magnitude_spectrum(samples: &[f32]) -> Vec<f32> {
    let mut buffer: Vec<Complex32> = samples
        .iter()
        .enumerate()
        .map(|(index, &sample)| {
            let phase = std::f32::consts::TAU * index as f32 / samples.len() as f32;
            Complex32::new(sample * (0.5 - 0.5 * phase.cos()), 0.0)
        })
        .collect();
    FftPlanner::<f32>::new()
        .plan_fft_forward(buffer.len())
        .process(&mut buffer);
    buffer[..samples.len() / 2]
        .iter()
        .map(|c| c.norm())
        .collect()
}

/// The strongest peak within `half_width_hz` of `centre_hz`, refined to
/// sub-bin precision by fitting a parabola through the log magnitudes.
fn peak_near(spectrum: &[f32], centre_hz: f32, half_width_hz: f32) -> f32 {
    let bin_hz = SAMPLE_RATE_HZ / (2 * spectrum.len()) as f32;
    let low = (((centre_hz - half_width_hz) / bin_hz) as usize).max(1);
    let high = (((centre_hz + half_width_hz) / bin_hz) as usize).min(spectrum.len() - 2);
    let index = (low..=high)
        .max_by(|&a, &b| spectrum[a].total_cmp(&spectrum[b]))
        .unwrap();
    let (left, centre, right) = (
        spectrum[index - 1].ln(),
        spectrum[index].ln(),
        spectrum[index + 1].ln(),
    );
    let offset = 0.5 * (left - right) / (left - 2.0 * centre + right);
    (index as f32 + offset) * bin_hz
}

fn render(midi: u8) -> (Vec<f32>, f32) {
    let rate = SampleRate::new(SAMPLE_RATE_HZ).unwrap();
    let key = PianoKey::from_midi(midi).unwrap();
    let config = config_for_key(key, Tuning::default(), rate);
    let requested = config.inharmonicity;
    let mut string = PluckedString::new(config, rate).unwrap();
    string.pluck(0.8);
    (
        (0..FFT_LENGTH).map(|_| string.process()).collect(),
        requested,
    )
}

fn measure(midi: u8) -> KeyMeasurement {
    let nominal = PianoKey::from_midi(midi)
        .unwrap()
        .frequency(Tuning::default())
        .hertz();
    let (samples, requested_b) = render(midi);
    let spectrum = magnitude_spectrum(&samples);
    let first = peak_near(&spectrum, nominal, 0.2 * nominal);
    let (n, highest) = highest_partial(&spectrum, first, nominal);
    let ratio = highest / (n as f32 * first);
    let (squared, n_squared) = (ratio * ratio, (n * n) as f32);
    KeyMeasurement {
        midi,
        requested_b,
        realised_b: (squared - 1.0) / (n_squared - squared),
        fundamental_cents: 1200.0 * (first / nominal).log2(),
        stretch_cents: 1200.0 * ratio.log2(),
    }
}

/// Walks up the partials from `first`, each searched for where the one
/// below it predicts, and returns the highest one found with its number.
fn highest_partial(spectrum: &[f32], first: f32, nominal: f32) -> (usize, f32) {
    let mut found = (1, first);
    for n in 2..=HIGHEST_PARTIAL {
        let predicted = found.1 / found.0 as f32 * n as f32;
        if predicted > HIGHEST_MEASURED_HZ {
            break;
        }
        found = (n, peak_near(spectrum, predicted, 0.3 * nominal));
    }
    found
}

#[test]
fn every_key_is_in_tune_and_as_stiff_as_voicing_asked() {
    let measurements: Vec<KeyMeasurement> = (21u8..=108).map(measure).collect();
    println!("key  requested_B  realised_B  stretch(c)  fundamental(c)");
    for m in &measurements {
        println!(
            "{:3}  {:10.6}  {:10.6}  {:9.1}  {:+8.2}",
            m.midi, m.requested_b, m.realised_b, m.stretch_cents, m.fundamental_cents
        );
    }
    for m in &measurements {
        assert!(
            m.fundamental_cents.abs() < FUNDAMENTAL_TOLERANCE_CENTS,
            "key {}: fundamental {:+.2} cents off",
            m.midi,
            m.fundamental_cents
        );
        if m.stretch_cents < MIN_MEASURABLE_STRETCH_CENTS {
            continue;
        }
        let relative = (m.realised_b - m.requested_b).abs() / m.requested_b;
        assert!(
            relative < STIFFNESS_TOLERANCE,
            "key {}: asked for B={}, realised {}",
            m.midi,
            m.requested_b,
            m.realised_b
        );
    }
}
