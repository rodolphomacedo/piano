# Hammer↔String Coupling (issue #57) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the felt hammer's contact force respond to the string's own motion — a scattering-junction coupling between the hammer spring and the string's characteristic impedance, applied with the same formula for every string on the keyboard.

**Architecture:** A new pure, total function `piano_core::hammer::couple_contact_step` replaces the direct `contact_force[index]` lookup `PluckedString::write_excitation` and `PluckedString::next_contact_sample` use today, resolving each sample's force with a hard-capped (3-step) fixed-point iteration against `v_incoming` — the delay line's own current content. `hammer::simulate_contact` is untouched: it stays the reference curve for spectral cutoff sizing and normalisation.

**Tech Stack:** Rust, `no_std` + `alloc` in `piano-core`, `proptest` for totality, `criterion` for the new bench.

**Spec:** `docs/superpowers/specs/2026-09-13-hammer-string-coupling-design.md`

## Global Constraints

- The audio thread allocates nothing, locks nothing, panics nowhere, and has no unbounded loop (`docs/REALTIME-AUDIO-RULES.md`). `couple_contact_step`'s fixed-point refinement is a hard-capped `for _ in 0..COUPLING_FIXPOINT_STEPS`, never a `while !converged`.
- Every hot-path function is total: defined for `NaN`, `±∞`, zero and `usize::MAX`, proven by `proptest`, not argued.
- No `unwrap`, `expect`, `panic!` or `unimplemented!` outside test modules.
- No copyleft source may be read for this work — implement from Chaigne & Askenfelt (1994) and the citation already in `write_mixed_feedback`'s doc comment (J. O. Smith III), never from another project's code.
- `simulate_contact` is unchanged in signature and behaviour — every existing test in `hammer.rs` must keep passing untouched.
- `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `cargo build -p piano-core --no-default-features` must all be clean before this is considered done (`CLAUDE.md`).

---

### Task 1: Coupled contact-step primitives in `hammer.rs`

**Files:**
- Modify: `crates/piano-core/src/hammer.rs`

**Interfaces:**
- Consumes: `crate::math::{clamp_or_low, powf}` (already imported in this file).
- Produces: `pub(crate) struct ContactState { compression: f32, hammer_velocity: f32, separated: bool }` with `pub(crate) fn starting(velocity: f32) -> ContactState`; `pub(crate) fn couple_contact_step(state: ContactState, hammer: HammerConfig, v_incoming: f32, sample_rate_hz: f32) -> (ContactState, f32)` (returns a **raw**, non-normalised force); `pub(crate) fn uncoupled_contact_curve(velocity: f32, sample_rate_hz: f32, hammer: HammerConfig) -> ([f32; MAX_CONTACT_SAMPLES], usize, f32)` (curve, active count, peak); `pub(crate) fn normalize_by_peak(curve: &mut [f32; MAX_CONTACT_SAMPLES], active: usize, peak: f32)`; `HammerConfig::string_impedance: f32` (new pub field); `pub(crate) const MIN_STRING_IMPEDANCE: f32` / `MAX_STRING_IMPEDANCE: f32`. These six names are what Task 2 imports from `crate::hammer`.

- [ ] **Step 1: Write the failing tests**

Add to `hammer.rs`'s `#[cfg(test)] mod tests`, after the existing `a_stiffer_hammer_gets_a_brighter_excitation_than_a_softer_one` test:

```rust
    #[test]
    fn with_no_incoming_wave_and_impedance_at_its_ceiling_coupling_reproduces_the_uncoupled_curve() {
        // At v_incoming ≡ 0 and string_impedance at its ceiling, F/Z_c is
        // negligible, so v_string ≈ 0 throughout — exactly `simulate_contact`'s
        // own assumption (the string never moves). The two independently
        // written implementations must therefore agree, sample for sample
        // once each is normalised to its own peak, or they have silently
        // diverged from the same physical model.
        let hammer = HammerConfig {
            string_impedance: MAX_STRING_IMPEDANCE,
            ..DEFAULT_HAMMER
        };
        let (reference, active) = simulate_contact(0.6, 48_000.0, hammer);

        let mut state = ContactState::starting(0.6);
        let mut coupled = [0.0f32; MAX_CONTACT_SAMPLES];
        for sample in coupled.iter_mut().take(active) {
            let (next_state, force) = couple_contact_step(state, hammer, 0.0, 48_000.0);
            state = next_state;
            *sample = force;
        }
        let coupled_peak = coupled.iter().take(active).copied().fold(0.0f32, f32::max);
        assert!(coupled_peak > f32::EPSILON, "coupled curve never left zero");

        for index in 0..active {
            let expected = reference[index];
            let got = coupled[index] / coupled_peak;
            assert!(
                (expected - got).abs() < 1e-3,
                "sample {index}: reference {expected} vs coupled {got}"
            );
        }
    }

    #[test]
    fn contact_ends_and_stays_ended_once_compression_returns_to_zero() {
        let mut state = ContactState::starting(0.7);
        let hammer = DEFAULT_HAMMER;
        let mut saw_separation = false;
        for _ in 0..MAX_CONTACT_SAMPLES {
            let (next_state, _) = couple_contact_step(state, hammer, 0.0, 48_000.0);
            if next_state.separated {
                saw_separation = true;
                // Once separated, further calls must return exactly zero
                // force and never re-engage, whatever v_incoming does.
                let (still, force) = couple_contact_step(next_state, hammer, 5.0, 48_000.0);
                assert_eq!(force, 0.0);
                assert!(still.separated);
                break;
            }
            state = next_state;
        }
        assert!(saw_separation, "contact never ended within the sample cap");
    }

    #[test]
    fn a_returning_wave_changes_the_force_a_still_ringing_string_gets() {
        // The whole point of #57: two otherwise identical strikes differ
        // once one of them has real energy coming back through the loop.
        let hammer = DEFAULT_HAMMER;
        let state = ContactState::starting(0.6);
        let (_, silent) = couple_contact_step(state, hammer, 0.0, 48_000.0);
        let (_, loaded) = couple_contact_step(state, hammer, 0.3, 48_000.0);
        assert!(
            (silent - loaded).abs() > f32::EPSILON,
            "a nonzero v_incoming produced the same force as silence"
        );
    }

    proptest! {
        /// `couple_contact_step` must never panic, loop unboundedly, or
        /// produce a non-finite state or force, for every reachable
        /// compression, hammer velocity, `v_incoming` and `HammerConfig` —
        /// including NaN, +-infinity and zero in every field.
        #[test]
        fn couple_contact_step_is_total(
            compression in proptest::num::f32::ANY,
            hammer_velocity in proptest::num::f32::ANY,
            v_incoming in proptest::num::f32::ANY,
            sample_rate_hz in proptest::num::f32::ANY,
            contact_exponent in proptest::num::f32::ANY,
            stiffness in proptest::num::f32::ANY,
            mass in proptest::num::f32::ANY,
            string_impedance in proptest::num::f32::ANY,
        ) {
            let state = ContactState {
                compression,
                hammer_velocity,
                separated: false,
            };
            let hammer = HammerConfig {
                contact_exponent,
                stiffness,
                mass,
                string_impedance,
            };
            let (next_state, force) = couple_contact_step(state, hammer, v_incoming, sample_rate_hz);
            prop_assert!(next_state.compression.is_finite());
            prop_assert!(next_state.hammer_velocity.is_finite());
            prop_assert!(force.is_finite());
            prop_assert!(next_state.compression >= 0.0);
        }
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p piano-core --lib hammer:: 2>&1 | tail -40`
Expected: FAIL to compile — `ContactState`, `couple_contact_step`, `MAX_STRING_IMPEDANCE` and `HammerConfig::string_impedance` do not exist yet.

