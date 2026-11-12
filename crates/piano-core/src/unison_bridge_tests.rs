//! How a [`UnisonGroup`] behaves on the shared, cross-key bridge bus: what
//! the bus may cost a sounding note (nothing) and what it must still carry
//! (sympathetic resonance).

#![allow(clippy::float_cmp, clippy::unwrap_used, clippy::expect_used)]

use super::tests::{UNISON_BOUND, group_at};
use super::*;

#[test]
fn output_stays_bounded_with_global_bridge_coupling() {
    let mut group = group_at(220.0, 3);
    let mut bridge = BridgeBus::with_capacity(128);
    group.pluck(1.0);
    for block_index in 0..400 {
        bridge.begin_block();
        for sample_index in 0..128 {
            let sample = group.process_with_bridge(&mut bridge, sample_index);
            assert!(
                sample.is_finite(),
                "block {block_index} sample {sample_index} was not finite"
            );
            assert!(
                sample.abs() < UNISON_BOUND,
                "block {block_index} sample {sample_index} = {sample} escaped"
            );
        }
    }
}

/// One block's worth of samples, matching the block size every bridge
/// test here accumulates into.
const TEST_BLOCK: usize = 128;

/// Root-mean-square level of `samples` — what these decay tests compare,
/// since two runs that decay identically still differ sample-by-sample
/// once any coupling shifts their phase.
fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let mean_square = samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32;
    math::sqrt(mean_square)
}

/// Renders `blocks` blocks of `target` through a shared bridge, with
/// `others` sharing that same bridge, and returns `target`'s own output.
fn render_over_bridge(
    target: &mut UnisonGroup,
    others: &mut [&mut UnisonGroup],
    blocks: usize,
) -> Vec<f32> {
    let mut bridge = BridgeBus::with_capacity(TEST_BLOCK);
    let mut out = Vec::with_capacity(blocks * TEST_BLOCK);
    for _ in 0..blocks {
        bridge.begin_block();
        for index in 0..TEST_BLOCK {
            out.push(target.process_with_bridge(&mut bridge, index));
            for other in others.iter_mut() {
                let _ = other.process_with_bridge(&mut bridge, index);
            }
        }
    }
    out
}

/// The tail these decay tests measure: the last quarter of a render, by
/// which point any per-round-trip loss has compounded enough to dominate
/// the initial transient.
fn tail_level(samples: &[f32]) -> f32 {
    let start = samples.len() - samples.len() / 4;
    rms(samples.get(start..).unwrap_or(&[]))
}

/// A note sounding *alone* must decay the same whether or not it is
/// attached to a shared bridge. Regression test for the mixing law
/// `PluckedString::write_mixed_feedback` documents: while the global
/// path was a convex blend towards the bus readback, the bus was a whole
/// block old, so a lone note was blending towards a decorrelated copy of
/// itself and shedding `global_coupling_gain` of its own amplitude every
/// round trip. Measured at the time: 11x quieter by 0.5 s than the same
/// note with the bus disabled.
#[test]
fn a_lone_note_on_the_bridge_decays_like_one_with_no_bridge() {
    let mut on_bridge = group_at(261.63, 3);
    let mut off_bridge = group_at(261.63, 3);
    on_bridge.pluck(0.8);
    off_bridge.pluck(0.8);

    let coupled = render_over_bridge(&mut on_bridge, &mut [], 200);
    let solo: Vec<f32> = (0..coupled.len()).map(|_| off_bridge.process()).collect();

    let ratio = tail_level(&coupled) / tail_level(&solo);
    assert!(
        (0.9..=1.1).contains(&ratio),
        "the bridge cost a lone note {ratio:.4}x its tail level; it must cost nothing"
    );
}

/// A neighbouring key that is undamped but has already decayed to
/// inaudibility contributes ~0.0 into the bus every sample. Under the
/// old convex blend that halved the readback (`mean`, not sum), which
/// dragged a sounding note towards half its own amplitude every round
/// trip — measured at a further 40x loss. A silent string resting on a
/// bridge does not drain its neighbour.
#[test]
fn a_silent_receptive_neighbour_does_not_drain_a_sounding_note() {
    let mut alone = group_at(261.63, 3);
    let mut accompanied = group_at(261.63, 3);
    let mut bystander = group_at(440.0, 3);
    bystander.lift_damper(); // receptive, contributing, but never struck
    alone.pluck(0.8);
    accompanied.pluck(0.8);

    let without = render_over_bridge(&mut alone, &mut [], 200);
    let with = render_over_bridge(&mut accompanied, &mut [&mut bystander], 200);

    let ratio = tail_level(&with) / tail_level(&without);
    assert!(
        (0.9..=1.1).contains(&ratio),
        "a silent neighbour cost the note {ratio:.4}x its tail level"
    );
}

/// The audible symptom that started this: hold one key, strike another,
/// and the second one sounds dry. Polyphony must not shorten a note.
#[test]
fn a_ringing_neighbour_does_not_drain_a_freshly_struck_note() {
    let mut alone = group_at(261.63, 3);
    let mut accompanied = group_at(261.63, 3);
    let mut held = group_at(440.0, 3);
    held.pluck(0.8);
    let _ = render_over_bridge(&mut held, &mut [], 100); // let A4 ring first
    alone.pluck(0.8);
    accompanied.pluck(0.8);

    let without = render_over_bridge(&mut alone, &mut [], 200);
    let with = render_over_bridge(&mut accompanied, &mut [&mut held], 200);

    let ratio = tail_level(&with) / tail_level(&without);
    assert!(
        (0.9..=1.1).contains(&ratio),
        "a ringing neighbour cost the struck note {ratio:.4}x its tail level"
    );
}

/// The other half of the contract: making the bridge cost nothing must
/// not have made it *do* nothing. An undamped, never-struck key has to
/// ring up from a neighbour through the bus — sympathetic resonance is
/// what `PERF-008` bought this bus for.
#[test]
fn an_undamped_key_rings_up_sympathetically_from_a_struck_neighbour() {
    let mut struck = group_at(261.63, 3);
    let mut listener = group_at(130.81, 3); // C3, an octave below C4
    listener.lift_damper();
    struck.pluck(0.9);

    let mut bridge = BridgeBus::with_capacity(TEST_BLOCK);
    let mut heard = Vec::new();
    for _ in 0..200 {
        bridge.begin_block();
        for index in 0..TEST_BLOCK {
            let _ = struck.process_with_bridge(&mut bridge, index);
            heard.push(listener.process_with_bridge(&mut bridge, index));
        }
    }

    let onset = rms(heard.get(..TEST_BLOCK * 20).unwrap_or(&[]));
    let rung_up = rms(heard.get(TEST_BLOCK * 80..TEST_BLOCK * 100).unwrap_or(&[]));
    assert!(rung_up > 0.0, "the listening key never received anything");
    assert!(
        rung_up > onset,
        "the listening key did not ring up: onset {onset:e} -> {rung_up:e}"
    );
}

/// Zeroing the gain must silence that resonance completely — proof the
/// energy above travelled through the coupling path under test and not
/// through some other route.
#[test]
fn zeroing_the_global_gain_removes_sympathetic_resonance_entirely() {
    let mut struck = group_at(261.63, 3);
    let mut listener = group_at(130.81, 3);
    listener.set_global_coupling_gain(0.0);
    listener.lift_damper();
    struck.pluck(0.9);

    let heard = render_over_bridge(&mut listener, &mut [&mut struck], 200);
    assert_eq!(rms(&heard), 0.0, "an uncoupled key still heard something");
}
