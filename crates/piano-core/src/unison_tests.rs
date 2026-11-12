#![allow(clippy::float_cmp, clippy::unwrap_used, clippy::expect_used)]

use proptest::prelude::*;

use super::*;

pub(super) fn group_at(frequency: f32, count: usize) -> UnisonGroup {
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
    assert_eq!(detune_cents(1, 0, 1.0), 0.0);
}

#[test]
fn bichord_strings_are_detuned_symmetrically() {
    assert_eq!(detune_cents(2, 0, 1.0) + detune_cents(2, 1, 1.0), 0.0);
    assert_ne!(detune_cents(2, 0, 1.0), detune_cents(2, 1, 1.0));
}

/// A trichord at `-x, 0, +x` beats to a periodic dead null (issue #96), so
/// its middle string must not sit midway between the other two.
#[test]
fn a_trichord_is_mistuned_asymmetrically() {
    let [flat, middle, sharp] = [0, 1, 2].map(|index| detune_cents(3, index, 1.0));
    let midway = f32::midpoint(flat, sharp);
    assert!((middle - midway).abs() > 0.1, "{flat} {middle} {sharp}");
}

#[test]
fn the_outer_strings_sit_exactly_the_spread_apart() {
    for count in [2, 3] {
        let cents: Vec<f32> = (0..count)
            .map(|index| detune_cents(count, index, 0.8))
            .collect();
        let widest = cents.iter().copied().fold(f32::MIN, f32::max);
        let narrowest = cents.iter().copied().fold(f32::MAX, f32::min);
        assert!(
            (widest - narrowest - 0.8).abs() < 1e-6,
            "{count}: {cents:?}"
        );
    }
}

#[test]
fn a_hostile_spread_is_clamped_rather_than_trusted() {
    assert_eq!(detune_cents(2, 1, f32::NAN), 0.0);
    assert_eq!(detune_cents(2, 1, -3.0), 0.0);
    assert_eq!(
        detune_cents(2, 1, f32::INFINITY),
        0.5 * MAX_UNISON_SPREAD_CENTS
    );
}

#[test]
fn set_unison_spread_retunes_every_string_from_the_groups_own_pitch() {
    let mut group = group_at(220.0, 3);
    let mut reference = group_at(220.0, 3);
    group.set_unison_spread(2.0);
    for index in 0..3 {
        reference.set_string_detune(index, detune_cents(3, index, 2.0));
    }
    group.pluck(0.8);
    reference.pluck(0.8);
    for _ in 0..2_000 {
        assert_eq!(group.process(), reference.process());
    }
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
pub(super) const UNISON_BOUND: f32 = 16.0;

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
fn set_local_coupling_gain_clamps_into_zero_one() {
    let mut group = group_at(220.0, 3);
    group.set_local_coupling_gain(-5.0);
    assert_eq!(group.local_coupling_gain, 0.0);
    group.set_local_coupling_gain(5.0);
    assert_eq!(group.local_coupling_gain, MAX_LOCAL_COUPLING_GAIN);
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

#[test]
fn set_string_loop_zero_mix_changes_only_the_targeted_string() {
    assert_targets_only_string_one(|group| group.set_string_loop_zero_mix(1, 0.45));
}

#[test]
fn set_string_strike_position_changes_only_the_targeted_strings_next_pluck() {
    assert_targets_only_string_one_on_next_pluck(|group| {
        group.set_string_strike_position(1, 0.3);
    });
}

/// A strike position or a hammer only shows in a string's *next* strike,
/// so — unlike the other per-string setters — this strikes string 1 again
/// before comparing. Striking again before any sample has been rendered
/// only restarts the same blow, so without `apply` nothing would differ.
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
fn striking_one_string_again_before_it_has_moved_changes_nothing() {
    assert_next_pluck_leaves_every_string_alike(|_| {});
}

/// The control for the helper above: with nothing applied, striking string
/// 1 again must leave all three strings exactly as their reference.
fn assert_next_pluck_leaves_every_string_alike(apply: impl FnOnce(&mut UnisonGroup)) {
    let mut group = group_at(220.0, 3);
    group.pluck(1.0);
    let mut reference = group.clone();
    apply(&mut group);
    if let Some(string) = group.strings.get_mut(1).and_then(Option::as_mut) {
        string.pluck(1.0);
    }
    for index in 0..3 {
        let again = string_output(&mut group, index, 512);
        assert_eq!(
            again,
            string_output(&mut reference, index, 512),
            "string {index}"
        );
    }
}

#[test]
fn set_string_hammer_changes_only_the_targeted_strings_next_pluck() {
    assert_targets_only_string_one_on_next_pluck(|group| {
        group.set_string_hammer(
            1,
            hammer::HammerConfig {
                contact_exponent: 3.5,
                stiffness: 40.0 * hammer::DEFAULT_HAMMER.stiffness,
                ..hammer::DEFAULT_HAMMER
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
    group.set_string_loop_zero_mix(9, 0.3);
    group.set_string_strike_position(9, 0.2);
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
