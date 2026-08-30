//! A note's own unison strings, coupled through their shared bridge contact.
//!
//! Most piano notes are struck by more than one physical string tuned very
//! slightly apart. This is what produces the beating and the two-stage decay
//! real piano notes have that a single Karplus-Strong-style string never
//! can: G. Weinreich, "Coupled Piano Strings" (JASA 62(6), 1974-1484, 1977)
//! is the seminal measurement and model of exactly this — near-unison
//! strings coupled through the bridge's shared, finite admittance produce a
//! fast initial "pre-decay" (the strings beating against and dephasing from
//! each other) followed by a slower "aftersound" (once they settle into a
//! shared decaying mode). This module reuses [`PluckedString`] rather than
//! forking it — a unison group is a small number of independent strings,
//! not a new DSP primitive.
//!
//! # Two couplings, two honesty notes
//!
//! Weinreich's own model treats every string, whether same-note unison or a
//! different key entirely, as coupled through one shared bridge admittance.
//! This project splits that into two engineering tiers for feasibility:
//!
//! - **Local coupling** (this module): a note's own 1-3 strings couple
//!   sample-accurately, every sample, with no extra latency — cheap because
//!   a unison group already processes all its strings together in one call.
//! - **Global coupling** ([`crate::bridge::BridgeBus`]): cross-*key*
//!   sympathetic resonance, e.g. via the sustain pedal, is coupled with one
//!   block of latency (`PERF-008` in `docs/PERFORMANCE.md`) because summing
//!   all 88 keys' contributions every sample would need a full O(N) pass
//!   *before* any string could read back this sample's total, which is not
//!   representable in a per-voice, per-sample call. A few milliseconds of
//!   latency in a slowly-building resonance effect is inaudible; see the
//!   module docs on [`crate::bridge::BridgeBus`] for the full reasoning.
//!
//! # Register boundaries and unison counts
//!
//! A standard modern piano is single-strung (monochord) in the bass,
//! double-strung (bichord) through the tenor, and triple-strung (trichord)
//! for the rest of the treble — A. Reblitz, *Piano Servicing, Tuning, and
//! Rebuilding* (1993), describing the layout piano technicians work to;
//! consistent with the qualitative bass/tenor/treble description in
//! Fletcher & Rossing, *The Physics of Musical Instruments* (already cited
//! in `crate::dispersion` for this project's inharmonicity figures). Exact
//! break points vary by piano model and scale design, so the specific key
//! boundaries `piano_audio::voicing` chooses are this project's own
//! representative choice within that convention (12 single-strung keys, 18
//! double-strung, 58 triple-strung — chosen to land on the full 88-key
//! total), not a measurement from one specific instrument.

use crate::{
    bridge::BridgeBus,
    error::ParamError,
    hammer, math,
    string::{PluckedString, StringConfig},
    units::{Hz, SampleRate},
};

/// The most strings a real piano note is ever struck by.
pub const MAX_UNISON_STRINGS: usize = 3;

/// How strongly this note's own strings couple to *each other*, every
/// sample, through the local bridge contact — see the module docs.
///
/// The mixing this weights is a **convex combination** (`crate::string`'s
/// `PluckedString::write_mixed_feedback` doc comment explains why that,
/// not a raw sum, is what keeps a coupled group stable for any `sustain`)
/// so this constant can be chosen purely for how audible the beating is,
/// with no stability constraint of its own — any value in `[0, 1]` is
/// safe. Weinreich (1977) reports the coupled-string beating and
/// two-stage decay arise from a bridge admittance small relative to a
/// string's own characteristic impedance, i.e. weak coupling; this
/// project has no per-instrument admittance measurement to fit to, so
/// this specific figure is a reasoned choice within that "weak coupling"
/// order of magnitude, honestly labelled as such — the same pattern
/// `piano_audio::voicing`'s per-partial decay anchors already use.
const DEFAULT_LOCAL_COUPLING_GAIN: f32 = 0.15;

/// Detuning, in cents, for a 2-string (bichord) unison group, symmetric
/// around the nominal pitch.
///
/// The *existence* of a small mistuning between unison strings, and that it
/// is what produces beating, is Weinreich's (1977) central finding; the
/// specific cents figure is this project's own reasoned choice within the
/// "a few cents" order of magnitude that literature reports for real
/// instruments, not a measurement of one.
const DETUNE_CENTS_BICHORD: [f32; 2] = [-2.5, 2.5];

/// Detuning, in cents, for a 3-string (trichord) unison group — the middle
/// string left at the nominal pitch, the outer two spread symmetrically.
/// See [`DETUNE_CENTS_BICHORD`] for the honesty note on the exact figure.
const DETUNE_CENTS_TRICHORD: [f32; 3] = [-4.0, 0.0, 4.0];

