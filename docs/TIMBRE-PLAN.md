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

### D3 — The soundboard is absent above 200 Hz.

`DEFAULT_MODES` is 8 modes, the highest at 1400 Hz. A real soundboard has
hundreds, well past 5 kHz, and is a large part of why a piano sounds like a
wooden box rather than a wire. The instrument has effectively no body above
the low mid-range.

> **This entry originally opened "the soundboard is inaudible", on the
> strength of its impulse response peaking at 0.015 for a unit impulse and a
> mixed-in centroid moving only 6%. Both numbers were real and the conclusion
> drawn from them was wrong** — see D8. A very narrow resonator has a *small*
> impulse peak and an enormous *steady-state* gain at its own frequency, and
> a spectral centroid is a poor detector of a handful of loud, narrow,
> low-frequency tones sitting under a broadband note. The soundboard was in
> fact about 30 dB too loud. The scope this entry hands to F3 — more modes,
> reaching higher — is unchanged; the "inaudible" framing is not.

### D8 — *Bug*: soundboard mode gain is not normalised, and the modes ring like bells. *(the reported "metallic knocking" in the mid register)*

Found after F1 shipped, from a second listener report: *"the mid-register
strings still sound bad; something metallic is knocking."* Reproduce with:

```sh
cargo test --release -p piano-audio --test soundboard_ring_diagnostic -- --nocapture --test-threads=1
```

**D8a — `Resonator::new` did the opposite of what it documented.** Its
`input_gain` line claimed `gain * (1 - radius)` "normalises the resonator's
peak steady-state gain to roughly `gain`". Measured, driving each mode with
a unit sine at its own resonant frequency:

| mode | asked for | delivered | error |
|---|---|---|---|
| 80 Hz | 1.00 | **47.75** | +34 dB |
| 130 Hz | 0.80 | 23.51 | +29 dB |
| 420 Hz | 0.50 | 4.55 | +19 dB |
| 1400 Hz | 0.25 | 0.69 | +9 dB |

A two-pole resonator's peak is `g / ((1-r)·|1 - r·e^{-j2θ}|)`. Dividing out
`(1 - r)` alone leaves `1/|1 - r·e^{-j2θ}|` ≈ `1/(2·sin θ)` standing in the
signal path — a boost that grows without limit as the mode frequency falls.
Every note therefore carried a fixed low thump louder than its own
fundamental: measured on A4, the 80 Hz mode came out **+13.4 dB above that
note's own H1**.

**D8b — the mode table was a bell, not a wooden box.** `Q = π·f·τ` for the
shipped decay times:

| | Q |
|---|---|
| ribbed spruce soundboard (`η` ≈ 0.02-0.05) | 20-50 |
| shipped table | **302 … 1100** |

−3 dB bandwidths of 0.27 Hz to 1.27 Hz: eight essentially pure sine
oscillators at fixed pitches, ringing for up to 1.2 s under every note. Where
one landed near a partial, the two beat audibly — the 420 Hz mode sits **81
cents** from A4's fundamental, giving a 20 Hz beat. That beating is the
reported *knocking*, and it is worst in the mid register because that is
where modes 280/420/650/950 lie.

Fraction of measured energy sitting at mode frequencies rather than at the
note's own partials, A4:

| | t=0 | t=1s |
|---|---|---|
| string alone | 11% | 8% |
| **+ soundboard, before** | **90%** | 23% |
| + soundboard, after | 33% | 8% |

**Fixed.** `soundboard::resonant_peak_reciprocal` divides out both factors
exactly, and `DEFAULT_MODES` now derives every decay time from one stated
`DEFAULT_MODE_Q = 30` rather than eight hand-typed numbers, so the table
cannot drift back into bell territory unnoticed. `MIN_DECAY_SECONDS` dropped
from 10 ms to 0.1 ms, which was silently clamping any heavily damped mode
above about 1 kHz back into ringing. Guarded by four tests in
`piano_core::soundboard`; the two gain tests were confirmed to fail against
the old normalisation before being kept.

