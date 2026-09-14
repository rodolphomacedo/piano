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
fn a_hard_strike_on_the_highest_key_still_produces_signal_once_coupled() {
    // #57's audible claim shows up most in the upper treble, where
    // contact outlasts one round trip (loop_delay ≈ 0.24 ms at C8's
    // 4186 Hz, 48 kHz) — this is the sanity floor before the real
    // behavioural comparison in
    // `a_pending_contacts_tail_is_measurably_coupled_to_the_real_bridge_tap`
    // below: coupling must not silence the note outright.
    let mut string = string_at(4_186.0);
    string.pluck(0.9);
    let samples: Vec<f32> = (0..2_000).map(|_| string.process()).collect();
    assert!(
        samples.iter().any(|sample| sample.abs() > 1e-6),
        "a hard strike on the highest key produced no audible signal at all"
    );
}

#[test]
fn a_pending_contacts_tail_is_measurably_coupled_to_the_real_bridge_tap() {
    // The design doc's own testing plan (`docs/superpowers/specs/
    // 2026-09-13-hammer-string-coupling-design.md`, "Testing plan") asks
    // for exactly this: "a second strike's injected force differs
    // measurably from the same strike computed with `v_incoming` forced to
    // `0.0`" — proof the coupling is live, not inert.
    //
    // This test's predecessor instead varied `string_impedance` between two
    // full `process()` runs (`5.0e8` vs. the `MAX_STRING_IMPEDANCE`
    // default) and asserted `assert_ne!` on the raw `f32` output vectors.
    // That is not the comparison the design doc asked for, and it does not
    // honestly test coupling: `hammer::MIN_STRING_IMPEDANCE`'s doc comment
    // documents that `string_impedance`'s *entire* sanctioned range is
    // nearly inert (the reactive `force / string_impedance` term contributes
    // only ~7e-3 to ~7e-10 m/s against hammer velocities of 0.5-6 m/s,
    // changing peak force ~0.13% end to end) — so `assert_ne!` on two such
    // runs was only ever detecting float noise in low bits, and would keep
    // passing even if the actual coupling mechanism (`v_incoming`, unscaled
    // by impedance) were five-plus orders of magnitude weaker than intended
    // or removed outright.
    //
    // What actually varies the coupling is `v_incoming`, so this test
    // drives a real, hard C8 strike sample by sample with the exact public
    // pipeline `PluckedString::process` uses internally
    // (`read_bridge_tap`/`disperse`/`write_mixed_feedback`), captures the
    // live `PendingContact` state and the real bridge tap it is about to be
    // coupled against partway through the tail, and calls
    // `hammer::couple_contact_step` directly with that real tap versus the
    // same call with `v_incoming` forced to `0.0` — the comparison the
    // design doc specified, for the exact hammer/state/velocity a live tail
    // carries, with no `string_impedance` axis involved at all.
    let mut string = string_at(4_186.0);
    string.pluck(0.9);

    // At every sample the tail is still pending, compare what
    // `couple_contact_step` actually injects against the real tap versus
    // what it would inject with `v_incoming` forced to `0.0` — the exact
    // design-doc comparison — and keep the sample where that difference is
    // largest. An early tap (numerically indistinguishable from the
    // silence the delay line started at) or a moment where the reference
    // force happens to be huge would understate the effect; scanning the
    // whole tail for its strongest moment is what actually tests whether
    // the mechanism is load-bearing anywhere in a real strike, not just at
    // one arbitrarily chosen sample.
    let mut largest_relative_difference = 0.0f32;
    let mut worst_case = None;
    for _ in 0..2_000 {
        let Some(contact) = string.pending_contact else {
            break;
        };
        let tap = string.read_bridge_tap();
        let (_, coupled_force) =
            hammer::couple_contact_step(contact.contact, contact.hammer, tap, string.sample_rate);
        let (_, silent_force) =
            hammer::couple_contact_step(contact.contact, contact.hammer, 0.0, string.sample_rate);
        let scale = coupled_force.max(silent_force).max(f32::EPSILON);
        let relative_difference = math::abs(coupled_force - silent_force) / scale;
        if relative_difference > largest_relative_difference {
            largest_relative_difference = relative_difference;
            worst_case = Some((tap, coupled_force, silent_force));
        }
        let dispersed = string.disperse(tap);
        string.write_mixed_feedback(tap, dispersed, 0.0);
    }
    let (tap, coupled_force, silent_force) = worst_case.expect(
        "a hard C8 strike should leave a pending contact tail with at least \
         one nonzero bridge tap to compare against",
    );
    assert!(
        // The strongest moment actually measures ~35% for this strike —
        // 2% is a wide margin below that (room for legitimate run-to-run
        // float variance) while still failing hard if the coupling nearly
        // vanished or were removed.
        largest_relative_difference > 0.02,
        "the largest difference the real bridge tap made to the pending \
         tail's injected force, anywhere across the tail, was only \
         {:.4}% relative to `v_incoming` forced to 0.0 (at tap {tap}: \
         coupled {coupled_force}, silent {silent_force}) — the coupling \
         looks nearly inert at every state a live tail carries",
        largest_relative_difference * 100.0
    );
}

