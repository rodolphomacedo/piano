//! The report `piano analyze` prints: every measurement for one note,
//! gathered into one serialisable value.

use piano_audio::voicing::config_for_key;
use piano_core::{SampleRate, Soundboard};
use serde::Serialize;

use crate::AnalysisError;
use crate::render::{NoteRequest, NoteSource, render_note};
use crate::spectrum::{WINDOW, magnitude_at, partial_hz, rms, seconds_to_fall, spectral_centroid};

/// How many partials the decay table follows. Above the 8th a real piano's
/// partials are buried in the noise floor for most of the note.
pub const DECAY_HARMONICS: u8 = 8;

/// How many partials the attack profile lists — enough to show the strike
/// position's notch near the 8th and what lies past it.
pub const PROFILE_HARMONICS: u8 = 16;

/// Partials above this fraction of the sample rate are skipped: too close
/// to Nyquist for a single-frequency DFT to read reliably.
const MAX_PARTIAL_FRACTION: f32 = 0.45;

/// How often the centroid and level are sampled over the note's life.
const OVER_TIME_STEP_SECONDS: f32 = 1.0;

/// How often the soundboard's own ring is sampled.
const RING_STEP_SECONDS: f32 = 0.5;

/// How long the soundboard's impulse response is rendered.
const RING_SECONDS: f32 = 3.0;

/// The lowest level a report prints, so silence reads as a number rather
/// than `-∞`.
const FLOOR_DB: f32 = -200.0;

/// How long one partial takes to fall 20 dB from its own peak.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct HarmonicDecay {
    /// Partial number, `1` for the fundamental.
    pub harmonic: u8,
    /// Where the partial sits, stretched by the string's inharmonicity.
    pub frequency_hz: f32,
    /// Seconds to fall 20 dB, or `None` if it had not within the render.
    pub seconds_to_fall_20_db: Option<f32>,
}

/// One partial's level in the attack window.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct HarmonicLevel {
    /// Partial number, `1` for the fundamental.
    pub harmonic: u8,
    /// Where the partial sits.
    pub frequency_hz: f32,
    /// Its level relative to the attack's strongest partial, in dB.
    pub db_below_strongest: f32,
}

/// The note's brightness and loudness at one moment.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct SpectrumAt {
    /// When, from the strike.
    pub seconds: f32,
    /// Spectral centroid of the window starting here.
    pub centroid_hz: f32,
    /// RMS level of that window, in dB full scale.
    pub rms_db: f32,
}

/// Every measurement for one note.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TimbreReport {
    /// The key's name, `A4`.
    pub note: String,
    /// The key's MIDI number.
    pub midi: u8,
    /// Its fundamental under the request's tuning.
    pub fundamental_hz: f32,
    /// The inharmonicity it is voiced with.
    pub inharmonicity: f32,
    /// What produced the samples.
    pub source: NoteSource,
    /// The strike velocity.
    pub velocity: f32,
    /// How long was rendered.
    pub seconds: f32,
    /// Per-partial decay: a real piano's upper partials die several times
    /// faster than its fundamental.
    pub harmonic_decay: Vec<HarmonicDecay>,
    /// Partial levels in the first analysis window.
    pub attack_profile: Vec<HarmonicLevel>,
    /// Centroid and level once a second: a real piano's centroid collapses
    /// as the note rings, and a flat one sounds metallic.
    pub over_time: Vec<SpectrumAt>,
}

/// The string facts the partial probes need.
#[derive(Debug, Clone, Copy)]
struct StringFacts {
    fundamental_hz: f32,
    inharmonicity: f32,
    sample_rate_hz: f32,
}

impl StringFacts {
    fn of(request: NoteRequest) -> Self {
        let config = config_for_key(request.key, request.tuning, request.sample_rate);
        Self {
            fundamental_hz: config.frequency.hertz(),
            inharmonicity: config.inharmonicity,
            sample_rate_hz: request.sample_rate.hertz(),
        }
    }

    /// Partials `1..=count` that sit low enough to measure, with where each
    /// sits.
    fn partials(self, count: u8) -> impl Iterator<Item = (u8, f32)> {
        (1..=count)
            .map(move |harmonic| {
                let hz = partial_hz(self.fundamental_hz, self.inharmonicity, harmonic);
                (harmonic, hz)
            })
            .filter(move |&(_, hz)| hz < MAX_PARTIAL_FRACTION * self.sample_rate_hz)
    }
}

/// Renders `request` from `source` and measures it.
///
/// # Errors
///
/// [`AnalysisError::Untunable`] if the key cannot be rendered at this rate.
pub fn analyze_note(
    request: NoteRequest,
    source: NoteSource,
) -> Result<TimbreReport, AnalysisError> {
    let samples = render_note(request, source)?;
    let facts = StringFacts::of(request);
    Ok(TimbreReport {
        note: request.key.name().to_string(),
        midi: request.key.midi_number(),
        fundamental_hz: facts.fundamental_hz,
        inharmonicity: facts.inharmonicity,
        source,
        velocity: request.velocity,
        seconds: request.seconds,
        harmonic_decay: harmonic_decay(&samples, facts),
        attack_profile: attack_profile(&samples, facts),
        over_time: spectrum_over_time(&samples, facts.sample_rate_hz, OVER_TIME_STEP_SECONDS),
    })
}

