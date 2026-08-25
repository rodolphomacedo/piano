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
mod voicing;

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

    /// Queues releasing one key early — a MIDI note-off, or a
    /// computer-keyboard key-up on a terminal that reports one. While the
    /// sustain pedal ([`AudioSession::set_sustain_pedal`]) is held down,
    /// the voice is held rather than released immediately, same as a real
    /// piano. Same drop-not-block behaviour as [`AudioSession::note_on`].
    pub fn note_off(&mut self, midi: u8) -> bool {
        self.producer.push(Command::NoteOff { midi }).is_ok()
    }

    /// Queues a new CC64 sustain-*pedal* hold state.
    ///
    /// **Not** the same control as [`AudioSession::set_sustain`] — that
    /// adjusts [`piano_core::PluckedString`]'s broadband decay-rate voicing
    /// parameter. This is the physical hold pedal: while `down`,
    /// [`AudioSession::note_off`] does not release its voice; the moment
    /// the pedal comes back up, everything it was holding is released for
    /// real. Same drop-not-block behaviour as [`AudioSession::note_on`].
    pub fn set_sustain_pedal(&mut self, down: bool) -> bool {
        self.producer.push(Command::SustainPedal { down }).is_ok()
    }

    /// Queues a new damping (high-frequency loss) for every voice, applied
    /// live to voices already ringing as well as future strikes. `damping`
    /// is clamped into `[0, 1]` on the audio thread. Same drop-not-block
    /// behaviour as [`AudioSession::note_on`].
    pub fn set_damping(&mut self, damping: f32) -> bool {
        self.producer.push(Command::SetDamping { damping }).is_ok()
    }

    /// Queues a new sustain (broadband loop gain) for every voice, applied
    /// live to voices already ringing as well as future strikes. `sustain`
    /// is clamped into `[0, 1]` on the audio thread. Same drop-not-block
    /// behaviour as [`AudioSession::note_on`].
    pub fn set_sustain(&mut self, sustain: f32) -> bool {
        self.producer.push(Command::SetSustain { sustain }).is_ok()
    }

    /// Queues rebuilding one of the soundboard's resonant modes live. See
    /// [`piano_core::soundboard::Soundboard::set_mode`]. `index` outside
    /// `0..piano_core::soundboard::MODE_COUNT` is silently ignored on the
    /// audio thread. Same drop-not-block behaviour as
    /// [`AudioSession::note_on`].
    pub fn set_soundboard_mode(
        &mut self,
        index: usize,
        mode: piano_core::soundboard::SoundboardMode,
    ) -> bool {
        self.producer
            .push(Command::SetSoundboardMode { index, mode })
            .is_ok()
    }

    /// Queues a new local (within-group) unison coupling gain for every
    /// voice, live. See
    /// [`piano_core::UnisonGroup::set_local_coupling_gain`]. `gain` is
    /// clamped into `[0, 1]` on the audio thread. Same drop-not-block
    /// behaviour as [`AudioSession::note_on`].
    pub fn set_local_coupling_gain(&mut self, gain: f32) -> bool {
        self.producer
            .push(Command::SetLocalCouplingGain { gain })
            .is_ok()
    }

    /// Queues a new global (cross-key, [`piano_core::BridgeBus`]) coupling
    /// gain for every voice, live. See
    /// [`piano_core::UnisonGroup::set_global_coupling_gain`]. `gain` is
    /// clamped into `[0, 1]` on the audio thread. Same drop-not-block
    /// behaviour as [`AudioSession::note_on`].
    pub fn set_global_coupling_gain(&mut self, gain: f32) -> bool {
        self.producer
            .push(Command::SetGlobalCouplingGain { gain })
            .is_ok()
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
