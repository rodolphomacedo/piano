//! Tests for [`crate::voicing`], split out of `voicing.rs` to keep that
//! file under the project's 500-line limit (`CONTRIBUTING.md`).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::float_cmp)]

use piano_core::string::PluckedString;
use proptest::prelude::*;

use super::*;

fn sample_rate() -> SampleRate {
    SampleRate::new(48_000.0).expect("48 kHz is valid")
}

fn key(midi: u8) -> PianoKey {
    PianoKey::from_midi(midi).expect("key is on the keyboard")
}

/// The ring-out time this key's *solved* voicing actually delivers for
/// `partial`, read back out of the same loop geometry [`solve_loop_losses`]
/// fits against: total round-trip gain is `sustain · |H(partial·f₀)|`, and
/// a string completes `f₀` round trips a second whatever partial is riding
/// on it.
fn achieved_decay_seconds(midi: u8, partial: f32) -> f32 {
    let tuning = Tuning::default();
    let frequency = key(midi).frequency(tuning).hertz();
    let voicing = voicing_for_key(key(midi), tuning, sample_rate());
    let filter_gain = LoopFilter::new(voicing.damping, voicing.zero_mix)
        .magnitude_at(frequency * partial, sample_rate().hertz());
    let loss = -math::ln(voicing.sustain * filter_gain);
    -math::ln(SILENCE_THRESHOLD) / (frequency * loss)
}

#[test]
fn bass_keys_get_more_inharmonicity_than_default_and_treble_keys_more_still() {
    let tuning = Tuning::default();
    let bass = voicing_for_key(key(LOWEST_PIANO_KEY), tuning, sample_rate());
    let treble = voicing_for_key(key(HIGHEST_PIANO_KEY), tuning, sample_rate());
    assert!(bass.inharmonicity < treble.inharmonicity);
}

#[test]
fn every_key_gets_a_finite_in_range_voicing() {
    let tuning = Tuning::default();
    for midi in LOWEST_PIANO_KEY..=HIGHEST_PIANO_KEY {
        let voicing = voicing_for_key(key(midi), tuning, sample_rate());
        assert!(voicing.damping.is_finite() && (0.0..=1.0).contains(&voicing.damping));
        assert!(voicing.sustain.is_finite() && (0.0..=1.0).contains(&voicing.sustain));
        assert!(
            voicing.inharmonicity.is_finite()
                && (0.0..=MAX_INHARMONICITY).contains(&voicing.inharmonicity)
        );
        assert!(voicing.zero_mix.is_finite() && (0.0..=0.5).contains(&voicing.zero_mix));
    }
}

/// The regression test for the defect this module's three-target solve
/// closes, and the check that would have caught it originally: with the
/// loop filter tuned against the fundamental alone, A0's 8th partial
/// decayed only `1.18x` faster than its fundamental — partials dying
/// together, which is what an organ or a struck bar sounds like, and the
/// reported "metallic". The bands are wide on purpose: they guard against
/// the ratio collapsing back toward 1 (or running away past the `1/n`
/// law), rather than restating the anchors, which `docs/PHYSICS.md`
/// records and `docs/TIMBRE-PLAN.md` explains.
///
/// C8's band is lower than the others because its 8th partial sits at
/// 33 kHz: [`solved_partials`] fits its 5.2nd instead, so the ratio is
/// measured against the same partial the solve actually targeted.
#[test]
fn upper_partials_decay_several_times_faster_than_the_fundamental() {
    for (name, midi, brightness_partial, band) in [
        ("A0", LOWEST_PIANO_KEY, BRIGHTNESS_PARTIAL, 4.0..9.0),
        ("A4", CONCERT_A_KEY, BRIGHTNESS_PARTIAL, 4.0..9.0),
        ("C8", HIGHEST_PIANO_KEY, 5.16, 2.0..6.0),
    ] {
        let fundamental = achieved_decay_seconds(midi, 1.0);
        let bright = achieved_decay_seconds(midi, brightness_partial);
        let ratio = fundamental / bright;
        assert!(
            band.contains(&ratio),
            "{name}: H1 {fundamental:.2}s / H{brightness_partial:.1} {bright:.2}s \
             gives a ratio of {ratio:.2}, outside the documented band {band:?}"
        );
    }
}

