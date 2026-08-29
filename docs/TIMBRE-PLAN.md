# Why it does not sound like a piano — diagnosis and plan

Written 2026-08-29, after a listener report: *"a very metallic, thin sound;
nothing like a piano."*

Everything in the "Diagnosis" section below is **measured**, not argued.
Reproduce it with:

```sh
cargo test --release -p piano-audio --test timbre_diagnostic -- --nocapture --test-threads=1
```

That harness (`crates/piano-audio/tests/timbre_diagnostic.rs`) was written
for this investigation because no existing test could have caught the main
defect: every prior spectral test measures the *attack* or the *total*
amplitude decay, and the defect is in neither.

---

## Diagnosis

### D1 — The model has no control over how fast harmonics decay. *(root cause)*

`voicing::loop_filter_coefficients` solves **one** equation — "the loop
filter's gain at the fundamental must leave room for the target decay time" —
using **two** free parameters (`pole`, `zero_mix`). The filter's slope is
therefore an uncontrolled side effect. Nobody ever specified how fast the
8th partial should die, so nothing does.

Measured, seconds for each partial to fall 20 dB from its own peak:

| key | H1 | H2 | H3 | H4 | H5 | H6 | H7 | H8 |
|---|---|---|---|---|---|---|---|---|
| A0 (27.5 Hz) | 8.87 | 8.70 | 8.70 | 8.53 | 8.36 | 8.02 | 7.85 | 7.51 |
| A2 (110 Hz) | 5.80 | 4.61 | 3.41 | 2.56 | 1.88 | 1.37 | 1.02 | 0.85 |
| A4 (440 Hz) | 2.39 | 0.51 | 0.17 | 0.34 | 0.17 | 0.17 | 0.17 | 0.17 |

Two different failures, in opposite directions:

- **A4 and up:** every harmonic is gone within one analysis window (0.17 s).
  After the attack, A4 **is a bare 440 Hz sine wave**. Its measured attack
  profile confirms it — H2 at −59 dB, H3 at −76 dB relative to the
  fundamental. Its spectral centroid collapses 1145 Hz → 455 Hz within one
  second and then sits flat at the fundamental for the rest of the note.
  This is precisely the reported *thin*.
- **A0:** the reverse. H1 and H8 decay at nearly the same rate (8.87 s vs
  7.51 s, a ratio of 1.18). A real bass string's 8th partial dies several
  times sooner than its fundamental. Partials that all decay together is the
  spectral signature of an organ or a struck bar — the reported *metallic*.

**Why**: a lowpass's magnitude deficit `1 - |H(f)|` grows as `f²` near DC.
The physics needs it to grow roughly as `f¹` (the required loss ratio between
H1 and H8 is ~7.3, an `f²` filter delivers ~64). The measured discrepancy at
A4 — H8 decaying 8.8× faster than it should — matches `64/7.3 = 8.8`
exactly. The filter is not merely mistuned; the constraint it is being tuned
against is the wrong constraint.

### D2 — `sustain` is being wasted, which is why the filter cannot win.

The loop filter's DC gain is **exactly 1** by construction. It therefore
cannot attenuate a fundamental at all; only `sustain`, the
frequency-independent broadband loss, can. Today's calibration drives
`sustain` toward 1.0 and asks the filter to carry the whole loss — which
forces the filter's corner down so far that it takes the harmonics with it.

Solving the filter alone against per-partial targets confirms the trap:
every fundamental comes out ~10× too long (A4 H1 = 93 s against an 11 s
target) while the harmonics land correctly.

**Validated fix.** Three unknowns (`pole`, `zero_mix`, `sustain`) against
three per-partial decay targets (H1, H3, H8) is solvable, per key:

| key | pole | zero_mix | sustain | achieved H1 / H3 / H8 | target H1 / H3 / H8 |
|---|---|---|---|---|---|
| A0 | 0.91125 | 0.07162 | 0.989815 | 30.5 / 19.8 / 5.97 s | 35 / 18 / 6 s |
| A2 | 0.59325 | 0.00651 | 0.995203 | 16.2 / 10.3 / 2.98 s | 20 / 9 / 3 s |
| A4 | 0.06684 | 0.03980 | 0.997936 | 9.27 / 5.54 / 1.49 s | 11 / 5 / 1.5 s |
| A6 | 0.00571 | 0.00000 | 0.998486 | 3.14 / 1.84 / 0.60 s | 4 / 1.6 / 0.6 s |
| C8 | 0.00001 | 0.00329 | 0.999026 | 1.51 / 0.80 / 0.30 s (H2/H5) | 1.5 / 0.8 / 0.3 s |

