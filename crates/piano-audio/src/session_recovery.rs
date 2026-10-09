//! Surviving device changes (issue #20): headphones unplugged mid-note, a
//! new default output, a stream the platform killed.
//!
//! None of this touches the audio thread. `cpal`'s error callback only
//! raises a flag; the control thread calls
//! [`AudioSession::recover_if_needed`] from its own loop, and that call
//! notices the flag (or the default device's name changing, which some
//! platforms report with no stream error at all), builds a fresh stream and
//! engine at the new device's sample rate — every delay line retuned by
//! construction — and replays the session's recorded settings into it a
//! queue's worth at a time. The new engine starts silent, so no note can be
//! left stuck across the change.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use piano_core::SampleRate;
use piano_params::Tuning;

use crate::commands::Command;
use crate::settings_log::SettingsLog;
use crate::{AudioError, AudioSession, stream};

/// How often the default device's name is re-read. Asking the platform is
/// cheap but not free, and a control loop ticks every few milliseconds.
const DEVICE_CHECK_INTERVAL: Duration = Duration::from_secs(1);

/// What [`AudioSession::recover_if_needed`] found.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Recovery {
    /// The stream is healthy and on the default device; nothing was done.
    Healthy,
    /// The stream was rebuilt on the current default device. Every setting
    /// the session had sent is being replayed; notes that were sounding
    /// are gone.
    Rebuilt {
        /// The new device's sample rate, which the engine is now tuned for.
        sample_rate: SampleRate,
    },
}

/// The control-thread state a rebuild needs.
pub(crate) struct RecoveryState {
    tuning: Tuning,
    failed: Arc<AtomicBool>,
    device_name: Option<String>,
    last_device_check: Instant,
    pub(crate) settings: SettingsLog,
    replay: VecDeque<Command>,
}

impl RecoveryState {
    pub(crate) fn new(
        tuning: Tuning,
        failed: Arc<AtomicBool>,
        device_name: Option<String>,
    ) -> Self {
        Self {
            tuning,
            failed,
            device_name,
            last_device_check: Instant::now(),
            settings: SettingsLog::default(),
            replay: VecDeque::new(),
        }
    }

    /// `true` when the stream died or, at most once per
    /// [`DEVICE_CHECK_INTERVAL`], the default device turns out to have
    /// changed since the stream was opened.
    fn needs_rebuild(&mut self) -> bool {
        if self.failed.load(Ordering::Acquire) {
            return true;
        }
        if self.last_device_check.elapsed() < DEVICE_CHECK_INTERVAL {
            return false;
        }
        self.last_device_check = Instant::now();
        stream::default_output_device_name() != self.device_name
    }
}

impl AudioSession {
    /// Rebuilds the stream if the device went away or the default output
    /// changed, and keeps feeding a rebuilt engine its replayed settings.
    /// Call it from the control loop every tick; on the common, healthy
    /// path it reads one atomic flag and, once a second, the default
    /// device's name.
    ///
    /// # Errors
    ///
    /// Returns [`AudioError`] if a rebuild was needed but no device could be
    /// opened — the caller decides whether to retry on its next tick or give
    /// up. The session keeps its recorded settings either way.
    pub fn recover_if_needed(&mut self) -> Result<Recovery, AudioError> {
        self.feed_replay();
        if !self.recovery.needs_rebuild() {
            return Ok(Recovery::Healthy);
        }
        self.rebuild()?;
        Ok(Recovery::Rebuilt {
            sample_rate: self.sample_rate,
        })
    }

    fn rebuild(&mut self) -> Result<(), AudioError> {
        let failed = Arc::new(AtomicBool::new(false));
        let started = stream::start(
            self.recovery.tuning,
            Arc::clone(&self.timer),
            Arc::clone(&failed),
        )?;
        self.stream = started.stream;
        self.producer = started.producer;
        self.sample_rate = started.sample_rate;
        self.recovery.failed = failed;
        self.recovery.device_name = started.device_name;
        self.recovery.last_device_check = Instant::now();
        self.recovery.replay = self.recovery.settings.replay().into();
        self.feed_replay();
        Ok(())
    }

    /// Pushes as much of the pending replay as the command queue takes now;
    /// the rest waits for the next tick.
    fn feed_replay(&mut self) {
        while let Some(command) = self.recovery.replay.front().copied() {
            if self.producer.push(command).is_err() {
                return;
            }
            self.recovery.replay.pop_front();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(failed: bool) -> RecoveryState {
        RecoveryState::new(
            Tuning::default(),
            Arc::new(AtomicBool::new(failed)),
            Some("device".into()),
        )
    }

    #[test]
    fn a_stream_error_asks_for_a_rebuild_at_once() {
        assert!(state(true).needs_rebuild());
    }

    #[test]
    fn a_healthy_stream_does_not_query_the_device_again_within_the_interval() {
        assert!(!state(false).needs_rebuild());
    }
}
