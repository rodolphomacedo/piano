//! Measured evidence for milestone M6's "two-stage decay" done-when
//! criterion (`docs/ROADMAP.md`): a struck note's envelope should show a
//! fast initial decay (unison strings beating and dephasing) followed by a
//! slower, roughly single-exponential tail (once they settle into a shared
//! decaying mode) — G. Weinreich, "Coupled Piano Strings" (JASA 62(6),
//! 1977). Measured with a time-domain peak-envelope follower (the
//! "envelope-follower" branch of the M6 task's "FFT/envelope-follower"
//! choice — a two-stage decay is a time-domain amplitude phenomenon, not a
//! spectral one, so a direct envelope measurement is the more direct tool
//! for it than an FFT would be) rather than asserted from the
//! implementation, the same discipline `m4_spectral.rs` established for
//! M4's two done-when criteria.
//!
//! # How this was actually found, not just how it reads now
//!
//! The methodology below (peak per fixed-length block, block-to-block
//! ratio) is exactly how this effect was first confirmed to be real during
//! development, before this file existed: a throwaway debug print over 100
//! blocks showed the ratio collapsing hard on the very first block (a
//! factor of ~15), then climbing back and settling to a constant ~0.838
//! per 0.1 s block within about two seconds — a textbook two-stage curve.
//! A different, RMS-windowed version of this test, comparing a trichord
//! render against a monochord control in a short (sub-200 ms) early
//! window, was tried first and did not show a clean difference — the
//! comparison window was simply too short relative to the beat period a
//! few cents of detuning actually produces (order of hundreds of
//! milliseconds to a couple of seconds, not tens of milliseconds), so that
//! approach is not what ended up here. Two real implementation bugs were
//! also found and fixed by this measurement disagreeing with expectation,
//! not by inspection: an additive (rather than convex) coupling term that
//! diverged for near-lossless strings, and every unison string sharing one
//! excitation noise seed, which suppressed the very beating this test
//! looks for. See `piano_core::unison` and `piano_core::bridge`'s module
//! docs for both.

#![allow(clippy::indexing_slicing, clippy::unwrap_used, clippy::expect_used)]

use piano_core::SampleRate;
use piano_params::{PianoKey, Tuning};
use piano_render::{RenderRequest, render_unison_note};

const SAMPLE_RATE_HZ: f32 = 48_000.0;

/// One measurement block's length, in samples — 0.1 s at 48 kHz, matching
/// the block size the effect was first confirmed with (see the module
/// docs).
const BLOCK_SAMPLES: usize = 4_800;

fn render(midi: u8, unison_count: usize, velocity: f32, seconds: f32) -> Vec<f32> {
    let request = RenderRequest {
        key: PianoKey::from_midi(midi).expect("valid MIDI note"),
        tuning: Tuning::default(),
        sample_rate: SampleRate::new(SAMPLE_RATE_HZ).expect("48 kHz is valid"),
        seconds,
        velocity,
    };
    render_unison_note(request, unison_count).expect("renders")
}

/// The peak absolute amplitude of each non-overlapping [`BLOCK_SAMPLES`]
/// window in `samples`.
fn peak_per_block(samples: &[f32]) -> Vec<f32> {
    samples
        .chunks(BLOCK_SAMPLES)
        .map(|chunk| {
            chunk
                .iter()
                .fold(0.0f32, |peak, sample| peak.max(sample.abs()))
        })
        .collect()
}

/// The average decay rate, in nepers per block, of `peaks` between block
/// indices `[start, end)` — the magnitude of the slope of `ln(peak)` over
/// that span, using the two endpoints directly (consistent with this
/// project's existing pragmatic measurement style, e.g. `m4_spectral.rs`'s
/// peak-picking, rather than a full least-squares fit). Peaks at or below
/// the noise floor are floored before taking the log, so an already-silent
/// tail cannot produce a `NaN`/`-inf` rate.
fn decay_rate_nepers_per_block(peaks: &[f32], start: usize, end: usize) -> f32 {
    const FLOOR: f32 = 1e-9;
    let first = peaks.get(start).copied().unwrap_or(FLOOR).max(FLOOR);
    let last = peaks.get(end).copied().unwrap_or(FLOOR).max(FLOOR);
    let blocks = end.saturating_sub(start).max(1) as f32;
    (first.ln() - last.ln()) / blocks
}

#[test]
fn a_trichord_note_decays_faster_at_first_than_once_it_has_settled() {
    // A4 is comfortably in the trichord (triple-strung) register per
    // `piano_audio::voicing`'s table, but this test drives
    // `render_unison_note` directly rather than going through that lookup,
    // so it stays a direct statement of what is being measured. 5 seconds
    // gives the settled tail (see the module docs' ~2 s to converge) room
    // to actually converge before the render ends.
    let peaks = peak_per_block(&render(69, 3, 0.9, 5.0));

    // Block 0 contains the hammer's own attack transient, which decays
    // fast for *any* struck string, coupled or not — not the effect under
    // test. "Early" starts one block later, at the beating stage itself;
    // "late" is deep in the settled tail the module docs' debug run showed
    // converging by ~2 s in.
    let early_rate = decay_rate_nepers_per_block(&peaks, 1, 4);
    let late_rate = decay_rate_nepers_per_block(&peaks, 30, 45);
    println!(
        "trichord A4: early decay rate {early_rate:.4} nepers/block, late (settled) rate {late_rate:.4} nepers/block"
    );

    assert!(
        early_rate > late_rate * 1.5,
        "early decay rate {early_rate} nepers/block was not measurably faster than the \
         settled rate {late_rate} nepers/block — the two-stage signature did not emerge"
    );
}

#[test]
fn a_trichord_note_settles_close_to_its_own_single_string_decay_rate() {
    // The other half of Weinreich's claim: once the beating dies out, the
    // coupled strings settle into a shared mode decaying close to a lone
    // string's own natural rate, not some unrelated rate the coupling
    // machinery invented. Compared here against the same note rendered as
    // a monochord (`unison_count = 1`) — a real single string, run through
    // the identical hammer/loop/dispersion model, with no coupling to
    // muddy the comparison.
    let trichord_peaks = peak_per_block(&render(69, 3, 0.9, 5.0));
    let mono_peaks = peak_per_block(&render(69, 1, 0.9, 5.0));

    let trichord_settled_rate = decay_rate_nepers_per_block(&trichord_peaks, 30, 45);
    let mono_rate = decay_rate_nepers_per_block(&mono_peaks, 30, 45);
    println!(
        "settled trichord rate {trichord_settled_rate:.4} nepers/block vs monochord rate {mono_rate:.4} nepers/block"
    );

    // "Close" rather than "equal": the settled trichord mode is not
    // required to be *identical* to the monochord's own rate (the coupling
    // does add a small amount of extra loss even once beating has died
    // down), only within the same order of magnitude, honestly reflecting
    // that this is a model of the phenomenon rather than a claim of exact
    // agreement.
    let ratio = trichord_settled_rate / mono_rate.max(1e-6);
    assert!(
        (0.5..2.0).contains(&ratio),
        "settled trichord decay rate {trichord_settled_rate} nepers/block is not close to the \
         monochord's own {mono_rate} nepers/block (ratio {ratio}) — the settled mode does not \
         resemble a single string's natural decay the way Weinreich's model predicts"
    );
}