#[test]
fn a_pending_contact_tail_is_forced_to_end_within_the_total_contact_sample_cap() {
    // Without `MAX_TOTAL_CONTACT_SAMPLES`, a pending tail's only exit is
    // `hammer::ContactState::separated`, which is guaranteed to happen
    // *eventually* (force stays non-negative, so `hammer_velocity` is
    // monotonically non-increasing) but not within any fixed sample count.
    // A `v_incoming` sequence that tracks the hammer's own velocity one
    // tick behind (`hammer_velocity - epsilon`, recomputed every sample
    // from the live state) keeps `compression` asymptotically near zero
    // without ever quite reaching it. Proven directly against
    // `hammer::couple_contact_step` first, bypassing `PendingContact`
    // entirely, for 5x `hammer::MAX_CONTACT_SAMPLES` steps without
    // separating once — this is the pathological sequence the sample cap
    // exists for.
    let hammer = hammer::DEFAULT_HAMMER;
    let epsilon = 0.01f32;
    let adversarial_bound = hammer::MAX_CONTACT_SAMPLES * 5;
    let mut state = hammer::ContactState::starting(0.9);
    for _ in 0..adversarial_bound {
        let v_incoming = state.hammer_velocity - epsilon;
        let (next, _) = hammer::couple_contact_step(state, hammer, v_incoming, 48_000.0);
        state = next;
    }
    assert!(
        !state.separated,
        "the adversarial v_incoming sequence separated on its own within \
         {adversarial_bound} steps — this no longer demonstrates the defect \
         the sample cap fixes; pick a more adversarial sequence"
    );

    // Same sequence, now driving the real `PendingContact` tail on a hard
    // C8 strike through `PluckedString::next_contact_sample` directly: the
    // tail must still end at or before `MAX_TOTAL_CONTACT_SAMPLES` total
    // samples (burst + tail), even though `separated` never physically
    // latches for this `v_incoming` sequence.
    let mut string = string_at(4_186.0);
    string.pluck(0.9);
    assert!(
        string.pending_contact.is_some(),
        "a hard C8 strike should leave a pending contact tail to test against"
    );
    let burst_length = string.loop_delay as usize + 1;
    let mut tail_samples = 0usize;
    for _ in 0..adversarial_bound {
        let Some(contact) = string.pending_contact else {
            break;
        };
        let v_incoming = contact.contact.hammer_velocity - epsilon;
        string.next_contact_sample(v_incoming);
        tail_samples += 1;
    }
    assert!(
        string.pending_contact.is_none(),
        "the pending tail never ended within {adversarial_bound} samples"
    );
    let total_samples = burst_length + tail_samples;
    assert!(
        total_samples <= MAX_TOTAL_CONTACT_SAMPLES as usize,
        "contact ran for {total_samples} total samples, past the \
         {MAX_TOTAL_CONTACT_SAMPLES}-sample cap"
    );
}

