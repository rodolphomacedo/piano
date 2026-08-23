//! Tests for [`super`], split into its own file so `string.rs` itself stays
//! under this project's 500-line file limit (see `CONTRIBUTING.md`) — the
//! implementation is `#[path = "string_tests.rs"] mod tests;` at the bottom
//! of `string.rs`, so this still compiles as `crate::string::tests`.

#![allow(clippy::float_cmp, clippy::unwrap_used, clippy::expect_used)]

use proptest::prelude::*;

use super::*;

fn string_at(frequency: f32) -> PluckedString {
    let rate = SampleRate::new(48_000.0).expect("48 kHz is valid");
    let config = StringConfig::new(Hz::new(frequency).expect("frequency is valid"));
    PluckedString::new(config, rate).expect("frequency is representable at 48 kHz")
}

#[test]
fn rejects_frequencies_above_the_representable_range() {
    let rate = SampleRate::new(48_000.0).expect("48 kHz is valid");
    let config = StringConfig::new(Hz::new(30_000.0).expect("frequency is valid"));
    assert!(matches!(
        PluckedString::new(config, rate),
        Err(ParamError::FrequencyOutOfRange { .. })
    ));
}

#[test]
fn total_loop_length_matches_the_period() {
    // `loop_delay()` alone is no longer expected to sit close to the
    // period the way it did before M4: the loop filter's zero and the
    // dispersion cascade now both claim several samples of phase delay
    // at DC (by design — that claimed delay is exactly what makes upper
    // partials sit sharp). The invariant that still must hold is the
    // *total* loop length across delay line, loss filter, dispersion
    // cascade and feedback path summing back to the period, the same
    // reasoning `set_damping_keeps_the_total_loop_length_anchored_to_the_period`
    // checks after a live retune.
    let string = string_at(440.0);
    let period = 48_000.0 / 440.0;
    let total_delay = string.loop_delay()
        + string.loop_filter.phase_delay_at_dc()
        + string.dispersion.phase_delay_at_dc()
        + 1.0;
    assert!(
        (total_delay - period).abs() < 1e-3,
        "total delay {total_delay} drifted from period {period}"
    );
}

#[test]
fn is_silent_before_being_plucked() {
    let mut string = string_at(220.0);
    for _ in 0..1_000 {
        assert_eq!(string.process(), 0.0);
    }
    assert!(string.is_silent());
}

#[test]
fn plucking_produces_signal() {
    let mut string = string_at(220.0);
    string.pluck(1.0);
    let peak = (0..4_800)
        .map(|_| math::abs(string.process()))
        .fold(0.0f32, f32::max);
    assert!(peak > 0.05, "peak {peak} is inaudible");
}

#[test]
fn output_stays_bounded_for_a_full_second() {
    let mut string = string_at(27.5);
    string.pluck(1.0);
    for index in 0..48_000 {
        let sample = string.process();
        assert!(sample.is_finite(), "sample {index} was not finite");
        assert!(sample.abs() < 4.0, "sample {index} = {sample} escaped");
    }
}

#[test]
fn energy_decays_after_the_attack() {
    let mut string = string_at(440.0);
    string.pluck(1.0);
    for _ in 0..4_800 {
        string.process();
    }
    let early = string.envelope();
    for _ in 0..48_000 {
        string.process();
    }
    assert!(string.envelope() < early, "envelope grew from {early}");
}

#[test]
fn a_plucked_string_eventually_goes_quiet() {
    let mut string = string_at(440.0);
    string.pluck(1.0);
    for _ in 0..48_000 * 30 {
        string.process();
    }
    assert!(string.is_silent(), "envelope {}", string.envelope());
}

#[test]
fn the_same_seed_renders_the_same_note() {
    let mut left = string_at(440.0);
    let mut right = string_at(440.0);
    left.pluck(0.8);
    right.pluck(0.8);
    for _ in 0..2_400 {
        assert_eq!(left.process(), right.process());
    }
}