- [ ] **Step 3: Add `string_impedance` to `HammerConfig` and its bounds**

Modify the `HammerConfig` struct (currently lines 117-126):

```rust
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HammerConfig {
    /// Hertzian contact exponent, `F = K·x^p`. See [`CONTACT_EXPONENT`]'s
    /// doc comment for the physical range this represents.
    pub contact_exponent: f32,
    /// Hertzian contact stiffness, in the model's own normalised units.
    pub stiffness: f32,
    /// Hammer mass, normalised to 1 at the default.
    pub mass: f32,
    /// The string's own characteristic impedance at the contact point, in
    /// the model's own normalised units — how strongly the string pushes
    /// back on the hammer (`docs/superpowers/specs/
    /// 2026-09-13-hammer-string-coupling-design.md`). Large values make
    /// the string act as the rigid wall this model assumed before #57;
    /// [`MAX_STRING_IMPEDANCE`] is chosen to reproduce that curve exactly
    /// (`with_no_incoming_wave_and_impedance_at_its_ceiling_coupling_
    /// reproduces_the_uncoupled_curve`), so [`DEFAULT_HAMMER`] starts
    /// there rather than pre-empting the calibration this field still
    /// needs.
    pub string_impedance: f32,
}
```

Modify `DEFAULT_HAMMER` (currently lines 131-135):

```rust
pub const DEFAULT_HAMMER: HammerConfig = HammerConfig {
    contact_exponent: CONTACT_EXPONENT,
    stiffness: CONTACT_STIFFNESS,
    mass: HAMMER_MASS,
    string_impedance: MAX_STRING_IMPEDANCE,
};
```

Add new bounds constants after `MAX_MASS` (currently line 93):

```rust
/// Widest [`HammerConfig::string_impedance`] `sanitize_hammer` allows.
///
/// Large enough that `force / string_impedance` is negligible next to a
/// hammer velocity in `[MIN_STRIKE_MPS, MAX_STRIKE_MPS]` for any force this
/// model's `stiffness`/`mass`/`contact_exponent` range can produce — the
/// ceiling [`DEFAULT_HAMMER`] starts at, and the value
/// `with_no_incoming_wave_and_impedance_at_its_ceiling_coupling_reproduces_
/// the_uncoupled_curve` checks against `simulate_contact`. Provisional:
/// `docs/superpowers/specs/2026-09-13-hammer-string-coupling-design.md`
/// flags the exact register-by-register calibration of this field as an
/// open question for the validation task, same as `CONTACT_STIFFNESS` was
/// before it was checked empirically.
pub(crate) const MAX_STRING_IMPEDANCE: f32 = 1.0e13;
/// Lowest [`HammerConfig::string_impedance`] `sanitize_hammer` allows —
/// bounded away from zero for the same reason [`MIN_MASS`] is: it is a
/// divisor in [`couple_contact_step`]'s `force / string_impedance` term.
pub(crate) const MIN_STRING_IMPEDANCE: f32 = 1.0e6;
```

Modify `sanitize_hammer` (currently lines 166-176) to clamp the new field:

```rust
fn sanitize_hammer(hammer: HammerConfig) -> HammerConfig {
    HammerConfig {
        contact_exponent: math::clamp_or_low(
            hammer.contact_exponent,
            MIN_CONTACT_EXPONENT,
            MAX_CONTACT_EXPONENT,
        ),
        stiffness: math::clamp_or_low(hammer.stiffness, MIN_STIFFNESS, MAX_STIFFNESS),
        mass: math::clamp_or_low(hammer.mass, MIN_MASS, MAX_MASS),
        string_impedance: math::clamp_or_low(
            hammer.string_impedance,
            MIN_STRING_IMPEDANCE,
            MAX_STRING_IMPEDANCE,
        ),
    }
}
```

Every other existing `HammerConfig { .. }` struct literal in this file's tests (e.g. `a_stiffer_hammer_produces_a_shorter_contact`'s `soft`/`stiff`) already uses `..DEFAULT_HAMMER`, so they need no changes — confirm with:

```sh
grep -n "HammerConfig {" crates/piano-core/src/hammer.rs
```

- [ ] **Step 4: Extract `strike_mps_for` and add `ContactState`**

Just above `simulate_contact` (currently starting at line 330), add:

```rust
/// Converts the public `[0, 1]` strike velocity into metres/second, the
/// unit [`simulate_contact`] and [`couple_contact_step`] both integrate
/// in. Shared so the two cannot quietly disagree about what a given
/// velocity means.
#[inline]
fn strike_mps_for(velocity: f32) -> f32 {
    let velocity = math::clamp_or_low(velocity, 0.0, 1.0);
    MIN_STRIKE_MPS + velocity * (MAX_STRIKE_MPS - MIN_STRIKE_MPS)
}

/// How many fixed-point passes [`couple_contact_step`] runs to resolve one
/// sample's force against the string's own returning velocity.
///
/// The force sets `v_string` (`v_incoming + force / string_impedance`),
/// which sets the trial compression the force is then recomputed from —
/// an implicit relationship the *hard-capped* iteration below approximates
/// rather than solving exactly, per `PERF-007` in `docs/PERFORMANCE.md`
/// ("a bounded fixed-point iteration (2-4 steps, hard capped)"). `3` is a
/// starting point, not yet a measured one; the validation task checks
/// whether the force estimate has actually stabilised by this point across
/// the velocity/impedance range this model allows, the same way
/// `CONTACT_STIFFNESS` was checked against simulated output rather than
/// derived.
const COUPLING_FIXPOINT_STEPS: usize = 3;

