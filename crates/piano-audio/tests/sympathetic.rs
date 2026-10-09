//! Sympathetic resonance through the shared bridge (#62): a silently held
//! key answers a struck one in proportion to how many partials they share,
//! the sustain pedal blooms every note, and a whole keyboard of receptive
//! strings still loses energy.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use piano_audio::offline::OfflineEngine;
use piano_core::SampleRate;
use piano_params::Tuning;

const RATE: f32 = 48_000.0;

fn engine() -> OfflineEngine {
    let mut engine = OfflineEngine::new(SampleRate::new(RATE).unwrap(), Tuning::default());
    engine.set_action_noise_gain(0.0);
    engine
}

fn mean_square_db(samples: &[f32]) -> f32 {
    10.0 * (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32 + 1e-30).log10()
}

/// C4 struck and released with `silent` held down without sounding: how
/// loud what still rings afterwards is, relative to the note.
fn ringing_after_release_db(silent: Option<u8>) -> f32 {
    let mut engine = engine();
    if let Some(midi) = silent {
        engine.note_on(midi, 0.0);
    }
    engine.note_on(60, 0.8);
    let note = engine.render(0.3);
    engine.note_off(60);
    let _ = engine.render(0.2);
    mean_square_db(&engine.render(1.0)) - mean_square_db(&note)
}

#[test]
fn a_silently_held_octave_rings_and_an_unrelated_key_barely_does() {
    let octave = ringing_after_release_db(Some(72));
    let fifth = ringing_after_release_db(Some(67));
    let semitone = ringing_after_release_db(Some(61));
    println!("after C4: octave {octave:.1} dB, fifth {fifth:.1} dB, semitone {semitone:.1} dB");
    assert!((-45.0..-30.0).contains(&octave), "{octave:.1}");
    assert!(fifth < octave && fifth > semitone + 15.0, "{fifth:.1}");
    assert!(semitone < -60.0, "{semitone:.1}");
}

#[test]
fn the_sustain_pedal_blooms_a_note_well_under_its_own_level() {
    let render = |pedal: bool| {
        let mut engine = engine();
        engine.set_sustain_pedal(pedal);
        engine.note_on(60, 0.8);
        engine.render(3.0)
    };
    let with = render(true);
    let without = render(false);
    let bloom: Vec<f32> = with.iter().zip(&without).map(|(a, b)| a - b).collect();
    let tail = 48_000..;
    let level = mean_square_db(&bloom[tail.clone()]) - mean_square_db(&without[tail]);
    println!("pedal bloom {level:.1} dB relative to the note");
    assert!((-25.0..-10.0).contains(&level), "{level:.1}");
}

#[test]
fn a_full_keyboard_with_the_pedal_down_still_decays() {
    let mut engine = engine();
    engine.set_sustain_pedal(true);
    for midi in 21..=108 {
        engine.note_on(midi, 1.0);
    }
    let early = mean_square_db(&engine.render(2.0)[48_000..]);
    let _ = engine.render(2.0);
    let late = mean_square_db(&engine.render(2.0)[48_000..]);
    println!("full keyboard, pedal down: {early:.1} dB then {late:.1} dB");
    assert!(late < early - 6.0, "{early:.1} -> {late:.1}");
}