#[test]
fn set_damping_keeps_the_total_loop_length_anchored_to_the_period() {
    // set_damping's whole point: loop_delay plus the loss filter's own
    // phase delay plus the feedback path's one-sample delay must sum
    // back to the period, so the fundamental frequency does not drift
    // when damping changes — only loop_delay's *share* of that total
    // moves.
    let rate = SampleRate::new(48_000.0).expect("48 kHz is valid");
    let frequency = Hz::new(440.0).expect("440 Hz is valid");
    let period = 48_000.0 / 440.0;
    // 0.9 is a bright-to-dull swing any real voicing knob would use.
    // 0.999 is deliberately excluded: at that pole the filter's own
    // phase delay (999 samples) exceeds the whole period of a 440 Hz
    // string, so loop_delay floors at MIN_LOOP_DELAY instead of
    // preserving pitch — a real, documented degradation at extreme
    // settings, covered separately by
    // `live_damping_changes_never_break_the_string` (which asserts the
    // floor and finiteness hold for every damping, including that one).
    for damping in [0.0, 0.3, 0.6, 0.9] {
        let mut string =
            PluckedString::new(StringConfig::new(frequency), rate).expect("440 Hz is tunable");
        string.set_damping(damping);
        let total_delay = string.loop_delay()
            + string.loop_filter.phase_delay_at_dc()
            + string.dispersion.phase_delay_at_dc()
            + 1.0;
        assert!(
            (total_delay - period).abs() < 1e-3,
            "damping {damping}: total delay {total_delay} drifted from period {period}"
        );
    }
}

#[test]
fn set_damping_never_produces_a_non_finite_or_unbounded_signal() {
    let mut string = string_at(440.0);
    string.pluck(1.0);
    string.set_damping(1.0);
    for index in 0..48_000 {
        let sample = string.process();
        assert!(sample.is_finite(), "sample {index} was not finite");
        assert!(sample.abs() < 4.0, "sample {index} = {sample} escaped");
    }
}

#[test]
fn set_damping_is_audible_immediately_on_an_already_ringing_string() {
    let mut string = string_at(220.0);
    string.pluck(1.0);
    for _ in 0..2_000 {
        string.process();
    }
    let before = (0..200).map(|_| string.process()).collect::<Vec<_>>();

    let mut identical_twin = string_at(220.0);
    identical_twin.pluck(1.0);
    for _ in 0..2_000 {
        identical_twin.process();
    }
    identical_twin.set_damping(0.99);
    let after = (0..200)
        .map(|_| identical_twin.process())
        .collect::<Vec<_>>();

    assert_ne!(before, after, "changing damping had no audible effect");
}

#[test]
fn set_inharmonicity_retunes_the_loop_the_same_way_damping_does() {
    let mut string = string_at(440.0);
    let before = string.loop_delay();
    string.set_inharmonicity(0.02);
    assert_ne!(string.loop_delay(), before);
    let total_delay = string.loop_delay()
        + string.loop_filter.phase_delay_at_dc()
        + string.dispersion.phase_delay_at_dc()
        + 1.0;
    let period = 48_000.0 / 440.0;
    assert!(
        (total_delay - period).abs() < 1e-3,
        "total delay {total_delay} drifted from period {period}"
    );
}

#[test]
fn set_inharmonicity_never_produces_a_non_finite_or_unbounded_signal() {
    let mut string = string_at(220.0);
    string.pluck(1.0);
    string.set_inharmonicity(0.05);
    for index in 0..48_000 {
        let sample = string.process();
        assert!(sample.is_finite(), "sample {index} was not finite");
        assert!(sample.abs() < 4.0, "sample {index} = {sample} escaped");
    }
}

#[test]
fn set_sustain_does_not_change_loop_delay() {
    let mut string = string_at(440.0);
    let before = string.loop_delay();
    string.set_sustain(0.5);
    assert_eq!(string.loop_delay(), before);
}

#[test]
fn set_seed_changes_the_next_pluck_without_affecting_the_current_one() {
    let mut left = string_at(440.0);
    let mut right = string_at(440.0);
    left.pluck(0.8);
    right.pluck(0.8);
    for _ in 0..64 {
        assert_eq!(left.process(), right.process());
    }
    right.set_seed(0xDEAD_BEEF);
    left.pluck(0.8);
    right.pluck(0.8);
    let mut differed = false;
    for _ in 0..64 {
        if left.process() != right.process() {
            differed = true;
        }
    }
    assert!(differed, "reseeding did not change the next pluck");
}

#[test]
fn release_does_not_change_loop_delay() {
    // Same reasoning as `set_sustain_does_not_change_loop_delay`: release
    // only scales the loop's broadband gain, so it must never retune the
    // string.
    let mut string = string_at(440.0);
    let before = string.loop_delay();
    string.pluck(1.0);
    string.release();
    assert_eq!(string.loop_delay(), before);
}