/// The hammer's own state between one coupled contact sample and the next.
///
/// Carries what [`PendingContact`](crate::string) used to leave to a plain
/// array index: [`couple_contact_step`] needs the compression and hammer
/// velocity a precomputed curve never exposed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ContactState {
    pub(crate) compression: f32,
    pub(crate) hammer_velocity: f32,
    /// Once `true`, the hammer has left the string for the rest of this
    /// strike — checked by [`couple_contact_step`]'s caller *before*
    /// calling again, the same role `PendingContact::next_index >=
    /// contact_samples` plays for the uncoupled curve today.
    pub(crate) separated: bool,
}

impl ContactState {
    /// The hammer's state the instant it first touches the string at
    /// `velocity`: no compression yet, moving at `strike_mps_for(velocity)`.
    #[must_use]
    pub(crate) fn starting(velocity: f32) -> Self {
        Self {
            compression: 0.0,
            hammer_velocity: strike_mps_for(velocity),
            separated: false,
        }
    }
}

/// Advances a coupled hammer/string contact by one sample and returns the
/// force it produced.
///
/// Treats the contact point as a scattering junction between the felt
/// spring and the string's own characteristic impedance
/// (`docs/superpowers/specs/2026-09-13-hammer-string-coupling-design.md`):
/// `v_string = v_incoming + force / string_impedance`, where `v_incoming`
/// is whatever the string is already carrying back to this point this
/// sample (silence for most of a fresh strike; real, measured content
/// once a round trip has had time to return — see the design doc for why
/// that difference is what separates the bass from the treble here without
/// a register-specific branch anywhere in this function).
///
/// `force` sets `v_string`, which sets the compression `force` is in turn
/// derived from — resolved by [`COUPLING_FIXPOINT_STEPS`] fixed-point
/// passes, never an unbounded solve, before committing one
/// [`simulate_contact`]-style semi-implicit Euler step with the converged
/// value. Once `state.separated`, returns `state` unchanged and a force of
/// exactly `0.0` without doing any further work — contact has ended and
/// must not numerically re-engage.
///
/// Total for every input: `hammer` is sanitised by [`sanitize_hammer`],
/// `v_incoming` and `sample_rate_hz` are checked for finiteness the same
/// way [`simulate_contact`]'s inputs already are, and every intermediate
/// compression is clamped into `[0.0, MAX_COMPRESSION]` on every pass —
/// proven by `couple_contact_step_is_total`, not argued.
#[must_use]
pub(crate) fn couple_contact_step(
    state: ContactState,
    hammer: HammerConfig,
    v_incoming: f32,
    sample_rate_hz: f32,
) -> (ContactState, f32) {
    if state.separated {
        return (state, 0.0);
    }
    let hammer = sanitize_hammer(hammer);
    let v_incoming = if v_incoming.is_finite() { v_incoming } else { 0.0 };
    let dt = 1.0 / usable_sample_rate(sample_rate_hz);

    let mut force = hammer.stiffness * math::powf(state.compression, hammer.contact_exponent);
    for _ in 0..COUPLING_FIXPOINT_STEPS {
        let v_string = v_incoming + force / hammer.string_impedance;
        let trial_velocity = state.hammer_velocity - force / hammer.mass * dt;
        let trial_compression = math::clamp_or_low(
            state.compression + (trial_velocity - v_string) * dt,
            0.0,
            MAX_COMPRESSION,
        );
        force = hammer.stiffness * math::powf(trial_compression, hammer.contact_exponent);
    }

    let v_string = v_incoming + force / hammer.string_impedance;
    let hammer_velocity = state.hammer_velocity - force / hammer.mass * dt;
    let compression = math::clamp_or_low(
        state.compression + (hammer_velocity - v_string) * dt,
        0.0,
        MAX_COMPRESSION,
    );
    let force = hammer.stiffness * math::powf(compression, hammer.contact_exponent);
    let separated = compression <= 0.0;
    (
        ContactState {
            compression,
            hammer_velocity,
            separated,
        },
        force,
    )
}
```

- [ ] **Step 4b: Factor `simulate_contact`'s body out so its peak is reachable**

`couple_contact_step`'s force is physically raw (whatever `stiffness * compression^exponent` comes to — easily `1e8`+ in scale), but `PluckedString::blended_excitation`/`contact_force_diff_inv_peak` (Task 2) assume the same peak-normalised-to-`[0, 1]` convention `simulate_contact` returns. Rather than duplicate that normalisation, factor `simulate_contact`'s existing body into a new `pub(crate)` helper both it and `string.rs` can call, so `simulate_contact` itself becomes a two-line wrapper with **bit-identical output** to today:

```rust
/// [`simulate_contact`]'s own integration, before the final peak
/// normalisation — factored out so [`crate::string::PluckedString::
/// write_excitation`] and [`crate::string::PluckedString::
/// next_contact_sample`] can divide a coupled force by the same peak this
/// reference curve used, putting both on one scale, without running this
/// integration a second time.
pub(crate) fn uncoupled_contact_curve(
    velocity: f32,
    sample_rate_hz: f32,
    hammer: HammerConfig,
) -> ([f32; MAX_CONTACT_SAMPLES], usize, f32) {
    let velocity = math::clamp_or_low(velocity, 0.0, 1.0);
    let sample_rate_hz = usable_sample_rate(sample_rate_hz);
    let hammer = sanitize_hammer(hammer);
    let dt = 1.0 / sample_rate_hz;

    let mut force = [0.0f32; MAX_CONTACT_SAMPLES];
    let mut compression = 0.0f32;
    let mut hammer_velocity = strike_mps_for(velocity);
    let mut active = 0usize;
    let mut peak = 0.0f32;

    for sample in &mut force {
        if compression <= 0.0 && active > 0 {
            break;
        }
        let restoring =
            hammer.stiffness * math::powf(compression, hammer.contact_exponent) / hammer.mass;
        hammer_velocity -= restoring * dt;
        compression = math::clamp_or_low(compression + hammer_velocity * dt, 0.0, MAX_COMPRESSION);
        let applied_force = hammer.stiffness * math::powf(compression, hammer.contact_exponent);
        *sample = applied_force;
        peak = peak.max(applied_force);
        active += 1;
    }
    (force, active, peak)
}