/// The other half of the same defect: the old calibration drove `sustain`
/// toward 1.0 and asked the loop filter to carry the whole loss, which is
/// what forced its corner down far enough to take the harmonics with it
/// (`docs/TIMBRE-PLAN.md`, D2). The solve must actually spend `sustain` —
/// it is the only unknown that can attenuate a fundamental at all, since
/// the filter's DC gain is exactly 1.
#[test]
fn the_solve_spends_sustain_rather_than_leaving_it_at_unity() {
    let tuning = Tuning::default();
    for midi in [LOWEST_PIANO_KEY, CONCERT_A_KEY, HIGHEST_PIANO_KEY] {
        let sustain = voicing_for_key(key(midi), tuning, sample_rate()).sustain;
        assert!(
            sustain < 1.0,
            "MIDI {midi} left sustain at {sustain}, so nothing is damping its fundamental"
        );
    }
}

/// Every key's fundamental must land near the ring-out time
/// `docs/PHYSICS.md`'s "Typical decay" row asks for. The tolerance is a
/// factor of two either way: the fit is a weighted least squares across
/// three partials that cannot all be met exactly (see `PARTIAL_WEIGHTS`),
/// and this asserts the compromise stays musical, not that it is exact.
#[test]
fn every_fundamental_lands_within_a_factor_of_two_of_its_target() {
    let tuning = Tuning::default();
    let bass_hz = anchor_hz(LOWEST_PIANO_KEY, tuning);
    let mid_hz = anchor_hz(CONCERT_A_KEY, tuning);
    let treble_hz = anchor_hz(HIGHEST_PIANO_KEY, tuning);
    for midi in LOWEST_PIANO_KEY..=HIGHEST_PIANO_KEY {
        let frequency = key(midi).frequency(tuning).hertz();
        let target = decay_targets_for(
            frequency,
            bass_hz,
            mid_hz,
            treble_hz,
            (BASS_DECAY_SECONDS, MID_DECAY_SECONDS, TREBLE_DECAY_SECONDS),
        )
        .fundamental;
        let achieved = achieved_decay_seconds(midi, 1.0);
        assert!(
            achieved > target * 0.5 && achieved < target * 2.0,
            "MIDI {midi} targets {target:.2}s but the solve delivers {achieved:.2}s"
        );
    }
}

#[test]
fn treble_keys_ring_for_less_time_than_bass_keys() {
    let bass = achieved_decay_seconds(LOWEST_PIANO_KEY, 1.0);
    let treble = achieved_decay_seconds(HIGHEST_PIANO_KEY, 1.0);
    assert!(
        treble < bass,
        "treble decay {treble}s should be shorter than bass decay {bass}s"
    );
}

#[test]
fn config_for_key_uses_the_computed_voicing_not_the_global_default() {
    let tuning = Tuning::default();
    let config = config_for_key(key(HIGHEST_PIANO_KEY), tuning, sample_rate());
    let voicing = voicing_for_key(key(HIGHEST_PIANO_KEY), tuning, sample_rate());
    assert_eq!(config.inharmonicity, voicing.inharmonicity);
    assert_eq!(config.loop_zero_mix, voicing.zero_mix);
}

#[test]
fn bass_keys_are_single_strung_and_treble_keys_are_triple_strung() {
    assert_eq!(unison_count_for_key(key(LOWEST_PIANO_KEY)), 1);
    assert_eq!(unison_count_for_key(key(HIGHEST_PIANO_KEY)), 3);
}

#[test]
fn interpolation_never_produces_non_finite_values_across_odd_tunings() {
    // A pathological but constructible tuning: concert A far from 440,
    // which shifts every anchor frequency. The solve must stay total.
    let tuning = Tuning::with_concert_a(220.0).expect("220 Hz is a valid tuning");
    for midi in LOWEST_PIANO_KEY..=HIGHEST_PIANO_KEY {
        let voicing = voicing_for_key(key(midi), tuning, sample_rate());
        assert!(voicing.damping.is_finite());
        assert!(voicing.sustain.is_finite());
        assert!(voicing.inharmonicity.is_finite());
        assert!(voicing.zero_mix.is_finite());
    }
}

