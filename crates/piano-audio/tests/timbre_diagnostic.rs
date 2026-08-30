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
//! Run it for the numbers, not for a pass/fail:
//!
//! ```sh
//! cargo test -p piano-audio --test timbre_diagnostic -- --nocapture
//! ```

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use piano_audio::voicing::config_for_key;
use piano_core::string::PluckedString;
use piano_core::{SampleRate, Soundboard, UnisonGroup};
use piano_params::{PianoKey, Tuning};
use rustfft::{FftPlanner, num_complex::Complex32};

const SAMPLE_RATE_HZ: f32 = 48_000.0;
/// One analysis window. Long enough to resolve a bass fundamental's
/// harmonics, short enough that a treble note's decay is sampled several
/// times over its life.
const WINDOW: usize = 8192;
/// How many harmonics to track. Above about the 16th, a real piano's
/// partials are already buried in the noise floor for most of the note.
const HARMONICS: usize = 16;

fn sample_rate() -> SampleRate {
    SampleRate::new(SAMPLE_RATE_HZ).expect("48 kHz is valid")
}

/// Renders `seconds` of `midi` through the same `config_for_key` voicing the
/// engine uses. `with_soundboard` mirrors `Engine::process_chunk`'s own
/// parallel soundboard mix, so the two can be compared directly and the
/// soundboard's own contribution to the timbre isolated.
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
                // `Engine::process_chunk`'s own `SOUNDBOARD_MIX_GAIN`.
                dry + 0.5 * soundboard.process(dry)
            } else {
                dry
            }
        })
        .collect()
}

/// Magnitude of `samples` at `frequency_hz`, by direct evaluation of the
/// DFT at that one frequency — no interpolation error from snapping a
/// harmonic to the nearest FFT bin, which matters here because piano
/// partials sit deliberately sharp of exact multiples.
fn magnitude_at(samples: &[f32], frequency_hz: f32) -> f32 {
    let omega = std::f32::consts::TAU * frequency_hz / SAMPLE_RATE_HZ;
    let (mut real, mut imag) = (0.0f32, 0.0f32);
    for (index, &sample) in samples.iter().enumerate() {
        // Hann window: without it, a strong partial's energy smears across
        // neighbouring frequencies badly enough to swamp a weak upper one.
        let window =
            0.5 - 0.5 * (std::f32::consts::TAU * index as f32 / samples.len() as f32).cos();
        let phase = omega * index as f32;
        real += sample * window * phase.cos();
        imag -= sample * window * phase.sin();
    }
    (real * real + imag * imag).sqrt() / samples.len() as f32
}

/// Seconds for the partial at `frequency_hz` to fall 20 dB from its own
/// peak, or `None` if it never does within the render.
fn decay_to_minus_20db(samples: &[f32], frequency_hz: f32) -> Option<f32> {
    let windows: Vec<f32> = samples
        .chunks_exact(WINDOW)
        .map(|chunk| magnitude_at(chunk, frequency_hz))
        .collect();
    let (peak_index, peak) =
        windows
            .iter()
            .enumerate()
            .fold((0usize, 0.0f32), |best, (index, &magnitude)| {
                if magnitude > best.1 {
                    (index, magnitude)
                } else {
                    best
                }
            });
    if peak <= 0.0 {
        return None;
    }
    let target = peak * 0.1;
    windows[peak_index..]
        .iter()
        .position(|&m| m < target)
        .map(|offset| (offset * WINDOW) as f32 / SAMPLE_RATE_HZ)
}

fn spectral_centroid(samples: &[f32]) -> f32 {
    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(WINDOW);
    let mut buffer: Vec<Complex32> = samples
        .iter()
        .take(WINDOW)
        .enumerate()
        .map(|(index, &sample)| {
            let window = 0.5 - 0.5 * (std::f32::consts::TAU * index as f32 / WINDOW as f32).cos();
            Complex32::new(sample * window, 0.0)
        })
        .collect();
    buffer.resize(WINDOW, Complex32::new(0.0, 0.0));
    fft.process(&mut buffer);

    let (mut weighted, mut total) = (0.0f64, 0.0f64);
    for (bin, value) in buffer.iter().take(WINDOW / 2).enumerate() {
        let magnitude = f64::from(value.norm());
        weighted += magnitude * f64::from(bin as f32 * SAMPLE_RATE_HZ / WINDOW as f32);
        total += magnitude;
    }
    if total <= 0.0 {
        0.0
    } else {
        (weighted / total) as f32
    }
}