Compare A4 today (2.39 / 0.17 / 0.17 s) with A4 solved (9.27 / 5.54 /
1.49 s). That is the difference between a dying sine and a note.

The residual undershoot on H1 is a least-squares compromise across three
targets; weighting H1 higher, or adding a fourth partial, tightens it. Do
not treat the table above as final values — treat it as proof the solve
converges.

### D3 — The soundboard is inaudible, and absent above 200 Hz.

Measured impulse response of `Soundboard` alone: peak **0.015** for a unit
impulse, spectral centroid pinned at **170 Hz** for its whole life, fully
decayed by 1.5 s. Mixing it in moves a note's centroid from 1145 Hz to
1081 Hz — a 6% change.

`DEFAULT_MODES` is 8 modes, the highest at 1400 Hz. A real soundboard has
hundreds, well past 5 kHz, and is a large part of why a piano sounds like a
wooden box rather than a wire. The instrument currently has effectively no
body.

### D4 — The excitation is white noise, not a hammer.

`PluckedString::write_excitation` fills the delay line with
`rng.next_bipolar() * velocity * contact_force[i]`. Noise gives every
harmonic a **random amplitude and phase** on every strike. A real hammer
delivers a deterministic force pulse at **one point** on the string —
canonically ~1/8 of its length, which puts a deep notch at the 8th partial
and its multiples. That notch is a defining piano timbre feature and the
model has no strike position at all, nor a pickup position.

This is why A2's measured attack profile is ragged (H1 −1, H2 0, H3 −1,
H4 −13, H5 −34 dB) instead of smoothly falling with a notch.

### D5 — *Logic bug*: the `registers` block in `.piano.json` is parsed and ignored.

`crates/piano-studio/src/resolve.rs:159-163` calls `voicing_for_key`, which
reads Rust constants, and its own comment admits it: *"the file's own
(currently unused) `registers` block"*. `docs/PARAMETER-STUDIO.md` documents
this block as the register tier of the cascade. A user who edits
`decay_seconds`, `damping` or `inharmonicity` in their piano file gets
**silence — no error, no effect**.

### D6 — Most sound-shaping values are not reachable from a file or the studio.

Exposed today: 8 per-string parameters, 3 fields × 8 soundboard modes, 2
bridge gains. Not exposed, and each of them changes the timbre:

| Value | Where | What it does |
|---|---|---|
| `loop_zero_mix` | `string.rs` | loop-filter zero — added this week, wired nowhere |
| `SOUNDBOARD_MIX_GAIN` = 0.5 | `engine.rs` | how much body is in the mix |
| `MODE_COUNT` = 8 | `soundboard.rs` | fixed; cannot add modes |
| `RELEASE_LOSS_MULTIPLIER` = 0.4 | `string.rs` | damper strength |
| `EXCITATION_POLES` = 2 | `string.rs` | attack rolloff order |
| `EXCITATION_BANDWIDTH_FACTOR` = 15.0 | `hammer.rs` | attack brightness |
| `MIN/MAX_EXCITATION_CUTOFF_HZ` | `hammer.rs` | attack brightness bounds |
| `COEFFICIENT_GAIN` = 200.0 | `dispersion.rs` | dispersion strength |
| `DETUNE_CENTS_BICHORD/TRICHORD` | `unison.rs` | unison spread defaults |
| unison count per key | `unison.rs` | 1/2/3 strings, fixed boundaries |
| `BASS/MID/TREBLE_DECAY_SECONDS` | `voicing.rs` | register decay anchors |
| `BASS/TREBLE_DAMPING`, `..._INHARMONICITY` | `voicing.rs` | register anchors |
| `OUTPUT_LIMITER_THRESHOLD` = 0.9 | `limiter.rs` | output ceiling |
| **master gain** | — | **does not exist** |
| **velocity curve** | — | **does not exist**; raw MIDI velocity feeds `pluck` linearly |

