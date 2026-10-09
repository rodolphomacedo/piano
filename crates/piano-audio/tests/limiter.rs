//! The output limiter's threshold is a live setting (#82): moving it
//! reaches the rendered output, not only the constant it replaced.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use piano_audio::limiter::{MAX_LIMITER_THRESHOLD, MIN_LIMITER_THRESHOLD};
use piano_audio::offline::OfflineEngine;
use piano_core::SampleRate;
use piano_params::Tuning;

/// A mezzo-forte C-major chord with the limiter at `threshold`.
fn chord(threshold: f32) -> Vec<f32> {
    let mut engine = OfflineEngine::new(SampleRate::new(48_000.0).unwrap(), Tuning::default());
    engine.set_limiter_threshold(threshold);
    for midi in [36u8, 48, 52, 55, 60, 64, 67, 72] {
        engine.note_on(midi, 0.5);
    }
    engine.render(0.5)
}

fn chord_peak(threshold: f32) -> f32 {
    chord(threshold)
        .iter()
        .fold(0.0f32, |peak, sample| peak.max(sample.abs()))
}

fn chord_rms(threshold: f32) -> f32 {
    let samples = chord(threshold);
    (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
}

#[test]
fn a_lower_threshold_squeezes_a_loud_chord() {
    let open = chord_rms(MAX_LIMITER_THRESHOLD);
    let squeezed = chord_rms(MIN_LIMITER_THRESHOLD);
    println!("chord RMS: {open:.3} open, {squeezed:.3} squeezed");
    assert!(squeezed < open * 0.97, "open {open}, squeezed {squeezed}");
}

#[test]
fn any_threshold_keeps_the_output_finite_and_bounded() {
    for threshold in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 0.0, -1.0, 2.0] {
        let peak = chord_peak(threshold);
        assert!(peak.is_finite() && peak <= 1.0, "{threshold}: {peak}");
    }
}
