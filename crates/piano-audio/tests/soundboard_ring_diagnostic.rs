//! Evidence gathering (not pass/fail) for the report: "the mid-register
//! strings sound bad — something metallic is knocking".
//!
//! Prints, rather than asserts. Run:
//!
//! ```sh
//! cargo test --release -p piano-audio --test soundboard_ring_diagnostic -- --nocapture --test-threads=1
//! ```

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use piano_audio::voicing::config_for_key;
use piano_core::soundboard::DEFAULT_MODES;
use piano_core::string::PluckedString;
use piano_core::{SampleRate, Soundboard};
use piano_params::{PianoKey, Tuning};

const SAMPLE_RATE_HZ: f32 = 48_000.0;
const WINDOW: usize = 8192;

fn sample_rate() -> SampleRate {
    SampleRate::new(SAMPLE_RATE_HZ).expect("48 kHz is valid")
}

fn magnitude_at(samples: &[f32], frequency_hz: f32) -> f32 {
    let omega = std::f32::consts::TAU * frequency_hz / SAMPLE_RATE_HZ;
    let (mut real, mut imag) = (0.0f32, 0.0f32);
    for (index, &sample) in samples.iter().enumerate() {
        let window =
            0.5 - 0.5 * (std::f32::consts::TAU * index as f32 / samples.len() as f32).cos();
        let phase = omega * index as f32;
        real += sample * window * phase.cos();
        imag -= sample * window * phase.sin();
    }
    (real * real + imag * imag).sqrt() / samples.len() as f32
}

fn render(midi: u8, seconds: f32, with_soundboard: bool) -> Vec<f32> {
    let key = PianoKey::from_midi(midi).expect("key is on the keyboard");
    let config = config_for_key(key, Tuning::default(), sample_rate());
    let mut string = PluckedString::new(config, sample_rate()).expect("key is tunable");
    let mut soundboard = Soundboard::new(sample_rate());
    string.pluck(0.8);
    let count = (SAMPLE_RATE_HZ * seconds) as usize;
    (0..count)
        .map(|_| {
            let dry = string.process();
            if with_soundboard {
                dry + 0.5 * soundboard.process(dry)
            } else {
                dry
            }
        })
        .collect()
}

/// How resonant each soundboard mode actually is. `Q = pi * f * tau` for a
/// two-pole resonator whose envelope decays to `1/e` in `tau` seconds — the
/// number that says whether a mode is a wooden body's broad resonance or a
/// tuned bell.
#[test]
fn report_how_resonant_each_soundboard_mode_is() {
    println!("\n=== SOUNDBOARD MODE Q ===");
    println!("A real piano soundboard mode: Q roughly 20-100 (heavily damped wood).");
    println!("A tubular bell / struck metal bar: Q in the thousands.\n");
    for (index, mode) in DEFAULT_MODES.iter().enumerate() {
        let q = std::f32::consts::PI * mode.frequency_hz * mode.decay_seconds;
        let bandwidth_hz = mode.frequency_hz / q;
        println!(
            "  mode {index}: f={:7.1} Hz  tau={:.2}s  Q={q:9.0}  -3dB bandwidth={bandwidth_hz:.3} Hz",
            mode.frequency_hz, mode.decay_seconds
        );
    }
}

/// `Resonator::new`'s `input_gain` comment claims `gain * (1 - radius)`
/// "normalises the resonator's peak steady-state gain to roughly `gain`,
/// independent of how long the mode rings for". This measures whether that
/// is true: drive one mode at a time with a unit sine at its own resonant
/// frequency and read the steady-state amplitude back out.
#[test]
fn report_the_actual_steady_state_gain_of_each_mode() {
    println!("\n=== MEASURED STEADY-STATE GAIN AT EACH MODE'S OWN FREQUENCY ===");
    println!("`Resonator::new` documents this as ~= the mode's `gain` field.\n");
    for (index, mode) in DEFAULT_MODES.iter().enumerate() {
        // One mode at a time: every other mode is silenced, so what comes
        // out is this resonator's response alone.
        let mut board = Soundboard::new(sample_rate());
        for (other, entry) in DEFAULT_MODES.iter().enumerate() {
            if other != index {
                let mut silent = *entry;
                silent.gain = 0.0;
                board.set_mode(other, silent);
            }
        }
        let settle = (SAMPLE_RATE_HZ * mode.decay_seconds * 6.0) as usize;
        let omega = std::f32::consts::TAU * mode.frequency_hz / SAMPLE_RATE_HZ;
        let mut output = Vec::with_capacity(WINDOW);
        for step in 0..(settle + WINDOW) {
            let driven = board.process((omega * step as f32).sin());
            if step >= settle {
                output.push(driven);
            }
        }
        let measured = magnitude_at(&output, mode.frequency_hz) * 4.0;
        println!(
            "  mode {index}: f={:7.1} Hz  declared gain={:.2}  measured gain={measured:7.2}  ({:+.0} dB off)",
            mode.frequency_hz,
            mode.gain,
            20.0 * (measured / mode.gain).log10()
        );
    }
}

