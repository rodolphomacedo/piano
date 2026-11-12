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

/// The whole loop's length in samples at the string's own fundamental —
/// what must equal the period for the string to be in tune (issue #96).
fn loop_length_at_fundamental(string: &PluckedString) -> f32 {
    let omega = core::f32::consts::TAU / string.period;
    crate::delay::DelayLine::allpass_read_phase_delay(string.loop_delay(), omega)
        + string.loop_filter.phase_delay_at(omega)
        + string.dispersion.phase_delay_at(omega)
        + 1.0
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
    let total_delay = loop_length_at_fundamental(&string);
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
fn a_hard_strike_on_the_highest_key_still_produces_signal() {
    // C8's loop is a quarter of a millisecond, shorter than the felt stays
    // on the string: the hammer is still there when its own wave returns.
    let mut string = string_at(4_186.0);
    string.pluck(0.9);
    let peak = (0..2_000)
        .map(|_| math::abs(string.process()))
        .fold(0.0f32, f32::max);
    assert!(
        peak > 1e-3,
        "a hard strike on the highest key peaked at {peak}"
    );
}

/// The force a strike at `velocity` puts on `string`, sample by sample,
/// beside the force the same hammer would put on an endless string, from
/// which nothing ever comes back.
fn contact_forces(string: &mut PluckedString, velocity: f32, samples: usize) -> Vec<(f32, f32)> {
    let solver = string.contact;
    let mut alone = hammer::HammerJunction::resting();
    alone.strike(hammer::strike_mps_for(velocity), &solver);
    string.pluck(velocity);
    (0..samples)
        .map(|_| {
            string.process();
            alone.step(&solver, 0.0);
            (string.junction.force(), alone.force())
        })
        .collect()
}

#[test]
fn the_hammer_feels_the_waves_the_string_sends_back() {
    // On C8 the reflections arrive while the felt is still compressed. Were
    // the two force histories alike, the contact would be ignoring the
    // string and every register would be struck by the same pulse.
    let forces = contact_forces(&mut string_at(4_186.0), 0.9, 2_000);
    let peak = forces
        .iter()
        .map(|&(_, alone)| alone)
        .fold(0.0f32, f32::max);
    let apart = forces
        .iter()
        .map(|&(on_string, alone)| math::abs(on_string - alone))
        .fold(0.0f32, f32::max);
    assert!(peak > 1.0, "the strike peaked at only {peak} N");
    assert!(apart > 0.1 * peak, "forces differ by {apart} N of {peak} N");
}

#[test]
fn a_long_string_is_struck_as_if_it_were_endless_until_its_wave_returns() {
    // A0's first reflection, from the agraffe, takes an eighth of 36 ms.
    let forces = contact_forces(&mut string_at(27.5), 0.9, 96);
    for (index, (on_string, alone)) in forces.into_iter().enumerate() {
        assert!(
            math::abs(on_string - alone) <= 1e-3 * alone.max(1.0),
            "sample {index}: {on_string} N on the string, {alone} N alone"
        );
    }
}

/// How long the felt of a strike at `velocity` touches a string at
/// `frequency`, in milliseconds.
fn contact_milliseconds(frequency: f32, velocity: f32) -> f32 {
    let forces = contact_forces(&mut string_at(frequency), velocity, 2_000);
    let touching = forces
        .iter()
        .filter(|&&(on_string, _)| on_string > 0.0)
        .count();
    touching as f32 / 48.0
}

#[test]
fn contact_lasts_milliseconds_in_the_bass_and_a_fraction_of_one_in_the_treble() {
    // A. Askenfelt & E. Jansson, "From touch to string vibrations. I",
    // JASA 88(1), 1990: about 4 ms in the bass, 2 ms in the middle and
    // under 1 ms at the top, at mezzo-forte.
    let (bass, middle, treble) = (
        contact_milliseconds(27.5, 0.55),
        contact_milliseconds(261.6, 0.55),
        contact_milliseconds(4_186.0, 0.55),
    );
    assert!((1.5..5.0).contains(&bass), "bass {bass} ms");
    assert!((1.2..3.0).contains(&middle), "middle {middle} ms");
    assert!((0.2..1.0).contains(&treble), "treble {treble} ms");
}

#[test]
fn a_harder_blow_leaves_the_string_sooner() {
    for frequency in [27.5, 261.6, 880.0, 4_186.0] {
        let (soft, hard) = (
            contact_milliseconds(frequency, 0.3),
            contact_milliseconds(frequency, 1.0),
        );
        assert!(
            hard < soft,
            "{frequency} Hz: {soft} ms soft, {hard} ms hard"
        );
    }
}

#[test]
fn the_hammer_has_left_every_string_within_its_time_limit() {
    for frequency in [27.5, 440.0, 4_186.0] {
        let mut string = string_at(frequency);
        string.pluck(1.0);
        // 30 ms is the longest a strike may last; real ones take 0.5-5 ms.
        for _ in 0..(0.031 * 48_000.0) as usize {
            string.process();
        }
        assert!(!string.junction.is_striking(), "{frequency} Hz");
    }
}

#[test]
fn a_fortissimo_on_one_string_stays_near_full_scale() {
    // A string reports its bridge force over `FULL_SCALE_BRIDGE_NEWTONS`, a
    // scale chosen so that the hardest blow lands near 1.0 across the
    // keyboard instead of at tens of newtons.
    for frequency in [27.5, 110.0, 440.0, 1_760.0, 4_186.0] {
        let mut string = string_at(frequency);
        string.pluck(1.0);
        let peak = (0..4_800)
            .map(|_| math::abs(string.process()))
            .fold(0.0f32, f32::max);
        assert!((0.1..2.5).contains(&peak), "{frequency} Hz peaks at {peak}");
    }
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
fn the_same_strike_renders_the_same_note() {
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
        let total_delay = loop_length_at_fundamental(&string);
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
    let total_delay = loop_length_at_fundamental(&string);
    let period = 48_000.0 / 440.0;
    assert!(
        (total_delay - period).abs() < 1e-3,
        "total delay {total_delay} drifted from period {period}"
    );
}

#[test]
fn set_loop_zero_mix_retunes_the_loop_the_same_way_damping_does() {
    let mut string = string_at(440.0);
    let before = string.loop_delay();
    string.set_loop_zero_mix(0.4);
    assert_ne!(string.loop_delay(), before);
    let total_delay = loop_length_at_fundamental(&string);
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
    let total_delay = loop_length_at_fundamental(&string);
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

/// Samples a released string at 220 Hz takes to fall silent with the given
/// damper strength.
fn samples_to_silence_after_release(strength: f32) -> usize {
    let mut string = string_at(220.0);
    string.set_damper_strength(strength);
    string.pluck(1.0);
    for _ in 0..2_000 {
        string.process();
    }
    string.release();
    (0..200_000)
        .find(|_| {
            string.process();
            string.is_silent()
        })
        .unwrap_or(200_000)
}

#[test]
fn a_stronger_damper_stops_a_released_note_sooner() {
    let weak = samples_to_silence_after_release(MIN_DAMPER_STRENGTH);
    let default = samples_to_silence_after_release(DEFAULT_DAMPER_STRENGTH);
    let strong = samples_to_silence_after_release(MAX_DAMPER_STRENGTH);
    assert!(
        strong < default && default < weak,
        "{strong} / {default} / {weak}"
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
    // At 4 kHz the loop is a dozen samples, shorter than the felt's contact,
    // so the hammer keeps pushing while its own waves come back round. The
    // energy after the first loop must then be a real share of the first
    // loop's own, not the tail of a burst that was over before it returned.
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
    // A hammer at 1/8 of the string sits on a node of the 8th partial and
    // cannot excite it. Measured against the same partial of the same string
    // struck at 1/10, where it is 59 % of its largest, so the loop's own
    // spectral tilt is common to both sides.
    let fundamental = 220.0;
    let h8 = |strike: f32| {
        let mut string = harmonic_string_struck_at(fundamental, strike);
        let attack = render_attack(&mut string, 4_096);
        magnitude_at(&attack, fundamental * 8.0, 48_000.0)
    };
    let (on_the_node, off_it) = (h8(0.125), h8(0.1));
    assert!(
        on_the_node < 0.25 * off_it,
        "{on_the_node} against {off_it}"
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
fn a_silenced_string_struck_again_reproduces_the_attack_sample_for_sample() {
    // Nothing in a strike is drawn at random: the same blow on the same
    // string at rest is the same note.
    let mut string = string_at(196.0);
    let first = render_attack(&mut string, 4_096);
    string.silence();
    let second = render_attack(&mut string, 4_096);
    assert_eq!(first, second, "the second strike diverged from the first");
}

#[test]
fn a_string_struck_again_while_ringing_is_not_struck_from_rest() {
    // The hammer meets a string that is still moving, as on the instrument,
    // so a repeated note is not a copy of the first.
    let mut string = string_at(196.0);
    let first = render_attack(&mut string, 4_096);
    let second = render_attack(&mut string, 4_096);
    assert_ne!(
        first, second,
        "the string was cleared before the second strike"
    );
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

    /// The same totality guarantee as above for a string whose loop is
    /// shorter than the hammer's contact, so the felt is still compressed
    /// when its own waves return — the case
    /// `a_short_period_strike_keeps_injecting_contact_force_past_the_first_
    /// loop` exercises for a single velocity, fuzzed here.
    #[test]
    fn a_contact_outlasting_the_loop_never_breaks_a_short_string(velocity in proptest::num::f32::ANY) {
        let mut string = string_at(4_000.0);
        string.pluck(velocity);
        for _ in 0..2_000 {
            let sample = string.process();
            prop_assert!(sample.is_finite());
            prop_assert!(sample.abs() < 4.0, "sample {sample} escaped");
        }
    }

    /// Whatever strike position a caller sets — NaN, +-infinity, negative,
    /// past the string's midpoint — the next strike stays finite and
    /// bounded, and the hammer never reaches outside the delay line
    /// (`CLAUDE.md` rule 5, for the strike-position control of issue #32).
    #[test]
    fn any_strike_position_never_breaks_the_string(strike_position in proptest::num::f32::ANY) {
        // Fuzz across the register too: a bass string has the longest loop
        // and so the furthest strike tap, a treble string a loop shorter
        // than its own contact — both must stay safe.
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

    /// Any damper strength, `NaN` and infinities included, leaves a
    /// released string finite and bounded.
    #[test]
    fn any_damper_strength_keeps_the_string_bounded(strength in proptest::num::f32::ANY) {
        let mut string = string_at(220.0);
        string.pluck(1.0);
        string.set_damper_strength(strength);
        string.release();
        for _ in 0..2_000 {
            let sample = string.process();
            prop_assert!(sample.is_finite());
            prop_assert!(sample.abs() < 4.0, "sample {sample} escaped at {strength}");
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
    // Long enough for the struck wave to have reached the bridge.
    let mut buffer = [1.0f32; 256];
    string.process_block_add(&mut buffer);
    assert!(buffer.iter().any(|sample| (sample - 1.0).abs() > 1e-6));
}

/// Share of an attack's energy in its sample-to-sample differences: a
/// first difference is a +6 dB/octave tilt, so a brighter attack scores
/// higher whatever its level, with no FFT needed.
fn attack_brightness(string: &mut PluckedString, velocity: f32) -> f32 {
    string.pluck(velocity);
    let attack: Vec<f32> = (0..2_400).map(|_| string.process()).collect();
    let energy: f32 = attack.iter().map(|s| s * s).sum();
    let slope: f32 = attack
        .iter()
        .zip(attack.iter().skip(1))
        .map(|(before, after)| (after - before).powi(2))
        .sum();
    slope / energy
}

/// The felt's hardness reaches the string: the same blow through lacquered
/// felt is brighter than through needled felt (#82).
#[test]
fn harder_felt_brightens_the_attack() {
    let brightness_with = |hardness: f32| {
        let mut string = string_at(220.0);
        let felt = hammer::hammer_for_frequency(220.0);
        string.set_hammer(hammer::HammerConfig {
            stiffness: hardness * felt.stiffness,
            ..felt
        });
        attack_brightness(&mut string, 0.7)
    };
    let (needled, lacquered) = (brightness_with(0.25), brightness_with(4.0));
    assert!(
        lacquered > 1.2 * needled,
        "needled {needled}, lacquered {lacquered}"
    );
}

/// What makes a piano's loud notes loud is not only level: the felt
/// stiffens as it is compressed, so a harder blow is a shorter one.
#[test]
fn a_harder_blow_is_brighter_and_not_only_louder() {
    let soft = attack_brightness(&mut string_at(220.0), 0.2);
    let loud = attack_brightness(&mut string_at(220.0), 1.0);
    assert!(loud > 1.2 * soft, "soft {soft}, loud {loud}");
}