#[test]
fn handing_the_contact_over_to_the_pending_tail_injects_no_step_in_force() {
    // The excitation is the contact force's first difference scaled by
    // `contact_force_diff_inv_peak`, which is calibrated to the *largest*
    // step the reference curve takes between two adjacent samples. A tail
    // that restarts its `prev` at `0.0` therefore fakes a step the size of
    // the whole pulse — 56x a real one for this note — and the loop plays
    // it back one round trip later as a single full-scale spike. A4 at 0.8
    // is the case that caught it: 178 contact samples against a 105-sample
    // loop, so the tail runs for most of the contact.
    let mut string = string_at(440.0);
    string.pluck(0.8);
    let samples: Vec<f32> = (0..1_000).map(|_| string.process()).collect();
    let peak = samples.iter().copied().map(f32::abs).fold(0.0, f32::max);
    assert!(
        peak < 1.0,
        "a single string peaks at {peak}, so the hand-off to the pending contact is injecting a \
         step rather than continuing the force pulse"
    );
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

/// Driving the loop with nothing must be indistinguishable from not passing
/// a coupling term at all — the property that makes an idle bridge free, and
/// the one the old convex blend could not have: blending towards `0.0` was a
/// loss, not a no-op.
#[test]
fn zero_coupling_writes_back_exactly_the_uncoupled_signal() {
    let mut coupled = string_at(220.0);
    let mut plain = string_at(220.0);
    coupled.pluck(1.0);
    plain.pluck(1.0);
    for index in 0..4_800 {
        let tap = coupled.read_bridge_tap();
        let dispersed = coupled.disperse(tap);
        let driven = coupled.write_mixed_feedback(tap, dispersed, 0.0);
        assert_eq!(driven, plain.process(), "diverged at sample {index}");
    }
}

/// Coupling is scaled by `1 - loop_gain`, so a string that loses nothing per
/// round trip receives nothing through the bridge — the limit that makes the
/// additive term unconditionally stable rather than merely small.
#[test]
fn a_lossless_string_takes_nothing_from_the_bridge() {
    let mut string = string_at(220.0);
    string.set_sustain(1.0);
    string.pluck(1.0);
    let tap = string.read_bridge_tap();
    let dispersed = string.disperse(tap);
    let mut reference = string.clone();
    string.write_mixed_feedback(tap, dispersed, 1000.0);
    reference.write_mixed_feedback(tap, dispersed, 0.0);
    assert_eq!(
        string.read_bridge_tap(),
        reference.read_bridge_tap(),
        "a lossless string was moved by the bridge"
    );
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
fn set_frequency_retunes_within_reserved_headroom() {
    let rate = SampleRate::new(48_000.0).expect("48 kHz is valid");
    let base_frequency = Hz::new(220.0).expect("220 Hz is valid");
    let mut string =
        PluckedString::new(StringConfig::new(base_frequency), rate).expect("220 Hz is tunable");

    // Detune down by the full reserved range — the lowest frequency
    // `PluckedString::new` sized the delay line's headroom for.
    let detuned_hertz = 220.0 * math::powf(2.0, -MAX_LIVE_DETUNE_CENTS / 1200.0);
    let detuned = Hz::new(detuned_hertz).expect("detuned frequency stays positive");
    string.set_frequency(detuned);

    let period = rate.hertz() / detuned_hertz;
    let total_delay = string.loop_delay()
        + string.loop_filter.phase_delay_at_dc()
        + string.dispersion.phase_delay_at_dc()
        + 1.0;
    assert!(
        (total_delay - period).abs() < 1e-3,
        "total delay {total_delay} drifted from the detuned period {period} — \
         reserved headroom was insufficient"
    );
}

#[test]
fn set_frequency_to_the_same_pitch_does_not_move_loop_delay() {
    let mut string = string_at(440.0);
    let before = string.loop_delay();
    string.set_frequency(Hz::new(440.0).expect("440 Hz is valid"));
    assert!(
        (string.loop_delay() - before).abs() < 1e-3,
        "retuning to the same frequency should not move loop_delay"
    );
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
fn a_short_period_strike_keeps_injecting_contact_force_past_the_first_loop() {
    // At 4 kHz the loop is on the order of 10-12 samples, far shorter than
    // the default hammer's 3.5-6.5 ms (~170-310 samples at 48 kHz) contact
    // — exactly the case `docs/PHYSICS.md`'s "What the hammer still gets
    // wrong" describes. Measure total energy over the first 400 samples
    // (comfortably past one loop length but still within the contact
    // window) against energy in just the first loop length: if the
    // extension is doing anything, later round trips must carry
    // meaningfully more than what a single loop length's initial burst
    // alone would explain once it has already started decaying.
    let mut string = string_at(4_000.0);
    string.pluck(1.0);
    let loop_length = string.loop_delay() as usize + 1;

    let mut early_energy = 0.0f32;
    for _ in 0..loop_length {
        let sample = string.process();
        early_energy += sample * sample;
    }
    let mut later_energy = 0.0f32;
    for _ in 0..(400 - loop_length) {
        let sample = string.process();
        later_energy += sample * sample;
    }

    assert!(
        later_energy > early_energy * 0.1,
        "later energy {later_energy} should be a substantial fraction of the \
         first loop's {early_energy}, not a rapidly decaying tail with nothing \
         still being injected"
    );
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

/// A string at `frequency` with a chosen strike position and no
/// inharmonicity, so the strike-position comb's notch lands on an exact
/// harmonic a test can measure without dispersion smearing it.
fn harmonic_string_struck_at(frequency: f32, strike_position: f32) -> PluckedString {
    let rate = SampleRate::new(48_000.0).expect("48 kHz is valid");
    let mut config = StringConfig::new(Hz::new(frequency).expect("frequency is valid"));
    config.inharmonicity = 0.0;
    config.strike_position = strike_position;
    PluckedString::new(config, rate).expect("frequency is representable at 48 kHz")
}

/// Windowed single-bin magnitude at `frequency_hz`, a dependency-free DFT so
/// a `piano-core` test can weigh one partial without pulling in an FFT crate.
/// A Hann window keeps a strong partial's energy from smearing into the bins
/// this test compares against.
fn magnitude_at(samples: &[f32], frequency_hz: f32, sample_rate: f32) -> f32 {
    let n = samples.len() as f64;
    let (mut re, mut im) = (0.0f64, 0.0f64);
    for (index, &sample) in samples.iter().enumerate() {
        let window = 0.5 - 0.5 * (core::f64::consts::TAU * index as f64 / n).cos();
        let phase = core::f64::consts::TAU * f64::from(frequency_hz) * index as f64
            / f64::from(sample_rate);
        re += window * f64::from(sample) * phase.cos();
        im -= window * f64::from(sample) * phase.sin();
    }
    (re * re + im * im).sqrt() as f32
}

/// Renders the attack of a struck string into a fresh buffer.
fn render_attack(string: &mut PluckedString, samples: usize) -> alloc::vec::Vec<f32> {
    let mut buffer = alloc::vec![0.0f32; samples];
    string.pluck(1.0);
    for slot in &mut buffer {
        *slot = string.process();
    }
    buffer
}

#[test]
fn striking_at_one_eighth_attenuates_the_eighth_partial() {
    // A hammer at 1/8 of the string puts a node — a deep comb notch — on the
    // 8th partial. Measured against the same note with the comb turned off
    // (`strike_position` 0), so the loop filter's own spectral tilt is
    // common to both sides and only the strike position differs: issue #32's
    // "the expected partials are measurably attenuated". Compared to the
    // *same* partial rather than to neighbours, since the loop's rolloff
    // makes the spectrum slope so steeply that a lower neighbour outranks H8
    // with or without a notch.
    let fundamental = 220.0;
    let sample_rate = 48_000.0;
    let h8 = |strike: f32| {
        let mut string = harmonic_string_struck_at(fundamental, strike);
        let attack = render_attack(&mut string, 4_096);
        magnitude_at(&attack, fundamental * 8.0, sample_rate)
    };
    assert!(
        h8(0.125) < 0.4 * h8(0.0),
        "the 1/8 strike's H8 ({}) should be a fraction of the no-comb H8 ({})",
        h8(0.125),
        h8(0.0)
    );
}

#[test]
fn moving_the_strike_position_moves_the_notch() {
    // Issue #32's second half: "moving the strike position audibly changes
    // the tone in the expected direction." Striking at 1/16 moves the notch
    // to H16, so H8 — notched at 1/8 — is now left ringing.
    let fundamental = 220.0;
    let sample_rate = 48_000.0;
    let h8_at = |strike: f32| {
        let mut string = harmonic_string_struck_at(fundamental, strike);
        let attack = render_attack(&mut string, 4_096);
        magnitude_at(&attack, fundamental * 8.0, sample_rate)
    };
    assert!(
        h8_at(1.0 / 16.0) > 2.0 * h8_at(1.0 / 8.0),
        "H8 should be far louder struck at 1/16 ({}) than at 1/8 ({})",
        h8_at(1.0 / 16.0),
        h8_at(1.0 / 8.0)
    );
}

#[test]
fn re_striking_one_string_reproduces_the_attack_sample_for_sample() {
    // Issue #77's first "done when": two strikes at the same velocity and
    // seed produce identical attacks. The noise excitation could not — its
    // generator advanced across the first strike, so the second drew a
    // different burst — which is what made the attack profile unrepeatable.
    // `write_excitation` now rewinds the generator to the string's seed on
    // every strike.
    let mut string = string_at(196.0);
    let first = render_attack(&mut string, 4_096);
    let second = render_attack(&mut string, 4_096);
    assert_eq!(first, second, "the second strike diverged from the first");
}

#[test]
fn a_purely_deterministic_strike_never_consults_the_noise_generator() {
    // At `excitation_noise_mix` 0 the excitation is the contact-force pulse
    // alone, so the seed cannot matter: two strings differing only in seed
    // must render the same attack. Any dependence on the seed would mean
    // noise is still leaking in.
    let deterministic = |seed: u32| {
        let rate = SampleRate::new(48_000.0).expect("48 kHz is valid");
        let mut config = StringConfig::new(Hz::new(196.0).expect("frequency is valid"));
        config.seed = seed;
        config.excitation_noise_mix = 0.0;
        let mut string =
            PluckedString::new(config, rate).expect("frequency is representable at 48 kHz");
        render_attack(&mut string, 4_096)
    };
    assert_eq!(deterministic(0x2545_F491), deterministic(0xDEAD_BEEF));
}

#[test]
fn set_excitation_noise_mix_changes_the_next_strike_without_affecting_the_current_one() {
    // Same contract as `set_seed` and `set_strike_position`: a live control
    // takes effect on the *next* strike, never a note already ringing.
    let mut left = string_at(196.0);
    let mut right = string_at(196.0);
    left.pluck(0.8);
    right.pluck(0.8);
    for _ in 0..64 {
        assert_eq!(left.process(), right.process());
    }
    right.set_excitation_noise_mix(1.0);
    for _ in 0..64 {
        assert_eq!(
            left.process(),
            right.process(),
            "a ringing string was disturbed"
        );
    }
    left.pluck(0.8);
    right.pluck(0.8);
    let mut differed = false;
    for _ in 0..256 {
        if left.process() != right.process() {
            differed = true;
        }
    }
    assert!(differed, "the mix change never reached the next strike");
}

proptest! {
    /// The stability claim `write_mixed_feedback` makes, checked rather
    /// than argued: for *any* `sustain` and *any* coupling weight in
    /// `[0, 1]`, feeding the loop back into itself through the coupling
    /// term — the worst case, a bridge perfectly correlated with the string
    /// driving it — stays bounded, because the resulting loop gain is
    /// `loop_gain·(1 + 1 - loop_gain) = 1 - (1 - loop_gain)^2 <= 1`.
    ///
    /// This is the failure mode the *previous* additive coupling had, when
    /// it scaled the external term by `loop_gain` instead: there the same
    /// worst case gives `loop_gain·(1 + weight)`, which exceeds 1 and
    /// diverges as soon as `sustain` approaches 1.
    #[test]
    fn any_coupling_and_sustain_stays_bounded(
        sustain in 0.0f32..=1.0,
        weight in 0.0f32..=1.0,
    ) {
        let mut string = string_at(220.0);
        string.set_sustain(sustain);
        string.pluck(1.0);
        let mut fed_back = 0.0f32;
        for _ in 0..48_000 {
            let tap = string.read_bridge_tap();
            let dispersed = string.disperse(tap);
            fed_back = string.write_mixed_feedback(tap, dispersed, weight * fed_back);
            prop_assert!(fed_back.is_finite());
            prop_assert!(fed_back.abs() < 4.0, "sample {fed_back} escaped");
        }
    }

    /// Totality (`CLAUDE.md` rule 5) for the coupling term specifically:
    /// whatever a caller drives in — NaN, +-infinity, `f32::MAX` — the
    /// string must keep producing a value rather than panicking.
    #[test]
    fn any_coupling_value_never_panics(coupling in proptest::num::f32::ANY) {
        let mut string = string_at(220.0);
        string.pluck(1.0);
        for _ in 0..256 {
            let tap = string.read_bridge_tap();
            let dispersed = string.disperse(tap);
            let _ = string.write_mixed_feedback(tap, dispersed, coupling);
        }
    }

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

    /// Whatever frequency is requested, live retuning never breaks the
    /// string: loop_delay stays within the delay line's reserved capacity
    /// and every subsequent sample stays finite. `hertz` values that fail
    /// `Hz::new` (non-finite or non-positive) are skipped, since
    /// `set_frequency` takes an already-validated `Hz` — this proptest is
    /// about `set_frequency`'s own totality, not `Hz::new`'s, which
    /// `units.rs` covers separately.
    #[test]
    fn set_frequency_is_total_for_any_hertz(hertz in proptest::num::f32::ANY) {
        let mut string = string_at(220.0);
        string.pluck(1.0);
        if let Ok(frequency) = Hz::new(hertz) {
            string.set_frequency(frequency);
        }
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

    /// The same totality guarantee as above, specifically for a short-period
    /// string whose `PendingContact` (a hammer contact outlasting one loop
    /// length) stays active across many `process` calls — the case
    /// `a_short_period_strike_keeps_injecting_contact_force_past_the_first_
    /// loop` exercises for a single velocity, fuzzed here.
    #[test]
    fn a_pending_contact_never_breaks_a_short_period_string(velocity in proptest::num::f32::ANY) {
        let mut string = string_at(4_000.0);
        string.pluck(velocity);
        for _ in 0..2_000 {
            let sample = string.process();
            prop_assert!(sample.is_finite());
            prop_assert!(sample.abs() < 4.0, "sample {sample} escaped");
        }
    }

    /// Whatever strike position a caller sets — NaN, +-infinity, negative,
    /// past the loop's midpoint — the next strike's comb stays finite and
    /// bounded, and the comb delay never reads outside the delay line
    /// (`CLAUDE.md` rule 5, for the strike-position control of issue #32).
    #[test]
    fn any_strike_position_never_breaks_the_string(strike_position in proptest::num::f32::ANY) {
        // Fuzz across the register too: a bass string has the longest loop
        // and so the deepest comb reach, a treble string the shortest and a
        // live `PendingContact` the comb does not touch — both must stay safe.
        for frequency in [55.0, 220.0, 4_000.0] {
            let mut string = harmonic_string_struck_at(frequency, 0.125);
            string.set_strike_position(strike_position);
            string.pluck(1.0);
            for _ in 0..2_000 {
                let sample = string.process();
                prop_assert!(sample.is_finite());
                prop_assert!(sample.abs() < 4.0, "sample {sample} escaped at {frequency} Hz");
            }
        }
    }

    /// Whatever excitation noise mix a caller sets — NaN, +-infinity,
    /// negative, far past 1 — the next strike stays finite and bounded
    /// across the register (`CLAUDE.md` rule 5, for the mix control of
    /// issue #77). Covers a short-period treble string too, whose
    /// `PendingContact` continuation also blends through the same mix.
    #[test]
    fn any_excitation_noise_mix_never_breaks_the_string(mix in proptest::num::f32::ANY) {
        for frequency in [55.0, 220.0, 4_000.0] {
            let mut string = harmonic_string_struck_at(frequency, 0.125);
            string.set_excitation_noise_mix(mix);
            string.pluck(1.0);
            for _ in 0..2_000 {
                let sample = string.process();
                prop_assert!(sample.is_finite());
                prop_assert!(sample.abs() < 4.0, "sample {sample} escaped at {frequency} Hz");
            }
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
