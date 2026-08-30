//! Tests for [`super`], split into its own file so `engine.rs` itself
//! stays under this project's 500-line file limit (see
//! `CONTRIBUTING.md`) — `#[path = "engine_tests.rs"] mod tests;` at the
//! bottom of `engine.rs` still compiles this as `engine::tests`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use std::time::Duration;

use rustfft::{FftPlanner, num_complex::Complex32};

use super::*;

const DIAGNOSTIC_SAMPLE_RATE_HZ: f32 = 48_000.0;
const DIAGNOSTIC_WINDOW: usize = 8192;

/// Direct DFT magnitude at one frequency — same technique
/// `tests/timbre_diagnostic.rs` uses, duplicated rather than shared because
/// that file is a separate binary this crate's private `Engine` is not
/// visible to.
fn magnitude_at(samples: &[f32], frequency_hz: f32) -> f32 {
    let omega = core::f32::consts::TAU * frequency_hz / DIAGNOSTIC_SAMPLE_RATE_HZ;
    let (mut real, mut imag) = (0.0f32, 0.0f32);
    for (index, &sample) in samples.iter().enumerate() {
        let window =
            0.5 - 0.5 * (core::f32::consts::TAU * index as f32 / samples.len() as f32).cos();
        let phase = omega * index as f32;
        real += sample * window * phase.cos();
        imag -= sample * window * phase.sin();
    }
    (real * real + imag * imag).sqrt() / samples.len() as f32
}

fn spectral_centroid(samples: &[f32]) -> f32 {
    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(DIAGNOSTIC_WINDOW);
    let mut buffer: Vec<Complex32> = samples
        .iter()
        .take(DIAGNOSTIC_WINDOW)
        .enumerate()
        .map(|(index, &sample)| {
            let window = 0.5
                - 0.5 * (core::f32::consts::TAU * index as f32 / DIAGNOSTIC_WINDOW as f32).cos();
            Complex32::new(sample * window, 0.0)
        })
        .collect();
    buffer.resize(DIAGNOSTIC_WINDOW, Complex32::new(0.0, 0.0));
    fft.process(&mut buffer);

    let (mut weighted, mut total) = (0.0f64, 0.0f64);
    for (bin, value) in buffer.iter().take(DIAGNOSTIC_WINDOW / 2).enumerate() {
        let magnitude = f64::from(value.norm());
        weighted += magnitude
            * f64::from(bin as f32 * DIAGNOSTIC_SAMPLE_RATE_HZ / DIAGNOSTIC_WINDOW as f32);
        total += magnitude;
    }
    if total <= 0.0 {
        0.0
    } else {
        (weighted / total) as f32
    }
}

/// Renders `seconds` of `midi` struck alone, through the real [`Engine`] —
/// unison, bridge bus, soundboard and output limiter all included, exactly
/// the path `make run-studio`/`AudioSession` takes. Every earlier timbre
/// measurement in this project rendered a bare [`piano_core::PluckedString`]
/// or a `PluckedString` + [`piano_core::Soundboard`] pair, never the whole
/// engine — this is deliberately the first one that does.
fn render_key_through_engine(midi: u8, seconds: f32) -> Vec<f32> {
    let mut engine = engine();
    press(&mut engine, midi);
    let sample_count = (DIAGNOSTIC_SAMPLE_RATE_HZ * seconds) as usize;
    let mut samples = vec![0.0f32; sample_count];
    for chunk in samples.chunks_mut(512) {
        engine.process_block(chunk);
    }
    samples
}

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

/// Evidence gathering for the report: "A5 sounds terrible, like hitting a
/// tin can with a muffled iron" — through the *real* engine (unison, bridge,
/// soundboard, limiter), not a bare string. Prints, not pass/fail; compares
/// A5 against A4, whose per-partial decay `docs/TIMBRE-PLAN.md`'s F1 already
/// measured as fixed, to see whether A5 is worse in a way A4 is not.
#[test]
fn report_a5_versus_a4_per_harmonic_decay_through_the_full_engine() {
    println!("\n=== A5 vs A4, PER-HARMONIC DECAY THROUGH THE FULL ENGINE ===");
    for (name, midi) in [("A4", 69u8), ("A5", 81)] {
        let samples = render_key_through_engine(midi, 4.0);
        let fundamental = PianoKey::from_midi(midi)
            .expect("key")
            .frequency(Tuning::default())
            .hertz();
        print!("{name} (f0={fundamental:.1} Hz)  ");
        let windows: Vec<Vec<f32>> = samples
            .chunks(DIAGNOSTIC_WINDOW)
            .map(<[f32]>::to_vec)
            .collect();
        for harmonic in 1..=8 {
            let frequency = fundamental * harmonic as f32;
            if frequency > DIAGNOSTIC_SAMPLE_RATE_HZ * 0.45 {
                break;
            }
            let magnitudes: Vec<f32> = windows
                .iter()
                .map(|window| magnitude_at(window, frequency))
                .collect();
            let peak = magnitudes.iter().copied().fold(0.0f32, f32::max);
            if peak <= 0.0 {
                print!("H{harmonic}=silent ");
                continue;
            }
            let target = peak * 0.1;
            match magnitudes.iter().position(|&m| m < target) {
                Some(offset) => {
                    let seconds = (offset * DIAGNOSTIC_WINDOW) as f32 / DIAGNOSTIC_SAMPLE_RATE_HZ;
                    print!("H{harmonic}={seconds:.2}s ");
                }
                None => print!("H{harmonic}=>4s "),
            }
        }
        println!();
    }
}

