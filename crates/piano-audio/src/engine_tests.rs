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
fn set_soundboard_mode_out_of_range_index_never_panics() {
    let mut engine = engine();
    let (mut producer, mut consumer) = ring_buffer();
    producer
        .push(Command::SetSoundboardMode {
            index: 999,
            mode: SoundboardMode {
                frequency_hz: f32::NAN,
                decay_seconds: f32::NAN,
                gain: f32::NAN,
            },
        })
        .expect("queue has room");
    engine.drain_commands(&mut consumer);
    let mut buffer = [0.0f32; 32];
    engine.process_block(&mut buffer);
}

#[test]
fn set_soundboard_mode_measurably_changes_the_soundboard_tail() {
    let mut quiet = engine();
    let mut loud = engine();
    let (mut producer, mut consumer) = ring_buffer();

    producer
        .push(Command::SetSoundboardMode {
            index: 0,
            mode: SoundboardMode {
                frequency_hz: 300.0,
                decay_seconds: 2.0,
                gain: 3.0,
            },
        })
        .expect("queue has room");
    loud.drain_commands(&mut consumer);

    for engine in [&mut quiet, &mut loud] {
        producer
            .push(Command::NoteOn {
                midi: 69,
                velocity: 1.0,
            })
            .expect("queue has room");
        engine.drain_commands(&mut consumer);
    }

    let mut quiet_out = [0.0f32; 512];
    let mut loud_out = [0.0f32; 512];
    quiet.process_block(&mut quiet_out);
    loud.process_block(&mut loud_out);
    assert_ne!(
        quiet_out.to_vec(),
        loud_out.to_vec(),
        "SetSoundboardMode had no audible effect"
    );
}

#[test]
fn set_local_coupling_gain_reaches_an_already_ringing_voice() {
    let mut engine = engine();
    let (mut producer, mut consumer) = ring_buffer();
    producer
        .push(Command::NoteOn {
            midi: 69,
            velocity: 1.0,
        })
        .expect("queue has room");
    engine.drain_commands(&mut consumer);

    let mut before = [0.0f32; 512];
    engine.process_block(&mut before);

    producer
        .push(Command::SetLocalCouplingGain { gain: 0.0 })
        .expect("queue has room");
    engine.drain_commands(&mut consumer);
    let mut after = [0.0f32; 512];
    engine.process_block(&mut after);
    assert_ne!(
        before.to_vec(),
        after.to_vec(),
        "SetLocalCouplingGain had no audible effect on a ringing voice"
    );
}

#[test]
fn set_string_damping_reaches_an_already_ringing_voice() {
    let mut engine = engine();
    let (mut producer, mut consumer) = ring_buffer();
    producer
        .push(Command::NoteOn {
            midi: 69,
            velocity: 1.0,
        })
        .expect("queue has room");
    engine.drain_commands(&mut consumer);

    let mut before = [0.0f32; 512];
    engine.process_block(&mut before);

    producer
        .push(Command::SetStringDamping {
            midi: 69,
            string_index: 0,
            damping: 0.99,
        })
        .expect("queue has room");
    engine.drain_commands(&mut consumer);
    let mut after = [0.0f32; 512];
    engine.process_block(&mut after);
    assert_ne!(
        before.to_vec(),
        after.to_vec(),
        "SetStringDamping had no audible effect"
    );
}

#[test]
fn set_string_sustain_reaches_an_already_ringing_voice() {
    let mut engine = engine();
    let (mut producer, mut consumer) = ring_buffer();
    producer
        .push(Command::NoteOn {
            midi: 69,
            velocity: 1.0,
        })
        .expect("queue has room");
    engine.drain_commands(&mut consumer);

    let mut before = [0.0f32; 512];
    engine.process_block(&mut before);

    producer
        .push(Command::SetStringSustain {
            midi: 69,
            string_index: 0,
            sustain: 0.01,
        })
        .expect("queue has room");
    engine.drain_commands(&mut consumer);
    let mut after = [0.0f32; 512];
    engine.process_block(&mut after);
    assert_ne!(
        before.to_vec(),
        after.to_vec(),
        "SetStringSustain had no audible effect"
    );
}

#[test]
fn set_string_inharmonicity_reaches_an_already_ringing_voice() {
    let mut engine = engine();
    let (mut producer, mut consumer) = ring_buffer();
    producer
        .push(Command::NoteOn {
            midi: 69,
            velocity: 1.0,
        })
        .expect("queue has room");
    engine.drain_commands(&mut consumer);

    let mut before = [0.0f32; 512];
    engine.process_block(&mut before);

    producer
        .push(Command::SetStringInharmonicity {
            midi: 69,
            string_index: 0,
            inharmonicity: 0.05,
        })
        .expect("queue has room");
    engine.drain_commands(&mut consumer);
    let mut after = [0.0f32; 512];
    engine.process_block(&mut after);
    assert_ne!(
        before.to_vec(),
        after.to_vec(),
        "SetStringInharmonicity had no audible effect"
    );
}

#[test]
fn set_string_detune_reaches_an_already_ringing_voice() {
    let mut engine = engine();
    let (mut producer, mut consumer) = ring_buffer();
    producer
        .push(Command::NoteOn {
            midi: 69,
            velocity: 1.0,
        })
        .expect("queue has room");
    engine.drain_commands(&mut consumer);

    let mut before = [0.0f32; 512];
    engine.process_block(&mut before);

    producer
        .push(Command::SetStringDetune {
            midi: 69,
            string_index: 0,
            cents: 40.0,
        })
        .expect("queue has room");
    engine.drain_commands(&mut consumer);
    let mut after = [0.0f32; 512];
    engine.process_block(&mut after);
    assert_ne!(
        before.to_vec(),
        after.to_vec(),
        "SetStringDetune had no audible effect"
    );
}

