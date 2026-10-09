//! The room (`piano_core::room`): heard behind the instrument at the live
//! default, never in front of it, and absent from dry offline renders.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use piano_audio::offline::OfflineEngine;
use piano_core::SampleRate;
use piano_core::room::DEFAULT_ROOM_MIX;
use piano_params::Tuning;

const SECONDS: f32 = 2.0;

fn render(midi: u8, mix: f32) -> (Vec<f32>, Vec<f32>) {
    let mut engine = OfflineEngine::new(SampleRate::new(48_000.0).unwrap(), Tuning::default());
    engine.set_room_mix(mix);
    engine.note_on(midi, 0.6);
    engine.render_stereo(SECONDS)
}

fn energy(samples: &[f32]) -> f32 {
    samples.iter().map(|s| s * s).sum()
}

/// The room's energy relative to the dry instrument's, in dB.
fn room_below_direct_db(midi: u8) -> f32 {
    let (wet_left, wet_right) = render(midi, DEFAULT_ROOM_MIX);
    let (dry_left, dry_right) = render(midi, 0.0);
    let room: f32 = wet_left
        .iter()
        .zip(&dry_left)
        .chain(wet_right.iter().zip(&dry_right))
        .map(|(wet, dry)| (wet - dry).powi(2))
        .sum();
    10.0 * (room / (energy(&dry_left) + energy(&dry_right))).log10()
}

#[test]
fn the_room_sits_behind_the_instrument() {
    for midi in [33u8, 60, 84] {
        let level = room_below_direct_db(midi);
        println!("MIDI {midi}: room {level:.1} dB relative to the direct sound");
        assert!((-18.0..-4.0).contains(&level), "MIDI {midi}: {level:.1} dB");
    }
}

/// Energy left in the last half second after a note's key comes up, with
/// the room's bass reverberation time set to `seconds`.
fn late_room_energy(seconds: f32) -> f32 {
    let mut engine = OfflineEngine::new(SampleRate::new(48_000.0).unwrap(), Tuning::default());
    engine.set_room_mix(DEFAULT_ROOM_MIX);
    engine.set_room_reverb_seconds(seconds);
    engine.set_room_treble_reverb_seconds(seconds);
    engine.note_on(60, 0.6);
    let _ = engine.render_stereo(0.5);
    engine.note_off(60);
    let _ = engine.render_stereo(1.0);
    let (left, right) = engine.render_stereo(0.5);
    energy(&left) + energy(&right)
}

/// The shape setters reach the live engine, not only `Room` in isolation:
/// a cathedral still rings after the dampers have stopped the strings.
#[test]
fn a_longer_room_keeps_ringing_after_the_strings_are_damped() {
    let studio = late_room_energy(0.3);
    let cathedral = late_room_energy(6.0);
    assert!(
        cathedral > studio * 100.0,
        "studio {studio:e}, cathedral {cathedral:e}"
    );
}
