#![allow(clippy::float_cmp, clippy::unwrap_used, clippy::expect_used)]

use proptest::prelude::*;

use super::*;

const SAMPLE_RATE: f32 = 48_000.0;

/// What one strike against a string with no reflections did.
struct Strike {
    peak_newtons: f32,
    contact_seconds: f32,
    impulse: f32,
    radiated_joules: f32,
    rebound_mps: f32,
}

/// Strikes an endless string — nothing ever comes back — at `strike_mps`.
fn strike_endless_string(hammer: HammerConfig, sample_rate: f32, strike_mps: f32) -> Strike {
    let solver = ContactSolver::new(hammer, sample_rate);
    let mut junction = HammerJunction::resting();
    junction.strike(strike_mps, &solver);
    let step = 1.0 / usable_sample_rate(sample_rate);
    let (mut peak, mut impulse, mut radiated, mut touching) = (0.0f32, 0.0f32, 0.0f32, 0u32);
    let mut rebound = 0.0;
    let mut previous = 0.0f32;
    while junction.is_striking() {
        junction.step(&solver, 0.0);
        let force = junction.force();
        peak = peak.max(force);
        impulse += f32::midpoint(previous, force) * step;
        radiated += force * force / (2.0 * hammer.string_impedance) * step;
        touching += u32::from(force > 0.0);
        rebound = if junction.is_striking() {
            junction.velocity
        } else {
            rebound
        };
        previous = force;
    }
    Strike {
        peak_newtons: peak,
        contact_seconds: touching as f32 * step,
        impulse,
        radiated_joules: radiated,
        rebound_mps: rebound,
    }
}

/// The contact equation solved by bisection, as slowly and surely as it can be.
fn bisected_compression(solver: &ContactSolver, unloaded: f32) -> f32 {
    let (mut low, mut high) = (0.0f64, f64::from(unloaded));
    let gain = f64::from(solver.stiffness_gain);
    let exponent = f64::from(solver.exponent);
    for _ in 0..200 {
        let middle = f64::midpoint(low, high);
        if middle + gain * middle.powf(exponent) < f64::from(unloaded) {
            low = middle;
        } else {
            high = middle;
        }
    }
    f64::midpoint(low, high) as f32
}

#[test]
fn a_resting_hammer_adds_nothing_and_stays_at_rest() {
    let solver = ContactSolver::new(DEFAULT_HAMMER, SAMPLE_RATE);
    let mut junction = HammerJunction::resting();
    for incoming in [0.0, 1.0, -3.0, f32::NAN, f32::INFINITY] {
        assert_eq!(junction.step(&solver, incoming), 0.0);
    }
    assert!(!junction.is_striking());
    assert_eq!(junction, HammerJunction::default());
}

#[test]
fn a_strike_without_speed_leaves_the_hammer_at_rest() {
    let solver = ContactSolver::new(DEFAULT_HAMMER, SAMPLE_RATE);
    for speed in [0.0, -1.0, f32::NAN, f32::NEG_INFINITY] {
        let mut junction = HammerJunction::resting();
        junction.strike(speed, &solver);
        assert!(!junction.is_striking(), "speed {speed} started a strike");
    }
}

#[test]
fn the_hammer_loses_exactly_the_momentum_it_gives_the_string() {
    for strike_mps in [0.5, 2.0, 4.5] {
        let strike = strike_endless_string(DEFAULT_HAMMER, SAMPLE_RATE, strike_mps);
        let lost = DEFAULT_HAMMER.mass * (strike_mps - strike.rebound_mps);
        let error = (strike.impulse - lost).abs() / lost;
        assert!(
            error < 1e-3,
            "at {strike_mps} m/s the string received {} N·s, the hammer lost {lost}",
            strike.impulse
        );
    }
}

