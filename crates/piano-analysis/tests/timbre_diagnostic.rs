//! Measures *why* a rendered note sounds the way it does, rather than only
//! that it is finite and decays.
//!
//! The existing spectral tests (`piano-render/tests/m4_spectral.rs` and
//! friends) measure the *attack's* spectral centroid, the partials'
//! sharpness, and total amplitude decay. None catches the defect this file
//! exists for: a note whose harmonics all decay at the same rate. A real
//! piano string's upper partials die far faster than its fundamental — that
//! collapsing spectrum over the life of the note is most of what makes it
//! read as "piano" rather than as an organ, a bell or a comb filter. A model
//! that gets the fundamental's decay right and the harmonics' decay wrong
//! measures fine on every existing test and still sounds metallic.
//!
//! The measurements live in `piano_analysis`, shared with `piano analyze`
//! (issue #86); this file keeps the assertions and prints the same numbers.
//! Run it for them:
//!
//! ```sh
//! cargo test -p piano-analysis --test timbre_diagnostic -- --nocapture
//! ```
//!
//! or, for one note, `piano analyze --note A4 --source string`.

#![allow(clippy::expect_used, clippy::indexing_slicing)]

use piano_analysis::spectrum::{WINDOW, rms};
use piano_analysis::{NoteRequest, NoteSource, analyze_note, render_note, soundboard_ring};
use piano_audio::voicing::config_for_key;
use piano_core::{SampleRate, UnisonGroup};
use piano_params::{PianoKey, Tuning};

const SAMPLE_RATE_HZ: f32 = 48_000.0;

fn sample_rate() -> SampleRate {
    SampleRate::new(SAMPLE_RATE_HZ).expect("48 kHz is valid")
}

fn request(midi: u8, seconds: f32) -> NoteRequest {
    NoteRequest {
        key: PianoKey::from_midi(midi).expect("key is on the keyboard"),
        tuning: Tuning::default(),
        sample_rate: sample_rate(),
        velocity: 0.8,
        seconds,
    }
}

#[test]
fn report_per_harmonic_decay_across_the_keyboard() {
    println!("\n=== PER-HARMONIC DECAY (seconds to -20 dB from that partial's own peak) ===");
    println!("A real piano: H8 should die several times faster than H1.\n");

    for (midi, seconds) in [(21u8, 20.0f32), (45, 12.0), (69, 8.0), (81, 6.0)] {
        let report = analyze_note(request(midi, seconds), NoteSource::String).expect("renders");
        print!("{} (f0={:.1} Hz)  ", report.note, report.fundamental_hz);
        for decay in &report.harmonic_decay {
            match decay.seconds_to_fall_20_db {
                Some(fall) => print!("H{}={fall:.2}s ", decay.harmonic),
                None => print!("H{}=>{seconds:.0}s ", decay.harmonic),
            }
        }
        println!();
    }
}

/// Isolates whether a treble collapse comes from unison combination (three
/// detuned strings blended together) rather than the string/dispersion
/// layer `report_per_harmonic_decay_across_the_keyboard` already measures.
/// Bridge-free (`UnisonGroup::process`), no soundboard: exactly the local
/// three-string blend and nothing else.
#[test]
fn report_a5_unison_group_decay_in_isolation() {
    const STEP: usize = (SAMPLE_RATE_HZ * 0.02) as usize;

    let tuning = Tuning::default();
    for midi in [69u8, 79, 80, 81, 82, 83, 93, 108] {
        let key = PianoKey::from_midi(midi).expect("key is real");
        let config = config_for_key(key, tuning, sample_rate());
        for (label, count) in [("1 string", 1), ("3 strings (trichord)", 3)] {
            println!(
                "\n=== {} UNISON GROUP, {label} — RMS per 20ms ===",
                key.name()
            );
            let mut group = UnisonGroup::new(config, count, sample_rate()).expect("key is tunable");
            group.pluck(0.8);

            let mut buffer = [0.0f32; STEP];
            for _ in 0..25 {
                for sample in &mut buffer {
                    *sample = group.process();
                }
                print!("{:.4} ", rms(&buffer));
            }
            println!();
        }
    }
}

