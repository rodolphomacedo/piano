//! Timbre measurements of rendered piano notes (issue #86).
//!
//! Per-partial decay, the attack's harmonic profile, the spectral centroid
//! over the life of a note and the soundboard's own ring: the numbers that
//! made the metallic-sound diagnosis possible. They used to live only inside
//! a test, reachable through `cargo test -- --nocapture`. Here they are a
//! library, so `piano analyze` and the diagnostic test share one
//! implementation, and the report serialises for anything that wants to
//! consume it — CI, or an MCP tool (#85).
//!
//! Everything here allocates and runs offline. None of it may be called
//! from an audio callback.

#![forbid(unsafe_code)]

mod render;
mod report;
pub mod spectrum;

pub use render::{NoteRequest, NoteSource, render_note};
pub use report::{
    DECAY_HARMONICS, HarmonicDecay, HarmonicLevel, PROFILE_HARMONICS, SoundboardRing, SpectrumAt,
    TimbreReport, analyze_note, soundboard_ring,
};

use piano_core::ParamError;
use piano_params::PianoKey;

/// Why a note could not be analysed.
#[derive(Debug, thiserror::Error)]
pub enum AnalysisError {
    /// The key's string could not be built at the requested sample rate.
    #[error("cannot build a string for {key:?} at this sample rate: {source}")]
    Untunable {
        /// The key asked for.
        key: PianoKey,
        /// What the string model rejected.
        source: ParamError,
    },
}