/// The heart of the report. For a mid-register note, compare how long the
/// note's own harmonics ring against how long the soundboard's fixed mode
/// frequencies ring. A body resonance should fade with the note; a bell
/// rings on by itself at a pitch unrelated to what was played.
#[test]
fn report_what_still_rings_after_the_note_has_gone() {
    println!("\n=== WHAT IS STILL RINGING, PER SECOND (dB rel. to t=0 H1) ===");
    for (name, midi) in [("C4", 60u8), ("A4", 69), ("E4", 64)] {
        let fundamental = PianoKey::from_midi(midi)
            .expect("key")
            .frequency(Tuning::default())
            .hertz();
        let samples = render(midi, 6.0, true);
        println!("\n{name} (f0={fundamental:.1} Hz), string + soundboard:");

        let reference = magnitude_at(&samples[..WINDOW], fundamental).max(1e-12);
        print!("  {:>24}", "harmonic H1");
        for step in 0..6 {
            let offset = step * SAMPLE_RATE_HZ as usize;
            if offset + WINDOW > samples.len() {
                break;
            }
            let magnitude = magnitude_at(&samples[offset..offset + WINDOW], fundamental);
            print!(" t{step}={:6.1}", 20.0 * (magnitude / reference).log10());
        }
        println!();

        for mode in DEFAULT_MODES {
            // How far this mode sits from the nearest harmonic of the note:
            // a mode a few cents off a harmonic beats against it, which is
            // audibly different from one that simply adds a foreign pitch.
            let nearest_harmonic = (mode.frequency_hz / fundamental).round().max(1.0);
            let cents_off = 1200.0
                * (mode.frequency_hz / (fundamental * nearest_harmonic))
                    .log2()
                    .abs();
            print!("  mode {:7.1} Hz ({cents_off:5.0}c)", mode.frequency_hz);
            for step in 0..6 {
                let offset = step * SAMPLE_RATE_HZ as usize;
                if offset + WINDOW > samples.len() {
                    break;
                }
                let magnitude = magnitude_at(&samples[offset..offset + WINDOW], mode.frequency_hz);
                print!(" t{step}={:6.1}", 20.0 * (magnitude / reference).log10());
            }
            println!();
        }
    }
}

/// Direct A/B: the same note with and without the soundboard mixed in,
/// measured at each soundboard mode frequency. Whatever the soundboard adds
/// at a frequency the string itself is not producing is, by definition,
/// energy the instrument invented.
#[test]
fn report_energy_the_soundboard_invents() {
    println!("\n=== ENERGY ADDED BY THE SOUNDBOARD AT ITS OWN MODE FREQUENCIES ===");
    println!("dB of (string+soundboard) over (string alone), measured at t=2s.\n");
    for (name, midi) in [("C4", 60u8), ("A4", 69)] {
        let dry = render(midi, 4.0, false);
        let wet = render(midi, 4.0, true);
        let offset = 2 * SAMPLE_RATE_HZ as usize;
        print!("{name}: ");
        for mode in DEFAULT_MODES {
            let dry_magnitude =
                magnitude_at(&dry[offset..offset + WINDOW], mode.frequency_hz).max(1e-12);
            let wet_magnitude =
                magnitude_at(&wet[offset..offset + WINDOW], mode.frequency_hz).max(1e-12);
            print!(
                "{:.0}Hz:{:+.0}dB ",
                mode.frequency_hz,
                20.0 * (wet_magnitude / dry_magnitude).log10()
            );
        }
        println!();
    }
}

/// How much of the sound sits at frequencies that are not harmonics of the
/// note being played. A piano is close to harmonic (a little sharp, from
/// stiffness); a gong or a struck plate is not. This is the single number
/// closest to what "metallic" means.
#[test]
fn report_inharmonic_energy_fraction_over_time() {
    println!("\n=== FRACTION OF ENERGY AT SOUNDBOARD MODES, NOT AT THE NOTE'S PARTIALS ===");
    for (name, midi) in [("C4", 60u8), ("A4", 69), ("A2", 45)] {
        let fundamental = PianoKey::from_midi(midi)
            .expect("key")
            .frequency(Tuning::default())
            .hertz();
        for (label, with_soundboard) in [("string only ", false), ("+ soundboard", true)] {
            let samples = render(midi, 6.0, with_soundboard);
            print!("{name} {label}: ");
            for step in 0..6 {
                let offset = step * SAMPLE_RATE_HZ as usize;
                if offset + WINDOW > samples.len() {
                    break;
                }
                let window = &samples[offset..offset + WINDOW];
                let harmonic: f32 = (1..=16)
                    .map(|n| magnitude_at(window, fundamental * n as f32))
                    .sum();
                let modal: f32 = DEFAULT_MODES
                    .iter()
                    .map(|mode| magnitude_at(window, mode.frequency_hz))
                    .sum();
                let total = (harmonic + modal).max(1e-12);
                print!("t{step}={:4.0}% ", 100.0 * modal / total);
            }
            println!();
        }
    }
}
