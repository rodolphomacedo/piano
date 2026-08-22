//! Lock-free callback timing histogram — issue #18.
//!
//! Mean callback duration hides exactly the failure that matters: one late
//! buffer in a thousand is an audible click, and an average buries it
//! completely. This records every callback's duration into fixed atomic
//! buckets — nothing the audio thread rules forbid — so a percentile can be
//! read back from any other thread once the run is over.

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::Duration;

/// Width of one histogram bucket, in microseconds.
const BUCKET_WIDTH_MICROS: u64 = 50;

/// Bucket count, covering `0..=10 ms` before the last bucket catches overflow.
const BUCKET_COUNT: usize = 200;

/// Records callback durations without allocating, locking or blocking.
#[derive(Debug)]
pub(crate) struct CallbackTimer {
    buckets: [AtomicU32; BUCKET_COUNT],
    count: AtomicU64,
    max_micros: AtomicU64,
}

impl CallbackTimer {
    /// An empty timer, ready to record from the audio thread.
    #[must_use]
    pub(crate) fn new() -> Self {
        Self {
            buckets: [const { AtomicU32::new(0) }; BUCKET_COUNT],
            count: AtomicU64::new(0),
            max_micros: AtomicU64::new(0),
        }
    }

    /// Records one callback's duration.
    ///
    /// Safe to call from the audio thread: only atomic stores, no
    /// allocation, no lock, no unbounded loop.
    pub(crate) fn record(&self, elapsed: Duration) {
        let micros = u64::try_from(elapsed.as_micros()).unwrap_or(u64::MAX);
        let bucket = usize::try_from(micros / BUCKET_WIDTH_MICROS).unwrap_or(usize::MAX);
        let bucket = bucket.min(BUCKET_COUNT - 1);
        if let Some(slot) = self.buckets.get(bucket) {
            slot.fetch_add(1, Ordering::Relaxed);
        }
        self.count.fetch_add(1, Ordering::Relaxed);
        self.max_micros.fetch_max(micros, Ordering::Relaxed);
    }

    /// Reads back a percentile report. Intended for the control thread, after
    /// the stream has stopped or between callbacks — reading is `O(bucket
    /// count)` and not bounded tightly enough for the audio thread itself.
    #[must_use]
    pub(crate) fn report(&self) -> TimingReport {
        let total = self.count.load(Ordering::Relaxed);
        if total == 0 {
            return TimingReport::default();
        }
        TimingReport {
            p50_micros: self.percentile(total, 0.50),
            p95_micros: self.percentile(total, 0.95),
            p99_micros: self.percentile(total, 0.99),
            p99_9_micros: self.percentile(total, 0.999),
            max_micros: self.max_micros.load(Ordering::Relaxed),
            callback_count: total,
        }
    }

    /// Lower bound of the smallest bucket whose cumulative count reaches
    /// `fraction` of `total` recorded callbacks.
    ///
    /// Reports the lower bound, not the upper one: since every value in a
    /// bucket is `>=` that bound, this guarantees `percentile <= max_micros`
    /// for any distribution. Reporting the upper bound would let a p99.9
    /// figure exceed the true maximum whenever every recorded value fell in
    /// one bucket — a histogram lying about the worst case is worse than a
    /// slightly conservative one.
    fn percentile(&self, total: u64, fraction: f64) -> u64 {
        let target = ((total as f64) * fraction).ceil().max(1.0) as u64;
        let mut cumulative = 0u64;
        for (index, bucket) in self.buckets.iter().enumerate() {
            cumulative += u64::from(bucket.load(Ordering::Relaxed));
            if cumulative >= target {
                return index as u64 * BUCKET_WIDTH_MICROS;
            }
        }
        (BUCKET_COUNT as u64 - 1) * BUCKET_WIDTH_MICROS
    }
}

impl Default for CallbackTimer {
    fn default() -> Self {
        Self::new()
    }
}

/// A snapshot of callback timing distribution, in microseconds.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TimingReport {
    /// Median callback duration.
    pub p50_micros: u64,
    /// 95th percentile callback duration.
    pub p95_micros: u64,
    /// 99th percentile callback duration.
    pub p99_micros: u64,
    /// 99.9th percentile callback duration — the number that predicts
    /// audible clicks.
    pub p99_9_micros: u64,
    /// Slowest callback observed during the run.
    pub max_micros: u64,
    /// Total callbacks recorded.
    pub callback_count: u64,
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn an_empty_timer_reports_zero() {
        let timer = CallbackTimer::new();
        assert_eq!(timer.report(), TimingReport::default());
    }

    #[test]
    fn percentiles_track_recorded_durations() {
        let timer = CallbackTimer::new();
        for micros in 0..1_000u64 {
            timer.record(Duration::from_micros(micros));
        }
        let report = timer.report();
        assert!(report.p50_micros < report.p99_9_micros);
        assert!(report.max_micros >= 900);
        assert_eq!(report.callback_count, 1_000);
    }

    #[test]
    fn no_percentile_ever_exceeds_the_true_maximum() {
        // A regression guard for the bug this test would have caught: every
        // callback landing in the same bucket used to make p99.9 report the
        // bucket's *upper* edge, which can sit above the true max.
        let timer = CallbackTimer::new();
        for _ in 0..50 {
            timer.record(Duration::from_micros(22));
        }
        let report = timer.report();
        assert!(report.p50_micros <= report.max_micros);
        assert!(report.p99_9_micros <= report.max_micros);
    }

    #[test]
    fn durations_far_beyond_the_histogram_never_panic() {
        let timer = CallbackTimer::new();
        timer.record(Duration::from_secs(3_600));
        let report = timer.report();
        assert_eq!(report.callback_count, 1);
        assert!(report.max_micros > 0);
    }
}
