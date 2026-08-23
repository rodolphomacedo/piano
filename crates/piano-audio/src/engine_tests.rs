//! Tests for [`super`], split into its own file so `engine.rs` itself
//! stays under this project's 500-line file limit (see
//! `CONTRIBUTING.md`) — `#[path = "engine_tests.rs"] mod tests;` at the
//! bottom of `engine.rs` still compiles this as `engine::tests`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::Duration;

use super::*;

fn engine() -> Engine {
    let rate = SampleRate::new(48_000.0).expect("48 kHz is valid");
    Engine::new(rate, Tuning::default())
}

fn ring_buffer() -> (rtrb::Producer<Command>, Consumer<Command>) {
    ring_buffer_with_capacity(16)
}

fn ring_buffer_with_capacity(capacity: usize) -> (rtrb::Producer<Command>, Consumer<Command>) {
    rtrb::RingBuffer::new(capacity)
}

#[test]
fn a_note_on_command_produces_sound() {
    let mut engine = engine();
    let (mut producer, mut consumer) = ring_buffer();
    producer
        .push(Command::NoteOn {
            midi: 69,
            velocity: 1.0,
        })
        .expect("queue has room");
    engine.drain_commands(&mut consumer);

    let mut buffer = [0.0f32; 512];
    engine.process_block(&mut buffer);
    assert!(buffer.iter().any(|sample| sample.abs() > 1e-3));
}

#[test]
fn an_out_of_range_note_is_ignored_without_panicking() {
    let mut engine = engine();
    let (mut producer, mut consumer) = ring_buffer();
    producer
        .push(Command::NoteOn {
            midi: 200,
            velocity: 1.0,
        })
        .expect("queue has room");
    engine.drain_commands(&mut consumer);

    let mut buffer = [0.0f32; 64];
    engine.process_block(&mut buffer);
    assert!(buffer.iter().all(|sample| *sample == 0.0));
}

#[test]
fn all_notes_off_silences_every_voice() {
    let mut engine = engine();
    let (mut producer, mut consumer) = ring_buffer();
    producer
        .push(Command::NoteOn {
            midi: 69,
            velocity: 1.0,
        })
        .expect("queue has room");
    engine.drain_commands(&mut consumer);
    producer.push(Command::AllNotesOff).expect("queue has room");
    engine.drain_commands(&mut consumer);

    let mut buffer = [0.0f32; 64];
    engine.process_block(&mut buffer);
    assert!(buffer.iter().all(|sample| *sample == 0.0));
}

#[test]
fn striking_every_key_at_once_never_panics() {
    let mut engine = engine();
    let (mut producer, mut consumer) = ring_buffer_with_capacity(KEY_COUNT);
    for midi in LOWEST_PIANO_KEY..=HIGHEST_PIANO_KEY {
        producer
            .push(Command::NoteOn {
                midi,
                velocity: 1.0,
            })
            .expect("queue has room");
    }
    engine.drain_commands(&mut consumer);

    let mut buffer = [0.0f32; 64];
    engine.process_block(&mut buffer);
    assert!(buffer.iter().any(|sample| sample.is_finite()));
}

#[test]
fn re_striking_the_same_key_never_panics() {
    let mut engine = engine();
    let (mut producer, mut consumer) = ring_buffer_with_capacity(64);
    for _ in 0..64 {
        producer
            .push(Command::NoteOn {
                midi: 69,
                velocity: 0.9,
            })
            .expect("queue has room");
    }
    engine.drain_commands(&mut consumer);
    let mut buffer = [0.0f32; 64];
    engine.process_block(&mut buffer);
}

#[test]
fn flooding_the_queue_beyond_the_drain_cap_never_panics() {
    let mut engine = engine();
    let (mut producer, mut consumer) = ring_buffer();
    for _ in 0..(MAX_COMMANDS_PER_CALLBACK + 8) {
        let _ = producer.push(Command::NoteOn {
            midi: 69,
            velocity: 0.5,
        });
    }
    engine.drain_commands(&mut consumer);
    let mut buffer = [0.0f32; 32];
    engine.process_block(&mut buffer);
}