/// `pole_for_axis` claims to invert `a = 4p/(1 − p)²`. Check the round trip
/// rather than trusting the algebra in its doc comment.
#[test]
fn the_pole_axis_inverts_its_own_loss_shape() {
    for axis in [0.0f32, 0.25, 0.5, 0.75, 1.0] {
        let pole = pole_for_axis(axis);
        let recovered = 4.0 * pole / ((1.0 - pole) * (1.0 - pole));
        let low = math::ln(MIN_POLE_SHAPE);
        let high = math::ln(MAX_POLE_SHAPE);
        let expected = math::exp(low + axis * (high - low));
        assert!(
            (recovered / expected - 1.0).abs() < 1e-2,
            "axis {axis} gave pole {pole}, whose shape is {recovered}, not {expected}"
        );
    }
}

/// Same check for `zero_mix_for_axis` and `b = 4z(1 − z)`.
#[test]
fn the_zero_mix_axis_inverts_its_own_loss_shape() {
    for axis in [0.0f32, 0.25, 0.5, 0.75, 1.0] {
        let zero_mix = zero_mix_for_axis(axis);
        let recovered = 4.0 * zero_mix * (1.0 - zero_mix);
        assert!(
            (recovered - axis).abs() < 1e-4,
            "axis {axis} gave zero_mix {zero_mix}, whose shape is {recovered}"
        );
        assert!((0.0..=0.5).contains(&zero_mix));
    }
}

/// The target curve must fall with partial index everywhere, not only at
/// the three anchors — a bulge between them would ask the solver for a
/// filter that rises with frequency.
#[test]
fn the_target_curve_falls_monotonically_with_partial_index() {
    let targets = DecayTargets {
        fundamental: BASS_DECAY_SECONDS,
        mid_partial: BASS_MID_PARTIAL_DECAY_SECONDS,
        brightness: BASS_BRIGHTNESS_DECAY_SECONDS,
    };
    let mut previous = f32::INFINITY;
    for step in 0..64u16 {
        let partial = 1.0 + f32::from(step) * 7.0 / 63.0;
        let seconds = targets.seconds_for_partial(partial);
        assert!(
            seconds < previous,
            "partial {partial} rose to {seconds}s from {previous}s"
        );
        previous = seconds;
    }
}

#[test]
fn c8_is_fitted_below_nyquist_and_a0_gets_the_full_eighth_partial() {
    let tuning = Tuning::default();
    let c8 = solved_partials(
        key(HIGHEST_PIANO_KEY).frequency(tuning).hertz(),
        sample_rate(),
    );
    let a0 = solved_partials(
        key(LOWEST_PIANO_KEY).frequency(tuning).hertz(),
        sample_rate(),
    );
    let highest_c8 = c8.last().copied().expect("three partials");
    assert!(
        (MIN_SOLVED_PARTIAL..BRIGHTNESS_PARTIAL).contains(&highest_c8),
        "C8 was fitted at partial {highest_c8}, which does not fit under Nyquist"
    );
    assert_eq!(
        a0.last().copied().expect("three partials"),
        BRIGHTNESS_PARTIAL
    );
}

/// Renders a real, fully-built `PluckedString` (the same `config_for_key`
/// path `Engine` uses) and returns how many seconds it actually takes to
/// decay to `SILENCE_THRESHOLD`. Samples elapsed, not round trips, is the
/// right unit: `PluckedString::is_silent` tracks a fast, sample-rate-scaled
/// envelope meant for voice reclaiming, not a physics measurement, so
/// converting its own per-sample updates back to seconds is what makes this
/// honest.
fn measured_decay_seconds(midi: u8) -> f32 {
    let config = config_for_key(key(midi), Tuning::default(), sample_rate());
    let mut string = PluckedString::new(config, sample_rate()).expect("key is tunable");
    string.pluck(1.0);

    // `BASS_DECAY_SECONDS` is the longest any anchor's target ever asks
    // for, so a cap of `1.5x` that comfortably bounds every measurement
    // this module takes without the cap itself becoming the thing tested.
    let sample_count_cap = (sample_rate().hertz() * BASS_DECAY_SECONDS * 1.5) as u32;
    let mut samples_elapsed = 0u32;
    while !string.is_silent() && samples_elapsed < sample_count_cap {
        let _ = string.process();
        samples_elapsed += 1;
    }
    samples_elapsed as f32 / sample_rate().hertz()
}

