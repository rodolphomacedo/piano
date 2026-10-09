//! The keybed thump (issue #64), measured as the difference between a note
//! rendered with and without it: present, velocity-correlated, and well
//! under the string's own attack.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use piano_audio::offline::OfflineEngine;
use piano_core::SampleRate;
use piano_core::action::DEFAULT_THUMP_GAIN;
use piano_params::Tuning;

const SAMPLE_RATE_HZ: f32 = 48_000.0;
const WINDOW: usize = 2_400;

fn render(midi: u8, velocity: f32, gain: f32) -> Vec<f32> {
    let mut engine =
        OfflineEngine::new(SampleRate::new(SAMPLE_RATE_HZ).unwrap(), Tuning::default());
    engine.set_action_noise_gain(gain);
    engine.note_on(midi, velocity);
    engine.render(WINDOW as f32 / SAMPLE_RATE_HZ)
}

fn rms(samples: &[f32]) -> f32 {
    (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
}

/// The knock's RMS and the note's, over the first 50 ms, in dB.
fn knock_and_note_db(midi: u8, velocity: f32) -> (f32, f32) {
    let with = render(midi, velocity, DEFAULT_THUMP_GAIN);
    let without = render(midi, velocity, 0.0);
    let knock: Vec<f32> = with.iter().zip(&without).map(|(a, b)| a - b).collect();
    (20.0 * rms(&knock).log10(), 20.0 * rms(&without).log10())
}

#[test]
fn the_keybed_knocks_under_the_tone_and_harder_with_velocity() {
    let (soft_knock, _) = knock_and_note_db(69, 0.3);
    let (hard_knock, hard_note) = knock_and_note_db(69, 1.0);
    println!("A4 knock soft {soft_knock:.1} dB, hard {hard_knock:.1} dB, note {hard_note:.1} dB");
    assert!(
        hard_knock > soft_knock + 10.0,
        "{soft_knock:.1} -> {hard_knock:.1} dB"
    );
    assert!(
        (hard_note - 35.0..hard_note - 12.0).contains(&hard_knock),
        "knock {hard_knock:.1} dB against note {hard_note:.1} dB"
    );
}

#[test]
fn the_knock_stands_out_more_in_the_treble_than_in_the_middle() {
    let (middle_knock, middle_note) = knock_and_note_db(60, 0.8);
    let (treble_knock, treble_note) = knock_and_note_db(96, 0.8);
    println!(
        "C4 knock/note {:.1} dB, C7 knock/note {:.1} dB",
        middle_knock - middle_note,
        treble_knock - treble_note
    );
    assert!(treble_knock - treble_note > middle_knock - middle_note);
}
