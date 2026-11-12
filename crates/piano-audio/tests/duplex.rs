//! The duplex scale (`piano_core::duplex`): the treble's free segments ring
//! well under the note, and the bass has none.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use piano_audio::offline::OfflineEngine;
use piano_audio::voicing::DUPLEX_GAIN;
use piano_core::SampleRate;
use piano_params::Tuning;

fn render(midi: u8, gain: f32) -> Vec<f32> {
    let mut engine = OfflineEngine::new(SampleRate::new(48_000.0).unwrap(), Tuning::default());
    engine.set_action_noise_gain(0.0);
    engine.set_duplex_gain(gain);
    engine.note_on(midi, 0.6);
    engine.render(1.5)
}

fn mean_square_db(samples: &[f32]) -> f32 {
    10.0 * (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32 + 1e-30).log10()
}

/// The duplex ring's level relative to the note, over the whole render.
fn duplex_below_note_db(midi: u8) -> f32 {
    let with = render(midi, DUPLEX_GAIN);
    let without = render(midi, 0.0);
    let ring: Vec<f32> = with.iter().zip(&without).map(|(a, b)| a - b).collect();
    mean_square_db(&ring) - mean_square_db(&without)
}

/// The resonance is a hertz or two wide and the string's real partial
/// lands a few cents either side of where its `B` predicts, so how hard a
/// segment rings differs from key to key by tens of decibels, as it does
/// on an instrument whose duplex bars were set by ear. What is fixed is
/// where the keyboard as a whole sits, and that no key's duplex comes near
/// the note itself.
#[test]
fn the_treble_rings_with_its_duplex_well_under_the_note() {
    let mut levels: Vec<f32> = (60u8..=96).step_by(3).map(duplex_below_note_db).collect();
    println!("duplex under the note, every third key from C4: {levels:.1?}");
    levels.sort_by(f32::total_cmp);
    let (median, loudest) = (levels[levels.len() / 2], levels[levels.len() - 1]);
    assert!((-45.0..-20.0).contains(&median), "median {median:.1} dB");
    assert!(loudest < -15.0, "loudest {loudest:.1} dB");
}

#[test]
fn the_bass_has_no_duplex() {
    assert!(duplex_below_note_db(40) < -200.0);
}