#[test]
fn the_energy_the_hammer_loses_is_the_energy_the_string_carries_away() {
    for strike_mps in [0.5, 2.0, 4.5] {
        let strike = strike_endless_string(DEFAULT_HAMMER, SAMPLE_RATE, strike_mps);
        let before = 0.5 * DEFAULT_HAMMER.mass * strike_mps * strike_mps;
        let after = 0.5 * DEFAULT_HAMMER.mass * strike.rebound_mps * strike.rebound_mps;
        let error = (strike.radiated_joules - (before - after)).abs() / before;
        assert!(
            error < 0.02,
            "at {strike_mps} m/s: radiated {} J, hammer lost {} J",
            strike.radiated_joules,
            before - after
        );
    }
}

#[test]
fn a_harder_blow_is_shorter_and_more_than_proportionally_stronger() {
    let soft = strike_endless_string(DEFAULT_HAMMER, SAMPLE_RATE, 1.0);
    let hard = strike_endless_string(DEFAULT_HAMMER, SAMPLE_RATE, 4.0);
    assert!(
        hard.contact_seconds < soft.contact_seconds,
        "contact {} s soft, {} s hard",
        soft.contact_seconds,
        hard.contact_seconds
    );
    assert!(
        hard.peak_newtons > 4.0 * soft.peak_newtons,
        "peak {} N soft, {} N hard",
        soft.peak_newtons,
        hard.peak_newtons
    );
}

#[test]
fn the_strike_does_not_depend_on_the_sample_rate() {
    for frequency in [27.5, 261.6, 2093.0] {
        let hammer = hammer_for_frequency(frequency);
        let coarse = strike_endless_string(hammer, 44_100.0, 3.0);
        let fine = strike_endless_string(hammer, 384_000.0, 3.0);
        let peak_error = (coarse.peak_newtons - fine.peak_newtons).abs() / fine.peak_newtons;
        assert!(
            peak_error < 0.03,
            "{frequency} Hz: peak {} N at 44.1 kHz, {} N at 384 kHz",
            coarse.peak_newtons,
            fine.peak_newtons
        );
        let one_sample = 1.0 / 44_100.0;
        assert!((coarse.contact_seconds - fine.contact_seconds).abs() <= 2.0 * one_sample);
    }
}

#[test]
fn a_string_already_moving_away_softens_the_blow() {
    let solver = ContactSolver::new(DEFAULT_HAMMER, SAMPLE_RATE);
    let peak_against = |incoming: f32| {
        let mut junction = HammerJunction::resting();
        junction.strike(2.0, &solver);
        let mut peak = 0.0f32;
        while junction.is_striking() {
            junction.step(&solver, incoming);
            peak = peak.max(junction.force());
        }
        peak
    };
    assert!(peak_against(1.0) < peak_against(0.0));
    assert!(peak_against(-1.0) > peak_against(0.0));
}

#[test]
fn every_strike_ends_within_its_time_limit() {
    // A hammer far heavier than its string never turns back on its own.
    let stubborn = HammerConfig {
        mass: MAX_MASS,
        string_impedance: MIN_STRING_IMPEDANCE,
        ..DEFAULT_HAMMER
    };
    let solver = ContactSolver::new(stubborn, SAMPLE_RATE);
    let mut junction = HammerJunction::resting();
    junction.strike(1.0, &solver);
    let mut steps = 0u32;
    while junction.is_striking() {
        junction.step(&solver, 0.0);
        steps += 1;
        assert!(
            steps <= solver.max_steps,
            "still striking after {steps} steps"
        );
    }
}

#[test]
fn key_velocity_maps_to_hammer_speed_through_zero_and_up_to_the_maximum() {
    assert_eq!(strike_mps_for(0.0), 0.0);
    assert_eq!(strike_mps_for(1.0), MAX_STRIKE_MPS);
    assert_eq!(strike_mps_for(7.0), MAX_STRIKE_MPS);
    assert_eq!(strike_mps_for(f32::NAN), 0.0);
    assert!(strike_mps_for(0.3) < strike_mps_for(0.6));
}

