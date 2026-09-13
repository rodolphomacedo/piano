# Hammer↔string coupling (issue #57) — design

Status: proposed. Implements P2.2 of `docs/MODEL-REVIEW.md`'s plan and closes
claim 2 there ("the model does not couple back to string motion"). Builds on
the already-landed strike-position comb (#32) and force pulse (#77), and on
the measurement harness (#87) this design validates against before it can be
called done.

## Context

`piano_core::hammer::simulate_contact` solves only the hammer's side of the
Hertzian contact — `m·ẍ_h = −K·x^p` — treating the string as immobile for the
whole 1-4 ms contact window. `hammer.rs`'s own module doc already names this
as the model's deepest remaining simplification and says a real coupled solve
"belongs, if ever built, inside the per-sample loop with a hard-capped
fixed-point iteration" (`PERF-007`). This design is that iteration.

**Why it matters audibly**: contact duration sets the excitation's spectral
cutoff. A string that gives way under the hammer shortens contact further
than the hammer-alone model predicts, and — for any string whose period is
shorter than the contact (the upper treble, where `PendingContact` already
exists) — energy already circulating in the string measurably loads the next
part of the same strike. Today neither effect exists: every strike at a given
velocity and seed is completely independent of what the string was doing.

## Goal

The same coupling formula applies to every string on the keyboard — no
register-specific branch in the coupling logic itself. The audible difference
between bass and treble falls out of each string's own state (how much is
already returning through the loop when the hammer is still in contact), not
from a special case in the code.

## Non-goals

- Carrying a previous strike's residual energy into a fresh `pluck()` — the
  delay line is still cleared unconditionally at the top of `pluck()`, as
  today; this design does not touch that. (A restrike interacting with a
  *still-ringing* voice before `pluck()` clears it is a separate, larger
  question — sustain/retrigger behavior — out of scope here.)
- Deriving `string_impedance` from real physical units (tension, linear mass
  density). This project's hammer model is explicitly **not** calibrated to
  absolute SI units (see `HAMMER_MASS`'s doc comment); the new field follows
  the same normalised-units, measured-by-comparison convention as `stiffness`
  and `mass`.
- Changing `write_mixed_feedback`'s convex/additive mixing laws, the bridge
  bus, or unison coupling. This is strictly the hammer/string junction.

## The physical model

Treat the contact point as a scattering junction between the felt spring and
the string's characteristic impedance — the same framing
`write_mixed_feedback`'s doc comment already uses (and cites, J. O. Smith
III) for the bridge, applied one level down at the hammer:

```
v_string(t) = v_incoming(t) + F(t) / Z_c
η(t + dt)   = η(t) + (v_h(t) − v_string(t)) · dt
v_h(t + dt) = v_h(t) − (F(t) / mass) · dt
F(t)        = stiffness · max(η(t), 0) ^ contact_exponent
```

`v_incoming` is whatever is already travelling back through the string at the
contact point this sample — genuinely `0.0` through most of a fresh strike
(the line was just cleared), and non-zero exactly when a full round trip has
had time to complete during contact, i.e. `PendingContact`'s existing regime.
`Z_c` (`HammerConfig::string_impedance`, new field) is a real, resistive
constant per string, calibrated by register the same way `contact_exponent`
already is. `Z_c → MAX_STRING_IMPEDANCE` must recover today's curve exactly
(the string presents infinite impedance — a rigid wall).

`F` appears on both sides of the loop (it sets `v_string`, which sets `η`,
which sets `F`), so each sample resolves it with a small, hard-capped
fixed-point iteration — never a `while !converged`, per
`docs/REALTIME-AUDIO-RULES.md`.

## Changes by file

### `crates/piano-core/src/hammer.rs`

- `HammerConfig` gains `string_impedance: f32`. `sanitize_hammer` clamps it
  into `[MIN_STRING_IMPEDANCE, MAX_STRING_IMPEDANCE]`, `NaN` mapping to the
  low bound like every other field here.
- `simulate_contact` **is unchanged**. It keeps its existing job: the
  reference (uncoupled) curve that sizes `excitation_cutoff_hz`,
  `contact_force_diff_inv_peak`, and the hard cap on how many samples a
  contact can run. The coupled solve below perturbs the *actual* per-sample
  force around this reference; it does not replace what the reference is
  used for.
- New pure function, e.g. `couple_contact_step`:

  ```rust
  fn couple_contact_step(
      state: ContactState,       // { compression, hammer_velocity }
      hammer: HammerConfig,      // already sanitised
      v_incoming: f32,
      dt: f32,
  ) -> (ContactState, f32)       // (next state, force)
  ```

  Internally: sanitise `v_incoming` (`NaN`/`±∞` → `0.0`, the
  `clamp_or_low` convention), then run `COUPLING_FIXPOINT_STEPS` (proposed
  `3`, named constant with a doc comment citing the convergence check that
  sets it) iterations of the four equations above, clamping `compression`
  into `[0.0, MAX_COMPRESSION]` every step exactly as `simulate_contact`
  already does. Total for every input, proven by `proptest`, matching
  `simulate_contact_is_total`'s pattern.

### `crates/piano-core/src/string.rs`

- `PendingContact` gains `compression: f32` and `hammer_velocity: f32` —
  live state carried between samples, replacing a plain index into a
  precomputed array for the *value actually injected* (the precomputed
  array stays, for normalisation — see above).
- `write_excitation`: inside the existing `for index in 0..burst_length`
  loop, each iteration now calls `couple_contact_step` with
  `v_incoming = self.delay.read(...)` (raw, not yet through `loop_filter`/
  `dispersion` — those apply later, in real time, once `process()` starts;
  documented as an approximation that only affects strings whose
  `loop_delay` is shorter than the burst, and only for the earliest
  self-reflection of the current strike, never a previous one, since
  `pluck()` clears the line first).
- `next_contact_sample`: same call, `v_incoming = tap` — the real, already
  filtered signal `write_mixed_feedback` passes in. No approximation here;
  this is true real-time coupling.
- `contact_force` / `contact_force_diff_inv_peak` are unchanged in purpose:
  still the reference curve's normalisation. The value that gets
  differentiated and injected each step now comes from the coupled force,
  not `contact_force[index]` directly.

### `crates/piano-core/src/voicing.rs`

- `string_impedance` joins the per-register anchor table (bass/mid/treble)
  the same way `contact_exponent` already does — no new mechanism, one more
  column.

### `crates/piano-core/benches/components.rs`

- New `hammer_couple_contact_step` bench alongside
  `hammer_simulate_contact_one_strike`, so `PERF-007` closes with a
  measurement (project rule 3: an entry closes only with a measurement).

## Testing plan

- `couple_contact_step` totality via `proptest` (velocity space already
  covered by `simulate_contact_is_total`; this adds `v_incoming` and the
  carried state to the input space).
- Regression: `string_impedance` at its maximum reproduces
  `simulate_contact`'s existing curve to within float tolerance — proves the
  new code is a strict generalisation, not a behaviour change for anyone who
  never touches the new field.
- New behaviour: on a string whose `loop_delay` is short enough that
  `PendingContact` is active (a real upper-treble key), a second strike's
  injected force differs measurably from the same strike computed with
  `v_incoming` forced to `0.0` — proof the coupling is live, not inert.
- Full-engine regression: run `crates/piano-audio/tests/engine_timbre.rs`'s
  88-key sweep (`OfflineEngine`, #87) before and after, confirm every key
  stays inside its committed bands. This is not optional — it is the exact
  safety net `docs/MODEL-REVIEW.md` built for changes like this one.
- `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D
  warnings`, `cargo test --workspace`, `cargo build -p piano-core
  --no-default-features` (per `CLAUDE.md`), plus a render-and-listen pass on
  at least one bass, one mid and one treble note.

## Docs to update

- `hammer.rs` module doc: remove/revise "the string is treated as immobile
  during the contact" — it no longer is, keyboard-wide.
- `docs/PHYSICS.md`: the hammer section's stated simplification.
- `docs/MODEL-REVIEW.md`: claim 2 → done, with the measured before/after
  (contact duration and brightness vs. `v_incoming`) the same way #78/#79
  document their own measurements.
- `docs/PERFORMANCE.md`: `PERF-007` status → implemented, with the new
  bench's number.
- `docs/TIMBRE-PLAN.md`: new entry for this item, matching the existing
  F-series format.

## Risks and open questions carried into implementation

- **Fixed-point step count.** `3` is a starting proposal, not a measured
  value. The implementation plan must include a convergence check (does the
  force estimate stabilise within 3 steps across the velocity/impedance
  range, or does a pathological combination need 4?) before the constant is
  treated as settled — same empirical-calibration discipline
  `CONTACT_STIFFNESS` already models in this file.
- **`string_impedance`'s default register anchors** are uncalibrated until
  measured against `engine_timbre.rs`'s bands and, where possible, Chaigne &
  Askenfelt's reported contact-duration curves per register.
- **Loudness stability.** Because normalisation still derives from the
  uncoupled reference curve while the injected values now differ from it,
  the full-engine regression sweep (above) is what confirms this did not
  quietly shift levels register-by-register — not an assumption.
- **Separation must latch.** Once `η` returns to `0`, the hammer has left the
  string and must stay separated — clamping `compression` into
  `[0.0, MAX_COMPRESSION]` alone is not enough, because a later
  `v_incoming` swing could numerically push `η` positive again and
  re-engage a contact that physically already ended.
  `ContactState` needs an explicit `separated: bool` (or equivalent) set once
  and never cleared for the rest of that strike, checked before `couple_contact_step`
  runs at all — the same role `active`/`next_index >= contact_samples`
  already plays in `next_contact_sample` today, just also guarding the
  coupled state now that it is no longer a pure array lookup.
