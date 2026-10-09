#![allow(clippy::float_cmp, clippy::unwrap_used, clippy::expect_used)]

use proptest::prelude::*;

use super::*;

const SAMPLE_RATE: f32 = 48_000.0;

#[test]
fn the_fit_places_every_audible_partial_within_a_few_cents_across_the_keyboard() {
    // (fundamental, B) pairs spanning A0 to C8 at physical values. A
    // listener does not hear a single partial a few cents off; the
    // previous design was off by tens of cents to whole semitones.
    for (frequency, b) in [
        (27.5, 0.000_3),
        (55.0, 0.000_15),
        (110.0, 0.000_1),
        (261.6, 0.000_35),
        (440.0, 0.000_45),
        (880.0, 0.001_2),
        (1760.0, 0.003_5),
        (4186.0, 0.015),
    ] {
        let period = SAMPLE_RATE / frequency;
        let target = fit_target(period, b).expect("fittable");
        let cascade = DispersionCascade::new(period, b);
        let cents = target.worst_audible_cents(cascade.design);
        assert!(
            cents < 4.0,
            "{frequency} Hz, B={b}: worst partial {cents} cents off"
        );
    }
}

#[test]
fn zero_inharmonicity_is_an_empty_pass_through() {
    let mut cascade = DispersionCascade::new(SAMPLE_RATE / 440.0, 0.0);
    assert_eq!(cascade.active_sections(), 0);
    assert_eq!(cascade.phase_delay_at_dc(), 0.0);
    for input in [0.3, -0.6, 0.9] {
        assert_eq!(cascade.process(input), input);
    }
}

#[test]
fn the_cascade_never_claims_more_than_its_share_of_the_loop() {
    for frequency in [27.5, 440.0, 2093.0, 4186.0] {
        let period = SAMPLE_RATE / frequency;
        let cascade = DispersionCascade::new(period, MAX_INHARMONICITY);
        assert!(
            cascade.phase_delay_at_dc() <= period * MAX_DELAY_SHARE + 1e-3,
            "{frequency} Hz claimed {} of a {period}-sample loop",
            cascade.phase_delay_at_dc()
        );
    }
}

#[test]
fn set_inharmonicity_refits_without_resetting_state() {
    let mut cascade = DispersionCascade::new(SAMPLE_RATE / 440.0, 0.000_45);
    let _ = cascade.process(1.0);
    let before = cascade.coefficient();
    cascade.set_inharmonicity(0.004);
    assert_ne!(cascade.coefficient(), before);
    assert_ne!(cascade.process(0.0), 0.0, "state was cleared");
}

#[test]
fn set_inharmonicity_from_empty_runs_the_full_fit() {
    let mut cascade = DispersionCascade::new(SAMPLE_RATE / 440.0, 0.0);
    cascade.set_inharmonicity(0.000_45);
    assert!(cascade.active_sections() > 0);
    cascade.set_inharmonicity(0.0);
    assert_eq!(cascade.active_sections(), 0);
}

#[test]
fn reset_clears_every_section() {
    let mut cascade = DispersionCascade::new(SAMPLE_RATE / 27.5, 0.000_3);
    let _ = cascade.process(1.0);
    cascade.reset();
    assert_eq!(cascade.process(0.0), 0.0);
}

proptest! {
    /// The cascade must stay bounded and finite for every reachable
    /// inharmonicity and period, including NaN, +-infinity and zero.
    #[test]
    fn cascade_never_diverges(
        inharmonicity in proptest::num::f32::ANY,
        period in proptest::num::f32::ANY,
        input in -1.0f32..1.0,
    ) {
        let mut cascade = DispersionCascade::new(period, inharmonicity);
        let mut output = 0.0;
        for _ in 0..1_000 {
            output = cascade.process(input);
        }
        prop_assert!(output.is_finite());
        prop_assert!(output.abs() <= 1.0 + 1e-3, "output {output} exceeded input bound");
        prop_assert!(cascade.active_sections() <= MAX_SECTIONS);
        prop_assert!(cascade.phase_delay_at(0.3).is_finite());
    }

    /// The live setter is total too, from any starting design.
    #[test]
    fn live_refit_is_total(
        start in 0.0f32..0.05,
        next in proptest::num::f32::ANY,
        period in 2.0f32..2000.0,
    ) {
        let mut cascade = DispersionCascade::new(period, start);
        cascade.set_inharmonicity(next);
        prop_assert!(cascade.coefficient().abs() <= MAX_COEFFICIENT);
        prop_assert!(cascade.phase_delay_at_dc().is_finite());
    }
}
