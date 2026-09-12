# External model review — assessment and plan

An external reviewer (another AI) read this repository on GitHub at commit
`808432a` and produced a critique of the physical model. This document records
that critique claim by claim, marks each **accepted**, **partly accepted** or
**rejected** with the evidence for the verdict, records the literature it
pointed at, and turns what survives into a prioritised plan.

Written 2026-08-30. The review is a good one and most of it is accepted. This
document exists so that the parts that are *not* accepted are not quietly
re-adopted later, and so the parts that are accepted do not get lost.

---

## Context the review could not have had

**The reviewer read `808432a`, which was `HEAD`. Five real defects were fixed
after that commit and were still uncommitted when the review was written.** The
reviewer therefore assessed a build containing all of them, and framed the
result as "well architected, physically simplified."

That framing is too generous, and acting on it directly would misdirect the
work. A large part of what was wrong was not simplified physics — it was
**broken code**:

| Defect | Magnitude | Where |
|---|---|---|
| Soundboard resonator gain normalisation wrong | up to **+34 dB**, bass-tilted | `TIMBRE-PLAN.md` D8a |
| Soundboard mode `Q` 302–1100 instead of 20–50 | rings like a struck bar | D8b |
| Trichord collapses to silence in ~120 ms, G5–B5 | note dies in 0.12 s instead of seconds | D10 |
| Loop filter solved against one constraint | all partials decay together | D1/F1 |
| `registers` block parsed and discarded | file edits did nothing | D5/P1 |

None of these are "the model is a simplification". They are bugs. **The
lesson is not "add more physics" — it is "we had no measurement that would
have caught any of this."** See item **N1** below, which is why it is ranked
first in the plan.

---

## Claim-by-claim assessment