/// Divides `curve`'s first `active` entries by `peak`, in place, or leaves
/// them untouched if `peak` is too small to divide by safely — the
/// normalisation [`simulate_contact`] applies to
/// [`uncoupled_contact_curve`]'s output, factored out so a caller
/// normalising a *single* coupled sample (dividing by the same `peak`) gets
/// the identical convention.
pub(crate) fn normalize_by_peak(curve: &mut [f32; MAX_CONTACT_SAMPLES], active: usize, peak: f32) {
    if peak > f32::EPSILON {
        for sample in curve.iter_mut().take(active) {
            *sample /= peak;
        }
    }
}
```

Replace `simulate_contact`'s entire body (currently the integration loop plus its own final normalisation, lines ~334-366) with:

```rust
pub fn simulate_contact(
    velocity: f32,
    sample_rate_hz: f32,
    hammer: HammerConfig,
) -> ([f32; MAX_CONTACT_SAMPLES], usize) {
    let (mut force, active, peak) = uncoupled_contact_curve(velocity, sample_rate_hz, hammer);
    normalize_by_peak(&mut force, active, peak);
    (force, active)
}
```

This changes only *how* `simulate_contact` computes its result, not the result itself — every existing test in this file (`the_force_envelope_peaks_at_one`, `contact_duration_is_physically_plausible`, `simulate_contact_is_total`, etc.) must keep passing bit-for-bit unchanged, which is exactly what re-running them in Step 5 confirms.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p piano-core --lib hammer:: 2>&1 | tail -60`
Expected: PASS, including every pre-existing `hammer.rs` test unchanged (`simulate_contact_is_total`, `contact_duration_is_physically_plausible`, etc. — confirms the `strike_mps_for` extraction changed nothing observable).

- [ ] **Step 6: Lint and format**

Run: `cargo fmt -p piano-core && cargo clippy -p piano-core --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 7: Commit**

```bash
git add crates/piano-core/src/hammer.rs
git commit -m "feat(piano-core): add a coupled hammer/string contact step (#57)

couple_contact_step resolves each contact sample's force against the
string's own returning velocity (v_incoming) via a hard-capped 3-step
fixed-point iteration, treating the contact point as a scattering
junction between the felt spring and the string's characteristic
impedance. simulate_contact is untouched; it stays the reference curve
for spectral cutoff sizing and normalisation.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01WAgKg3t7qXqmeCWwZ4ZMjw"
```

---

### Task 2: Wire the coupled step into `PluckedString`

**Files:**
- Modify: `crates/piano-core/src/string.rs:236-266` (`PendingContact`), `:654-700` (`write_excitation`), `:728-766` (`next_contact_sample`), `:932-950` (`write_mixed_feedback`)
- Modify: `crates/piano-core/src/string_tests.rs` — this file, not an inline module, is where `string.rs`'s tests live (`#[path = "string_tests.rs"] mod tests;` at the bottom of `string.rs`, so it still compiles as `crate::string::tests`; kept separate to keep `string.rs` under this project's 500-line file limit). It already opens with `use super::*;` and a `string_at(frequency: f32) -> PluckedString` helper (48 kHz, default `StringConfig`) — use that helper rather than reconstructing `StringConfig`/`Hz`/`SampleRate` by hand.

**Interfaces:**
- Consumes: `crate::hammer::{ContactState, couple_contact_step}` (Task 1).
- Produces: `PluckedString::next_contact_sample(&mut self, v_incoming: f32) -> f32` (signature changes — was `fn next_contact_sample(&mut self) -> f32`; every existing caller is `write_mixed_feedback`, updated in this same task).

- [ ] **Step 1: Write the failing test**

Add to `string_tests.rs`, near the other single-string behavioural tests (e.g. next to `plucking_produces_signal`):

```rust
    #[test]
    fn a_hard_strike_on_the_highest_key_still_produces_signal_once_coupled() {
        // #57's audible claim shows up most in the upper treble, where
        // contact outlasts one round trip (loop_delay ≈ 0.24 ms at C8's
        // 4186 Hz, 48 kHz) — this is the sanity floor before the real
        // behavioural comparison in
        // `coupling_changes_a_high_key_s_pending_contact_versus_an_uncoupled_run`
        // below: coupling must not silence the note outright.
        let mut string = string_at(4_186.0);
        string.pluck(0.9);
        let samples: Vec<f32> = (0..2_000).map(|_| string.process()).collect();
        assert!(
            samples.iter().any(|sample| sample.abs() > 1e-6),
            "a hard strike on the highest key produced no audible signal at all"
        );
    }
```

- [ ] **Step 2: Run test to verify it compiles and passes against today's code**

Run: `cargo test -p piano-core --lib string::tests::a_hard_strike_on_the_highest_key 2>&1 | tail -20`
Expected: PASS already — this only proves the fixture (`string_at`, `.pluck`, `.process`) works before the real coupling change lands in Step 3.

- [ ] **Step 3: Extend `PendingContact` and replace the array-lookup path**

Modify `PendingContact` (currently lines 252-266):

```rust
#[derive(Debug, Clone, Copy)]
struct PendingContact {
    /// The hammer's own live state — compression, hammer velocity, and
    /// whether it has already separated from the string
    /// ([`hammer::ContactState`]). Replaces a plain array index now that
    /// each sample's force comes from [`hammer::couple_contact_step`]
    /// rather than a precomputed, uncoupled curve.
    contact: hammer::ContactState,
    /// This strike's own [`HammerConfig`], carried so
    /// [`PluckedString::next_contact_sample`] can call
    /// [`hammer::couple_contact_step`] without re-reading
    /// [`PluckedString::hammer`], which a live [`PluckedString::set_hammer`]
    /// call could have changed mid-strike.
    hammer: hammer::HammerConfig,
    /// The reference (uncoupled) curve's peak force for *this* strike —
    /// [`hammer::couple_contact_step`]'s raw output is divided by this, the
    /// same scale [`PluckedString::write_excitation`] normalised the first
    /// `burst_length` samples by, so [`PluckedString::contact_force_diff_inv_peak`]
    /// stays meaningful across the whole strike, not just its first loop
    /// length.
    peak: f32,
    /// Continuation of the same shaping filter chain
    /// [`PluckedString::write_excitation`] started, so the spectrum does not
    /// discontinuously change where the first loop length's worth left off.
    felt: [OnePoleLowpass; EXCITATION_POLES],
}
```

Modify `write_excitation` (currently lines 654-700) — replace the `for index in 0..burst_length` loop's body and the `pending_contact`/`contact_force` tail:

