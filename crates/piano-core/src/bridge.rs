//! The shared bridge bus — the `O(N)` alternative to all-to-all string
//! coupling (`PERF-008` in `docs/PERFORMANCE.md`).
//!
//! Naive sympathetic-resonance coupling has every string read every other
//! string every sample: `O(N²)`, 57 600 multiply-adds/sample at the
//! documented `N = 240`, infeasible. G. Weinreich, "Coupled Piano Strings"
//! (JASA 62(6), 1977) explains why real pianos do not pay that cost either:
//! strings do not couple to each other directly at all. They each couple
//! only to the bridge — one shared mechanical connection whose own motion
//! is driven by the sum of every string's force on it, and which drives a
//! little motion back into every string in return. That is exactly a
//! shared-bus topology, not a mesh: every string writes into one running
//! total and reads back a value everyone else's writes already went into,
//! `O(N)` total.
//!
//! # Why the readback is an average, not a sum — a bug this project found
//! # by testing, not by inspection
//!
//! [`BridgeBus::add_and_read`] returns the *mean* of everything contributed
//! last block, not the raw total. An earlier version returned the raw sum.
//! Each individual string's own mixing step
//! ([`crate::string::PluckedString::write_mixed_feedback`]) scales whatever
//! it is handed by that string's own round-trip loss, which is bounded by
//! construction for any single string in
//! isolation — but a *raw sum* shared by up to 88 keys is not itself bounded
//! by any single string's own signal: as more voices become receptive
//! (e.g. every idle voice, the moment the sustain pedal goes down — see
//! `piano_audio::engine::Engine::set_sustain_pedal`), the shared total
//! keeps growing with the *number* of contributors, and every one of them
//! reads a larger and larger share back next block, contributing a larger
//! share the block after. `piano_audio::engine::tests::note_off_while_the_
//! pedal_is_down_does_not_release_until_pedal_up` caught this diverging to
//! infinity within a few blocks of holding the pedal — a real bug, found by
//! running the test, not predicted from the design. Dividing by how many
//! voices actually contributed turns the readback back into a **bounded
//! average**: whatever its size, it can never exceed the largest individual
//! contribution, so driving any one string with it stays bounded by that
//! string's own signal and everyone else's, exactly as the single-voice
//! case already was. Tracking "how many contributed" costs one counter
//! alongside the running total, not a second `O(N)` pass.
//!
//! Averaging keeps the bus bounded; it does **not** make the bus free to
//! read. Dividing by the contributor count means a voice reading back its
//! own contribution alongside `N-1` others gets `1/N` of what it put in, so
//! any mixing law that treats the readback as a *replacement* for the
//! string's own signal charges each voice for its neighbours' silence. That
//! is a real bug this bus's first consumer had, and why
//! [`crate::string::PluckedString::write_mixed_feedback`] now drives the
//! readback in **additively** — see its doc comment for the measurement and
//! the stability argument.
//!
//! # Why this bus is one block of latency, not sample-accurate
//!
//! A truly sample-accurate shared sum would need every one of (up to) 88
//! keys' unison groups to have already written this sample's contribution
//! *before* any of them can read the total back — impossible in a
//! voice-outer, sample-inner call from a single per-voice method, and
//! restructuring to a sample-outer, voice-inner loop would sacrifice the
//! cache locality `PERF-010` already established (`docs/PERFORMANCE.md`)
//! matters more than instruction count at this working-set size.
//!
//! [`BridgeBus`] instead swaps buffers once per block: every string reads
//! back the *previous* block's finished average while writing into the
//! *current* block's running one. At a typical 128-sample block and 48 kHz
//! that is about 2.7 ms of latency in the cross-key coupling path — for an
//! effect that builds and decays over hundreds of milliseconds to seconds
//! (sympathetic resonance ringing up after the sustain pedal goes down), a
//! few milliseconds of feedback delay is not a claim this project has
//! measured as inaudible, but it is smaller than the timescale the effect
//! itself operates on by two to three orders of magnitude, which is why it
//! was accepted as the honest engineering trade `PERF-008` asked for rather
//! than restructuring the whole engine's loop order to remove it. A note's
//! own unison strings, which need much tighter coupling to produce
//! Weinreich's two-stage decay correctly, are coupled separately and
//! sample-accurately instead — see `crate::unison`.

