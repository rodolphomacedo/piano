//! Rendering the note a report measures: either one bare string, exactly as
//! the engine voices it, or the whole engine, as a listener hears it.

use piano_audio::DEFAULT_SOUNDBOARD_MIX_GAIN;
use piano_audio::offline::OfflineEngine;
use piano_audio::voicing::config_for_key;
use piano_core::string::PluckedString;
use piano_core::{SampleRate, Soundboard};
use piano_params::{PianoKey, Tuning};
use serde::Serialize;

use crate::AnalysisError;

/// What produces the samples a report measures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NoteSource {
    /// The full offline engine: unison, hammer, soundboard, duplex,
    /// phantom partials and action noise — what the instrument sounds like.
    Engine,
    /// One string voiced by `config_for_key`, nothing else — isolates the
    /// string model from everything mixed on top of it.
    String,
    /// [`NoteSource::String`] plus the soundboard at the engine's default
    /// mix, so the board's own contribution can be heard by difference.
    StringWithSoundboard,
}

/// One note to render and measure.
#[derive(Debug, Clone, Copy)]
pub struct NoteRequest {
    /// Which key.
    pub key: PianoKey,
    /// The tuning it is played in.
    pub tuning: Tuning,
    /// The rate it is rendered at.
    pub sample_rate: SampleRate,
    /// How hard it is struck, `0..=1`.
    pub velocity: f32,
    /// How long to render, in seconds.
    pub seconds: f32,
}

impl NoteRequest {
    /// How many samples `seconds` lasts.
    fn sample_count(self) -> usize {
        (self.sample_rate.hertz() * self.seconds.max(0.0)) as usize
    }
}

/// Renders `request` from `source`.
///
/// # Errors
///
/// [`AnalysisError::Untunable`] if the key cannot be built as a string at
/// this sample rate.
pub fn render_note(request: NoteRequest, source: NoteSource) -> Result<Vec<f32>, AnalysisError> {
    match source {
        NoteSource::Engine => Ok(render_engine(request)),
        NoteSource::String => render_string(request, 0.0),
        NoteSource::StringWithSoundboard => render_string(request, DEFAULT_SOUNDBOARD_MIX_GAIN),
    }
}

fn render_engine(request: NoteRequest) -> Vec<f32> {
    let mut engine = OfflineEngine::new(request.sample_rate, request.tuning);
    engine.note_on(request.key.midi_number(), request.velocity);
    engine.render(request.seconds)
}

/// One string with the soundboard mixed in at `soundboard_mix`, the way
/// `Engine::process_stereo_chunk` mixes its own; `0` leaves it dry.
fn render_string(request: NoteRequest, soundboard_mix: f32) -> Result<Vec<f32>, AnalysisError> {
    let config = config_for_key(request.key, request.tuning, request.sample_rate);
    let mut string = PluckedString::new(config, request.sample_rate).map_err(|source| {
        AnalysisError::Untunable {
            key: request.key,
            source,
        }
    })?;
    let mut soundboard = Soundboard::new(request.sample_rate);
    string.pluck(request.velocity);
    Ok((0..request.sample_count())
        .map(|_| {
            let dry = string.process();
            dry + soundboard_mix * soundboard.process(dry)
        })
        .collect())
}