```rust
    fn write_excitation(&mut self, velocity: f32) {
        let (mut reference_curve, contact_samples, peak) =
            hammer::uncoupled_contact_curve(velocity, self.sample_rate, self.hammer);
        hammer::normalize_by_peak(&mut reference_curve, contact_samples, peak);
        let cutoff_hz = hammer::excitation_cutoff_hz(contact_samples, self.sample_rate);
        let mut felt = [OnePoleLowpass::from_cutoff(cutoff_hz, self.sample_rate); EXCITATION_POLES];
        self.contact_force_diff_inv_peak = diff_inv_peak_of(&reference_curve, contact_samples);
        // A given velocity and seed is one strike, every time it is played.
        self.rng = Xorshift32::new(self.seed);
        let burst_length = self.loop_delay as usize + 1;
        let mut contact = hammer::ContactState::starting(velocity);
        let mut prev_shape = 0.0f32;
        for index in 0..burst_length {
            // The earliest self-reflection of this very strike, read raw
            // (not yet through `loop_filter`/`dispersion`, which only run
            // once real per-sample playback starts) — see the design doc's
            // note on why this approximation only matters for strings
            // whose `loop_delay` is shorter than the burst.
            let v_incoming = if index >= self.loop_delay as usize {
                self.delay.read(index - self.loop_delay as usize)
            } else {
                0.0
            };
            let (next_contact, raw_force) =
                hammer::couple_contact_step(contact, self.hammer, v_incoming, self.sample_rate);
            contact = next_contact;
            // Same peak the reference curve above was normalised by, so
            // `contact_force_diff_inv_peak` stays on the scale it was
            // calibrated against — see `PendingContact::peak`'s doc comment.
            let shape = if peak > f32::EPSILON { raw_force / peak } else { 0.0 };
            let excitation = self.blended_excitation(shape, prev_shape) * velocity;
            prev_shape = shape;
            let shaped = felt
                .iter_mut()
                .fold(excitation, |sample, stage| stage.process(sample));
            self.delay.write(shaped);
        }
        let comb_delay = (self.strike_position * self.loop_delay + 0.5) as usize;
        let comb_delay = comb_delay.min(self.delay.max_delay().saturating_sub(burst_length));
        self.delay.apply_strike_comb(burst_length, comb_delay);
        self.pending_contact = (contact_samples > burst_length && !contact.separated).then_some(
            PendingContact {
                contact,
                hammer: self.hammer,
                peak,
                felt,
            },
        );
    }
```

Modify `next_contact_sample` (currently lines 728-766) to take `v_incoming: f32` and use the coupled step:

```rust
    #[inline]
    fn next_contact_sample(&mut self, v_incoming: f32) -> f32 {
        let Some(mut contact) = self.pending_contact.take() else {
            return 0.0;
        };
        let (next_state, raw_force) = hammer::couple_contact_step(
            contact.contact,
            contact.hammer,
            v_incoming,
            self.sample_rate,
        );
        contact.contact = next_state;
        // Same peak `write_excitation` normalised this strike's reference
        // curve by (`PendingContact::peak`'s doc comment).
        let shape = if contact.peak > f32::EPSILON {
            raw_force / contact.peak
        } else {
            0.0
        };
        let prev = self.last_pending_force;
        self.last_pending_force = shape;
        let excitation = self.blended_excitation(shape, prev);
        let shaped = contact
            .felt
            .iter_mut()
            .fold(excitation, |sample, stage| stage.process(sample));
        if !next_state.separated {
            self.pending_contact = Some(contact);
        }
        shaped
    }
```

Add a new field `last_pending_force: f32` to `PluckedString` (next to `contact_force_diff_inv_peak`), initialised to `0.0` in `PluckedString::new`, reset to `0.0` at the top of `pluck()` alongside the other resets, so `next_contact_sample` can differentiate the coupled force the same way `write_excitation`'s `prev_shape` does. Remove the now-unused `contact_force: Box<[f32; hammer::MAX_CONTACT_SAMPLES]>` field, its allocation in `PluckedString::new`, and the `*self.contact_force = contact_force;` line `write_excitation` no longer has — `couple_contact_step` needs no precomputed array, only the reference curve's `contact_samples`/`diff_inv_peak`, both already local values.

Modify `write_mixed_feedback` (currently line 944, inside the existing method) — change:

```rust
        let reflected = driven * loop_gain + self.next_contact_sample();
```

to:

```rust
        let reflected = driven * loop_gain + self.next_contact_sample(tap);
```

`tap` is already this method's first parameter — the real, current bridge-tap value — so this is the one line that makes `next_contact_sample`'s coupling genuine rather than approximated.

- [ ] **Step 4: Run the crate's tests**

Run: `cargo test -p piano-core --lib 2>&1 | tail -80`
Expected: every existing `string.rs` test still passes (comb, strike position, pending-contact continuation length, etc.) plus the new test from Step 1.

- [ ] **Step 5: Write the coupling-is-live test and verify it passes**

Add to `string_tests.rs`:

```rust
    #[test]
    fn coupling_changes_a_high_key_s_pending_contact_versus_an_uncoupled_run() {
        let rate = SampleRate::new(48_000.0).expect("48 kHz is valid");
        let frequency = Hz::new(4_186.0).expect("C8 is representable");

        let mut coupled_config = StringConfig::new(frequency);
        coupled_config.hammer.string_impedance = 5.0e8;
        let mut coupled = PluckedString::new(coupled_config, rate).expect("C8 is representable");
        coupled.pluck(0.9);

        // `StringConfig::new` already starts `hammer` at `DEFAULT_HAMMER`,
        // whose `string_impedance` is `MAX_STRING_IMPEDANCE` (Task 1) — the
        // rigid-wall case needs no override.
        let rigid_config = StringConfig::new(frequency);
        let mut rigid = PluckedString::new(rigid_config, rate).expect("C8 is representable");
        rigid.pluck(0.9);

        let coupled_samples: Vec<f32> = (0..512).map(|_| coupled.process()).collect();
        let rigid_samples: Vec<f32> = (0..512).map(|_| rigid.process()).collect();
        assert_ne!(
            coupled_samples, rigid_samples,
            "a finite string_impedance produced the same output as the rigid-wall default"
        );
    }
```

Run: `cargo test -p piano-core --lib string::tests::coupling_changes 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 6: Run the full workspace test suite, lint and format**

Run: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace 2>&1 | tail -100`
Expected: clean. `piano-audio`/`piano-studio` reference `PendingContact`/`contact_force` nowhere outside `string.rs` (confirm with `grep -rn "contact_force\b" crates/ --include=*.rs 2>/dev/null || grep -rln "contact_force" crates/`), so no other crate needs a change for this task.

- [ ] **Step 7: Commit**

```bash
git add crates/piano-core/src/string.rs crates/piano-core/src/string_tests.rs
git commit -m "feat(piano-core): couple write_excitation and PendingContact to the real string (#57)

write_excitation now reads back the current strike's own earliest
self-reflection and next_contact_sample reads the real, filtered bridge
tap write_mixed_feedback already computes — both feed
hammer::couple_contact_step instead of indexing a precomputed,
uncoupled force curve. contact_force is gone; nothing needed it once
the force is computed live.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01WAgKg3t7qXqmeCWwZ4ZMjw"
```