/// A group of 1-3 [`PluckedString`]s struck together by one hammer,
/// coupled to each other through a local bridge contact.
///
/// Allocates once, in [`UnisonGroup::new`] (each constituent
/// [`PluckedString`] allocates its own delay line there). Every other
/// method is allocation-free, lock-free and panic-free.
#[derive(Debug, Clone)]
pub struct UnisonGroup {
    strings: [Option<PluckedString>; MAX_UNISON_STRINGS],
    count: usize,
    /// See [`UnisonGroup::set_local_coupling_gain`].
    local_coupling_gain: f32,
    /// See [`UnisonGroup::set_global_coupling_gain`].
    global_coupling_gain: f32,
    /// `base_config.frequency` as given to [`UnisonGroup::new`], before any
    /// per-string detuning — the reference [`UnisonGroup::set_string_detune`]
    /// computes an absolute retuned frequency from.
    base_frequency: Hz,
}

impl UnisonGroup {
    /// Builds a group of `unison_count` strings (clamped into
    /// `[1, MAX_UNISON_STRINGS]`) around `base_config`'s frequency, each
    /// detuned by a small fixed offset from [`DETUNE_CENTS_BICHORD`] or
    /// [`DETUNE_CENTS_TRICHORD`] (a single string gets no detuning — a real
    /// monochord bass note has nothing to beat against) and reseeded per
    /// string ([`reseed_for_string`]) — a real hammer strike does not
    /// deliver bit-for-bit identical excitation to each unison string
    /// (each has its own contact point and felt compliance), and leaving
    /// every string at `base_config`'s one seed made the strings' initial
    /// bursts near-identical, masking the beating this whole module exists
    /// to produce: caught by `crates/piano-render/tests/m6_spectral.rs`
    /// measuring almost no early/late decay-rate difference between a
    /// trichord and a monochord control before this fix, not by
    /// inspection.
    ///
    /// # Errors
    ///
    /// Returns [`ParamError::FrequencyOutOfRange`] if the detuned frequency
    /// cannot be represented at `sample_rate` — same condition
    /// [`PluckedString::new`] rejects, propagated from whichever
    /// constituent string hits it first.
    pub fn new(
        base_config: StringConfig,
        unison_count: usize,
        sample_rate: SampleRate,
    ) -> Result<Self, ParamError> {
        let count = unison_count.clamp(1, MAX_UNISON_STRINGS);
        let mut strings: [Option<PluckedString>; MAX_UNISON_STRINGS] = [None, None, None];
        for (index, slot) in strings.iter_mut().take(count).enumerate() {
            let mut config = base_config;
            config.frequency = detune(base_config.frequency, detune_cents(count, index));
            config.seed = reseed_for_string(base_config.seed, index);
            *slot = Some(PluckedString::new(config, sample_rate)?);
        }
        Ok(Self {
            strings,
            count,
            local_coupling_gain: DEFAULT_LOCAL_COUPLING_GAIN,
            global_coupling_gain: DEFAULT_GLOBAL_COUPLING_GAIN,
            base_frequency: base_config.frequency,
        })
    }

    /// Strikes every string in the group at the same `velocity` — a real
    /// hammer strikes all of one note's unison strings simultaneously.
    pub fn pluck(&mut self, velocity: f32) {
        for string in self.strings.iter_mut().flatten() {
            string.pluck(velocity);
        }
    }

    /// Releases every string in the group. See [`PluckedString::release`].
    pub fn release(&mut self) {
        for string in self.strings.iter_mut().flatten() {
            string.release();
        }
    }

    /// Lifts every string's damper without a fresh strike. See
    /// [`PluckedString::lift_damper`].
    pub fn lift_damper(&mut self) {
        for string in self.strings.iter_mut().flatten() {
            string.lift_damper();
        }
    }

    /// Applies a new damping to every string in the group. See
    /// [`PluckedString::set_damping`].
    pub fn set_damping(&mut self, damping: f32) {
        for string in self.strings.iter_mut().flatten() {
            string.set_damping(damping);
        }
    }

    /// Applies a new sustain to every string in the group. See
    /// [`PluckedString::set_sustain`].
    pub fn set_sustain(&mut self, sustain: f32) {
        for string in self.strings.iter_mut().flatten() {
            string.set_sustain(sustain);
        }
    }

