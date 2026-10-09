//! The stereo image: bass to the player's left, treble to the right, a
//! soundboard that sounds different in each ear, and a mono mean that is
//! exactly the mono render every other test calibrates against.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use piano_audio::offline::OfflineEngine;
use piano_core::SampleRate;
use piano_params::Tuning;

const SECONDS: f32 = 0.5;

fn engine() -> OfflineEngine {
    OfflineEngine::new(SampleRate::new(48_000.0).unwrap(), Tuning::default())
}

fn stereo(midi: u8) -> (Vec<f32>, Vec<f32>) {
    let mut engine = engine();
    engine.note_on(midi, 0.6);
    engine.render_stereo(SECONDS)
}

fn energy(samples: &[f32]) -> f32 {
    samples.iter().map(|s| s * s).sum()
}

fn left_over_right_db(midi: u8) -> f32 {
    let (left, right) = stereo(midi);
    10.0 * (energy(&left) / energy(&right)).log10()
}

#[test]
fn the_stereo_mean_is_the_mono_render() {
    let (left, right) = stereo(60);
    let mut mono_engine = engine();
    mono_engine.note_on(60, 0.6);
    let mono = mono_engine.render(SECONDS);
    assert_eq!(mono.len(), left.len());
    for ((l, r), m) in left.iter().zip(&right).zip(&mono) {
        assert!((0.5 * (l + r) - m).abs() < 1e-6, "{l} {r} {m}");
    }
}

#[test]
fn the_bass_sits_left_and_the_treble_right() {
    let bass = left_over_right_db(21);
    let treble = left_over_right_db(108);
    println!("A0 left-right {bass:.1} dB, C8 left-right {treble:.1} dB");
    assert!(bass > 4.0, "{bass:.1}");
    assert!(treble < -4.0, "{treble:.1}");
}

#[test]
fn the_soundboard_sounds_different_in_each_ear() {
    let knock = |gain: f32| {
        let mut engine = engine();
        engine.set_action_noise_gain(gain);
        engine.note_on(64, 1.0);
        engine.render_stereo(0.05)
    };
    let (with_left, with_right) = knock(piano_core::action::DEFAULT_THUMP_GAIN);
    let (without_left, without_right) = knock(0.0);
    let left: Vec<f32> = with_left
        .iter()
        .zip(&without_left)
        .map(|(a, b)| a - b)
        .collect();
    let right: Vec<f32> = with_right
        .iter()
        .zip(&without_right)
        .map(|(a, b)| a - b)
        .collect();
    let cross: f32 = left.iter().zip(&right).map(|(l, r)| l * r).sum();
    let correlation = cross / (energy(&left) * energy(&right)).sqrt();
    println!("soundboard left/right correlation {correlation:.3}");
    assert!(correlation < 0.9, "{correlation}");
}
