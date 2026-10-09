//! The three pedals through the full engine: half-pedalling (#61), the
//! sostenuto (#60) and the una corda (#59), each measured from the
//! rendered sound rather than from the engine's internal state.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use piano_audio::offline::OfflineEngine;
use piano_core::SampleRate;
use piano_params::{PianoKey, Tuning};

const SAMPLE_RATE_HZ: f32 = 48_000.0;

fn engine() -> OfflineEngine {
    OfflineEngine::new(SampleRate::new(SAMPLE_RATE_HZ).unwrap(), Tuning::default())
}

fn rms_db(samples: &[f32]) -> f32 {
    10.0 * (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).log10()
}

/// Magnitude of `samples` at `frequency_hz`, by a Hann-windowed single-bin
/// DFT.
fn magnitude_at(samples: &[f32], frequency_hz: f32) -> f32 {
    let omega = std::f32::consts::TAU * frequency_hz / SAMPLE_RATE_HZ;
    let (mut real, mut imag) = (0.0f32, 0.0f32);
    for (index, &sample) in samples.iter().enumerate() {
        let window =
            0.5 - 0.5 * (std::f32::consts::TAU * index as f32 / samples.len() as f32).cos();
        real += sample * window * (omega * index as f32).cos();
        imag -= sample * window * (omega * index as f32).sin();
    }
    (real * real + imag * imag).sqrt() / samples.len() as f32
}

fn hz(midi: u8) -> f32 {
    PianoKey::from_midi(midi)
        .unwrap()
        .frequency(Tuning::default())
        .hertz()
}

/// The level of A3, struck and released at once, one second after release
/// with the sustain pedal resting at `position`.
fn ring_after_release_db(position: f32) -> f32 {
    let mut engine = engine();
    engine.set_sustain_pedal_position(position);
    engine.note_on(57, 0.7);
    let _ = engine.render(0.2);
    engine.note_off(57);
    let _ = engine.render(1.0);
    rms_db(&engine.render(0.5))
}

#[test]
fn half_pedalling_grades_the_ring_between_damped_and_held() {
    let damped = ring_after_release_db(0.0);
    let half = ring_after_release_db(0.47);
    let held = ring_after_release_db(1.0);
    assert!(
        damped + 6.0 < half && half + 6.0 < held,
        "damped {damped:.1} dB, half {half:.1} dB, held {held:.1} dB"
    );
}

#[test]
fn the_sostenuto_holds_only_the_keys_down_when_it_was_pressed() {
    let (low, high) = (48u8, 78u8);
    let mut engine = engine();
    engine.note_on(low, 0.7);
    let _ = engine.render(0.1);
    engine.set_sostenuto_pedal(true);
    engine.note_off(low);
    engine.note_on(high, 0.7);
    let _ = engine.render(0.3);
    engine.note_off(high);
    let _ = engine.render(1.0);
    let late = engine.render(0.5);
    let caught = magnitude_at(&late, hz(low));
    let free = magnitude_at(&late, hz(high));
    assert!(
        caught > 30.0 * free,
        "caught C3 {caught:e} should ring far above released F#5 {free:e}"
    );
}

#[test]
fn releasing_the_sostenuto_damps_what_it_caught() {
    let mut engine = engine();
    engine.note_on(48, 0.7);
    engine.set_sostenuto_pedal(true);
    engine.note_off(48);
    let _ = engine.render(0.3);
    let holding = rms_db(&engine.render(0.2));
    engine.set_sostenuto_pedal(false);
    let _ = engine.render(1.0);
    let released = rms_db(&engine.render(0.2));
    assert!(
        released + 20.0 < holding,
        "{holding:.1} -> {released:.1} dB"
    );
}

/// A4's attack level and how far it has fallen half a second to a second
/// later, in dB — the prompt sound against the start of the aftersound.
fn attack_and_prompt_drop_db(soft: bool) -> (f32, f32) {
    let mut engine = engine();
    engine.set_soft_pedal(soft);
    engine.note_on(69, 0.7);
    let samples = engine.render(1.0);
    let attack = rms_db(&samples[..9_600]);
    let after = rms_db(&samples[24_000..]);
    (attack, attack - after)
}

/// With one string of three left unstruck, the struck pair's in-phase
/// motion drives the third through the coupling, so less of the note is
/// spent in the fast prompt decay: the soft-pedal note is quieter *and*
/// falls away more slowly at first (measured: 11.2 dB against 7.9 dB over
/// the first second at A4) — Weinreich's account of why the una corda
/// sounds veiled rather than merely softer.
#[test]
fn the_una_corda_changes_the_colour_not_only_the_level() {
    let (normal_attack, normal_drop) = attack_and_prompt_drop_db(false);
    let (soft_attack, soft_drop) = attack_and_prompt_drop_db(true);
    assert!(
        soft_attack < normal_attack - 1.0,
        "striking two of three strings should be quieter: {soft_attack:.1} vs {normal_attack:.1} dB"
    );
    assert!(
        soft_drop + 2.0 < normal_drop,
        "the missed string should soften the prompt decay: {soft_drop:.1} vs {normal_drop:.1} dB"
    );
}