| # | Claim | Verdict |
|---|---|---|
| 1 | Excitation is white noise, not a localised hammer strike | **Accepted** — strike position done (#32); force pulse open (#77) |
| 2 | Hammer does not couple back to string motion | **Accepted** |
| 3 | Bridge is an average, not a mechanical admittance | **Partly accepted** |
| 4 | `BridgeBus` block latency is a physical error | **Rejected as a priority** |
| 5 | Soundboard is a post-mix effect, not a mechanical load | **Accepted — deepest item** |
| 6 | 8 soundboard modes, too few and too smooth | **Done (#78)** — 28 modes, irregular, radiation-shaped, to 6.3 kHz |
| 7 | Per-key parameters are over-interpolated | **Accepted** |
| 8 | Fit parameters against real recordings | **Accepted with a hard constraint** |
| 9 | Do not spend effort on performance now | **Accepted with a caveat** |

### 1. The excitation is white noise, not a hammer — **accepted**

Verified in `crates/piano-core/src/string.rs:453`:

```rust
let excitation = self.rng.next_bipolar() * velocity * shape;
```

written across `burst_length = self.loop_delay as usize + 1` — that is, the
**whole delay line**. There is no strike position anywhere in the model.

The reviewer's physics is right. A hammer applies force at one point `x_h`, and
in a modal description mode `n` is excited in proportion to
`sin(n·π·x_h/L)`, so a strike at `x_h/L = 1/8` puts a deep notch on the 8th
partial. That notch, and the comb structure around it, is a large part of why
a piano sounds like a piano.

**One correction to the reviewer's prescription.** It describes the fix in
modal terms. In a **digital waveguide** you do not excite modes directly — you
inject the force pulse into the delay line **at the position corresponding to
`x_h`**, and the same comb structure follows as a natural consequence of the
two travelling waves. That is the cheaper and more architecturally native
implementation here, and it is `O(1)`. The modal formula is the right way to
*verify* the result, not to implement it.

Already tracked: **#77** (deterministic force pulse) and **#32** (strike
position and comb filtering). They are one piece of work, not two — but the
comb half can land on the existing noise burst without the pulse, and now
has: **#32 is done**. `DelayLine::apply_strike_comb` injects the excitation
plus its inverted reflection from `strike_position · L` samples back, exactly
the waveguide implementation this note prescribed (`O(1)`, no modal solve),
and the modal formula above is what verifies it — `piano-core`'s
`striking_at_one_eighth_attenuates_the_eighth_partial`. `strike_position` is
per key from `piano_audio::voicing`. Still open: **#77**, the pulse itself —
a deterministic pulse *without* this comb under-excites the treble (it
regressed the treble-decay gate and was reverted), so the geometry had to
come first.

### 2. No hammer↔string feedback — **accepted**

Verified in `crates/piano-core/src/hammer.rs:35`, which states the
simplification in its own words: *"the string is treated as immobile during
the contact."*

So the model solves roughly `m·ẍ_h = −K·x^p` and never the real coupled form
`F(t) = K(x_h(t) − y(x_h, t))^p`.

**Why this matters more than it looks.** Contact duration is what sets the
excitation's spectral cutoff. In a real piano a harder blow *shortens* contact,
which is why fortissimo is **brighter**, not merely louder. Without the
feedback loop, velocity mostly scales amplitude. This is the single largest
missing piece of *expressiveness*.

Already tracked: **#57**.

### 3. The bridge is an average, not an admittance — **partly accepted**

The direction is right, but the review understates where the code already is.
`PluckedString::write_mixed_feedback` already scales cross-string coupling by
`1 − loop_gain` and justifies it, in its own doc comment, as bridge admittance
with a citation to Weinreich — *"a string that barely gives energy to the
bridge barely receives any back through it."*

So the real gap is **not** "there is no admittance model". It is that the
admittance is a **scalar**, when a real bridge's mobility is strongly
**frequency-dependent**. That is a narrower and more actionable statement than
the review makes.

### 4. The `BridgeBus` block latency — **rejected as a priority**

Two problems with this one.

First, the number is wrong: the review computes 256 samples ≈ 5.33 ms. The code
uses **128-sample** bridge blocks, so it is 2.67 ms.

Second, and more importantly, that latency applies **only to cross-key
sympathetic resonance** — a slowly-building, low-level effect. 2.67 ms on a
sympathetic tail is inaudible. The one-block delay was a deliberate `O(N)`
rather than `O(N²)` decision, documented as `PERF-008`, and the review itself
concedes the engineering logic.

**Not scheduled.** If a measurement ever shows it audible, it can be revisited
with evidence.

### 5. The soundboard is a post-mix effect, not a mechanical load — **accepted, and it is the deepest item**

Verified: `Engine::process_chunk` runs strings → mix → soundboard → limiter.
The soundboard therefore colours the output but does not **load** the strings.

Physically the string's energy loss *is* the bridge/soundboard impedance it
radiates into. Today that loss is instead **fitted**: `voicing.rs` solves
`(pole, zero_mix, sustain)` against target decay times. The result can be made
to sound plausible, but the aftersound, the way the body responds to
polyphony, and the coupling between notes are all either absent or faked.

**Why it is nonetheless ranked after items 1 and 2.** Making the loss derive
from a shared admittance means re-deriving the entire per-key voicing solve.
It is the most invasive change in this document and it should be attempted
against a working measurement harness and a fixed hammer, not before.

### 6. Eight soundboard modes, too few and too smooth — **accepted; done (#78)**

The table was a hand-drawn smooth curve (80/130/190/280/420/650/950/1400 Hz,
gains falling 1.0 → 0.25). Real soundboard modes are irregular in frequency,
`Q` and radiation efficiency. The module's own docs already admit the numbers
are representative rather than measured.

**Done (#78).** `DEFAULT_MODES` is now 28 modes, 52 Hz to 6.3 kHz,
irregular in spacing and `Q` (bounded to the 16–44 band a spruce board
occupies), with a `gain` column shaped like a radiator's efficiency curve
rather than a monotone slide from the bass — peak ~200–550 Hz, treble tail
~13 dB down instead of ~34. `SoundboardMode` gained the `bridge_coupling`
field below (carried, not yet read — that is P4). The soundboard mix gain
became `Engine::soundboard_mix_gain`, live- and file-adjustable. `MODE_COUNT`
stayed a compile-time constant: the audio thread cannot reallocate the
resonator array, so "configurable" means the parameter cascade, not the
count. Gate and what was dropped from it: `TIMBRE-PLAN.md` F3.

The review's suggested shape is right:

```rust
struct SoundboardMode {
    frequency: f32,
    decay: f32,
    output_gain: f32,
    bridge_coupling: f32,   // new: how strongly it loads the strings
}
```

That `bridge_coupling` field is what makes item 5 implementable per-mode.

Already tracked as **#78** — but **that issue's premise is wrong** and must be
rewritten. It says the soundboard "is inaudible and contributes nothing above
200 Hz". Measurement showed the opposite: it was **30 dB too loud**, and on A4
its 80 Hz mode came out **+13.4 dB above the note's own fundamental**. The
"inaudible" reading came from measuring the impulse-response peak of very
narrow resonators, which is low even when steady-state gain is enormous. See
`TIMBRE-PLAN.md` D3's correction note.

### 7. Per-key parameters are over-interpolated — **accepted**

Three anchors (A0, A4, C8) with log interpolation between them. A real piano
does not vary smoothly: there are abrupt physical breaks at the wound→plain
string transition and at the monochord→bichord→trichord boundaries, and the
"killer octave" region is a known discontinuity.

The review's suggestion of ~10 anchor keys and a per-key parameter table is
cheap, low-risk and high-value. **Not currently tracked as an issue** — #80 is
about re-tuning the existing three anchors, which is a different thing.

### 8. Fit parameters against real recordings — **accepted, with a hard constraint the review could not know**

The approach is right and this project should end up here. But `CLAUDE.md`
states, without qualification, that there are **no samples or recordings
anywhere in this repository and none may be added**.

That does not kill the idea, but it fixes its shape:

- **Allowed:** fitting against *published measurement tables* in the
  literature; running an optimiser against a recording that lives **outside**
  the repository and committing only the resulting parameter numbers;
  comparing spectra by ear or by measurement without storing audio.
- **Not allowed:** committing a `.wav`, a sampled impulse response, or any
  captured-audio asset, in any form, for any purpose — including "just for the
  test fixture".

This tension is already tracked as **#65** ("Decide how instrument-specific
scale data can be sourced without recordings"), which shows the project had
already seen the problem.

### 9. Do not spend effort on performance now — **accepted, with a caveat**

Agreed that CPU is not the current limit; the model is. But two items here are
genuinely expensive and must not be waved through:

- hammer↔string coupling adds an iteration per sample **during contact**;
- a frequency-dependent bridge admittance adds a filter **per string**, and
  there are 222 strings.

Per this project's own hard rule 3, each gets a `PERF-xxx` entry in
`PERFORMANCE.md` **when identified, not when fixed**.

---

## What the review missed

### N1. Nothing has ever measured the full engine — *this is the highest-priority item in this document*

Every timbre diagnostic in the repository renders either a bare
`PluckedString` or a `PluckedString` + `Soundboard` pair. **None of them
render the `Engine`** — the actual path that unison coupling, the bridge bus,
the soundboard mix and the output limiter all live on, and the only path a
player ever hears.

That is exactly how the A5 trichord collapse survived: the string measured
fine in isolation, and the collapse only existed once three detuned strings
were blended. Three of the five defects listed at the top of this document
would have been caught immediately by a full-engine measurement.

**Required: a per-key, all-88-keys, full-engine regression measurement** with
committed thresholds — fundamental decay, per-partial decay ratio, and total
energy — so that any key whose behaviour departs from its own solved intent
fails a test instead of waiting for someone to notice by ear.

This is cheap. It is also the thing that stops the repeated cycle of "it is
still bad" → fix one defect → "it is still bad".

**Done (#87).** `piano_audio::offline::OfflineEngine` renders the real
`Engine` — unison coupling, bridge bus, soundboard mix, limiter — with no
`cpal` stream or command ring. `crates/piano-audio/tests/engine_timbre.rs`
measures every key against its own `voicing::solved_decay_targets`:
fundamental −20 dB time, H1/H(bright) decay ratio, early- and late-window
RMS, and trichord-vs-monochord late energy (the same key forced to one
string through `Engine::with_unison_override`). The all-88 sweep is
`#[ignore]`d for runtime; a 14-key representative slice runs in the normal
gate. Committed bands are calibrated to bracket the current 88-key
population, so the test guards against regression rather than re-tuning.

The sweep immediately earned its place: it found that the D10 region is not
fully clear. Across the upper treble the trichord radiates less sustained
energy than its own monochord, worst at A5 (fundamental at 0.42× its solved
intent, trichord at 0.09× its monochord — every other key is 0.79–1.32× and
0.3–2.55×). It is inside the committed bands but the closest any key comes
to failing. Filed as **#92**; its real fix is P4 (and #89 / N2), not a
second dispersion tweak.

### N2. The voicing solve and the coupling losses contradict each other

`voicing.rs` solves each key's `(pole, zero_mix, sustain)` **for a string in
isolation**. The engine then adds unison blending and bridge coupling, both of
which remove further energy that the solve never accounted for.

Measured, A4, seconds to −20 dB on the fundamental:

| Path | H1 decay |
|---|---|
| bare string | 2.56 s |
| full engine | 2.05 s |

So the engine delivers roughly **20% shorter** than the solve intends, and the
error is **polyphony-dependent** — it grows as more strings contribute. This is
an unmodelled discrepancy sitting in the one place `sustain` is supposed to be
the only authority, and it is closely related to item 5.

### N3. Velocity→timbre is essentially absent

A consequence of item 2, but worth stating on its own because it is what a
player notices first: striking harder should change the *spectrum*, not just
the level. Tracked by **#79**, now fully landed
(`Engine::master_gain`, `piano_audio::velocity_curve::warp_velocity`,
`docs/TIMBRE-PLAN.md` F5) — but #79 was about *gain* mapping (how loud a
strike sounds), not about timbre, and `warp_velocity` only reshapes the
strike velocity `pluck` receives, not the spectrum a given velocity
produces. **This item is still open**: velocity already reaches the
hammer's own contact model (harder strikes are shorter and brighter, see
`piano_core::hammer`'s velocity → brightness cue), but nothing measures
whether that spectral change is *audible enough*, only that a curve now
exists to make it *loud enough*.

---

## The plan

Ordered by what unblocks what, and by audible return per unit of risk.

### P0 — Land what is already fixed — **done (`9a510bb`)**

Five defects were fixed and uncommitted. Committed in `9a510bb` with CI
green. Closes **#76** and **#81**.

### P1 — Build the measurement safety net *(N1)* — **done (#87)**

All 88 keys, through the real `Engine`, with committed thresholds. Ranked
first because every later item in this plan is a change to the sound, and
right now **we have no way to tell whether a change made things better or
worse** other than listening to one note at a time. This is the item that
breaks the whack-a-mole cycle.

Landed as `OfflineEngine` + `tests/engine_timbre.rs` — see N1 above for what
it measures and the A5 residual (#92) it surfaced on its first run.

### P2 — Make the hammer a hammer *(claims 1 and 2)*

The single largest timbre win, and self-contained.

1. **Strike position** — #32, **done**: the excitation is summed with its
   inverted reflection from `x_h/L` samples back
   (`DelayLine::apply_strike_comb`), verified against `F_n ∝ sin(n·π·x_h/L)`.
   These were meant to land together with the **deterministic force pulse**
   (#77, still open), but a pulse without this comb under-excites the treble
   and regressed the treble-decay gate, so the geometry came first; #77 now
   builds on it.
2. **Hammer↔string coupling** — #57. `F(t) = K(x_h − y(x_h,t))^p`, a fixed
   number of iterations, no unbounded loop. Needs a `PERF-xxx` entry.
3. **Velocity→contact-time→brightness** falls out of 2; verify it explicitly
   *(N3)*.

Deferred within this group: Stulov felt hysteresis. It is a refinement on top
of a coupled hammer and is meaningless before one exists.

### P3 — Per-key physical truth *(claim 7)*

Replace 3-anchor interpolation with ~10 anchor keys and a real per-key table,
with the wound/plain and monochord/bichord/trichord breaks represented as
breaks. Cheap, low risk, and it is what stops the keyboard sounding like one
instrument stretched across 88 notes. **New issue needed.**

### P4 — Bridge and soundboard as one mechanical system *(claims 3, 5, 6)*

The deepest change; attempt only with P1 in place and P2 landed.

1. Frequency-dependent bridge admittance replacing the scalar *(claim 3)*.
2. Soundboard modes **load** the strings rather than only colouring the
   output *(claim 5)*. The `bridge_coupling` term is already on
   `SoundboardMode` and the `DEFAULT_MODES` table (done in #78), carried but
   read nowhere — this step is what makes it live. Resolve **N2** here: the
   voicing solve must account for the coupling loss, or derive from it.
3. ~~More modes, irregular, reaching above 5 kHz~~ *(claim 6)* — **done in
   #78**; the bank is 28 modes to 6.3 kHz. This point is closed.

### P5 — Calibration *(claim 8)*

Parameter fitting, under the no-recordings constraint spelled out above.
**#65** decides the sourcing question first.

### Not scheduled

- Bridge block latency *(claim 4)* — 2.67 ms on sympathetic resonance,
  inaudible, deliberate `O(N)` trade.
- SIMD/GPU/threading — the model is the limit, not the CPU.

---

## Literature register

Papers to work from. This project implements **from published literature only**
— never from copyleft source — so this register is the working surface. See
`PRIOR-ART.md` for the licence discipline that makes it mandatory.

| Paper | What we take from it | Status |
|---|---|---|
| **Chaigne & Askenfelt (1994)**, "Numerical simulations of piano strings I", *JASA* 95(2):1112 | Hammer–string interaction, felt nonlinearity, contact dynamics. **The primary source for P2.** | New — not yet cited in code |
| **Bensa, Bilbao, Kronland-Martinet & Smith (2003)**, "The simulation of piano string vibration: from physical models to finite difference schemes and digital waveguides", *JASA* 114(2):1095 | The bridge between finite-difference physics and the **waveguide** formulation this project uses. Most directly applicable paper to this architecture. | New |
| **Weinreich (1977)**, "Coupled piano strings", *JASA* 62(6):1474 | Unison coupling, two-stage decay, aftersound, bridge admittance. Source for P4. | Already cited in `unison.rs` |
| **Fletcher (1964)**, "Normal vibration frequencies of a stiff piano string", *JASA* 36(1):203 | Inharmonicity, `f_n = n·f_1·√(1+B·n²)`. | Already cited in `dispersion.rs` |
| **Stulov (1995)**, "Hysteretic model of the grand piano hammer felt", *JASA* 97(4):2577 | Felt hysteresis — force depends on compression *history*, not only on current compression. Refinement after P2.2. | New |
| **Boutillon & Ege (2013)**, "Vibroacoustics of the piano soundboard", arXiv:1305.3057 | Bridge mobility, modal density, radiation regimes. Source for P4.3. | New |
| **Bank et al.**, "A modal-based real-time piano synthesizer", *IEEE TASLP* | Real-time modal synthesis under a CPU budget; longitudinal modes; aliasing and stability. | Already referenced for soundboard resonator banks |
| **Smith**, *Physical Audio Signal Processing* | Waveguide fundamentals, two-pole resonators, allpass, scattering junctions. | Already the foundational reference |

**Future papers go in this table.** When one arrives: add the row, say what it
is for, and open or update the issue it serves — so nothing is read once and
lost.

---

## Constraints that bound everything above

These are not negotiable and every item in the plan is subject to them.

1. **No recordings, ever.** No `.wav`, no sampled impulse response, no
   captured-audio asset in this repository — including as a test fixture. See
   claim 8.
2. **The audio thread allocates nothing, locks nothing, panics nowhere, has no
   unbounded loop.** Hammer↔string iteration must be a fixed count.
3. **No copyleft source may be read** for a component being written. Papers,
   not code. `PRIOR-ART.md`.
4. **Every hot-path function is total** — defined for `NaN`, `±∞`, zero and
   `usize::MAX`, proven by `proptest`.
5. **New bottlenecks get a `PERF-xxx` entry when identified, not when fixed.**