use alloc::{boxed::Box, vec};
use core::mem;

/// One running total plus how many contributions it has received, shared by
/// every string that couples through it.
///
/// Allocates four times, in [`BridgeBus::with_capacity`] — a sum and a
/// count buffer for the in-progress block, and the same pair for the
/// previous, finished one. Every other method is allocation-free,
/// lock-free and panic-free.
#[derive(Debug, Clone)]
pub struct BridgeBus {
    current_sum: Box<[f32]>,
    current_count: Box<[u32]>,
    previous_sum: Box<[f32]>,
    previous_count: Box<[u32]>,
}

impl BridgeBus {
    /// Allocates a bus able to accumulate a block of up to `capacity`
    /// samples. `capacity` is floored at 1 rather than rejected, so this
    /// never fails and never panics — a `0`-capacity bus is simply one
    /// that couples nothing, not a construction error.
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        let capacity = capacity.max(1);
        Self {
            current_sum: vec![0.0; capacity].into_boxed_slice(),
            current_count: vec![0; capacity].into_boxed_slice(),
            previous_sum: vec![0.0; capacity].into_boxed_slice(),
            previous_count: vec![0; capacity].into_boxed_slice(),
        }
    }

    /// How many samples of one block this bus can accumulate.
    #[inline]
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.current_sum.len()
    }

    /// Starts a new block: the block just finished becomes what every
    /// string reads back this block (see the module docs on latency), and
    /// the new block's running total and contributor count both start at
    /// zero.
    ///
    /// A caller processing more samples than [`BridgeBus::capacity`] in one
    /// logical block calls this once per `capacity`-sized chunk — see
    /// `piano_audio::engine::Engine::process_block`.
    pub fn begin_block(&mut self) {
        mem::swap(&mut self.current_sum, &mut self.previous_sum);
        mem::swap(&mut self.current_count, &mut self.previous_count);
        self.current_sum.fill(0.0);
        self.current_count.fill(0);
    }

    /// Adds `contribution` into the running total at `index` (counting it
    /// as one contributor), and returns the **mean** of everyone who
    /// contributed to `index` in the *previous* block — one call covers
    /// both halves of a string's bridge interaction (write this sample's
    /// contribution, read back the bounded average of what everyone
    /// contributed to the equivalent sample last block).
    ///
    /// `0.0` when nobody contributed to `index` last block, never a
    /// division by zero. Total: an `index` at or beyond
    /// [`BridgeBus::capacity`] is only reachable if a caller processes a
    /// chunk longer than the bus was sized for. Rather than panicking, the
    /// write is silently dropped and the read returns `0.0` — a documented
    /// degradation (no coupling for samples beyond the provisioned block
    /// size), not a crash.
    #[inline]
    pub fn add_and_read(&mut self, index: usize, contribution: f32) -> f32 {
        if let Some(sum) = self.current_sum.get_mut(index) {
            *sum += contribution;
        }
        if let Some(count) = self.current_count.get_mut(index) {
            *count += 1;
        }
        let sum = self.previous_sum.get(index).copied().unwrap_or(0.0);
        let count = self.previous_count.get(index).copied().unwrap_or(0);
        if count == 0 { 0.0 } else { sum / count as f32 }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp, clippy::unwrap_used, clippy::expect_used)]

    use proptest::prelude::*;

    use super::*;

    #[test]
    fn starts_silent() {
        let mut bus = BridgeBus::with_capacity(8);
        assert_eq!(bus.add_and_read(0, 0.0), 0.0);
    }

    #[test]
    fn a_contribution_is_read_back_only_after_the_next_block_begins() {
        let mut bus = BridgeBus::with_capacity(8);
        assert_eq!(
            bus.add_and_read(2, 5.0),
            0.0,
            "not visible within the same block yet"
        );
        bus.begin_block();
        assert_eq!(
            bus.add_and_read(2, 0.0),
            5.0,
            "visible in the following block"
        );
    }

    #[test]
    fn contributions_at_the_same_index_are_averaged_not_summed() {
        let mut bus = BridgeBus::with_capacity(8);
        bus.add_and_read(1, 3.0);
        bus.add_and_read(1, 5.0);
        bus.begin_block();
        assert_eq!(
            bus.add_and_read(1, 0.0),
            4.0,
            "mean of 3.0 and 5.0, not their sum"
        );
    }

    #[test]
    fn the_readback_never_exceeds_the_largest_single_contribution() {
        // The property that actually matters for stability
        // (`PluckedString::write_mixed_feedback`'s doc comment): however
        // many voices contribute, the average they produce cannot exceed
        // the largest of them — unlike a raw sum, which grows with the
        // contributor count.
        let mut bus = BridgeBus::with_capacity(4);
        for contributor in 0..87i16 {
            let _ = bus.add_and_read(0, f32::from(contributor % 3));
        }
        bus.begin_block();
        let readback = bus.add_and_read(0, 0.0);
        assert!(
            readback <= 2.0,
            "readback {readback} exceeded every contribution"
        );
    }

    #[test]
    fn a_new_block_does_not_carry_over_the_block_before_last() {
        let mut bus = BridgeBus::with_capacity(8);
        bus.add_and_read(0, 1.0);
        bus.begin_block();
        bus.begin_block();
        assert_eq!(bus.add_and_read(0, 0.0), 0.0, "two blocks old is gone");
    }

    #[test]
    fn zero_capacity_is_floored_rather_than_rejected() {
        let bus = BridgeBus::with_capacity(0);
        assert_eq!(bus.capacity(), 1);
    }

    #[test]
    fn an_out_of_range_index_never_panics() {
        let mut bus = BridgeBus::with_capacity(4);
        assert_eq!(bus.add_and_read(usize::MAX, 1.0), 0.0);
        bus.begin_block();
        assert_eq!(bus.add_and_read(usize::MAX, 0.0), 0.0);
    }

    proptest! {
        /// Whatever index and contribution a caller supplies, across
        /// however many blocks, the bus never panics and never produces a
        /// non-finite value.
        #[test]
        fn any_sequence_of_writes_and_blocks_is_total(
            operations in proptest::collection::vec(
                (0usize..16, -10.0f32..10.0, proptest::bool::ANY),
                0..200,
            ),
        ) {
            let mut bus = BridgeBus::with_capacity(8);
            for (index, contribution, new_block) in operations {
                if new_block {
                    bus.begin_block();
                }
                let value = bus.add_and_read(index, contribution);
                prop_assert!(value.is_finite());
            }
        }

        /// However many voices contribute to the same index in a block
        /// (up to a realistic 88-key upper bound) and whatever they
        /// contribute, the following block's readback never exceeds the
        /// largest single contribution — the boundedness property the
        /// module docs describe, checked for arbitrary inputs rather than
        /// only the fixed example
        /// `the_readback_never_exceeds_the_largest_single_contribution`
        /// covers.
        #[test]
        fn readback_is_always_bounded_by_the_largest_contribution(
            contributions in proptest::collection::vec(-10.0f32..10.0, 1..88),
        ) {
            let mut bus = BridgeBus::with_capacity(4);
            let largest = contributions.iter().copied().fold(f32::MIN, f32::max);
            for contribution in contributions {
                let _ = bus.add_and_read(0, contribution);
            }
            bus.begin_block();
            let readback = bus.add_and_read(0, 0.0);
            prop_assert!(readback <= largest + 1e-3, "readback {readback} exceeded largest contribution {largest}");
        }
    }
}