---

### Task 3: `string_impedance` in the `.piano.json` cascade

**Files:**
- Modify: `crates/piano-studio/src/format.rs:16-23` (`HammerOverrides`)
- Modify: `crates/piano-studio/src/resolve.rs:296-302` (`resolve_hammer`)

**Interfaces:**
- Consumes: `piano_core::hammer::HammerConfig::string_impedance` (Task 1).
- Produces: `HammerOverrides::string_impedance: Option<f32>` — read by `piano-studio`'s existing group/string override loop, no other new entry point.

- [ ] **Step 1: Write the failing test**

First read the existing hammer-override test(s) in `resolve.rs` to copy the exact `PianoFile`/`resolve` construction shape already used: `grep -n "fn resolve\b" -B2 -A 20 crates/piano-studio/src/resolve.rs | head -40` and `grep -n "contact_exponent" crates/piano-studio/src/resolve.rs`. Then add, mirroring that shape exactly:

```rust
    #[test]
    fn string_impedance_resolves_through_the_same_cascade_as_the_other_hammer_fields() {
        let mut file = PianoFile::default();
        file.defaults.hammer.string_impedance = Some(2.0e9);
        let resolved = resolve(&file, SampleRate::from_hertz(48_000.0)).expect("resolves");
        let string = resolved
            .strings
            .iter()
            .find(|s| s.midi == 60 && s.string_index == 0)
            .expect("middle C has a string 0");
        assert_eq!(string.hammer.string_impedance, 2.0e9);
    }

    #[test]
    fn string_impedance_defaults_to_the_uncoupled_ceiling_when_absent() {
        let file = PianoFile::default();
        let resolved = resolve(&file, SampleRate::from_hertz(48_000.0)).expect("resolves");
        let string = &resolved.strings[0];
        assert_eq!(
            string.hammer.string_impedance,
            piano_core::hammer::DEFAULT_HAMMER.string_impedance
        );
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p piano-studio --lib resolve:: string_impedance 2>&1 | tail -30`
Expected: FAIL to compile — `HammerOverrides` has no `string_impedance` field yet.

- [ ] **Step 3: Add the field and wire it through `resolve_hammer`**

Modify `HammerOverrides` (currently lines 16-23):

```rust
pub struct HammerOverrides {
    /// See [`piano_core::hammer::HammerConfig::contact_exponent`].
    pub contact_exponent: Option<f32>,
    /// See [`piano_core::hammer::HammerConfig::stiffness`].
    pub stiffness: Option<f32>,
    /// See [`piano_core::hammer::HammerConfig::mass`].
    pub mass: Option<f32>,
    /// See [`piano_core::hammer::HammerConfig::string_impedance`].
    pub string_impedance: Option<f32>,
}
```

Modify `resolve_hammer` (currently lines 296-302):

```rust
fn resolve_hammer(base: HammerConfig, overrides: &HammerOverrides) -> HammerConfig {
    HammerConfig {
        contact_exponent: overrides.contact_exponent.unwrap_or(base.contact_exponent),
        stiffness: overrides.stiffness.unwrap_or(base.stiffness),
        mass: overrides.mass.unwrap_or(base.mass),
        string_impedance: overrides.string_impedance.unwrap_or(base.string_impedance),
    }
}
```