    /// Adjusts how strongly this group's own strings couple to each other,
    /// live — see [`DEFAULT_LOCAL_COUPLING_GAIN`] for the physical meaning.
    /// Clamped into `[0, 1]`, the same range [`blend`] itself expects
    /// (`blend` clamps too, so this is a defensive, redundant-by-design
    /// second clamp — the same pattern [`crate::string::PluckedString::
    /// set_damping`] already uses for its own pole coefficient).
    pub fn set_local_coupling_gain(&mut self, gain: f32) {
        self.local_coupling_gain = math::clamp_or_low(gain, 0.0, 1.0);
    }

    /// Adjusts how strongly this group blends with the shared, cross-key
    /// bridge bus's readback, live — see [`DEFAULT_GLOBAL_COUPLING_GAIN`]
    /// for the physical meaning. Clamped the same way as
    /// [`UnisonGroup::set_local_coupling_gain`].
    pub fn set_global_coupling_gain(&mut self, gain: f32) {
        self.global_coupling_gain = math::clamp_or_low(gain, 0.0, 1.0);
    }

    /// Adjusts one string's damping, live, leaving its unison siblings
    /// untouched. `string_index` outside the group's active string count
    /// is silently ignored — the same "no error channel on the audio
    /// thread" contract every other live setter in this crate uses.
    pub fn set_string_damping(&mut self, string_index: usize, damping: f32) {
        let Some(string) = self.strings.get_mut(string_index).and_then(Option::as_mut) else {
            return;
        };
        string.set_damping(damping);
    }

    /// Adjusts one string's sustain, live. See
    /// [`UnisonGroup::set_string_damping`] for the out-of-range contract.
    pub fn set_string_sustain(&mut self, string_index: usize, sustain: f32) {
        let Some(string) = self.strings.get_mut(string_index).and_then(Option::as_mut) else {
            return;
        };
        string.set_sustain(sustain);
    }

    /// Adjusts one string's inharmonicity coefficient, live. See
    /// [`UnisonGroup::set_string_damping`] for the out-of-range contract.
    pub fn set_string_inharmonicity(&mut self, string_index: usize, inharmonicity: f32) {
        let Some(string) = self.strings.get_mut(string_index).and_then(Option::as_mut) else {
            return;
        };
        string.set_inharmonicity(inharmonicity);
    }

    /// Reseeds one string's excitation noise, for its *next* strike. See
    /// [`UnisonGroup::set_string_damping`] for the out-of-range contract.
    pub fn set_string_seed(&mut self, string_index: usize, seed: u32) {
        let Some(string) = self.strings.get_mut(string_index).and_then(Option::as_mut) else {
            return;
        };
        string.set_seed(seed);
    }

    /// Changes one string's felt-contact physics, for its *next* strike.
    /// See [`UnisonGroup::set_string_damping`] for the out-of-range
    /// contract.
    pub fn set_string_hammer(&mut self, string_index: usize, hammer: hammer::HammerConfig) {
        let Some(string) = self.strings.get_mut(string_index).and_then(Option::as_mut) else {
            return;
        };
        string.set_hammer(hammer);
    }

    /// Retunes one string to `cents` away from the group's own
    /// [`UnisonGroup::base_frequency`] — an absolute retune from the
    /// group's reference pitch, not additive to whatever detune the
    /// string already carries from [`UnisonGroup::new`]. See
    /// [`UnisonGroup::set_string_damping`] for the out-of-range contract.
    pub fn set_string_detune(&mut self, string_index: usize, cents: f32) {
        let Some(string) = self.strings.get_mut(string_index).and_then(Option::as_mut) else {
            return;
        };
        string.set_frequency(detune(self.base_frequency, cents));
    }

    /// Whether every string in the group has decayed below audibility.
    #[must_use]
    pub fn is_silent(&self) -> bool {
        self.strings.iter().flatten().all(PluckedString::is_silent)
    }

    /// Whether at least one string's damper is lifted — see
    /// [`PluckedString::damper_engaged`] for why the engine needs this
    /// distinct from [`UnisonGroup::is_silent`].
    #[must_use]
    pub fn is_receptive(&self) -> bool {
        self.strings
            .iter()
            .flatten()
            .any(|string| !string.damper_engaged())
    }

    /// The loudest constituent string's envelope, for voice reclaiming.
    #[must_use]
    pub fn envelope(&self) -> f32 {
        self.strings
            .iter()
            .flatten()
            .fold(0.0f32, |loudest, string| loudest.max(string.envelope()))
    }

    /// Produces one output sample with only local (this group's own
    /// strings) coupling — no shared, cross-key bridge. Used where there is
    /// nothing else to couple to, e.g. rendering a single note offline.
    #[inline]
    pub fn process(&mut self) -> f32 {
        self.process_impl(None)
    }