### D7 — There is no MCP server.

---

## Plan

Ordered by audible impact per unit of work. Each phase is independently
testable and independently shippable; `timbre_diagnostic.rs` is the gate for
every phase in Part 1.

### Part 1 — Make it sound like a piano

**F1. Per-partial decay targets, and a three-unknown solve.** *(the big one)*
Replace `loop_filter_coefficients`' single-constraint bisection with a solve
for (`pole`, `zero_mix`, `sustain`) against per-partial decay targets. Add
`brightness_decay_seconds` (H8's target) and a mid-partial target to the
register anchors. Expected: A4 goes from `H1=2.4s, H8=0.17s` to
`H1≈9s, H8≈1.5s`. Gate: the decay table in D1 moves toward the D2 table.

**F2. Deterministic hammer pulse + strike position.** Replace the noise
excitation with the contact force itself injected at a strike position
(`strike_position` ≈ 0.12 of the loop, per-key, configurable), summed with
its inverted reflection so the comb notch falls out of the geometry rather
than being applied as a filter. Keep a small noise component as a
configurable `excitation_noise_mix` — real strikes do have a broadband
component. Gate: the attack profile grows a visible notch near H8 and stops
being ragged.

**F3. A soundboard worth hearing.** Raise `MODE_COUNT` to a configurable
bank (24–32 modes), extend it past 5 kHz, and expose `SOUNDBOARD_MIX_GAIN`.
Gate: the impulse response's centroid rises well above 170 Hz and the
mixed-in centroid moves by much more than 6%.

**F4. Pickup position.** A second comb, from where the bridge samples the
string. Cheap once F2's geometry exists.

**F5. Velocity curve + master gain.** A configurable curve (exponent or
breakpoints) from MIDI velocity to strike velocity, and a master output
gain. Today velocity is linear and there is no volume control at all.

**F6. Re-tune the register anchors by ear** against F1–F5, and update
`docs/PHYSICS.md`'s "Numbers worth having" with what the model actually
achieves.

### Part 2 — Make everything configurable

**P1. Fix D5 first.** Wire the `registers` block into resolution so the
cascade documented in `docs/PARAMETER-STUDIO.md` is real. This is a bug fix,
not a feature, and every later parameter depends on the tier working.

**P2. Move every value in D6's table out of `const` and into the cascade.**
Instrument tier for the global ones (soundboard mix, limiter threshold,
master gain, velocity curve), register tier for the per-register ones,
per-string tier for the rest. Each needs a live setter, a `Command` variant,
a documented range, and — per the top-level `CLAUDE.md`'s hard rule 5 — a
`proptest` proving totality across the **new** range, not just the old
constant.

**P3. Ranges and defaults that are actually useful to drag.** The stated
goal is "so I can tweak it and improve the sound." A slider whose whole
audible action happens in its first 2% is not tweakable: `HammerStiffness`'s
range spans `1.7e9`, and `Inharmonicity`'s useful range is logarithmic.
Give each parameter a display curve (linear / log / exponential) alongside
its range, so the UI can map the knob to perception rather than to the raw
number.

**P4. Sanity gate.** A test asserting every parameter reachable from
`piano-core` appears in `STRING_PARAMETERS`/the instrument tier — so the next
parameter added cannot silently repeat D6.

### Part 3 — Control surfaces

**S1. Studio UI for the new parameters**, including the per-partial decay
targets from F1, which become the primary timbre control.

**S2. `piano-mcp` crate.** An MCP server over the same command queue every
other controller uses (`docs/ARCHITECTURE.md`: one path, not three). Tools:
read the resolved state, set any parameter at any tier, play a note, run
`timbre_diagnostic`'s measurements and return the numbers, save/load a piano
file. That last one matters most — it lets an agent close the loop: change a
parameter, measure, compare, iterate.

**S3. `piano analyze` CLI subcommand** wrapping the same measurements, so
the numbers are available without a test runner.

---

## Sequencing

F1 alone should be audible immediately and is the single highest-value
change in this document. P1 is a small bug fix that unblocks Part 2. F2 and
F3 are the next two large timbre gains. Everything in Part 3 is worth
doing only once Part 2 has something worth controlling.