`HammerOverrides` already derives `Default`/`Deserialize`/`Serialize` (matching `ParameterOverrides` a few lines below it), so an absent `string_impedance` in a `.piano.json` file deserialises to `None` automatically, exactly like the other three fields — confirm the derive list before assuming this, and add `#[serde(default)]` at the field level only if the struct-level one does not already cover it (check `ParameterOverrides`'s own fields for the pattern this file uses).

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p piano-studio --lib resolve:: 2>&1 | tail -60`
Expected: PASS, including every pre-existing `resolve.rs` test.

- [ ] **Step 5: Lint, format, full crate test**

Run: `cargo fmt -p piano-studio && cargo clippy -p piano-studio --all-targets -- -D warnings && cargo test -p piano-studio 2>&1 | tail -60`
Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add crates/piano-studio/src/format.rs crates/piano-studio/src/resolve.rs
git commit -m "feat(piano-studio): resolve string_impedance through the hammer cascade (#57)

Fourth field on HammerOverrides/resolve_hammer, alongside
contact_exponent/stiffness/mass — same defaults < groups < strings
cascade, no new mechanism.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01WAgKg3t7qXqmeCWwZ4ZMjw"
```

---

### Task 4: Studio live-parameter parity

**Files:**
- Modify: `crates/piano-studio/src/edit.rs` (`StringParameter` enum, `STRING_PARAMETERS`, range table)
- Modify: `crates/piano-studio/src/live.rs:354-356,391-393,421` (apply match, command-emission match, `StringSnapshot` construction)
- Modify: `crates/piano-studio/src/snapshot.rs:38-44,95-100,145-157,238` (`StringSnapshot` field, range struct field, `from_definitions`, test fixture)

**Interfaces:**
- Consumes: `piano_core::hammer::HammerConfig::string_impedance` (Task 1), `HammerOverrides::string_impedance` (Task 3).
- Produces: `StringParameter::HammerStringImpedance` — a new value client code (the browser page, tests) can serialise as `"hammer_string_impedance"`.

- [ ] **Step 1: Write the failing tests**

First confirm the real name of `snapshot.rs`'s range-table struct: `grep -n "^pub struct" crates/piano-studio/src/snapshot.rs`. Use that exact name below wherever `<RangeTableName>` appears.

Add to `edit.rs`'s test module, next to the existing `serialises` test for `HammerContactExponent`:

```rust
    #[test]
    fn hammer_string_impedance_serialises_snake_case() {
        let json = serde_json::to_string(&StringParameter::HammerStringImpedance).expect("serialises");
        assert_eq!(json, "\"hammer_string_impedance\"");
    }
```

Add to `snapshot.rs`'s test module:

```rust
    #[test]
    fn hammer_string_impedance_range_matches_the_definition() {
        let ranges = <RangeTableName>::from_definitions();
        assert_eq!(
            ranges.hammer_string_impedance,
            StringParameter::HammerStringImpedance.range()
        );
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p piano-studio --lib edit:: snapshot:: hammer_string_impedance 2>&1 | tail -30`
Expected: FAIL to compile.

- [ ] **Step 3: Add the enum variant, range and every match arm**

In `edit.rs`, add to `StringParameter` (after `HammerMass`):

```rust
    /// See [`piano_core::hammer::HammerConfig::string_impedance`].
    HammerStringImpedance,
```

Add to `STRING_PARAMETERS` (bump the array length annotation from `8` to `9`):

```rust
pub const STRING_PARAMETERS: [StringParameter; 9] = [
    StringParameter::Damping,
    StringParameter::Sustain,
    StringParameter::Inharmonicity,
    StringParameter::DetuneCents,
    StringParameter::Seed,
    StringParameter::HammerContactExponent,
    StringParameter::HammerStiffness,
    StringParameter::HammerMass,
    StringParameter::HammerStringImpedance,
];
```

Add a range constant near `MASS_RANGE`:

```rust
/// Mirrors `piano_core::hammer`'s private `MIN_STRING_IMPEDANCE`/
/// `MAX_STRING_IMPEDANCE`. See [`CONTACT_EXPONENT_RANGE`]. Provisional
/// until the validation task in `docs/superpowers/plans/
/// 2026-09-13-hammer-string-coupling.md` calibrates real defaults.
const STRING_IMPEDANCE_RANGE: ParameterRange = ParameterRange::new(1.0e6, 1.0e13, 1.0e6);
```

Add a match arm to `StringParameter::range`:

```rust
            Self::HammerStringImpedance => STRING_IMPEDANCE_RANGE,
```

In `live.rs`, add to the apply match (near lines 354-356):

```rust
        StringParameter::HammerStringImpedance => string.hammer.string_impedance = value as f32,
```

Add `StringParameter::HammerStringImpedance` to the or-pattern feeding `StudioCommand::SetStringHammer` (lines 391-393):

```rust
        StringParameter::HammerContactExponent
        | StringParameter::HammerStiffness
        | StringParameter::HammerMass
        | StringParameter::HammerStringImpedance => StudioCommand::SetStringHammer {
```

Add to the `StringSnapshot` construction (near line 421):

```rust
        hammer_string_impedance: string.hammer.string_impedance,
```

In `snapshot.rs`, add the field to `StringSnapshot` (near lines 38-44):

```rust
    /// See [`StringParameter::HammerStringImpedance`].
    pub hammer_string_impedance: f32,
```

Add the field to the range struct (near lines 95-100) and to `from_definitions` (near lines 145-157):

```rust
    /// See [`StringParameter::HammerStringImpedance`].
    pub hammer_string_impedance: ParameterRange,
```

```rust
            hammer_string_impedance: StringParameter::HammerStringImpedance.range(),
```

And to the test fixture literal at line 238:

```rust
            hammer_string_impedance: piano_core::hammer::DEFAULT_HAMMER.string_impedance,
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p piano-studio --lib 2>&1 | tail -100`
Expected: PASS, including every pre-existing `edit.rs`/`live.rs`/`snapshot.rs` test. First check whether any test asserts `STRING_PARAMETERS.len()` or enumerates it exhaustively (`grep -n "STRING_PARAMETERS" crates/piano-studio/src/*.rs`) — update any such count to `9`.

- [ ] **Step 5: Lint, format, full crate test**

Run: `cargo fmt -p piano-studio && cargo clippy -p piano-studio --all-targets -- -D warnings && cargo test -p piano-studio 2>&1 | tail -100`
Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add crates/piano-studio/src/edit.rs crates/piano-studio/src/live.rs crates/piano-studio/src/snapshot.rs
git commit -m "feat(piano-studio): expose string_impedance to the studio's live surface (#57)

Fourth StringParameter alongside the other three hammer fields — same
range-table/apply/command-emission/snapshot treatment, no special case.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01WAgKg3t7qXqmeCWwZ4ZMjw"
```

---

### Task 5: Measure the coupled step's cost (`PERF-007`)

**Files:**
- Modify: `crates/piano-core/benches/components.rs:74-83`

**Interfaces:**
- Consumes: `piano_core::hammer::{couple_contact_step, ContactState, DEFAULT_HAMMER}` (Task 1) — `couple_contact_step`/`ContactState` are `pub(crate)`; check first whether this bench file is compiled as part of `piano-core`'s own crate (in which case `pub(crate)` already reaches it) or as an external harness (in which case they must become `pub`): `cat crates/piano-core/Cargo.toml | grep -A3 "\[\[bench\]\]"` and `head -10 crates/piano-core/benches/components.rs`.

- [ ] **Step 1: Add the bench**

Add to `components.rs`, right after `hammer_simulate_contact_one_strike`'s closing `});`:

```rust
    c.bench_function("hammer_couple_contact_step_one_sample", |b| {
        let state = piano_core::hammer::ContactState::starting(0.8);
        b.iter(|| {
            black_box(piano_core::hammer::couple_contact_step(
                black_box(state),
                black_box(piano_core::hammer::DEFAULT_HAMMER),
                black_box(0.1),
                black_box(48_000.0),
            ))
        });
    });
```

If the check above shows this bench cannot see `pub(crate)` items, change `couple_contact_step` and `ContactState` (and its `starting` method) to `pub` in `hammer.rs` instead — `HammerConfig`'s own fields are already fully `pub`, so this does not widen the crate's real API surface beyond what is already exposed; re-run Task 1's tests afterward to confirm nothing there assumed crate-privacy specifically.

- [ ] **Step 2: Run the bench**

Run: `cargo bench -p piano-core --bench components -- hammer_couple_contact_step 2>&1 | tail -20`
Expected: a completed run with a nanoseconds-per-iteration figure. Record this number — it goes into Task 7's `PERF-007` update.

- [ ] **Step 3: Commit**

```bash
git add crates/piano-core/benches/components.rs crates/piano-core/src/hammer.rs
git commit -m "perf(piano-core): measure couple_contact_step's per-sample cost (#57, PERF-007)

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01WAgKg3t7qXqmeCWwZ4ZMjw"
```

---

### Task 6: Full-workspace validation and calibration

**Files:** none created; this task runs the existing safety net and adjusts constants from Tasks 1-4 if it finds a problem.

- [ ] **Step 1: Run the full check list from `CLAUDE.md`**

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build -p piano-core --no-default-features
```

Expected: all four clean. Fix at the root and re-run if not — do not proceed while any is red.

- [ ] **Step 2: Run the 88-key full-engine regression sweep**

```sh
cargo test -p piano-audio --release --test engine_timbre -- --ignored 2>&1 | tail -120
```

Expected: every key stays inside its committed band. If a key now fails: the most likely cause is `DEFAULT_HAMMER.string_impedance = MAX_STRING_IMPEDANCE` not actually being negligible enough for that key's `stiffness`/`contact_exponent` combination — re-verify Task 1's `with_no_incoming_wave_and_impedance_at_its_ceiling_coupling_reproduces_the_uncoupled_curve` test still passes (if it does, the regression is real and new, not a leftover from the ceiling not being high enough) and narrow down which key/register moved before touching any constant.

- [ ] **Step 3: Check the fixed-point step count actually converges**

Write a throwaway local test (not committed, or committed and then removed — either way, do not leave it in the final diff): call `couple_contact_step` in a loop with `COUPLING_FIXPOINT_STEPS` temporarily raised to `6` for a hard case (lowest `string_impedance` the range allows, highest velocity, smallest `mass`) and compare the returned force against the `3`-step result. If they differ by more than roughly 1%, raise `COUPLING_FIXPOINT_STEPS` to `4` in `hammer.rs`, re-run Tasks 1-2's tests, and note the finding in Task 7's `PERF-007` update. If they agree, `3` stands — record that finding too.

- [ ] **Step 4: Render and listen**

Using the existing `piano-cli`/studio flow (`docs/pt-BR/studio-como-usar.md`), strike one bass note (e.g. A1), one mid note (A4) and the highest treble note (C8) at both a soft and a hard velocity. Confirm: no clicks, pops or instability; the hard strike is audibly not just louder but brighter than the soft one (already true before this change, from #77/#79 — confirm it did not regress); on C8, listen for any new roughness or metallic artefact the coupling might have introduced (the register where `v_incoming` is genuinely non-zero during contact).

- [ ] **Step 5: Measure the fundamental on at least one note**

Since this change touches excitation, confirm tuning did not shift: render A4 with `OfflineEngine`, measure its fundamental via the same method `crates/piano-audio/tests/engine_timbre.rs` or `crates/piano-render`'s spectral tests already use, and confirm it is still within the existing tolerance those tests already assert.

- [ ] **Step 6: No separate commit for this task** — if Step 3 changed `COUPLING_FIXPOINT_STEPS`, fold that into a small follow-up commit against `hammer.rs` (`git commit -m "fix(piano-core): raise COUPLING_FIXPOINT_STEPS to 4 — measured, 3 did not converge (#57)"`); if nothing needed changing, this task produces no commit of its own.

---

### Task 7: Documentation

**Files:**
- Modify: `crates/piano-core/src/hammer.rs:28-53` (module doc)
- Modify: `docs/PHYSICS.md` (hammer section)
- Modify: `docs/MODEL-REVIEW.md` (claim 2 section, and N3's cross-reference to it)
- Modify: `docs/PERFORMANCE.md:426-465` (`PERF-007` status)
- Modify: `docs/TIMBRE-PLAN.md` (new F-series entry, milestone summary table)
- Modify: `docs/PARAMETER-STUDIO.md:88-90,126` (reachability table, JSON example)

- [ ] **Step 1: Update `hammer.rs`'s module doc**

Replace the "The simplification this makes, stated plainly" section (lines 28-48) with a short paragraph stating the string is no longer treated as immobile — link to `couple_contact_step`'s own doc comment and the design doc for the physical model, rather than re-explaining it in two places.

- [ ] **Step 2: Update `docs/PHYSICS.md`**

Find the hammer section's stated simplification (search: `grep -n "immobile\|string treated as" docs/PHYSICS.md`) and replace it with the coupled model, citing Chaigne & Askenfelt (1994) same as `hammer.rs` does.

- [ ] **Step 3: Update `docs/MODEL-REVIEW.md`**

In claim 2's section ("No hammer↔string feedback — accepted"), add a "Done (#57)" note with the measured contact-duration/brightness difference from Task 6's Step 2/4/5 findings — the same style #78/#79 already use elsewhere in this file (a measured before/after table or figure, not an assertion). Update N3's cross-reference: it currently says velocity→spectrum coupling is "still open" pending exactly this item; state plainly that it has now landed and what, if anything, N3 still leaves open (whether the spectral change is *large enough*, which is a separate, still-unmeasured perceptual question — do not overstate what this change proves).

- [ ] **Step 4: Update `docs/PERFORMANCE.md`**

In `PERF-007`'s entry, add a new `*Status (M?):*` paragraph: implemented, with Task 5's measured nanoseconds-per-sample figure and Task 6 Step 3's fixed-point step-count finding. State plainly whether this entry now closes (measured, per the project's own rule 3) or what specifically remains open if it does not.

- [ ] **Step 5: Update `docs/TIMBRE-PLAN.md`**

Add a new entry in the existing F-series format (matching F5's structure) documenting this item, and append "— **done**" to its row in the milestone summary table.

- [ ] **Step 6: Update `docs/PARAMETER-STUDIO.md`**

In the reachability table (lines 88-90), add a row for `hammer.string_impedance`, matching the exact phrasing style the three existing rows use once you re-read them (some say "No" for a still-shared constant; this one is "Yes" only after Task 3 lands). In the JSON example (line 126), add `"string_impedance": <value>` to the `"hammer": { ... }` object, using `DEFAULT_HAMMER.string_impedance`'s actual value from `hammer.rs` at the time of writing, not a placeholder.

- [ ] **Step 7: Commit**

```bash
git add crates/piano-core/src/hammer.rs docs/PHYSICS.md docs/MODEL-REVIEW.md docs/PERFORMANCE.md docs/TIMBRE-PLAN.md docs/PARAMETER-STUDIO.md
git commit -m "docs: hammer-string coupling (#57) — measured, closes PERF-007

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01WAgKg3t7qXqmeCWwZ4ZMjw"
```

---

### Task 8: Open a PR and close the issue

- [ ] **Step 1: Push the branch**

```sh
git push -u origin timbre/57-hammer-string-coupling
```

- [ ] **Step 2: Open the PR**

```sh
gh pr create --title "feat: couple the hammer solver to real string motion (#57)" --body "$(cat <<'EOF'
Implements the scattering-junction coupling docs/superpowers/specs/2026-09-13-hammer-string-coupling-design.md designed: the hammer's contact force now resolves against the string's own returning velocity (v_incoming) via a hard-capped fixed-point iteration, applied with the same formula for every string — the bass/treble difference falls out of each string's own state rather than a register-specific branch.

Closes #57. Closes PERF-007 (docs/PERFORMANCE.md) with a measured per-sample cost.

🤖 Generated with [Claude Code](https://claude.com/claude-code)"
```

- [ ] **Step 3: Wait for CI, per `CLAUDE.md`'s immutable clause**

```sh
gh pr checks --watch
```

Fix at the root and push again if anything is red; repeat until every check is green.

- [ ] **Step 4: Merge and clean up**

```sh
gh pr merge --squash --delete-branch
```

`gh pr merge --squash` with "Closes #57" in the PR body auto-closes the issue on merge — verify with `gh issue view 57` afterward that it actually did (the same auto-close silently failing is exactly what happened to issue #81 earlier this session).