    /// Produces one output sample coupled through both this group's own
    /// strings (local) and the shared, cross-key bridge bus at
    /// `bridge_index` within the block `bridge` is currently accumulating
    /// (global, `PERF-008`). See the module docs for why these are two
    /// separate couplings rather than one.
    #[inline]
    pub fn process_with_bridge(&mut self, bridge: &mut BridgeBus, bridge_index: usize) -> f32 {
        self.process_impl(Some((bridge, bridge_index)))
    }

    /// The shared implementation behind [`UnisonGroup::process`] and
    /// [`UnisonGroup::process_with_bridge`].
    ///
    /// Three passes, each doing one thing:
    /// 1. Read every active string's bridge tap and its own dispersed
    ///    signal ([`PluckedString::disperse`]), tracking which strings are
    ///    currently receptive (`!damper_engaged`) — only a receptive
    ///    string both contributes to and receives coupling, per
    ///    [`PluckedString::write_mixed_feedback`]'s own damper invariant.
    /// 2. Fold this group's receptive strings into one shared bridge
    ///    contribution ([`local_group_mean`]) and exchange it with the
    ///    global bus, if there is one.
    /// 3. Blend each receptive string's own dispersed signal with the
    ///    other local strings' mean and the global bus's readback — both
    ///    blends are convex combinations, chained — and write it back.
    fn process_impl(&mut self, bridge: Option<(&mut BridgeBus, usize)>) -> f32 {
        let mut samples = [StringSample::default(); MAX_UNISON_STRINGS];
        let mut local_sum = 0.0f32;
        let mut receptive_count = 0usize;
        for (string, sample) in self
            .strings
            .iter_mut()
            .zip(samples.iter_mut())
            .take(self.count)
        {
            let Some(string) = string.as_mut() else {
                continue;
            };
            let tap = string.read_bridge_tap();
            let dispersed = string.disperse(tap);
            let receptive = !string.damper_engaged();
            *sample = StringSample {
                tap,
                dispersed,
                receptive,
            };
            if receptive {
                receptive_count += 1;
                local_sum += dispersed;
            }
        }

        // An absent bridge drives nothing, which is now the same arithmetic
        // as a present bridge nobody has contributed to: adding `0.0` on
        // top of the string's own signal is a true no-op. It was not always
        // — while the global path was a convex *blend*, "read back 0.0"
        // meant drifting towards silence every sample, so a missing bridge
        // had to be threaded down as `None` to avoid a decay-curve bug a
        // render check caught. Additive coupling removes that trap at the
        // source; see [`PluckedString::write_mixed_feedback`].
        let group_mean = local_group_mean(local_sum, receptive_count);
        let bridge_drive = bridge.map_or(0.0, |(bus, index)| bus.add_and_read(index, group_mean))
            * self.global_coupling_gain;

        let mut output = 0.0f32;
        for (string, sample) in self.strings.iter_mut().zip(samples.iter()).take(self.count) {
            let Some(string) = string.as_mut() else {
                continue;
            };
            // A damped string neither radiates into the bridge nor takes
            // anything back from it, so it writes only its own signal —
            // `PluckedString::write_mixed_feedback`'s damper invariant.
            let (mixed, coupling) = if sample.receptive {
                let local_mean = other_strings_mean(local_sum, sample.dispersed, receptive_count);
                (
                    blend(sample.dispersed, local_mean, self.local_coupling_gain),
                    bridge_drive,
                )
            } else {
                (sample.dispersed, 0.0)
            };
            output += string.write_mixed_feedback(sample.tap, mixed, coupling);
        }
        output
    }
}

/// One string's per-sample scratch, read in [`UnisonGroup::process_impl`]'s
/// first pass and consumed in its second — bundled into one struct instead
/// of three parallel arrays so every access goes through
/// [`core::iter::Iterator::zip`] rather than an indexing expression
/// clippy's `indexing_slicing` lint (denied workspace-wide, per
/// `docs/REALTIME-AUDIO-RULES.md`) would otherwise flag as unproven-safe.
#[derive(Debug, Clone, Copy, Default)]
struct StringSample {
    tap: f32,
    dispersed: f32,
    receptive: bool,
}

/// How hard the shared, cross-key bridge bus's previous-block readback
/// *drives* a receptive string — an additive term, unlike
/// [`DEFAULT_LOCAL_COUPLING_GAIN`]'s convex [`blend`]. See
/// [`crate::string::PluckedString::write_mixed_feedback`]'s doc comment for
/// why the two paths need opposite mixing laws, and why scaling this by the
/// string's own round-trip loss keeps any value in `[0, 1]` safe regardless
/// of `sustain`.
///
/// Kept smaller than [`DEFAULT_LOCAL_COUPLING_GAIN`]: cross-key sympathetic
/// resonance is a subtler effect than one note's own unison beating — same
/// literature-order-of-magnitude honesty note as that constant.
const DEFAULT_GLOBAL_COUPLING_GAIN: f32 = 0.08;

