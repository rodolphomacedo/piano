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

/// What a release adds over the same release with a silent action, in dB
/// relative to the ringing note, over the 50 ms after the key comes up.
fn release_thump_db(pedal: bool) -> f32 {
    let render = |gain: f32| {
        let mut engine =
            OfflineEngine::new(SampleRate::new(SAMPLE_RATE_HZ).unwrap(), Tuning::default());
        engine.set_action_noise_gain(gain);
        engine.set_sustain_pedal(pedal);
        engine.note_on(60, 0.6);
        let _ = engine.render(0.5);
        engine.note_off(60);
        engine.render(WINDOW as f32 / SAMPLE_RATE_HZ)
    };
    let with = render(DEFAULT_THUMP_GAIN);
    let without = render(0.0);
    let thump: Vec<f32> = with.iter().zip(&without).map(|(a, b)| a - b).collect();
    20.0 * (rms(&thump) + 1e-12).log10() - 20.0 * rms(&without).log10()
}

#[test]
fn a_released_key_lets_its_damper_land_softly() {
    let released = release_thump_db(false);
    let pedal_held = release_thump_db(true);
    println!("damper landing {released:.1} dB, under the pedal {pedal_held:.1} dB");
    assert!((-35.0..-15.0).contains(&released), "{released:.1}");
    // Under the pedal nothing lands; what is left is only the soundboard
    // still ringing from the strike's own keybed knock half a second ago.
    assert!(pedal_held < released - 30.0, "{pedal_held:.1}");
}
