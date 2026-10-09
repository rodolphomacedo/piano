//! Neighbouring keys no longer share one attack (#58): rendering the
//! attack of every key from C4 to C5 with its own felt and with its
//! register's smooth curve, each key's brightness moves by its own amount.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use piano_audio::voicing::config_for_key;
use piano_core::SampleRate;
use piano_core::excitation::hammer_for_frequency;
use piano_core::hammer::HammerConfig;
use piano_core::string::PluckedString;
use piano_params::{PianoKey, Tuning};
use rustfft::FftPlanner;
use rustfft::num_complex::Complex;

const RATE: f32 = 48_000.0;
const ATTACK_SAMPLES: usize = 2_400;

/// The attack's spectral centroid in hertz — the usual single-number proxy
/// for perceived brightness.
fn attack_brightness(midi: u8, hammer: impl Fn(HammerConfig, f32) -> HammerConfig) -> f32 {
    let rate = SampleRate::new(RATE).unwrap();
    let key = PianoKey::from_midi(midi).unwrap();
    let mut config = config_for_key(key, Tuning::default(), rate);
    config.hammer = hammer(config.hammer, key.frequency(Tuning::default()).hertz());
    let mut string = PluckedString::new(config, rate).unwrap();
    string.pluck(0.7);
    let attack: Vec<f32> = (0..ATTACK_SAMPLES).map(|_| string.process()).collect();
    spectral_centroid_hz(&attack)
}

fn spectral_centroid_hz(samples: &[f32]) -> f32 {
    let mut spectrum: Vec<Complex<f32>> = samples.iter().map(|&s| Complex::new(s, 0.0)).collect();
    FftPlanner::new()
        .plan_fft_forward(spectrum.len())
        .process(&mut spectrum);
    let bin_hz = RATE / spectrum.len() as f32;
    let half = &spectrum[..spectrum.len() / 2];
    let weighted: f32 = half
        .iter()
        .enumerate()
        .map(|(bin, value)| bin as f32 * bin_hz * value.norm())
        .sum();
    weighted / half.iter().map(|value| value.norm()).sum::<f32>()
}

/// How much each key's attack centroid moves, in percent, when its own
/// felt replaces its register's smooth curve.
fn centroid_change_percent(midi: u8) -> f32 {
    let uneven = attack_brightness(midi, |hammer, _| hammer);
    let smooth = attack_brightness(midi, |_, hz| hammer_for_frequency(hz));
    (uneven / smooth - 1.0) * 100.0
}

/// The smooth curve already lets neighbouring keys differ by ~12% in
/// attack centroid (strike-point combing and partial placement), so
/// comparing jitter against it buries the felt. This isolates the felt
/// instead: each key's own hammer must move its attack by its own amount,
/// in both directions, with some keys moving well past a few percent.
#[test]
fn per_key_felt_moves_each_attack_by_its_own_amount() {
    let changes: Vec<f32> = (60u8..=72).map(centroid_change_percent).collect();
    println!("C4-C5 attack centroid change vs the smooth curve, %: {changes:.1?}");
    let largest = changes.iter().fold(0.0f32, |most, c| most.max(c.abs()));
    assert!(largest > 8.0, "no key moved more than {largest:.1}%");
    assert!(changes.iter().any(|c| *c > 2.0) && changes.iter().any(|c| *c < -2.0));
    let mean = changes.iter().sum::<f32>() / changes.len() as f32;
    let spread =
        (changes.iter().map(|c| (c - mean).powi(2)).sum::<f32>() / changes.len() as f32).sqrt();
    assert!(spread > 3.0, "keys moved together: spread {spread:.1}%");
}