/// The mean dispersed signal across this group's `receptive_count`
/// receptive strings — `0.0` when none are (nothing to contribute; also
/// keeps this total rather than dividing by zero).
#[inline]
fn local_group_mean(local_sum: f32, receptive_count: usize) -> f32 {
    if receptive_count == 0 {
        0.0
    } else {
        local_sum / receptive_count as f32
    }
}

/// The mean dispersed signal of every *other* receptive string in the
/// group, excluding `own_signal` — falls back to `own_signal` itself when
/// there is no other receptive string to average (so blending with it is a
/// no-op, never a division by zero), which is exactly the monochord (bass,
/// single-string) case: nothing to beat against.
#[inline]
fn other_strings_mean(local_sum: f32, own_signal: f32, receptive_count: usize) -> f32 {
    if receptive_count <= 1 {
        own_signal
    } else {
        (local_sum - own_signal) / (receptive_count - 1) as f32
    }
}

/// A convex combination of `own` and `other`, weighted by `weight` (clamped
/// into `[0, 1]`): `weight = 0` returns `own` unchanged, `weight = 1`
/// returns `other` unchanged. Bounded by construction between `own` and
/// `other`, for any weight — the property `write_mixed_feedback`'s doc
/// comment relies on for stability regardless of `sustain`.
#[inline]
fn blend(own: f32, other: f32, weight: f32) -> f32 {
    let weight = math::clamp_or_low(weight, 0.0, 1.0);
    own + weight * (other - own)
}

/// How many strings a key with `key_index` unison strings should detune
/// string `string_index` by, in cents. `_ => 0.0` covers the single-string
/// case (nothing to detune against) and any out-of-table index, total
/// rather than requiring the caller to pre-validate.
fn detune_cents(count: usize, string_index: usize) -> f32 {
    match count {
        2 => DETUNE_CENTS_BICHORD
            .get(string_index)
            .copied()
            .unwrap_or(0.0),
        3 => DETUNE_CENTS_TRICHORD
            .get(string_index)
            .copied()
            .unwrap_or(0.0),
        _ => 0.0,
    }
}

/// A distinct odd constant per string index, `XOR`ed into the group's base
/// seed so each unison string draws an independent excitation noise
/// sequence rather than replaying the same one — see [`UnisonGroup::new`]'s
/// doc comment for why identical seeds across strings was a real bug, not
/// a harmless shortcut. Odd, large and unrelated to each other by
/// construction (arbitrary odd 32-bit constants), so XOR-ing one in does
/// not collapse back onto another string's seed for any base seed.
const RESEED_CONSTANTS: [u32; MAX_UNISON_STRINGS] = [0x0000_0000, 0x9E37_79B1, 0x85EB_CA77];

/// Deterministically derives string `string_index`'s own seed from the
/// group's shared `base_seed`, keeping renders reproducible (same base
/// seed, same detuning, same result every time — this project's existing
/// determinism guarantee, `docs/ARCHITECTURE.md`'s testing strategy)
/// while giving each string an independent-looking noise sequence.
/// Total: `string_index` beyond the table falls back to the base seed
/// unchanged rather than panicking, which in practice is unreachable since
/// every caller clamps to [`MAX_UNISON_STRINGS`] first.
fn reseed_for_string(base_seed: u32, string_index: usize) -> u32 {
    base_seed ^ RESEED_CONSTANTS.get(string_index).copied().unwrap_or(0)
}

/// Shifts `frequency` by `cents`, falling back to the original frequency in
/// the unreachable case the shifted value is not representable — keeps this
/// total rather than propagating an error for what is, in practice, always
/// a small perturbation of an already-valid frequency.
fn detune(frequency: Hz, cents: f32) -> Hz {
    let ratio = math::powf(2.0, cents / 1200.0);
    Hz::new(frequency.hertz() * ratio).unwrap_or(frequency)
}