#[test]
fn set_string_seed_changes_the_next_strike() {
    let mut baseline = engine();
    let mut reseeded = engine();
    let (mut producer, mut consumer) = ring_buffer();

    producer
        .push(Command::SetStringSeed {
            midi: 69,
            string_index: 0,
            seed: 0xDEAD_BEEF,
        })
        .expect("queue has room");
    reseeded.drain_commands(&mut consumer);

    for engine in [&mut baseline, &mut reseeded] {
        producer
            .push(Command::NoteOn {
                midi: 69,
                velocity: 1.0,
            })
            .expect("queue has room");
        engine.drain_commands(&mut consumer);
    }

    let mut baseline_out = [0.0f32; 512];
    let mut reseeded_out = [0.0f32; 512];
    baseline.process_block(&mut baseline_out);
    reseeded.process_block(&mut reseeded_out);
    assert_ne!(
        baseline_out.to_vec(),
        reseeded_out.to_vec(),
        "SetStringSeed had no effect on the next strike"
    );
}

#[test]
fn set_string_hammer_changes_the_next_strike() {
    let mut baseline = engine();
    let mut retuned = engine();
    let (mut producer, mut consumer) = ring_buffer();

    producer
        .push(Command::SetStringHammer {
            midi: 69,
            string_index: 0,
            hammer: piano_core::hammer::HammerConfig {
                contact_exponent: 6.0,
                stiffness: 1.0e8,
                mass: 20.0,
            },
        })
        .expect("queue has room");
    retuned.drain_commands(&mut consumer);

    for engine in [&mut baseline, &mut retuned] {
        producer
            .push(Command::NoteOn {
                midi: 69,
                velocity: 1.0,
            })
            .expect("queue has room");
        engine.drain_commands(&mut consumer);
    }

    let mut baseline_out = [0.0f32; 512];
    let mut retuned_out = [0.0f32; 512];
    baseline.process_block(&mut baseline_out);
    retuned.process_block(&mut retuned_out);
    assert_ne!(
        baseline_out.to_vec(),
        retuned_out.to_vec(),
        "SetStringHammer had no effect on the next strike"
    );
}

#[test]
fn per_string_commands_with_an_unrecognised_midi_note_never_panic() {
    let mut engine = engine();
    let (mut producer, mut consumer) = ring_buffer();
    producer
        .push(Command::SetStringDamping {
            midi: 255,
            string_index: 0,
            damping: 0.5,
        })
        .expect("queue has room");
    producer
        .push(Command::SetStringDetune {
            midi: 69,
            string_index: 250,
            cents: 10.0,
        })
        .expect("queue has room");
    engine.drain_commands(&mut consumer);
    let mut buffer = [0.0f32; 32];
    engine.process_block(&mut buffer);
}

#[test]
fn set_global_coupling_gain_out_of_range_never_panics() {
    let mut engine = engine();
    let (mut producer, mut consumer) = ring_buffer();
    producer
        .push(Command::SetGlobalCouplingGain { gain: f32::NAN })
        .expect("queue has room");
    producer
        .push(Command::SetLocalCouplingGain { gain: 50.0 })
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

/// M7's own callback-timing harness (`docs/PERFORMANCE.md`, "Metrics we
/// track separately": "Worst-case callback time (p99.9, not p50) — one
/// late buffer is an audible click; averages hide it"). Unlike
/// `callback_time_at_full_88_voice_polyphony_clears_the_deadline` above,
/// which only ever reports one aggregate `elapsed / count` mean, this
/// records *every* block's own duration into a
/// [`crate::timing::CallbackTimer`] — the same lock-free histogram
/// `piano_audio::stream` already uses on the real audio thread — and reads
/// back p50 through p99.9 plus the true max. A real, repeated-sampling
/// distribution, not a single run: `CALLBACK_TIMING_BLOCKS` (1 000)
/// independent measurements at full 88-key/222-string polyphony.
///
/// `#[ignore]`d for the same reason as the test above: wall-clock timing on
/// a shared CI runner is not a fair pass/fail gate. Run by hand with
/// `cargo test --release -p piano-audio -- --ignored --nocapture
/// callback_timing_distribution`.
#[test]
#[ignore = "wall-clock measurement, run manually with --ignored --nocapture"]
fn callback_timing_distribution_at_full_polyphony_reports_p99_9() {
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

    let timer = crate::timing::CallbackTimer::new();
    let mut buffer = vec![0.0f32; 128];
    for _ in 0..CALLBACK_TIMING_BLOCKS {
        let started = std::time::Instant::now();
        engine.process_block(&mut buffer);
        timer.record(started.elapsed());
    }

    let report = timer.report();
    println!(
        "88-voice callback timing over {} blocks: p50={} us, p95={} us, \
         p99={} us, p99.9={} us, max={} us (budget 2 670 us at 48 kHz)",
        report.callback_count,
        report.p50_micros,
        report.p95_micros,
        report.p99_micros,
        report.p99_9_micros,
        report.max_micros,
    );
    assert!(
        report.p99_9_micros < 2_670,
        "p99.9 {} us exceeded the 2.67 ms deadline",
        report.p99_9_micros
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