fn harmonic_decay(samples: &[f32], facts: StringFacts) -> Vec<HarmonicDecay> {
    facts
        .partials(DECAY_HARMONICS)
        .map(|(harmonic, frequency_hz)| HarmonicDecay {
            harmonic,
            frequency_hz,
            seconds_to_fall_20_db: seconds_to_fall(samples, frequency_hz, facts.sample_rate_hz),
        })
        .collect()
}

fn attack_profile(samples: &[f32], facts: StringFacts) -> Vec<HarmonicLevel> {
    let attack = samples.get(..WINDOW).unwrap_or(samples);
    let levels: Vec<(u8, f32, f32)> = facts
        .partials(PROFILE_HARMONICS)
        .map(|(harmonic, hz)| (harmonic, hz, magnitude_at(attack, hz, facts.sample_rate_hz)))
        .collect();
    let strongest = levels
        .iter()
        .map(|&(_, _, level)| level)
        .fold(0.0f32, f32::max);
    levels
        .into_iter()
        .map(|(harmonic, frequency_hz, level)| HarmonicLevel {
            harmonic,
            frequency_hz,
            db_below_strongest: decibels(level / strongest),
        })
        .collect()
}

/// Centroid and level of one [`WINDOW`] every `step_seconds`, for as long
/// as a whole window still fits.
fn spectrum_over_time(samples: &[f32], sample_rate_hz: f32, step_seconds: f32) -> Vec<SpectrumAt> {
    let step = ((sample_rate_hz * step_seconds) as usize).max(1);
    (0..)
        .map(|index: usize| index.saturating_mul(step))
        .map_while(|offset| {
            let window = samples.get(offset..offset.saturating_add(WINDOW))?;
            Some(SpectrumAt {
                seconds: offset as f32 / sample_rate_hz,
                centroid_hz: spectral_centroid(window, sample_rate_hz),
                rms_db: decibels(rms(window)),
            })
        })
        .collect()
}

/// `ratio` in decibels, never below [`FLOOR_DB`] — `NaN` and silence
/// included.
fn decibels(ratio: f32) -> f32 {
    let db = 20.0 * ratio.log10();
    if db.is_nan() {
        FLOOR_DB
    } else {
        db.max(FLOOR_DB)
    }
}

/// The soundboard on its own, struck by a unit impulse.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SoundboardRing {
    /// Largest absolute response to the impulse.
    pub peak: f32,
    /// Centroid and level every half second.
    pub over_time: Vec<SpectrumAt>,
}

/// Measures the soundboard's own impulse response at `sample_rate`.
#[must_use]
pub fn soundboard_ring(sample_rate: SampleRate) -> SoundboardRing {
    let mut soundboard = Soundboard::new(sample_rate);
    let rate = sample_rate.hertz();
    let count = (rate * RING_SECONDS) as usize;
    let samples: Vec<f32> = (0..count)
        .map(|index| soundboard.process(if index == 0 { 1.0 } else { 0.0 }))
        .collect();
    SoundboardRing {
        peak: samples
            .iter()
            .map(|sample| sample.abs())
            .fold(0.0f32, f32::max),
        over_time: spectrum_over_time(&samples, rate, RING_STEP_SECONDS),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::float_cmp)]

    use super::*;
    use piano_params::{PianoKey, Tuning};

    fn rate() -> SampleRate {
        SampleRate::new(48_000.0).expect("48 kHz is valid")
    }

    fn request(midi: u8, seconds: f32) -> NoteRequest {
        NoteRequest {
            key: PianoKey::from_midi(midi).expect("key is on the keyboard"),
            tuning: Tuning::default(),
            sample_rate: rate(),
            velocity: 0.8,
            seconds,
        }
    }

    #[test]
    fn an_a4_report_names_its_note_and_measures_every_section() {
        let report = analyze_note(request(69, 3.0), NoteSource::String).expect("A4 renders");
        assert_eq!(report.note, "A4");
        assert_eq!(report.harmonic_decay.len(), usize::from(DECAY_HARMONICS));
        assert_eq!(report.attack_profile.len(), usize::from(PROFILE_HARMONICS));
        assert_eq!(report.over_time.len(), 3);
        assert!(
            report
                .attack_profile
                .iter()
                .any(|level| level.db_below_strongest == 0.0)
        );
    }

    #[test]
    fn a_c8_report_skips_partials_too_close_to_nyquist() {
        let report = analyze_note(request(108, 1.0), NoteSource::String).expect("C8 renders");
        assert!(report.harmonic_decay.len() < usize::from(DECAY_HARMONICS));
    }

    #[test]
    fn a_render_shorter_than_a_window_still_reports() {
        let report = analyze_note(request(69, 0.01), NoteSource::Engine).expect("A4 renders");
        assert_eq!(report.over_time, Vec::new());
    }

    #[test]
    fn silence_reads_as_the_floor_not_minus_infinity() {
        assert_eq!(decibels(0.0), FLOOR_DB);
        assert_eq!(decibels(f32::NAN), FLOOR_DB);
    }

    #[test]
    fn the_soundboard_answers_an_impulse_and_then_dies_away() {
        let ring = soundboard_ring(rate());
        assert!(ring.peak > 0.0);
        let first = ring.over_time.first().map_or(FLOOR_DB, |at| at.rms_db);
        let last = ring.over_time.last().map_or(0.0, |at| at.rms_db);
        assert!(last < first - 20.0, "{first} dB -> {last} dB");
    }
}
