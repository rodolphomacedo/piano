//! Phantom partials (issue #54), measured as the difference between a note
//! rendered with and without them: there in the bass, growing with the
//! square of the strike, and absent from the treble.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use piano_audio::offline::OfflineEngine;
use piano_audio::voicing::PHANTOM_GAIN_IN_BASS;
use piano_core::SampleRate;
use piano_params::Tuning;

const SAMPLE_RATE_HZ: f32 = 48_000.0;
const WINDOW_SECONDS: f32 = 0.3;

fn render(midi: u8, velocity: f32, gain: f32) -> Vec<f32> {
    let mut engine =
        OfflineEngine::new(SampleRate::new(SAMPLE_RATE_HZ).unwrap(), Tuning::default());
    engine.set_action_noise_gain(0.0);
    engine.set_phantom_gain(gain);
    engine.note_on(midi, velocity);
    engine.render(WINDOW_SECONDS)
}

fn rms_db(samples: &[f32]) -> f32 {
    10.0 * (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32 + 1e-30).log10()
}

/// The phantom component's level relative to the note's, in dB.
fn phantom_below_note_db(midi: u8, velocity: f32) -> f32 {
    let with = render(midi, velocity, PHANTOM_GAIN_IN_BASS);
    let without = render(midi, velocity, 0.0);
    let phantom: Vec<f32> = with.iter().zip(&without).map(|(a, b)| a - b).collect();
    rms_db(&phantom) - rms_db(&without)
}

#[test]
fn the_bass_carries_phantoms_well_under_the_tone_at_fortissimo() {
    for midi in [21u8, 33, 45] {
        let level = phantom_below_note_db(midi, 1.0);
        println!("MIDI {midi}: phantoms {level:.1} dB under the note at ff");
        assert!(
            (-32.0..-15.0).contains(&level),
            "MIDI {midi}: {level:.1} dB"
        );
    }
}

#[test]
fn phantoms_grow_relative_to_the_tone_with_velocity() {
    let soft = phantom_below_note_db(33, 0.3);
    let loud = phantom_below_note_db(33, 1.0);
    println!("A1 phantoms: p {soft:.1} dB, ff {loud:.1} dB");
    assert!(loud > soft + 10.0, "{soft:.1} -> {loud:.1} dB");
}

#[test]
fn the_treble_has_no_phantoms() {
    assert!(phantom_below_note_db(84, 1.0) < -200.0);
}