/// The regression test for the report this module's earlier fix shipped
/// for: with the loop filter's zero fixed at Nyquist and a mid-register
/// damping applied uncorrected, a real C8 string measured about 12 ms to
/// silence against a documented 1-2 s target — audibly a click, not a note.
/// Kept because the three-target solve replaced the machinery that closed
/// it, and a rendered string is the only thing that proves the analytic fit
/// survives contact with the hammer and the delay line.
#[test]
fn treble_notes_no_longer_die_in_milliseconds() {
    let measured = measured_decay_seconds(HIGHEST_PIANO_KEY);
    assert!(
        measured > TREBLE_DECAY_SECONDS * 0.7,
        "measured {measured}s should land close to the {TREBLE_DECAY_SECONDS}s target, \
         not the ~12ms the uncalibrated filter and the truncated excitation used to produce"
    );
}

/// A rendered A4 must still be ringing long after the analysis window that
/// used to contain its entire harmonic life — the "thin" half of the report
/// (`docs/TIMBRE-PLAN.md`, D1).
#[test]
fn a_mid_register_note_rings_for_seconds_not_fractions_of_one() {
    let measured = measured_decay_seconds(CONCERT_A_KEY);
    assert!(
        measured > 2.0,
        "A4 fell silent after {measured}s; it should ring for seconds"
    );
}

/// The regression test for `docs/TIMBRE-PLAN.md`'s D5/P1: `registers` used
/// to be parsed and silently discarded. `RegisterOverrides::default()` must
/// reproduce [`voicing_for_key`]'s own output exactly, on every key — the
/// guarantee every other test in this section relies on to isolate what an
/// override actually changes.
#[test]
fn registers_default_matches_voicing_for_key_on_every_key() {
    let tuning = Tuning::default();
    for midi in LOWEST_PIANO_KEY..=HIGHEST_PIANO_KEY {
        let plain = voicing_for_key(key(midi), tuning, sample_rate());
        let with_default_registers = voicing_for_key_with_registers(
            key(midi),
            tuning,
            sample_rate(),
            RegisterOverrides::default(),
        );
        assert_eq!(
            plain.damping, with_default_registers.damping,
            "MIDI {midi} damping diverged with no registers set"
        );
        assert_eq!(plain.sustain, with_default_registers.sustain);
        assert_eq!(plain.inharmonicity, with_default_registers.inharmonicity);
        assert_eq!(plain.zero_mix, with_default_registers.zero_mix);
    }
}

/// The literal bug report: editing `decay_seconds` on the bass register in
/// a `.piano.json` file must change the bass fundamental's achieved ring-out
/// time, and must leave the treble anchor untouched.
#[test]
fn overriding_the_bass_decay_target_moves_the_bass_fundamental_without_moving_treble() {
    let tuning = Tuning::default();
    let baseline = voicing_for_key_with_registers(
        key(LOWEST_PIANO_KEY),
        tuning,
        sample_rate(),
        RegisterOverrides::default(),
    );
    let mut overrides = RegisterOverrides::default();
    overrides.bass.decay_seconds = Some(BASS_DECAY_SECONDS * 3.0);
    let overridden =
        voicing_for_key_with_registers(key(LOWEST_PIANO_KEY), tuning, sample_rate(), overrides);
    assert_ne!(
        baseline.sustain, overridden.sustain,
        "tripling the bass decay target did not change the solved sustain"
    );

    let treble_baseline = voicing_for_key_with_registers(
        key(HIGHEST_PIANO_KEY),
        tuning,
        sample_rate(),
        RegisterOverrides::default(),
    );
    let treble_overridden =
        voicing_for_key_with_registers(key(HIGHEST_PIANO_KEY), tuning, sample_rate(), overrides);
    assert_eq!(
        treble_baseline.sustain, treble_overridden.sustain,
        "overriding the bass anchor moved the treble anchor too"
    );
}

/// An inharmonicity override at one anchor lands there and does not disturb
/// the other anchors.
#[test]
fn overriding_an_inharmonicity_anchor_lands_there_and_leaves_the_others_alone() {
    let tuning = Tuning::default();
    let mut overrides = RegisterOverrides::default();
    overrides.bass.inharmonicity = Some(0.05);
    let bass =
        voicing_for_key_with_registers(key(LOWEST_PIANO_KEY), tuning, sample_rate(), overrides);
    assert!(
        (bass.inharmonicity - 0.05).abs() < 1e-4,
        "bass anchor inharmonicity was {}, not the overridden 0.05",
        bass.inharmonicity
    );

    let plain_treble = voicing_for_key(key(HIGHEST_PIANO_KEY), tuning, sample_rate());
    let treble =
        voicing_for_key_with_registers(key(HIGHEST_PIANO_KEY), tuning, sample_rate(), overrides);
    assert_eq!(
        plain_treble.inharmonicity, treble.inharmonicity,
        "overriding the bass anchor's inharmonicity moved the treble anchor too"
    );
}