#[test]
fn release_makes_a_ringing_note_decay_much_faster() {
    let mut held = string_at(220.0);
    held.pluck(1.0);
    let mut released = string_at(220.0);
    released.pluck(1.0);

    // Let both ring identically for a while, then release only one.
    for _ in 0..2_000 {
        held.process();
        released.process();
    }
    released.release();

    // Same number of further samples for both: nowhere near enough for
    // the held string to reach silence on its own (per
    // `a_plucked_string_eventually_goes_quiet`, that takes tens of
    // seconds), but comfortably enough for the released one to.
    for _ in 0..20_000 {
        held.process();
        released.process();
    }

    assert!(
        released.is_silent(),
        "released string should have reached silence, envelope {}",
        released.envelope()
    );
    assert!(
        !held.is_silent(),
        "unreleased twin should still be ringing for comparison"
    );
}

#[test]
fn release_before_plucking_is_harmless() {
    let mut string = string_at(440.0);
    string.release();
    for index in 0..1_000 {
        let sample = string.process();
        assert_eq!(sample, 0.0, "sample {index} was not silent");
    }
    assert!(string.is_silent());
}

#[test]
fn re_plucking_after_release_lifts_the_damper_again() {
    let mut string = string_at(440.0);
    string.pluck(1.0);
    string.release();
    for _ in 0..4_000 {
        string.process();
    }
    assert!(string.is_silent(), "the released note should have died out");

    string.pluck(1.0);
    for _ in 0..2_000 {
        string.process();
    }
    assert!(
        !string.is_silent(),
        "re-plucking after release should undamp the string and let it ring again"
    );
}

proptest! {
    /// Whatever damping is requested, live retuning never produces a
    /// loop shorter than the minimum representable delay, and the
    /// string never blows up.
    #[test]
    fn live_damping_changes_never_break_the_string(damping in proptest::num::f32::ANY) {
        let mut string = string_at(220.0);
        string.pluck(1.0);
        string.set_damping(damping);
        prop_assert!(string.loop_delay() >= MIN_LOOP_DELAY);
        for _ in 0..1_000 {
            let sample = string.process();
            prop_assert!(sample.is_finite());
        }
    }

    /// Same guarantee as `live_damping_changes_never_break_the_string`,
    /// for the dispersion cascade's own live control.
    #[test]
    fn live_inharmonicity_changes_never_break_the_string(inharmonicity in proptest::num::f32::ANY) {
        let mut string = string_at(220.0);
        string.pluck(1.0);
        string.set_inharmonicity(inharmonicity);
        prop_assert!(string.loop_delay() >= MIN_LOOP_DELAY);
        for _ in 0..1_000 {
            let sample = string.process();
            prop_assert!(sample.is_finite());
        }
    }

    /// Whatever velocity a strike uses, the excitation stays finite and
    /// bounded — including NaN, +-infinity, zero and the extremes of
    /// the clamped range.
    #[test]
    fn plucking_at_any_velocity_never_breaks_the_string(velocity in proptest::num::f32::ANY) {
        let mut string = string_at(220.0);
        string.pluck(velocity);
        for _ in 0..2_000 {
            let sample = string.process();
            prop_assert!(sample.is_finite());
            prop_assert!(sample.abs() < 4.0, "sample {sample} escaped");
        }
    }

    /// However many times `release` is called, and whenever it is called
    /// relative to plucking, the string stays finite and bounded — the
    /// same totality guarantee every other live control gets.
    #[test]
    fn release_any_number_of_times_never_breaks_the_string(release_calls in 0usize..10) {
        let mut string = string_at(220.0);
        string.pluck(1.0);
        for _ in 0..release_calls {
            string.release();
        }
        for _ in 0..2_000 {
            let sample = string.process();
            prop_assert!(sample.is_finite());
            prop_assert!(sample.abs() < 4.0, "sample {sample} escaped");
        }
    }
}

#[test]
fn block_processing_adds_into_the_buffer() {
    let mut string = string_at(440.0);
    string.pluck(1.0);
    let mut buffer = [1.0f32; 64];
    string.process_block_add(&mut buffer);
    assert!(buffer.iter().any(|sample| (sample - 1.0).abs() > 1e-6));
}
