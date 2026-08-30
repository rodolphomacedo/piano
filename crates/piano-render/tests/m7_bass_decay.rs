//! Measured evidence for `PERF-013` (`docs/PERFORMANCE.md`): does `f32`
//! precision in a long bass decay produce audible quantisation in the tail,
//! or a decay that stalls instead of continuing towards zero?
//!
//! `f32`'s 24-bit mantissa gives roughly constant *relative* precision
//! (about 1 part in 2^23) regardless of a value's magnitude, which is why a
//! recursive IIR filter in floating point does not suffer the classic
//! fixed-point "limit cycle" stall — each round trip should keep shrinking
//! the signal by very close to the same *fraction*, all the way down to the
//! smallest representable `f32` magnitudes, rather than getting stuck at a
//! fixed absolute floor. This test renders A0 (`piano_core`'s lowest
//! string) for 60 seconds at this project's own real bass voicing (see
//! `bass_voicing` below) and checks that hypothesis against a real render,
//! per this entry's own "test that would catch it" — rather than trusting
//! the "floating point doesn't do that" argument on its own.
//!
//! # Why this duplicates `piano_audio::voicing`'s formula instead of
//! # depending on it
//!
//! `piano-render` does not depend on `piano-audio` (`docs/ARCHITECTURE.md`:
//! dependencies point one way, and `piano-audio` is the crate closer to the
//! platform, not the other way round). The three loop coefficients used
//! here are the ones `piano_audio::voicing::solve_loop_losses` actually
//! solves A0 to at 48 kHz, printed by that module's own
//! `report_the_solved_voicing_at_each_anchor` — copied rather than
//! imported, so this test exercises the same real-world parameters the
//! engine actually voices A0 with, not the crate's own generic
//! `StringConfig::new` default (whose `sustain` targets no particular decay
//! time at all).
//!
//! They are a snapshot, not a derivation: if the register anchors move, the
//! numbers below go stale and should be re-copied from that report. What
//! this test measures — whether `f32` can carry a 60-second decay without
//! stalling — does not depend on their exact values, only on their being a
//! real bass voicing rather than a default one.

#![allow(clippy::indexing_slicing, clippy::unwrap_used, clippy::expect_used)]

use piano_core::string::{PluckedString, StringConfig};
use piano_core::{Hz, SampleRate};

const SAMPLE_RATE_HZ: f32 = 48_000.0;
const A0_HZ: f32 = 27.5;
const RENDER_SECONDS: f32 = 60.0;

/// The loop-filter pole `solve_loop_losses` gives A0 at 48 kHz.
const BASS_POLE: f32 = 0.914_52;
/// The loop-filter zero mix it gives A0 at the same rate.
const BASS_ZERO_MIX: f32 = 0.448_18;
/// The broadband round-trip gain it gives A0 at the same rate.
const BASS_SUSTAIN: f32 = 0.990_982;
/// `piano_audio::voicing::BASS_INHARMONICITY`.
const BASS_INHARMONICITY: f32 = 0.000_1;

fn bass_voicing() -> StringConfig {
    let mut config = StringConfig::new(Hz::new(A0_HZ).expect("A0 is a valid frequency"));
    config.damping = BASS_POLE;
    config.sustain = BASS_SUSTAIN;
    config.loop_zero_mix = BASS_ZERO_MIX;
    config.inharmonicity = BASS_INHARMONICITY;
    config
}

/// Renders `seconds` of A0 at real bass voicing and returns the peak
/// magnitude of every 128-sample block — a coarse envelope, tracked
/// independently of [`piano_core::string::PluckedString::envelope`] itself
/// so this test is checking the actual rendered signal, not trusting the
/// same envelope follower that would also be affected by any precision bug.
fn bass_block_peaks(seconds: f32) -> Vec<f32> {
    const BLOCK: usize = 128;
    let rate = SampleRate::new(SAMPLE_RATE_HZ).expect("48 kHz is valid");
    let mut string = PluckedString::new(bass_voicing(), rate).expect("A0 is tunable at 48 kHz");
    string.pluck(1.0);

    let total_samples = (seconds * SAMPLE_RATE_HZ) as usize;
    let mut peaks = Vec::with_capacity(total_samples / BLOCK + 1);
    let mut buffer = [0.0f32; BLOCK];
    let mut rendered = 0usize;
    while rendered < total_samples {
        buffer.fill(0.0);
        string.process_block_add(&mut buffer);
        let peak = buffer
            .iter()
            .fold(0.0f32, |worst, sample| worst.max(sample.abs()));
        peaks.push(peak);
        rendered += BLOCK;
    }
    peaks
}

#[test]
fn a60_second_a0_render_is_finite_throughout() {
    let peaks = bass_block_peaks(RENDER_SECONDS);
    assert!(peaks.iter().all(|peak| peak.is_finite()));
}

#[test]
fn a60_second_a0_render_decays_monotonically_in_windows() {
    // A single struck string has no beating to produce a non-monotonic
    // envelope the way M6's unison groups do (`m6_spectral.rs`), so this
    // checks the coarser, more robust claim the loop-gain model actually
    // predicts: the peak magnitude of each one-second window is
    // non-increasing from window to window, once past the attack
    // transient's own first window. Comparing whole one-second windows
    // (not block-to-block, which follows the waveform's own oscillation
    // and is not remotely monotonic by itself) is what makes this
    // assertion meaningful rather than trivially false.
    let peaks = bass_block_peaks(RENDER_SECONDS);
    let blocks_per_second = (SAMPLE_RATE_HZ / 128.0) as usize;
    let window_peaks: Vec<f32> = peaks
        .chunks(blocks_per_second)
        .map(|window| window.iter().copied().fold(0.0f32, f32::max))
        .collect();

    // Tolerate a tiny amount of float noise (1e-6 relative) rather than
    // demanding bit-exact non-increase, which a healthy exponential decay
    // would already satisfy comfortably.
    let mut violations = 0usize;
    for pair in window_peaks.windows(2).skip(1) {
        if pair[1] > pair[0] * (1.0 + 1e-6) {
            violations += 1;
        }
    }
    assert_eq!(
        violations, 0,
        "found {violations} window(s) where the envelope rose instead of decaying: {window_peaks:?}"
    );
}

#[test]
fn a60_second_a0_render_does_not_stall_in_the_tail() {
    // The test PERF-013 itself specifies: does the decay keep decaying, or
    // does it get stuck at a small nonzero constant instead of continuing
    // towards zero? Compares the peak of the window ending at 30 s against
    // the window ending at 60 s: a healthy exponential decay at this
    // string's own sustain should show several more orders of magnitude of
    // decrease over that second 30 s span; a stalled filter would show the
    // two numbers sitting close together instead.
    let peaks = bass_block_peaks(RENDER_SECONDS);
    let blocks_per_second = (SAMPLE_RATE_HZ / 128.0) as usize;
    let window_peaks: Vec<f32> = peaks
        .chunks(blocks_per_second)
        .map(|window| window.iter().copied().fold(0.0f32, f32::max))
        .collect();

    let at_30s = window_peaks
        .get(29)
        .copied()
        .expect("60 s render has a 30 s window");
    let at_60s = window_peaks
        .last()
        .copied()
        .expect("60 s render has a final window");

    println!("A0 bass decay: peak at 30s = {at_30s:e}, peak at 60s = {at_60s:e}");
    assert!(at_60s.is_finite() && at_30s.is_finite());
    assert!(
        at_60s < at_30s * 0.5,
        "tail did not keep decaying: 30s peak {at_30s:e}, 60s peak {at_60s:e} (expected 60s peak below half of 30s peak)"
    );
}
