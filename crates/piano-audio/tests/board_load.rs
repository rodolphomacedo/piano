//! The soundboard as a load on the strings (issues #90, #91): measured
//! through the full engine, with the load off and on.
//!
//! What the measurements found, at the default gain `1.0`: every key loses
//! energy every second with the load on, never gains — at most about 3 dB
//! after five seconds in the bass. A single string's partials decay up to
//! 2.2× faster where a board mode sits (A0's fundamental) and within a few
//! percent elsewhere: the frequency-dependent decay #91 asks for. At `4.0`
//! the effect roughly triples and the instrument stays stable.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use piano_audio::offline::OfflineEngine;
use piano_core::SampleRate;
use piano_core::bridge_load::{DEFAULT_BOARD_LOAD_GAIN, MAX_BOARD_LOAD_GAIN};
use piano_params::{PianoKey, Tuning};

const RATE: f32 = 48_000.0;
const WINDOW: usize = 4_096;
const HOP: usize = 1_024;

fn engine(load_gain: f32) -> OfflineEngine {
    let mut engine = OfflineEngine::new(SampleRate::new(RATE).unwrap(), Tuning::default());
    engine.set_board_load_gain(load_gain);
    engine
}

fn fundamental_hz(midi: u8) -> f32 {
    PianoKey::from_midi(midi)
        .unwrap()
        .frequency(Tuning::default())
        .hertz()
}

/// Hann-windowed single-frequency DFT magnitude.
fn magnitude_at(samples: &[f32], frequency_hz: f32) -> f32 {
    let omega = std::f32::consts::TAU * frequency_hz / RATE;
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

/// Decay rate of the partial at `frequency_hz`, in dB per second: the
/// least-squares slope of its level from its peak down to 30 dB below it.
fn decay_db_per_second(samples: &[f32], frequency_hz: f32) -> f32 {
    let levels: Vec<(f32, f32)> = (0..=samples.len().saturating_sub(WINDOW))
        .step_by(HOP)
        .map(|start| {
            let level = 20.0 * magnitude_at(&samples[start..start + WINDOW], frequency_hz).log10();
            (start as f32 / RATE, level)
        })
        .collect();
    let peak =
        levels.iter().enumerate().fold(
            (0, f32::MIN),
            |best, (i, (_, l))| if *l > best.1 { (i, *l) } else { best },
        );
    let fit: Vec<(f32, f32)> = levels[peak.0..]
        .iter()
        .copied()
        .take_while(|(_, level)| *level > peak.1 - 30.0)
        .collect();
    let n = fit.len() as f32;
    let (mean_t, mean_l) = fit
        .iter()
        .fold((0.0, 0.0), |(t, l), (ft, fl)| (t + ft / n, l + fl / n));
    let covariance: f32 = fit.iter().map(|(t, l)| (t - mean_t) * (l - mean_l)).sum();
    let variance: f32 = fit.iter().map(|(t, _)| (t - mean_t).powi(2)).sum();
    -covariance / variance
}

fn monochord_partial_decays(midi: u8, load_gain: f32, partials: usize) -> Vec<f32> {
    let mut engine =
        OfflineEngine::with_unison_strings(SampleRate::new(RATE).unwrap(), Tuning::default(), 1);
    engine.set_board_load_gain(load_gain);
    engine.note_on(midi, 0.8);
    let samples = engine.render(3.0);
    let f0 = fundamental_hz(midi);
    (1..=partials)
        .map(|partial| decay_db_per_second(&samples, f0 * partial as f32))
        .collect()
}

fn rms_by_second(engine: &mut OfflineEngine, seconds: usize) -> Vec<f32> {
    let samples = engine.render(seconds as f32);
    samples
        .chunks(RATE as usize)
        .map(|second| (second.iter().map(|s| s * s).sum::<f32>() / second.len() as f32).sqrt())
        .collect()
}

fn note_rms_by_second(midi: u8, load_gain: f32) -> Vec<f32> {
    let mut engine = engine(load_gain);
    engine.note_on(midi, 0.8);
    rms_by_second(&mut engine, 3)
}

#[test]
fn the_board_only_ever_takes_energy_from_a_note() {
    for midi in [33u8, 57, 93] {
        let off = note_rms_by_second(midi, 0.0);
        let on = note_rms_by_second(midi, MAX_BOARD_LOAD_GAIN);
        for (second, (on, off)) in on.iter().zip(&off).enumerate() {
            assert!(
                on <= &(off * 1.01),
                "MIDI {midi}, second {second}: {on} > {off}"
            );
        }
    }
}

#[test]
fn no_partial_of_a_single_string_decays_slower_under_load() {
    for midi in [45u8, 57, 81] {
        let off = monochord_partial_decays(midi, 0.0, 6);
        let on = monochord_partial_decays(midi, MAX_BOARD_LOAD_GAIN, 6);
        for (partial, (on, off)) in on.iter().zip(&off).enumerate() {
            assert!(
                on / off > 0.95,
                "MIDI {midi} partial {}: {on} dB/s loaded vs {off} free",
                partial + 1
            );
        }
    }
}

#[test]
fn the_load_is_strongest_where_the_board_resonates() {
    let off = monochord_partial_decays(33, 0.0, 8);
    let on = monochord_partial_decays(33, DEFAULT_BOARD_LOAD_GAIN, 8);
    let ratios: Vec<f32> = on.iter().zip(&off).map(|(on, off)| on / off).collect();
    assert!(ratios.iter().any(|r| *r > 1.4), "ratios {ratios:?}");
    assert!(ratios.iter().any(|r| *r < 1.05), "ratios {ratios:?}");
}

#[test]
fn a_pedalled_glissando_at_full_load_stays_bounded_and_dies_away() {
    let mut engine = engine(MAX_BOARD_LOAD_GAIN);
    engine.set_sustain_pedal(true);
    for midi in 21u8..=108 {
        engine.note_on(midi, 1.0);
        let block = engine.render(0.01);
        assert!(block.iter().all(|s| s.is_finite() && s.abs() <= 1.0));
    }
    let tail = rms_by_second(&mut engine, 4);
    assert!(tail.iter().all(|rms| rms.is_finite()));
    assert!(tail[3] < tail[0] * 0.5, "tail by second {tail:?}");
}