#[test]
fn the_keyboards_hammers_get_lighter_harder_and_more_nonlinear_towards_the_treble() {
    let mut previous = hammer_for_frequency(55.0);
    for frequency in [110.0, 220.0, 440.0, 880.0, 1760.0, 3520.0] {
        let hammer = hammer_for_frequency(frequency);
        assert!(hammer.mass < previous.mass, "{frequency} Hz");
        assert!(
            hammer.contact_exponent > previous.contact_exponent,
            "{frequency} Hz"
        );
        assert!(
            hammer.string_impedance <= previous.string_impedance,
            "{frequency} Hz"
        );
        previous = hammer;
    }
    let (bass, treble) = (hammer_for_frequency(27.5), hammer_for_frequency(4186.0));
    assert!(treble.stiffness > 1_000.0 * bass.stiffness);
}

#[test]
fn a_string_of_a_trichord_feels_a_third_of_its_hammer() {
    // A4: three strings share a hammer of about 7 g.
    let share = hammer_for_frequency(440.0);
    assert!((2.0e-3..2.7e-3).contains(&share.mass), "{} kg", share.mass);
    // A0: one string takes the whole 10.5 g.
    let whole = hammer_for_frequency(27.5);
    assert!((whole.mass - 10.5e-3).abs() < 1e-5, "{} kg", whole.mass);
}

#[test]
fn a_hammer_outside_every_range_is_clamped_rather_than_trusted() {
    let hostile = HammerConfig {
        contact_exponent: f32::NAN,
        stiffness: f32::INFINITY,
        mass: -1.0,
        string_impedance: 0.0,
    };
    let strike = strike_endless_string(hostile, f32::NAN, 2.0);
    assert!(strike.peak_newtons.is_finite());
    assert!(strike.impulse.is_finite());
}

proptest! {
    /// The four fixed Newton steps land on the root bisection finds, for
    /// every hammer, string and compression the solver can be handed.
    #[test]
    fn the_fixed_newton_steps_reach_the_root(
        exponent in MIN_CONTACT_EXPONENT..MAX_CONTACT_EXPONENT,
        log_stiffness in 6.0f32..14.0,
        log_mass in -4.0f32..-1.0,
        log_impedance in -1.0f32..3.0,
        log_unloaded in -9.0f32..-1.4,
        sample_rate in 22_050.0f32..192_000.0,
    ) {
        let hammer = HammerConfig {
            contact_exponent: exponent,
            stiffness: 10.0f32.powf(log_stiffness),
            mass: 10.0f32.powf(log_mass),
            string_impedance: 10.0f32.powf(log_impedance),
        };
        let solver = ContactSolver::new(hammer, sample_rate);
        let unloaded = 10.0f32.powf(log_unloaded);
        let (compression, force) = solver.resolve(unloaded);
        let exact = bisected_compression(&solver, unloaded);
        prop_assert!(
            (compression - exact).abs() <= 1e-3 * exact + 1e-12,
            "newton {compression}, bisection {exact}"
        );
        prop_assert!(force >= 0.0 && force.is_finite());
    }

    /// Whatever the hammer, the string and the incoming wave — `NaN` and
    /// `±∞` included — a step returns a finite number and the strike ends.
    #[test]
    fn a_step_is_total(
        exponent in proptest::num::f32::ANY,
        stiffness in proptest::num::f32::ANY,
        mass in proptest::num::f32::ANY,
        impedance in proptest::num::f32::ANY,
        sample_rate in proptest::num::f32::ANY,
        strike_mps in proptest::num::f32::ANY,
        incoming in proptest::collection::vec(proptest::num::f32::ANY, 1..64),
    ) {
        let hammer = HammerConfig {
            contact_exponent: exponent,
            stiffness,
            mass,
            string_impedance: impedance,
        };
        let solver = ContactSolver::new(hammer, sample_rate);
        prop_assert!(solver.bridge_gain().is_finite());
        let mut junction = HammerJunction::resting();
        junction.strike(strike_mps, &solver);
        let mut steps = 0u32;
        for wave in incoming.iter().cycle() {
            if !junction.is_striking() {
                break;
            }
            let injected = junction.step(&solver, *wave);
            prop_assert!(injected.is_finite(), "injected {injected}");
            prop_assert!(junction.force().is_finite());
            steps += 1;
            prop_assert!(steps <= solver.max_steps);
        }
    }
}