/// An unset middle-anchor inharmonicity must fall back to exactly what the
/// original two-point bass-to-treble line already produced there —
/// [`inharmonicity_for`]'s degenerate-to-a-straight-line claim, checked
/// rather than trusted.
#[test]
fn an_unset_middle_inharmonicity_anchor_matches_the_original_two_point_line() {
    let tuning = Tuning::default();
    let bass_hz = anchor_hz(LOWEST_PIANO_KEY, tuning);
    let mid_hz = anchor_hz(CONCERT_A_KEY, tuning);
    let treble_hz = anchor_hz(HIGHEST_PIANO_KEY, tuning);
    let two_point_line = interpolate_log_frequency(
        mid_hz,
        bass_hz,
        BASS_INHARMONICITY,
        treble_hz,
        TREBLE_INHARMONICITY,
    );
    let three_point = inharmonicity_for(
        mid_hz,
        bass_hz,
        mid_hz,
        treble_hz,
        RegisterOverrides::default(),
    );
    assert!(
        (two_point_line - three_point).abs() < 1e-6,
        "an unset middle anchor changed the curve: {two_point_line} vs {three_point}"
    );
}

/// The regression test for [`RegisterAnchorOverride::damping`]'s scope
/// decision: a damping override pins *only* the exact key at that anchor's
/// resolved position, and a neighbouring key keeps its solved value.
#[test]
fn overriding_damping_pins_exactly_the_anchor_key_and_no_other() {
    let tuning = Tuning::default();
    let mut overrides = RegisterOverrides::default();
    overrides.bass.damping = Some(0.123_456);
    let pinned =
        voicing_for_key_with_registers(key(LOWEST_PIANO_KEY), tuning, sample_rate(), overrides);
    assert_eq!(pinned.damping, 0.123_456);

    let neighbour_plain = voicing_for_key(key(LOWEST_PIANO_KEY + 1), tuning, sample_rate());
    let neighbour_overridden =
        voicing_for_key_with_registers(key(LOWEST_PIANO_KEY + 1), tuning, sample_rate(), overrides);
    assert_eq!(
        neighbour_plain.damping, neighbour_overridden.damping,
        "a bass damping pin leaked into the neighbouring key"
    );
}

/// Moving an anchor's position (`anchor_midi`) shifts where its curve sits
/// — a key that used to be past the bass anchor, now short of it, must pick
/// up a fundamental target measurably different from the unmoved baseline.
#[test]
fn moving_an_anchor_position_shifts_where_its_curve_starts() {
    let tuning = Tuning::default();
    let probe = LOWEST_PIANO_KEY + 5;
    let baseline = voicing_for_key_with_registers(
        key(probe),
        tuning,
        sample_rate(),
        RegisterOverrides::default(),
    );
    let mut overrides = RegisterOverrides::default();
    overrides.bass.anchor_midi = Some(probe);
    overrides.bass.decay_seconds = Some(BASS_DECAY_SECONDS * 5.0);
    let moved = voicing_for_key_with_registers(key(probe), tuning, sample_rate(), overrides);
    assert_ne!(
        baseline.sustain, moved.sustain,
        "moving the bass anchor onto this key did not change its own voicing"
    );
}

/// An `anchor_midi` that is not a real piano key (including `0`, the wire
/// format's default for an absent field) must fall back to the built-in
/// anchor position rather than mistuning the whole curve toward whatever
/// `anchor_hz`'s own fallback (the tuning's reference pitch) would give.
#[test]
fn an_invalid_anchor_midi_falls_back_to_the_built_in_position_not_the_reference_pitch() {
    let tuning = Tuning::default();
    let mut overrides = RegisterOverrides::default();
    overrides.bass.anchor_midi = Some(0);
    let with_zero =
        voicing_for_key_with_registers(key(LOWEST_PIANO_KEY), tuning, sample_rate(), overrides);
    let baseline = voicing_for_key(key(LOWEST_PIANO_KEY), tuning, sample_rate());
    assert_eq!(
        baseline.sustain, with_zero.sustain,
        "anchor_midi: 0 moved the bass anchor instead of falling back"
    );
}