#[test]
fn report_harmonic_amplitude_profile_at_the_attack() {
    println!("\n=== ATTACK HARMONIC PROFILE (dB relative to the strongest partial) ===");
    println!("A real piano struck at ~1/8 of its length has a deep notch at H8.\n");

    for midi in [45u8, 69] {
        let report = analyze_note(request(midi, 1.0), NoteSource::String).expect("renders");
        print!("{}: ", report.note);
        for level in &report.attack_profile {
            print!("H{}={:.0}dB ", level.harmonic, level.db_below_strongest);
        }
        println!();
    }
}

#[test]
fn report_spectral_centroid_over_the_life_of_a_note() {
    println!("\n=== SPECTRAL CENTROID OVER TIME (Hz) ===");
    println!("A real piano's centroid collapses as the note rings. Flat = metallic.\n");

    for (label, source) in [
        ("string only", NoteSource::String),
        ("+ soundboard", NoteSource::StringWithSoundboard),
        ("whole engine", NoteSource::Engine),
    ] {
        let report = analyze_note(request(69, 6.0), source).expect("renders");
        print!("A4 {label:>13}: ");
        for at in &report.over_time {
            print!("t={:.0}s:{:.0}Hz ", at.seconds, at.centroid_hz);
        }
        println!();
    }
}

/// F3 gate (`docs/TIMBRE-PLAN.md`), engine side. The bank's own character
/// is gated in `piano_core::soundboard` (impulse-response centroid clear of
/// a thump, real energy above the old 1.4 kHz ceiling). This checks the
/// half that only shows up once the board is mixed into a note: adding it
/// must make an *audible* difference to the rendered sound, not the ~0%
/// the eight-mode table managed once its gain bug was fixed.
///
/// Measured as an energy change, not the centroid shift F3 originally
/// named: the D3 correction note in the plan already found a spectral
/// centroid to be a poor detector of a fast-decaying body sitting under a
/// broadband note, and at the mix level the instrument actually uses the
/// centroid moves only 2–3%. The board's contribution to the note's
/// radiated energy is the honest, direction-stable measurement.
#[test]
fn the_soundboard_makes_an_audible_difference_to_a_rendered_note() {
    let second = SAMPLE_RATE_HZ as usize;
    // Low mid-register (F2, F3), where a soundboard does most of its
    // audible work and the mode bank is densest.
    for midi in [41u8, 53] {
        let dry = render_note(request(midi, 2.0), NoteSource::String).expect("renders");
        let wet =
            render_note(request(midi, 2.0), NoteSource::StringWithSoundboard).expect("renders");
        let attack = |s: &[f32]| rms(&s[..WINDOW / 2]);
        let tail = |s: &[f32]| rms(&s[second..second * 2]);
        let attack_change = (attack(&wet) - attack(&dry)).abs() / attack(&dry);
        let tail_change = (tail(&wet) - tail(&dry)).abs() / tail(&dry);
        assert!(
            attack_change > 0.02 && tail_change > 0.02,
            "midi {midi}: adding the soundboard changed the attack by {:.1}% and the \
             sustained tail by {:.1}% — the body is still inaudible in a real note",
            attack_change * 100.0,
            tail_change * 100.0
        );
    }
}

#[test]
fn report_soundboard_ring_on_its_own() {
    println!("\n=== SOUNDBOARD IMPULSE RESPONSE ===");
    println!("Its own decay and centroid, driven by a single impulse.\n");
    let ring = soundboard_ring(sample_rate());
    println!("peak response to a unit impulse: {:.3}", ring.peak);
    for at in &ring.over_time {
        println!(
            "  t={:.1}s  rms={:.1}dB  centroid={:.0}Hz",
            at.seconds, at.rms_db, at.centroid_hz
        );
    }
}