/// The unison beat envelope: three strings a few cents apart should shimmer
/// slowly, not throb harshly. Prints the RMS of each 20 ms window over the
/// first second — a beat period short enough to read as "throbbing"/"tin
/// can" rather than "shimmering" would show up as fast, deep ripples here.
#[test]
fn report_a5_unison_beat_envelope_and_solved_voicing() {
    const STEP: usize = (DIAGNOSTIC_SAMPLE_RATE_HZ * 0.02) as usize;

    println!("\n=== A5 UNISON BEAT ENVELOPE (RMS per 20ms window, first 1s) ===");
    let samples = render_key_through_engine(81, 1.0);
    for window in samples.chunks(STEP) {
        let rms = (window.iter().map(|s| s * s).sum::<f32>() / window.len() as f32).sqrt();
        print!("{rms:.4} ");
    }
    println!();

    println!("\n=== A5 SOLO SOLVED VOICING (piano_audio::voicing) ===");
    let tuning = Tuning::default();
    let key = PianoKey::from_midi(81).expect("A5 is a real key");
    let sample_rate = SampleRate::new(DIAGNOSTIC_SAMPLE_RATE_HZ).expect("48kHz is valid");
    let voicing = crate::voicing::voicing_for_key(key, tuning, sample_rate);
    println!(
        "A5: pole={:.5} zero_mix={:.5} sustain={:.6} inharmonicity={:.6} unison_count={}",
        voicing.damping,
        voicing.zero_mix,
        voicing.sustain,
        voicing.inharmonicity,
        crate::voicing::unison_count_for_key(key)
    );
}

/// Spectral centroid over the life of the note — A4's already-fixed F1
/// curve keeps falling across several seconds (`docs/TIMBRE-PLAN.md`, F1).
/// A flat or erratic A5 centroid would point at something A5-specific,
/// rather than the general decay-shape defect F1 already closed.
#[test]
fn report_a5_versus_a4_spectral_centroid_through_the_full_engine() {
    println!("\n=== A5 vs A4, SPECTRAL CENTROID OVER TIME (Hz), FULL ENGINE ===");
    for (name, midi) in [("A4", 69u8), ("A5", 81)] {
        let samples = render_key_through_engine(midi, 4.0);
        print!("{name}: ");
        for step in 0..4 {
            let offset = step * DIAGNOSTIC_SAMPLE_RATE_HZ as usize;
            if offset + DIAGNOSTIC_WINDOW > samples.len() {
                break;
            }
            print!("t={step}s:{:.0}Hz ", spectral_centroid(&samples[offset..]));
        }
        println!();
    }
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

/// Renders `blocks` blocks of 512 samples and returns the RMS of the last
/// quarter of them — the tail, where any per-round-trip loss has compounded
/// enough to dominate the attack transient.
fn tail_rms(engine: &mut Engine, blocks: usize) -> f32 {
    let mut rendered = Vec::with_capacity(blocks * 512);
    for _ in 0..blocks {
        let mut buffer = [0.0f32; 512];
        engine.process_block(&mut buffer);
        rendered.extend_from_slice(&buffer);
    }
    let tail = rendered.split_off(rendered.len() - rendered.len() / 4);
    let mean_square = tail.iter().map(|s| s * s).sum::<f32>() / tail.len() as f32;
    mean_square.sqrt()
}

fn press(engine: &mut Engine, midi: u8) {
    let (mut producer, mut consumer) = ring_buffer();
    producer
        .push(Command::NoteOn {
            midi,
            velocity: 0.8,
        })
        .expect("queue has room");
    engine.drain_commands(&mut consumer);
}

/// The bug this coupling rework exists for, at the level a player hears it:
/// hold one key down, strike a second, and the second must sound as long as
/// it would have alone. It did not — the shared bridge's convex blend shed
/// part of every voice's own amplitude per round trip, in proportion to how
/// many voices were contributing, so the first note of a phrase rang and
/// everything after it sounded dry. See
/// `piano_core::string::PluckedString::write_mixed_feedback`.
#[test]
fn holding_one_key_does_not_shorten_the_next_note_struck() {
    let mut alone = engine();
    press(&mut alone, 60);
    let solo_tail = tail_rms(&mut alone, 200);

    let mut accompanied = engine();
    press(&mut accompanied, 69);
    let _ = tail_rms(&mut accompanied, 100); // let A4 ring first
    press(&mut accompanied, 60);
    let polyphonic_tail = tail_rms(&mut accompanied, 200);

    // The held A4 still sounds into the same buffer, so the mix can only be
    // *louder* than C4 alone. A tail quieter than C4's own means C4 itself
    // was drained.
    assert!(
        polyphonic_tail > solo_tail * 0.9,
        "C4's tail collapsed to {polyphonic_tail:e} against {solo_tail:e} played alone"
    );
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
