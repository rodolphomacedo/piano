//! Realtime audio output for the piano synthesiser.
//!
//! `piano-core` knows nothing about threads, devices or wall-clock time; this
//! crate is where that boundary is crossed. It owns the `cpal` output
//! stream, the lock-free command queue that reaches it ([ADR-0005]), and the
//! platform-specific denormal control (`PERF-002`) — the one place in the
//! project with `unsafe`, narrowly scoped and documented at its use.
//!
//! [ADR-0005]: https://github.com/rodolphomacedo/piano/blob/main/docs/adr/0005-lock-free-spsc-command-queue.md
//!
//! # Realtime contract
//!
//! Everything inside the audio callback allocates nothing, locks nothing,
//! panics nowhere and has no unbounded loop. See
//! `docs/REALTIME-AUDIO-RULES.md`. `AudioSession::start` is the one
//! allocating, blocking call in this crate's public API — everything after
//! it is safe to call from any thread at any rate.

mod commands;
mod denormals;
mod engine;
mod error;
mod stream;
#[cfg(test)]
mod tests_no_allocation;
mod timing;

use std::sync::Arc;

use piano_core::SampleRate;
use piano_params::Tuning;
use rtrb::Producer;

pub use error::AudioError;
pub use timing::TimingReport;

use commands::Command;
use timing::CallbackTimer;

/// A live playback session: an open output stream plus the means to talk to
/// it.
///
/// Dropping this stops playback — the `cpal` stream inside is closed on
/// drop, like any other `cpal` stream.
pub struct AudioSession {
    // Never read directly: kept alive purely for its `Drop` impl, which
    // stops playback when the session is dropped.
    #[allow(dead_code)]
    stream: cpal::Stream,
    producer: Producer<Command>,
    timer: Arc<CallbackTimer>,
    sample_rate: SampleRate,
}

impl AudioSession {
    /// Opens the default output device and starts playback immediately.
    ///
    /// # Errors
    ///
    /// Returns [`AudioError`] if no output device exists, its configuration
    /// cannot be read, or the stream cannot be built or started.
    pub fn start(tuning: Tuning) -> Result<Self, AudioError> {
        let timer = Arc::new(CallbackTimer::new());
        let (stream, producer, sample_rate) = stream::start(tuning, Arc::clone(&timer))?;
        Ok(Self {
            stream,
            producer,
            timer,
            sample_rate,
        })
    }

    /// Queues a note strike.
    ///
    /// Returns `false` if the command queue was full and the note was
    /// dropped rather than blocking the audio thread — see [ADR-0005].
    ///
    /// [ADR-0005]: https://github.com/rodolphomacedo/piano/blob/main/docs/adr/0005-lock-free-spsc-command-queue.md
    pub fn note_on(&mut self, midi: u8, velocity: f32) -> bool {
        self.producer
            .push(Command::NoteOn { midi, velocity })
            .is_ok()
    }

    /// Queues silencing every ringing voice. Same drop-not-block behaviour as
    /// [`AudioSession::note_on`].
    pub fn all_notes_off(&mut self) -> bool {
        self.producer.push(Command::AllNotesOff).is_ok()
    }

    /// The sample rate the engine is tuned for: the output device's own
    /// rate, not necessarily 48 kHz.
    #[inline]
    #[must_use]
    pub fn sample_rate(&self) -> SampleRate {
        self.sample_rate
    }

    /// A snapshot of callback timing so far: p50 through p99.9 and the max,
    /// in microseconds. See issue #18.
    #[must_use]
    pub fn timing_report(&self) -> TimingReport {
        self.timer.report()
    }
}

impl core::fmt::Debug for AudioSession {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("AudioSession")
            .field("sample_rate", &self.sample_rate)
            .finish_non_exhaustive()
    }
}