#[test]
fn set_damping_reaches_an_already_ringing_voice() {
    let mut engine = engine();
    let (mut producer, mut consumer) = ring_buffer();
    producer
        .push(Command::NoteOn {
            midi: 69,
            velocity: 1.0,
        })
        .expect("queue has room");
    engine.drain_commands(&mut consumer);

    let mut before = [0.0f32; 128];
    engine.process_block(&mut before);

    producer
        .push(Command::SetDamping { damping: 0.99 })
        .expect("queue has room");
    engine.drain_commands(&mut consumer);

    let mut after = [0.0f32; 128];
    engine.process_block(&mut after);
    assert_ne!(
        before.to_vec(),
        after.to_vec(),
        "SetDamping had no audible effect on a ringing voice"
    );
}

#[test]
fn set_sustain_never_panics_on_a_silent_engine() {
    let mut engine = engine();
    let (mut producer, mut consumer) = ring_buffer();
    producer
        .push(Command::SetSustain { sustain: 0.2 })
        .expect("queue has room");
    engine.drain_commands(&mut consumer);
    let mut buffer = [0.0f32; 32];
    engine.process_block(&mut buffer);
    assert!(buffer.iter().all(|sample| *sample == 0.0));
}

#[test]
fn set_damping_out_of_range_never_panics() {
    let mut engine = engine();
    let (mut producer, mut consumer) = ring_buffer();
    producer
        .push(Command::SetDamping { damping: f32::NAN })
        .expect("queue has room");
    producer
        .push(Command::SetDamping { damping: 50.0 })
        .expect("queue has room");
    engine.drain_commands(&mut consumer);
    let mut buffer = [0.0f32; 32];
    engine.process_block(&mut buffer);
}

#[test]
fn note_off_releases_a_ringing_voice_quickly() {
    let mut engine = engine();
    let (mut producer, mut consumer) = ring_buffer();
    producer
        .push(Command::NoteOn {
            midi: 69,
            velocity: 1.0,
        })
        .expect("queue has room");
    engine.drain_commands(&mut consumer);
    producer
        .push(Command::NoteOff { midi: 69 })
        .expect("queue has room");
    engine.drain_commands(&mut consumer);

    // Released notes reach silence in tens of milliseconds; a held one
    // would still be ringing after only 48_000 samples (one second).
    let mut buffer = vec![0.0f32; 48_000];
    engine.process_block(&mut buffer);
    let tail = buffer
        .get(buffer.len() - 128..)
        .expect("buffer has at least 128 samples");
    assert!(
        tail.iter().all(|sample| sample.abs() < 1e-3),
        "note-off should have let the voice ring out quickly"
    );
}

#[test]
fn note_off_while_the_pedal_is_down_does_not_release_until_pedal_up() {
    let mut engine = engine();
    let (mut producer, mut consumer) = ring_buffer();
    producer
        .push(Command::SustainPedal { down: true })
        .expect("queue has room");
    producer
        .push(Command::NoteOn {
            midi: 69,
            velocity: 1.0,
        })
        .expect("queue has room");
    producer
        .push(Command::NoteOff { midi: 69 })
        .expect("queue has room");
    engine.drain_commands(&mut consumer);

    // A held (pedal-down) note must still be clearly audible well past
    // when an actually-released one would have reached silence (tens of
    // milliseconds — `note_off_releases_a_ringing_voice_quickly` checks
    // that). This no longer checks a full second in: M6's unison strings
    // give a genuinely faster *natural* initial decay than a lone
    // near-lossless string had (the beating "pre-decay" stage real piano
    // notes have, `docs/ROADMAP.md`), so by one second in a pedal-held
    // note can have already decayed close to this test's audibility
    // threshold on its own, which is real physics, not the pending
    // release this test exists to catch. 0.2 s stays far inside the
    // window where "held" and "released" are unambiguously different.
    let mut held = vec![0.0f32; 9_600];
    engine.process_block(&mut held);
    let held_tail = held
        .get(held.len() - 128..)
        .expect("buffer has at least 128 samples");
    assert!(
        held_tail.iter().any(|sample| sample.abs() > 1e-3),
        "a pedal-held note-off should not have released the voice yet"
    );

    producer
        .push(Command::SustainPedal { down: false })
        .expect("queue has room");
    engine.drain_commands(&mut consumer);
    let mut released = vec![0.0f32; 48_000];
    engine.process_block(&mut released);
    let released_tail = released
        .get(released.len() - 128..)
        .expect("buffer has at least 128 samples");
    assert!(
        released_tail.iter().all(|sample| sample.abs() < 1e-3),
        "releasing the pedal should have released the held voice"
    );
}