/// Standard modern-piano unison count for `key_index` (`0` = A0), per the
/// module docs' register boundaries. `pub(crate)`-visible so
/// `piano_audio::voicing` can reuse the same table rather than duplicating
/// it — the boundaries are a physical-model fact, not per-key policy, so
/// they belong here alongside [`DETUNE_CENTS_BICHORD`]/
/// [`DETUNE_CENTS_TRICHORD`], even though *which key gets which count* is
/// looked up by the voicing layer.
#[must_use]
pub fn unison_count_for_key_index(key_index: u8) -> usize {
    const MONOCHORD_KEYS: u8 = 12;
    const BICHORD_KEYS: u8 = 18;
    if key_index < MONOCHORD_KEYS {
        1
    } else if key_index < MONOCHORD_KEYS + BICHORD_KEYS {
        2
    } else {
        3
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp, clippy::unwrap_used, clippy::expect_used)]

    use proptest::prelude::*;

    use super::*;

    fn group_at(frequency: f32, count: usize) -> UnisonGroup {
        let rate = SampleRate::new(48_000.0).expect("48 kHz is valid");
        let config = StringConfig::new(Hz::new(frequency).expect("frequency is valid"));
        UnisonGroup::new(config, count, rate).expect("frequency is representable at 48 kHz")
    }

    /// Renders `len` samples straight from one constituent string, bypassing
    /// [`UnisonGroup`]'s own mixing and coupling entirely — the per-string
    /// setter tests below use this to observe exactly one string's output
    /// in isolation, which is the only way to prove a sibling string's
    /// output is bit-for-bit unaffected.
    fn string_output(group: &mut UnisonGroup, string_index: usize, len: usize) -> Vec<f32> {
        let string = group
            .strings
            .get_mut(string_index)
            .and_then(Option::as_mut)
            .expect("string_index is within this test's group");
        let mut buffer = vec![0.0f32; len];
        string.process_block_add(&mut buffer);
        buffer
    }

    /// Asserts that calling `apply` (a per-string setter) on string `1` of a
    /// freshly-plucked trichord changes string `1`'s own future output but
    /// leaves strings `0` and `2`'s bit-for-bit identical to an unmutated
    /// clone — the "done when" criterion every `set_string_*` setter test
    /// below shares.
    fn assert_targets_only_string_one(apply: impl FnOnce(&mut UnisonGroup)) {
        let mut group = group_at(220.0, 3);
        group.pluck(1.0);
        let mut reference = group.clone();

        apply(&mut group);

        for index in 0..3 {
            let changed = string_output(&mut group, index, 512);
            let unchanged = string_output(&mut reference, index, 512);
            if index == 1 {
                assert_ne!(changed, unchanged, "string 1's own output did not change");
            } else {
                assert_eq!(
                    changed, unchanged,
                    "sibling string {index}'s output changed"
                );
            }
        }
    }

    #[test]
    fn a_single_string_group_needs_no_detuning() {
        assert_eq!(detune_cents(1, 0), 0.0);
    }

    #[test]
    fn bichord_strings_are_detuned_symmetrically() {
        assert_eq!(detune_cents(2, 0) + detune_cents(2, 1), 0.0);
        assert_ne!(detune_cents(2, 0), detune_cents(2, 1));
    }

    #[test]
    fn trichord_middle_string_is_not_detuned() {
        assert_eq!(detune_cents(3, 1), 0.0);
    }

    #[test]
    fn unison_count_matches_the_documented_register_boundaries() {
        assert_eq!(unison_count_for_key_index(0), 1);
        assert_eq!(unison_count_for_key_index(11), 1);
        assert_eq!(unison_count_for_key_index(12), 2);
        assert_eq!(unison_count_for_key_index(29), 2);
        assert_eq!(unison_count_for_key_index(30), 3);
        assert_eq!(unison_count_for_key_index(87), 3);
    }

    #[test]
    fn is_silent_before_being_plucked() {
        let mut group = group_at(220.0, 3);
        for _ in 0..1_000 {
            assert_eq!(group.process(), 0.0);
        }
        assert!(group.is_silent());
        assert!(!group.is_receptive());
    }

    #[test]
    fn plucking_lifts_the_damper_and_produces_signal() {
        let mut group = group_at(220.0, 3);
        group.pluck(1.0);
        assert!(group.is_receptive());
        let peak = (0..4_800)
            .map(|_| math::abs(group.process()))
            .fold(0.0f32, f32::max);
        assert!(peak > 0.05, "peak {peak} is inaudible");
    }

    #[test]
    fn lift_damper_makes_an_idle_group_receptive_without_a_strike() {
        let mut group = group_at(220.0, 2);
        assert!(!group.is_receptive());
        group.lift_damper();
        assert!(group.is_receptive());
        // Still silent: lifting the damper is not a strike.
        assert!(group.is_silent());
    }

    #[test]
    fn releasing_re_engages_the_damper_and_blocks_receptiveness() {
        let mut group = group_at(220.0, 3);
        group.pluck(1.0);
        group.release();
        assert!(!group.is_receptive());
    }

    // A lone string is bounded below 4.0 (`string_tests::output_stays_
    // bounded_for_a_full_second`); summing up to `MAX_UNISON_STRINGS = 3`
    // of them, plus a small local-coupling contribution, raises that
    // ceiling proportionally rather than staying at the single-string
    // bound — 16.0 leaves comfortable margin over the worst case without
    // being so loose it would miss real divergence.
    const UNISON_BOUND: f32 = 16.0;

    #[test]
    fn output_stays_bounded_for_a_full_second_with_local_coupling() {
        let mut group = group_at(220.0, 3);
        group.pluck(1.0);
        for index in 0..48_000 {
            let sample = group.process();
            assert!(sample.is_finite(), "sample {index} was not finite");
            assert!(
                sample.abs() < UNISON_BOUND,
                "sample {index} = {sample} escaped"
            );
        }
    }

    /// The regression test for the report: "A5 sounds terrible, like
    /// hitting a tin can with a muffled iron". `dispersion::section_count`
    /// computes exactly one active dispersion section across roughly
    /// C#5-B5, and a high enough inharmonicity there used to clamp that
    /// one section's coefficient close enough to the unit circle
    /// (`dispersion::MAX_COEFFICIENT`, before it was lowered) that its
    /// steep phase response spun three unison strings a few cents apart out
    /// of phase with each other within a handful of round trips — see that
    /// constant's own doc comment for the full mechanism. This group's RMS
    /// used to collapse from `0.14` to numerically silent within about
    /// 120 ms; a monochord at the same pitch, with nothing to diverge
    /// against, was fine throughout, which is what proves this was never
    /// about `sustain` or the loop filter.
    ///
    /// A5 (880 Hz) sits at the top of that single-section band and gets the
    /// highest inharmonicity within it — the worst point, not an arbitrary
    /// one.
    #[test]
    fn a_high_inharmonicity_trichord_in_the_single_dispersion_section_band_does_not_collapse() {
        const STEP: usize = 960; // 20 ms at 48 kHz.

        let rate = SampleRate::new(48_000.0).expect("48 kHz is valid");
        let mut config = StringConfig::new(Hz::new(880.0).expect("A5 is a valid frequency"));
        config.inharmonicity = 0.0345; // piano_audio::voicing's own solved A5 value.
        config.sustain = 0.998_808; // ditto — a near-lossless round trip.
        config.damping = 0.000_57; // ditto — a loop filter that is nearly transparent.
        let mut group = UnisonGroup::new(config, 3, rate).expect("A5 is representable at 48 kHz");
        group.pluck(0.8);

        let mut buffer = [0.0f32; STEP];
        let rms_of = |group: &mut UnisonGroup, buffer: &mut [f32; STEP]| {
            for sample in buffer.iter_mut() {
                *sample = group.process();
            }
            (buffer.iter().map(|s| s * s).sum::<f32>() / STEP as f32).sqrt()
        };

        let first = rms_of(&mut group, &mut buffer);
        for step in 1..25 {
            let rms = rms_of(&mut group, &mut buffer);
            assert!(
                rms > first * 0.01,
                "trichord collapsed by window {step} ({rms:.6} against an opening RMS of \
                 {first:.6}) — see dispersion::MAX_COEFFICIENT's doc comment"
            );
        }
    }

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

    #[test]
    fn set_local_coupling_gain_clamps_into_zero_one() {
        let mut group = group_at(220.0, 3);
        group.set_local_coupling_gain(-5.0);
        assert_eq!(group.local_coupling_gain, 0.0);
        group.set_local_coupling_gain(5.0);
        assert_eq!(group.local_coupling_gain, 1.0);
    }

    #[test]
    fn set_global_coupling_gain_clamps_into_zero_one() {
        let mut group = group_at(220.0, 3);
        group.set_global_coupling_gain(-5.0);
        assert_eq!(group.global_coupling_gain, 0.0);
        group.set_global_coupling_gain(5.0);
        assert_eq!(group.global_coupling_gain, 1.0);
    }

    #[test]
    fn changing_local_coupling_gain_measurably_changes_the_output() {
        let mut coupled = group_at(220.0, 3);
        let mut uncoupled = group_at(220.0, 3);
        uncoupled.set_local_coupling_gain(0.0);
        coupled.pluck(1.0);
        uncoupled.pluck(1.0);
        let difference: f32 = (0..2_000)
            .map(|_| (coupled.process() - uncoupled.process()).abs())
            .sum();
        assert!(
            difference > 1e-3,
            "zeroing local_coupling_gain did not change the output: diff {difference}"
        );
    }

    #[test]
    fn set_string_damping_changes_only_the_targeted_string() {
        assert_targets_only_string_one(|group| group.set_string_damping(1, 0.95));
    }

    #[test]
    fn set_string_sustain_changes_only_the_targeted_string() {
        assert_targets_only_string_one(|group| group.set_string_sustain(1, 0.02));
    }

    #[test]
    fn set_string_inharmonicity_changes_only_the_targeted_string() {
        assert_targets_only_string_one(|group| group.set_string_inharmonicity(1, 0.05));
    }

    #[test]
    fn set_string_detune_changes_only_the_targeted_string() {
        assert_targets_only_string_one(|group| group.set_string_detune(1, 35.0));
    }

    /// [`UnisonGroup::set_string_seed`] and [`UnisonGroup::set_string_hammer`]
    /// only take effect on a string's *next* strike (see their own doc
    /// comments), so — unlike the other four per-string setters — this
    /// re-plucks string 1 alone (not the whole group, which would also
    /// disturb strings 0 and 2's RNG state) before comparing.
    fn assert_targets_only_string_one_on_next_pluck(apply: impl FnOnce(&mut UnisonGroup)) {
        let mut group = group_at(220.0, 3);
        group.pluck(1.0);
        let mut reference = group.clone();

        apply(&mut group);
        group
            .strings
            .get_mut(1)
            .and_then(Option::as_mut)
            .expect("index 1 exists in a trichord")
            .pluck(1.0);

        for index in 0..3 {
            let changed = string_output(&mut group, index, 512);
            let unchanged = string_output(&mut reference, index, 512);
            if index == 1 {
                assert_ne!(
                    changed, unchanged,
                    "string 1's output did not change on its next pluck"
                );
            } else {
                assert_eq!(
                    changed, unchanged,
                    "sibling string {index}'s output changed"
                );
            }
        }
    }

    #[test]
    fn set_string_seed_changes_only_the_targeted_strings_next_pluck() {
        assert_targets_only_string_one_on_next_pluck(|group| group.set_string_seed(1, 0xDEAD_BEEF));
    }

    #[test]
    fn set_string_hammer_changes_only_the_targeted_strings_next_pluck() {
        assert_targets_only_string_one_on_next_pluck(|group| {
            group.set_string_hammer(
                1,
                hammer::HammerConfig {
                    contact_exponent: 6.0,
                    stiffness: 1.0e8,
                    mass: 20.0,
                },
            );
        });
    }

    #[test]
    fn per_string_setters_on_an_out_of_range_index_are_a_silent_no_op() {
        let mut group = group_at(220.0, 3);
        group.pluck(1.0);
        group.set_string_damping(9, 0.5);
        group.set_string_sustain(9, 0.5);
        group.set_string_inharmonicity(9, 0.01);
        group.set_string_seed(9, 42);
        group.set_string_hammer(9, hammer::DEFAULT_HAMMER);
        group.set_string_detune(9, 10.0);
        for _ in 0..512 {
            assert!(group.process().is_finite());
        }
    }

    proptest! {
        /// Whatever unison count and velocity, plucking and processing a
        /// group with local coupling active must stay finite and bounded —
        /// the same totality guarantee `string_tests` proves for a lone
        /// string.
        #[test]
        fn any_unison_count_and_velocity_stays_bounded(
            count in 1usize..=MAX_UNISON_STRINGS,
            velocity in proptest::num::f32::ANY,
        ) {
            let mut group = group_at(220.0, count);
            group.pluck(velocity);
            for _ in 0..2_000 {
                let sample = group.process();
                prop_assert!(sample.is_finite());
                prop_assert!(sample.abs() < UNISON_BOUND, "sample {sample} escaped");
            }
        }

        /// Whatever coupling gains a caller live-sets, including NaN and
        /// +-infinity, the group must stay finite and within the same
        /// bound as the default gains — `set_local_coupling_gain` and
        /// `set_global_coupling_gain` clamp exactly like every other live
        /// setter in this crate.
        #[test]
        fn any_coupling_gain_stays_bounded(
            local_gain in proptest::num::f32::ANY,
            global_gain in proptest::num::f32::ANY,
        ) {
            let mut group = group_at(220.0, 3);
            group.set_local_coupling_gain(local_gain);
            group.set_global_coupling_gain(global_gain);
            let mut bridge = BridgeBus::with_capacity(128);
            group.pluck(1.0);
            bridge.begin_block();
            for index in 0..128 {
                let sample = group.process_with_bridge(&mut bridge, index);
                prop_assert!(sample.is_finite());
                prop_assert!(sample.abs() < UNISON_BOUND, "sample {sample} escaped");
            }
        }
    }
}
