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

#[test]
fn the_treble_rings_with_its_duplex_well_under_the_note() {
    for midi in [60u8, 72, 84, 96] {
        let level = duplex_below_note_db(midi);
        println!("MIDI {midi}: duplex {level:.1} dB under the note");
        assert!(
            (-45.0..-20.0).contains(&level),
            "MIDI {midi}: {level:.1} dB"
        );
    }
}

#[test]
fn the_bass_has_no_duplex() {
    assert!(duplex_below_note_db(40) < -200.0);
}