proptest! {
    /// Whatever a `.piano.json` file's `registers` block contains —
    /// `NaN`, `±∞`, an out-of-range `anchor_midi`, anything — resolving a
    /// key through it must stay total, the same standard
    /// `solve_loop_losses_is_total` already holds the plain path to.
    #[test]
    fn voicing_for_key_with_registers_is_total(
        midi in proptest::num::u8::ANY,
        bass_anchor_midi in proptest::num::u8::ANY,
        bass_decay_seconds in proptest::num::f32::ANY,
        bass_damping in proptest::num::f32::ANY,
        bass_inharmonicity in proptest::num::f32::ANY,
    ) {
        let Ok(key) = PianoKey::from_midi(midi) else { return Ok(()); };
        let mut overrides = RegisterOverrides::default();
        overrides.bass.anchor_midi = Some(bass_anchor_midi);
        overrides.bass.decay_seconds = Some(bass_decay_seconds);
        overrides.bass.damping = Some(bass_damping);
        overrides.bass.inharmonicity = Some(bass_inharmonicity);

        let voicing =
            voicing_for_key_with_registers(key, Tuning::default(), sample_rate(), overrides);
        prop_assert!(voicing.damping.is_finite());
        prop_assert!(voicing.sustain.is_finite() && (0.0..=1.0).contains(&voicing.sustain));
        prop_assert!(voicing.inharmonicity.is_finite());
        prop_assert!(voicing.zero_mix.is_finite() && (0.0..=0.5).contains(&voicing.zero_mix));
    }
}

proptest! {
    /// `docs/REALTIME-AUDIO-RULES.md`'s totality rule, applied to the
    /// parameter-derivation path: whatever frequency and targets the solve
    /// is handed — `NaN`, `±∞`, zero, negative — it must return
    /// coefficients a waveguide can actually run, in bounded time, rather
    /// than a panic or a value that would poison the loop forever.
    #[test]
    fn solve_loop_losses_is_total(
        frequency in proptest::num::f32::ANY,
        fundamental in proptest::num::f32::ANY,
        mid_partial in proptest::num::f32::ANY,
        brightness in proptest::num::f32::ANY,
    ) {
        let losses = solve_loop_losses(
            frequency,
            DecayTargets { fundamental, mid_partial, brightness },
            sample_rate(),
        );
        prop_assert!(losses.pole.is_finite() && (0.0..=1.0).contains(&losses.pole));
        prop_assert!(losses.zero_mix.is_finite() && (0.0..=0.5).contains(&losses.zero_mix));
        prop_assert!(losses.sustain.is_finite() && (0.0..=1.0).contains(&losses.sustain));
    }
}

/// Prints the solved table for every anchor key. Not a pass/fail — run it
/// with `--nocapture` when re-tuning the anchors, the same way
/// `tests/timbre_diagnostic.rs` is meant to be read.
#[test]
fn report_the_solved_voicing_at_each_anchor() {
    let tuning = Tuning::default();
    println!("\n=== SOLVED VOICING (48 kHz) ===");
    for (name, midi) in [
        ("A0", LOWEST_PIANO_KEY),
        ("A2", 45),
        ("A4", CONCERT_A_KEY),
        ("A6", 93),
        ("C8", HIGHEST_PIANO_KEY),
    ] {
        let frequency = key(midi).frequency(tuning).hertz();
        let voicing = voicing_for_key(key(midi), tuning, sample_rate());
        let targets = decay_targets_for(
            frequency,
            anchor_hz(LOWEST_PIANO_KEY, tuning),
            anchor_hz(CONCERT_A_KEY, tuning),
            anchor_hz(HIGHEST_PIANO_KEY, tuning),
            (BASS_DECAY_SECONDS, MID_DECAY_SECONDS, TREBLE_DECAY_SECONDS),
        );
        print!(
            "{name} f0={frequency:7.1} Hz  pole={:.5} zero_mix={:.5} sustain={:.6}  rendered={:.2}s  ",
            voicing.damping,
            voicing.zero_mix,
            voicing.sustain,
            measured_decay_seconds(midi)
        );
        for partial in solved_partials(frequency, sample_rate()) {
            print!(
                "H{partial:.1}={:.2}s(target {:.2}s) ",
                achieved_decay_seconds(midi, partial),
                targets.seconds_for_partial(partial)
            );
        }
        println!();
    }
}