**Not fixed here, and still F3's:** with the gain honest, 8 modes topping out
at 1400 Hz is a small body. That is the same scope D3 hands to F3
(issue #78) — now an enhancement rather than a correction.

### D9 — *Bug*: `piano studio --midi` silently discarded every control change.

`crates/piano-cli/src/studio.rs` carried its own copy of `crate::midi`'s
event mapping. The copy handled note-on and note-off and ended with
`MidiEvent::ControlChange { .. } => {}`. The sustain pedal — CC64, the
control a pianist uses most after the keys — therefore worked under
`piano midi` and did **nothing at all** under `piano studio --midi`, with no
error and no log line, alongside the brightness knob (CC74) and the mod
wheel (CC1).

**Why no test caught it.** Every MIDI test this project had stopped at
`piano_midi::parse` (wire format) or `select_index` (port choice). The
mapping from a decoded event to the engine call it should make was covered
by nothing, in either subcommand, because `AudioSession` needs a real output
device and no CI machine has one.

**Fixed.** `crate::sink::NoteSink` is that mapping's boundary: `AudioSession`
implements it in production and a recorder implements it under test, so
`studio` now calls `crate::midi::apply` rather than copying it. Fifteen tests
drive the chain from **raw MIDI bytes** — the actual wire format — through
the real decoder to the recorded engine calls: note-on, note-off, the
zero-velocity note-off idiom, chords, all 16 channels, all 88 keys, every
velocity from 1 to 127, the CC64 threshold at exactly 63/64, CC74's
inversion, CC1, unmapped controllers, pitch bend / program change /
aftertouch / clock being dropped rather than misread, and truncated messages
not panicking on the driver's callback thread.

### D10 — *Bug*: a trichord collapses to silence within ~120 ms across G5-B5, the worst of it at A5. *(the reported "hitting a tin can with a muffled iron")*

Found from a third listener report, after D8/D9 shipped: A5 specifically
still sounded broken. Reproduce with:

```sh
cargo test --release -p piano-audio --test timbre_diagnostic report_a5_unison_group -- --nocapture
cargo test -p piano-core --lib unison::tests::a_high_inharmonicity_trichord
```

**Root cause.** `dispersion::section_count` computes exactly **one** active
allpass section for roughly C#5-B5 (554-988 Hz) — the module docs already
call "0-1 sections" a soft boundary there. A single allpass section is
magnitude-preserving at every frequency for any coefficient short of the
unit circle (that is the whole point of the structure), but its *phase*
response grows arbitrarily steep as the coefficient approaches it: at the
old `MAX_COEFFICIENT = 0.9`, one section's phase delay at DC is 19 samples,
against 4 samples at `0.5`. A5's solved inharmonicity (0.0345) clamped its
one section to exactly that steep coefficient. Steep phase response means
high sensitivity to small frequency differences — and `UnisonGroup` detunes
a note's own 1-3 strings by a few cents for the beating a real piano has.
Three strings a few cents apart, each individually stable, span out of
phase with each other within a handful of round trips once dispersed
through that one steep section. `UnisonGroup`'s local blend assumes "same
note, coupled with no latency, so the difference term is genuinely small" —
once that stopped being true, the blend fed each string a partly
self-cancelling signal on every round trip: an unbounded, compounding loss
`sustain` never predicted or accounted for.

Measured: A5's trichord RMS fell from `0.14` to numerically silent within
six 20 ms windows (~120 ms). The **same note plucked as a monochord** — one
string, nothing to diverge against — decayed completely normally, which is
what rules out the loop filter, `sustain`, or the dispersion coefficient
itself as the cause. A4, A6 and C8's trichords were all measured and found
healthy — this is specific to the single-section band, not unison strings
in general.

**Fixed.** `dispersion::MAX_COEFFICIENT` lowered from `0.9` to `0.8` — swept
across the whole affected band (G5 through B5) to confirm every trichord in
it decays smoothly afterward, not just A5. Guarded by
`piano_core::unison::tests::a_high_inharmonicity_trichord_in_the_single_
dispersion_section_band_does_not_collapse`, confirmed to fail at the old
value before being kept, plus the `piano-audio` diagnostic above kept as a
permanent measurement across G5-B5 and the healthy neighbours (A4, A6, C8).

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
**Done** — `voicing::solve_loop_losses`, issue #76. The single-constraint
bisection is gone; each key now carries three decay targets (its
fundamental's, its 3rd partial's and its 8th's) and `pole`, `zero_mix` and
`sustain` are fitted against them by a bounded search with a closed-form
inner solve. Measured against D1's own table, seconds to −20 dB:

| key | H1 before → after | H8 before → after | H1:H8 before → after |
|---|---|---|---|
| A0 | 8.87 → 8.53 | 7.51 → **1.54** | 1.18 → **5.54** |
| A2 | 5.80 → 5.63 | 0.85 → 0.85 | 6.8 → 6.6 |
| A4 | 2.39 → 2.56 | 0.17 → **0.34** | 14.1 → **7.5** |

A0 is the result that matters: partials no longer decay together, which was
the *metallic* half of the report. A4's spectral centroid now keeps falling
across the whole note (2308 → 737 → 565 → 507 → 482 → 469 Hz over five
seconds) instead of collapsing to the fundamental inside one second and
sitting there.

Two notes on reading those numbers. The diagnostic measures a 20 dB drop
while the solve targets 80 dB, so a measured figure is about a quarter of
the analytic one — A0's analytic H1 is 33.9 s and it measures 8.53 s. And
A4's H8 at 0.34 s is not the `≈1.5 s` this entry originally predicted
*measured*; its analytic value is 1.42 s, on target. The gap between the two
is F2's, not F1's: an excitation that never put energy into H2 (−57 dB) and
H3 (−76 dB) leaves the filter's now-correct slope with almost nothing to act
on. Achieved coefficients and per-partial times per register are in
`docs/PHYSICS.md`, "Why the loop is solved against three decay times, not
one".

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

**P1. Fix D5 first.** **Done.** Wire the `registers` block into resolution so
the cascade documented in `docs/PARAMETER-STUDIO.md` is real. This was a bug
fix, not a feature, and every later parameter depends on the tier working.

`piano_audio::voicing::voicing_for_key_with_registers` generalises the
built-in three-anchor curve so a file's `registers.bass`/`mid`/`treble` can
override each anchor's position (`anchor_midi`), fundamental decay target
(`decay_seconds`) and inharmonicity — all blended into the same curve the
built-in anchors always used, with `RegisterOverrides::default()` proven to
reproduce `voicing_for_key`'s output exactly (regression-tested per key).
`damping` is handled differently: it is normally a *solved output*, not an
independent anchor value, so an override pins only the one key exactly at
that anchor's resolved position rather than inventing a blended curve for a
value the physics doesn't anchor independently — documented on
`RegisterAnchorOverride::damping`. `piano-studio::resolve` now calls this
instead of the plain `voicing_for_key`, closing the reported gap: *"a user
who edits `decay_seconds`, `damping` or `inharmonicity` in their piano file
gets silence — no error, no effect."* Guarded by 8 tests in
`piano_audio::voicing` (including a totality proptest over garbage
`anchor_midi`/`NaN`/`±∞` overrides) and 6 in `piano_studio::resolve`
(including cascade precedence — a `strings[]` entry still outranks a
register — and that an empty `registers` block resolves identically to the
built-in anchors). Two of the `resolve` tests were confirmed to fail against
the old `voicing_for_key`-only call before being kept.

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

## Tracking

Every item above is a GitHub issue, ordered by priority label.

| Milestone | Issue | Plan item | Priority |
|---|---|---|---|
| M16 — Make it sound like a piano | [#76](https://github.com/rodolphomacedo/piano/issues/76) Loop filter solved against the wrong constraint — **done** | F1 | 1 — critical |
| M16 | [#77](https://github.com/rodolphomacedo/piano/issues/77) Replace the white-noise excitation with a hammer pulse | F2 | 1 — critical |
| M16 | [#32](https://github.com/rodolphomacedo/piano/issues/32) Model strike position and its comb filtering | F2 / F4 | 1 — critical |
| M16 | [#78](https://github.com/rodolphomacedo/piano/issues/78) The soundboard is inaudible above 200 Hz | F3 | 1 — critical |
| M16 | [#79](https://github.com/rodolphomacedo/piano/issues/79) No velocity curve and no master gain | F5 | 2 — high |
| M16 | [#80](https://github.com/rodolphomacedo/piano/issues/80) Re-tune anchors by ear; update PHYSICS.md and the pt-BR material | F6 | 2 — high |
| M17 — Everything configurable | [#81](https://github.com/rodolphomacedo/piano/issues/81) *Bug*: `registers` parsed and silently ignored | P1 | 1 — critical |
| M17 | [#82](https://github.com/rodolphomacedo/piano/issues/82) Move every remaining constant into the cascade | P2 | 2 — high |
| M17 | [#83](https://github.com/rodolphomacedo/piano/issues/83) Ranges and display curves worth dragging | P3 | 3 — medium |
| M17 | [#84](https://github.com/rodolphomacedo/piano/issues/84) Gate: every core parameter reachable from the studio | P4 | 3 — medium |
| M18 — Control surfaces | [#85](https://github.com/rodolphomacedo/piano/issues/85) `piano-mcp` crate | S2 | 3 — medium |
| M18 | [#86](https://github.com/rodolphomacedo/piano/issues/86) `piano analyze` CLI subcommand | S3 | 3 — medium |

[#63](https://github.com/rodolphomacedo/piano/issues/63) ("Expand the
soundboard's modal bank", M11) was closed as superseded by #78, which carries
the same scope plus the measurement that reframes it.

Documentation that must be updated as these close — tracked in #80, listed
here so it is not forgotten: `docs/PHYSICS.md`, this file,
`docs/PARAMETER-STUDIO.md`, and the pt-BR teaching material
(`docs/pt-BR/COMO-FUNCIONA.md` and the `.tex`/`.pdf` it generates), which
still describes an excitation that F2 replaces.

F1's share of that is done: `docs/PHYSICS.md` gained "Why the loop is solved
against three decay times, not one", and the pt-BR material gained the
matching lesson section in both the `.md` and the `.tex`. **The `.pdf` has
not been rebuilt** — no LaTeX toolchain was available on the machine that
made the edit, so `COMO-FUNCIONA.pdf` is one revision behind its `.tex`.