#[test]
fn report_per_harmonic_decay_across_the_keyboard() {
    println!("\n=== PER-HARMONIC DECAY (seconds to -20 dB from that partial's own peak) ===");
    println!("A real piano: H8 should die several times faster than H1.\n");

    for (name, midi, seconds) in [
        ("A0", 21u8, 20.0f32),
        ("A2", 45, 12.0),
        ("A4", 69, 8.0),
        ("A5", 81, 6.0),
    ] {
        let samples = render(midi, seconds, false);
        let fundamental = PianoKey::from_midi(midi)
            .expect("key")
            .frequency(Tuning::default())
            .hertz();
        print!("{name} (f0={fundamental:.1} Hz)  ");
        for harmonic in 1..=8 {
            let frequency = fundamental * harmonic as f32;
            if frequency > SAMPLE_RATE_HZ * 0.45 {
                break;
            }
            match decay_to_minus_20db(&samples, frequency) {
                Some(decay) => print!("H{harmonic}={decay:.2}s "),
                None => print!("H{harmonic}=>{seconds:.0}s "),
            }
        }
        println!();
    }
}

/// Isolates whether the reported A5 collapse comes from unison combination
/// (3 detuned strings blended together) rather than the string/dispersion
/// layer `report_per_harmonic_decay_across_the_keyboard` already measures
/// as reasonable in isolation. Bridge-free (`UnisonGroup::process`), no
/// soundboard: exactly the local 3-string blend and nothing else.
#[test]
fn report_a5_unison_group_decay_in_isolation() {
    const STEP: usize = (SAMPLE_RATE_HZ * 0.02) as usize;

    let tuning = Tuning::default();
    for (name, midi) in [
        ("A4", 69u8),
        ("G5", 79),
        ("G#5", 80),
        ("A5", 81),
        ("A#5", 82),
        ("B5", 83),
        ("A6", 93),
        ("C8", 108),
    ] {
        let key = PianoKey::from_midi(midi).expect("key is real");
        let config = config_for_key(key, tuning, sample_rate());
        for (label, count) in [("1 string", 1), ("3 strings (trichord)", 3)] {
            println!("\n=== {name} UNISON GROUP, {label} — RMS per 20ms ===");
            let mut group = UnisonGroup::new(config, count, sample_rate()).expect("key is tunable");
            group.pluck(0.8);

            let mut buffer = [0.0f32; STEP];
            for _ in 0..25 {
                for sample in &mut buffer {
                    *sample = group.process();
                }
                let rms = (buffer.iter().map(|s| s * s).sum::<f32>() / buffer.len() as f32).sqrt();
                print!("{rms:.4} ");
            }
            println!();
        }
    }
}

#[test]
fn report_harmonic_amplitude_profile_at_the_attack() {
    println!("\n=== ATTACK HARMONIC PROFILE (dB relative to the strongest partial) ===");
    println!("A real piano struck at ~1/8 of its length has a deep notch at H8.\n");

    for (name, midi) in [("A2", 45u8), ("A4", 69)] {
        let samples = render(midi, 1.0, false);
        let fundamental = PianoKey::from_midi(midi)
            .expect("key")
            .frequency(Tuning::default())
            .hertz();
        let magnitudes: Vec<f32> = (1..=HARMONICS)
            .map(|harmonic| magnitude_at(&samples[..WINDOW], fundamental * harmonic as f32))
            .collect();
        let peak = magnitudes.iter().copied().fold(0.0f32, f32::max).max(1e-12);
        print!("{name}: ");
        for (index, magnitude) in magnitudes.iter().enumerate() {
            print!("H{}={:.0}dB ", index + 1, 20.0 * (magnitude / peak).log10());
        }
        println!();
    }
}

#[test]
fn report_spectral_centroid_over_the_life_of_a_note() {
    println!("\n=== SPECTRAL CENTROID OVER TIME (Hz) ===");
    println!("A real piano's centroid collapses as the note rings. Flat = metallic.\n");

    for (label, with_soundboard) in [("string only", false), ("+ soundboard", true)] {
        let samples = render(69, 6.0, with_soundboard);
        print!("A4 {label:>13}: ");
        for step in 0..6 {
            let offset = step * SAMPLE_RATE_HZ as usize;
            if offset + WINDOW > samples.len() {
                break;
            }
            print!("t={step}s:{:.0}Hz ", spectral_centroid(&samples[offset..]));
        }
        println!();
    }
}

#[test]
fn report_soundboard_ring_on_its_own() {
    println!("\n=== SOUNDBOARD IMPULSE RESPONSE ===");
    println!("Its own decay and centroid, driven by a single impulse.\n");
    let mut soundboard = Soundboard::new(sample_rate());
    let mut samples = vec![soundboard.process(1.0)];
    samples.extend((0..(SAMPLE_RATE_HZ as usize * 3)).map(|_| soundboard.process(0.0)));

    let peak = samples.iter().map(|s| s.abs()).fold(0.0f32, f32::max);
    println!("peak response to a unit impulse: {peak:.3}");
    for step in 0..6 {
        let offset = step * (SAMPLE_RATE_HZ as usize / 2);
        if offset + WINDOW > samples.len() {
            break;
        }
        let window = &samples[offset..offset + WINDOW];
        let rms = (window.iter().map(|s| s * s).sum::<f32>() / WINDOW as f32).sqrt();
        println!(
            "  t={:.1}s  rms={rms:.6}  centroid={:.0}Hz",
            step as f32 * 0.5,
            spectral_centroid(window)
        );
    }
}