#[test]
fn re_striking_a_pedal_held_key_cancels_its_pending_release() {
    let mut engine = engine();
    let (mut producer, mut consumer) = ring_buffer();
    producer
        .push(Command::SustainPedal { down: true })
        .expect("queue has room");
    producer
        .push(Command::NoteOn {
            midi: 69,
            velocity: 1.0,
        })
        .expect("queue has room");
    producer
        .push(Command::NoteOff { midi: 69 })
        .expect("queue has room");
    // Re-struck by the finger while the pedal is still down.
    producer
        .push(Command::NoteOn {
            midi: 69,
            velocity: 1.0,
        })
        .expect("queue has room");
    engine.drain_commands(&mut consumer);

    producer
        .push(Command::SustainPedal { down: false })
        .expect("queue has room");
    engine.drain_commands(&mut consumer);

    // The re-strike is not pedal-held, so lifting the pedal must not
    // have released it — it should still be ringing from the second
    // NoteOn.
    let mut buffer = vec![0.0f32; 4_800];
    engine.process_block(&mut buffer);
    assert!(
        buffer.iter().any(|sample| sample.abs() > 1e-3),
        "re-striking should have cancelled the pending pedal release"
    );
}

#[test]
fn note_off_on_an_out_of_range_note_never_panics() {
    let mut engine = engine();
    let (mut producer, mut consumer) = ring_buffer();
    producer
        .push(Command::NoteOff { midi: 200 })
        .expect("queue has room");
    engine.drain_commands(&mut consumer);
    let mut buffer = [0.0f32; 64];
    engine.process_block(&mut buffer);
}

/// A real wall-clock measurement, not a strict CI gate: timing noise on a
/// shared CI runner would make this flaky as a pass/fail assertion, so it
/// stays `#[ignore]`d and is meant to be run by hand (`cargo test --release
/// -p piano-audio -- --ignored --nocapture`) whenever `docs/PERFORMANCE.md`
/// needs a fresh number for `PERF-003`/`PERF-006`/`PERF-010`. It only
/// checks that processing a full block at the documented 88-voice
/// polyphony (`docs/ROADMAP.md`'s M5 "done when") comfortably clears the
/// 2.67 ms callback deadline `docs/REALTIME-AUDIO-RULES.md` sets for a
/// 128-sample block at 48 kHz — printing the measured time for a human to
/// record.
const CALLBACK_TIMING_BLOCKS: u32 = 1_000;

#[test]
#[ignore = "wall-clock measurement, run manually with --ignored --nocapture"]
fn callback_time_at_full_88_voice_polyphony_clears_the_deadline() {
    let mut engine = engine();
    let (mut producer, mut consumer) = ring_buffer_with_capacity(KEY_COUNT);
    for midi in LOWEST_PIANO_KEY..=HIGHEST_PIANO_KEY {
        producer
            .push(Command::NoteOn {
                midi,
                velocity: 1.0,
            })
            .expect("queue has room");
    }
    engine.drain_commands(&mut consumer);

    let mut buffer = vec![0.0f32; 128];
    let started = std::time::Instant::now();
    for _ in 0..CALLBACK_TIMING_BLOCKS {
        engine.process_block(&mut buffer);
    }
    let elapsed = started.elapsed();
    let per_block = elapsed / CALLBACK_TIMING_BLOCKS;
    println!(
        "88-voice process_block: {per_block:?} per 128-sample block \
         (budget 2.67 ms at 48 kHz)"
    );
    assert!(
        per_block < Duration::from_micros(2_670),
        "per-block time {per_block:?} exceeded the 2.67 ms deadline"
    );
}

#[test]
fn a_note_the_engine_cannot_tune_is_ignored_without_panicking() {
    let rate = SampleRate::new(8_000.0).expect("8 kHz is valid");
    let mut engine = Engine::new(rate, Tuning::default());
    let (mut producer, mut consumer) = ring_buffer();
    let key = PianoKey::from_midi(HIGHEST_PIANO_KEY).expect("C8 is on the keyboard");
    producer
        .push(Command::NoteOn {
            midi: key.midi_number(),
            velocity: 1.0,
        })
        .expect("queue has room");
    engine.drain_commands(&mut consumer);
    let mut buffer = [0.0f32; 16];
    engine.process_block(&mut buffer);
}
