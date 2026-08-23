# Performance: the bottleneck register

This is the project's standing list of **known and anticipated bottlenecks**.
Every entry exists before the code that will hit it, on purpose: a physical piano
model has a small number of well-understood places where CPU time goes, and they
are all predictable from the physics.

Rules for this document:

- An entry is added **when the risk is identified**, not when it hurts.
- An entry is closed only with **a measurement**, never with an opinion.
- "Optimise later" is fine. "Optimise without measuring" is not.

## Budget

The target machine for milestones M0–M5 is a **2.3 GHz Intel Core i5-8259U**
(4 cores, AVX2, no AVX-512, 256 KB L2 per core, 6 MB shared L3).

At 48 kHz with a 128-sample buffer, the audio callback must finish in **2.67 ms**.
A safe working target is **50 % of that budget on one core**, which on this CPU is
roughly **3 000 cycles per output sample** for the whole instrument. A 60-voice
piano therefore has a budget of about **50 cycles per voice per sample**.
That number is the yardstick for every entry below.

## Metrics we track separately

Following the discipline of not collapsing everything into "fast":

| Metric | Why it is separate |
|---|---|
| Cycles per voice per sample | The steady-state cost that sets polyphony |
| Worst-case callback time (p99.9, not p50) | One late buffer is an audible click; averages hide it |
| Note-on spike cost | Excitation and voice setup are bursts, not steady load |
| Voice count at which the callback misses deadline | The number users actually feel |
| Allocation count inside `process` | Must be exactly zero, always |
| Memory bandwidth per sample | The real limit once polyphony is high |

## Register

| ID | Area | Milestone | Status |
|---|---|---|---|
| [PERF-001](#perf-001) | Bounds checks in the delay-line hot loop | M7 | Open |
| [PERF-002](#perf-002) | Denormal handling in decaying tails | M2 | Mitigated, unmeasured |
| [PERF-003](#perf-003) | Per-sample dispatch in the voice loop | M5 | Mitigated, unmeasured in isolation |
| [PERF-004](#perf-004) | Linear interpolation in the fractional delay | M4 | Implemented, unmeasured |
| [PERF-005](#perf-005) | Dispersion allpass cascade | M4 | Implemented, unmeasured |
| [PERF-006](#perf-006) | Polyphony and voice management | M5 | Mitigated (energy gating); stealing not needed |
| [PERF-007](#perf-007) | Hammer–string contact solver | M4 | Implemented, unmeasured |
| [PERF-008](#perf-008) | Sympathetic resonance coupling | M6 | Implemented, measured in aggregate |
| [PERF-009](#perf-009) | Soundboard convolution | M6 | Implemented (modal synthesis), measured in aggregate |
| [PERF-010](#perf-010) | Cache behaviour of the delay-line working set | M7 | Mitigated, unmeasured in isolation |
| [PERF-011](#perf-011) | WASM has no SIMD by default | M3 | Mitigated, unmeasured |
| [PERF-012](#perf-012) | Allocation at note-on | M2 | Closed |
| [PERF-013](#perf-013) | `f32` precision in long bass decays | M7 | Open |
| [PERF-014](#perf-014) | `wasm-bindgen` call overhead at the JS↔Wasm boundary | M3 | Open |

---

### PERF-001

**Bounds checks in the delay-line hot loop.**

`DelayLine::read` and `write` index a `Box<[f32]>` after masking. The mask makes
the index provably in range for a human, but LLVM cannot always prove
`index & mask < buffer.len()` because the relation between `mask` and `len` is
not visible in the type. Every sample therefore pays a compare and a
never-taken branch, twice.

*Cost if it bites*: 2–4 cycles per string per sample. At 60 voices that is up to
15 % of the per-sample budget spent on branches that never fire.

*Mitigations, in order of preference*: (1) make capacity a const generic so the
length is a compile-time constant; (2) hint the optimiser with a redundant
`if index >= len { return 0.0 }` that it can fold; (3) as a last resort, a
narrowly scoped `get_unchecked` behind an audited `unsafe` block in one function,
with a proof comment — never spread across the crate.

*Do not act before*: a benchmark shows the branches actually cost something.
Modern branch predictors handle always-taken branches almost for free.

---

### PERF-002

**Denormal handling in decaying tails.**

Every string decays towards zero and eventually produces denormal floats. On x86
each denormal arithmetic operation can cost 50–100 cycles, so a *silent* voice
can become more expensive than a loud one — the opposite of what you want.

`piano-core` currently flushes denormals in software (`math::flush_denormal`),
which costs a compare and a select per filter per sample.

*Mitigation*: set the CPU's flush-to-zero and denormals-are-zero modes once, on
the audio thread, in the host layer (`MXCSR` on x86, `FPCR` on AArch64). That
makes the whole problem free — but it needs `unsafe`, it is per-thread, and it is
platform specific, which is exactly why it belongs in `piano-audio` and not in
the portable core.

*Note*: keep the software flush even after adding the hardware mode. WASM has no
equivalent control, and the software path is the correctness backstop.

*Status (M2)*: implemented in `piano-audio::denormals::enable_flush_to_zero`,
called once per audio thread before its first callback (`MXCSR` via raw
`stmxcsr`/`ldmxcsr` on x86_64, `FPCR` via `mrs`/`msr` on AArch64). This is the
narrowly-scoped `unsafe` this entry anticipated. **Not yet measured** — no
before/after cycle count exists, so the entry stays open per this document's
own rule that a status closes only with a number, never an opinion.

---

### PERF-003

**Per-sample dispatch in the voice loop.**

If voices are stored as `Box<dyn Voice>`, every sample pays an indirect call that
cannot be inlined, defeating the optimiser across the whole DSP chain.

*Mitigation*: two decisions, both structural.

1. Voices are a **concrete type or a small enum**, never a trait object, in the
   hot path. Trait objects are fine at the engine boundary, once per block.
2. The engine processes **blocks, not samples**: `process_block(&mut [f32])` with
   64–128 samples, so any dispatch cost is amortised over the block and the inner
   loop is a tight, inlinable, vectorisable body.

*Status (M5)*: **Mitigated, unmeasured in isolation.** Both structural
mitigations turned out to already be in place, from M2/M4 rather than
freshly built for M5: `piano_audio::engine::Voice` wraps a concrete
`Option<PluckedString>`, never `Box<dyn Trait>` (verified by reading
`engine.rs`, not assumed), and `Engine::process_block` already calls
`PluckedString::process_block_add` once per voice for the whole block
rather than dispatching per sample. A real, if aggregate, number now
exists: `piano-audio`'s `callback_time_at_full_88_voice_polyphony_
clears_the_deadline` (`engine_tests.rs`, `#[ignore]`d — a wall-clock
measurement, not a CI gate) measured 221.9 µs to process a 128-sample
block at full 88-voice polyphony on the documented reference machine (2.3
GHz Intel Core i5-8259U), about 8 % of the 2.67 ms deadline. That number
is consistent with dispatch cost being negligible, but it is a whole-engine
measurement, not one that isolates dispatch overhead from arithmetic or
cache cost — decomposing it is left to M7, which this document already
schedules for exactly that kind of work.

*Update (M6)*: the same test now measures **697.2 µs/block**, still on
the reference machine, because "88 voices" now means up to 222 strings
(`docs/PHYSICS.md`'s unison counts), not 88 — see `PERF-008` for the full
number and what it does and does not isolate. Dispatch is still per-voice
(now per-`UnisonGroup`, still a concrete type, never `Box<dyn Trait>`), so
this entry's own concern stays addressed; the growth is expected extra
arithmetic, not a dispatch regression.

---

### PERF-004

**Linear interpolation in the fractional delay.**

`DelayLine::read_interpolated` uses linear interpolation. It is cheap and
unconditionally stable, but it is also a lowpass filter whose strength depends on
the fractional part of the delay. Two consequences:

- Notes are damped by an amount that varies with pitch, so the decay time is
  wrong in a way that changes across the keyboard.
- The effective delay is slightly short, so tuning drifts sharp at high pitches.

This is a **quality** bottleneck rather than a CPU one, but it is what limits how
good the instrument can sound at any CPU cost.

*Mitigation*: a first-order allpass interpolator (exact magnitude, correct tuning,
but transient-sensitive when the delay changes) or third-order Lagrange (well
behaved under modulation, ~4 extra operations). Choose by listening test, not by
op count.

*Status (M4)*: **Implemented, unmeasured.** `DelayLine::read_allpass`
replaces `read_interpolated` in the hot path (`PluckedString::process`),
using the standard first-order allpass fractional delay (Jaffe & Smith 1983;
Smith, *Physical Audio Signal Processing*, "First-Order Allpass
Interpolation"): unity magnitude at every frequency, so it no longer colours
brightness by pitch or fractional delay the way linear interpolation did.
One known caveat, found by this milestone's own `proptest` coverage rather
than by inspection: the allpass coefficient `η = (1-d)/(1+d)` approaches the
unit circle as the fractional delay `d` approaches 0, which makes the filter
resonant/transient-sensitive right at (or very near) a whole-sample delay —
`delay::tests::allpass_reading_any_delay_is_total` stays finite there
(verified), but a real string whose tuned `loop_delay` happens to land
exactly on an integer would ring more than intended for a moment after a
strike. No cycle count exists for the interpolator itself, and no listening
test has confirmed the resonance caveat is inaudible in practice — both
would close this entry.

---

### PERF-005

**Dispersion allpass cascade — the single largest CPU consumer.**

Real piano strings are stiff. Stiffness makes partial `n` sharper than `n·f₀`,
which is why a piano sounds like a piano and a pure delay line does not. Modelling
it requires a cascade of first-order allpass filters in the loop, and the number
of sections needed grows towards the bass: roughly 1–2 sections in the treble,
8 or more in the lowest octave.

*Cost if unmanaged*: 8 sections × 3 strings × 60 voices × 48 kHz ≈ **69 M filter
evaluations per second**, each 2 multiplies and 2 adds. This alone can exceed the
entire budget.

*Mitigations*:

- **Scale the order by register.** The treble genuinely needs almost none.
- **Precompute coefficients per key** into a table at engine construction; never
  solve for allpass coefficients at note-on.
- **SIMD across the unison strings.** The 2–3 strings of one note run the same
  filter structure with different coefficients — a natural 4-wide SIMD group.
- **Consider a single higher-order allpass** instead of a cascade, if the
  coefficient design is stable.

*Status (M4)*: **Implemented, unmeasured.** `piano_core::dispersion` adds a
cascade of up to `MAX_SECTIONS = 8` first-order allpass sections in the loop,
after the loss filter, with section count scaled by register from the
measured table in `docs/PHYSICS.md` (8 at A0, 2 at A4, 0-1 at C8) and a
single per-cascade coefficient derived from the live `inharmonicity`
parameter (`StringConfig::inharmonicity`, `PluckedString::set_inharmonicity`
— same live-control pattern as `damping`/`sustain`). The coefficient's
scaling constant was calibrated empirically against a real FFT measurement
(`crates/piano-render/tests/m4_spectral.rs`) rather than fit to Fletcher's
curve in closed form — fitting a cascade exactly is itself a numerical
optimisation problem Jaffe & Smith's own paper solves iteratively, which is
out of scope here; see the module doc comment in `dispersion.rs` for the
honest statement of that simplification. Measured result for A4 at the
default `B`: partials 1 through 7 sharpen with increasing partial number
(deviations from an exact harmonic series of roughly -2, +0.7, -0.2, +0.7,
+1.3, +3.6, +6.1 cents for partials 1-7 respectively) — the qualitative
signature the physics predicts, confirmed by measurement rather than
asserted. Not yet measured: cycles per section per sample (PERF-005's own
"largest CPU consumer" concern), and partial 8 for A4 was excluded from the
test's assertions because its energy sits close to this register's noise
floor and the peak-finder occasionally locks onto a spurious neighbouring
bin — a test-resolution limitation noted in the test file, not a claim about
the model itself.

---

### PERF-006

**Polyphony and voice management.**

An 88-key piano with 3 strings per note is up to 240 waveguides. With the sustain
pedal down, every undamped string is ringing, so "polyphony" is not bounded by how
many keys are held.

*Mitigations*:

- **Energy gating.** A voice below the audibility threshold is skipped entirely,
  not processed and multiplied by a tiny gain. `PluckedString::is_silent` exists
  for this.
- **Voice stealing** with an explicit, cheap policy (quietest first, then oldest),
  with a fade-out so stealing is not a click.
- **A hard voice cap** that degrades gracefully. Dropping a quiet note is always
  better than missing a buffer deadline.

*Status (M5)*: **Mitigated (energy gating); voice stealing not needed.**
Energy gating turned out to already exist, from M2 — `Engine::process_block`'s
`if string.is_silent() { continue; }`, confirmed by `git log -p` against
`engine.rs` rather than assumed — so M5's own contribution here was making
a *released* voice actually reach `is_silent` promptly: before M5,
`PluckedString` had no way to shorten a note early at all, so the gate only
ever paid off after a long natural decay. `PluckedString::release` plus a
faster envelope-follower forgetting rate once released
(`RELEASED_ENVELOPE_DECAY`, `string.rs`) closes that gap — see M5's
`docs/ROADMAP.md` entry for the numbers. Voice stealing (reassigning a slot
when out of slots) was deliberately **not built**: `PERF-012`'s one
permanent voice per key means there is never an "out of slots" case on a
standard 88-key instrument, so there is nothing to steal from — a hard
voice cap only becomes a real question if a future milestone allows more
than one voice per key (unison strings, M6) or lets polyphony exceed 88.
This entry stays open rather than closing, since no cycle-level measurement
isolates the gate's own saved cost (as opposed to the whole-engine number
`PERF-003` records) and a hard voice cap was not built.

*Update (M6)*: this entry's own reasoning about the gate's limits was
exactly right — "a hard voice cap only becomes a real question if a
future milestone allows more than one voice per key" is precisely what
happened. Effective polyphony is now up to 222 strings (`docs/PHYSICS.md`),
not 88, and sympathetic resonance (`PERF-008`) narrows the gate further:
`Engine::process_chunk` can only skip a voice that is *both* silent and
fully damped (`!UnisonGroup::is_receptive`) — a silent voice the pedal has
left undamped must still be processed so it can wake up from the shared
bridge. While the pedal is down, every voice becomes receptive, so the
gate's savings shrink to near zero for as long as the pedal is held; this
is by design (the pedal is meant to make the whole instrument audible) but
is worth naming as a real, if expected, cost regression in exactly the
scenario `PERF-008` was built for. Still open, same reason as before: no
isolated measurement of the gate's own saved cost exists, now further
complicated by measuring "saved cost" while pedal state changes what the
gate can save.

---

### PERF-007

**Hammer–string contact solver.**

A piano hammer is a nonlinear spring (felt compression follows roughly
`F = K·δ^p`, `p ≈ 2–3`). Solving hammer and string simultaneously during contact
is an implicit problem; done naively it becomes an iterative solve *per sample*.

*The hard constraint*: an audio callback may never contain an unbounded loop.
`while !converged` is forbidden. Any iteration must have a **compile-time
maximum** and a defined behaviour when it is reached.

*Mitigations*: an explicit scheme with a bounded fixed-point iteration (2–4 steps,
hard capped), or a precomputed force curve indexed by compression. Contact lasts
1–4 ms, so this is a bounded spike rather than a steady load — but the spike must
be bounded by construction, not by hope.

*Status (M4)*: **Implemented, unmeasured.** `piano_core::hammer::simulate_contact`
runs a bounded explicit (semi-implicit Euler) integration of the hammer's
side of the Hertzian contact (Chaigne & Askenfelt 1994, `F = K·x^p`,
`p = 2.5`), capped at `MAX_CONTACT_SAMPLES = 512` steps (a compile-time
bound, never a `while !converged`) — called once per
`PluckedString::pluck`, the same bounded-spike location the existing noise
burst already used, not from the per-sample loop. This is a deliberate
simplification of the full hammer/string solve this entry originally
described: rather than a simultaneous implicit solve with the string's own
motion feeding back into the contact force, the string is treated as
immobile during the ~1–4 ms contact window, and the resulting
velocity-shaped force envelope scales the existing excitation noise rather
than replacing it — documented in full in `hammer.rs`'s module doc comment,
including why that is an honest reduction in scope rather than a hidden one.
Measured result: at 48 kHz, a soft strike (velocity 0.15) produces a
spectral centroid of about 850 Hz and a hard strike (velocity 0.95) about
910 Hz for the same A4 note, neither loudness-normalised — confirming
velocity changes the excitation's spectral *shape*, not just its level
(`crates/piano-render/tests/m4_spectral.rs`,
`hitting_harder_makes_a_note_brighter_not_merely_louder`). Not yet measured:
cycles per contact simulation, and whether a true coupled hammer/string
solve (the fixed-point-iteration approach this entry originally described)
would sound meaningfully different — both would close this entry.

---

### PERF-008

**Sympathetic resonance coupling — the O(N²) trap.**

When the sustain pedal is down, every string couples to every other string
through the bridge. Modelling that literally is `N²` interactions per sample; at
`N = 240` that is 57 600 multiply-adds *per sample*, or 2.7 G/s. Infeasible.

*Mitigation, which must be designed in rather than retrofitted*: a **single shared
bridge bus**. Each string writes its bridge force into one summed bus and reads
the bus back, filtered by its own admittance. That is `O(N)` and is physically
defensible, because the bridge really is the shared mechanical connection.

*This entry exists now, before any coupling code, precisely so that nobody writes
the `N²` version first.*

*Status (M6)*: **Implemented, measured in aggregate.**
`piano_core::bridge::BridgeBus` is the shared bus this entry asked for: a
running average (not sum — see the module's own honesty note on a real
divergence bug an earlier, unnormalised-sum version had, caught by
`piano-audio`'s pedal-hold test diverging to infinity, not by inspection)
every voice writes into and reads back once per sample, `O(N)`. Wired into
`piano_audio::engine::Engine::process_block`, which chunks its output into
128-sample pieces (`BRIDGE_BLOCK_SAMPLES`) so the bus never sees a block
longer than it was sized for. `PERF-006`'s energy gating had to change to
match: a voice can now be silent yet still need processing, if the pedal
(or a held key) has lifted its damper — see
`UnisonGroup::is_receptive` and `Engine::process_chunk`'s skip condition.
A real number exists, but only an aggregate one, the same honest caveat
`PERF-003` already carries: `piano-audio`'s `callback_time_at_full_88_
voice_polyphony_clears_the_deadline` (run manually, `--release`, on the
documented reference machine, a 2.3 GHz Intel Core i5-8259U) now measures
**697.2 µs per 128-sample block** at full polyphony — every one of 88
keys struck, now with their real M6 unison-string counts (up to 222
strings total, see `docs/PHYSICS.md`), plus the bridge bus and the
soundboard both active — about 26 % of the 2.67 ms deadline, comfortably
inside it and up from M5's 221.9 µs/8 % now that roughly 2.5x as many
strings are being processed per block. This number does not isolate the
bridge's own cost from the unison strings' or the soundboard's — doing
that is left to M7, this document's own dedicated performance-engineering
milestone, the same deferral `PERF-003`/`PERF-010` already made for
isolating dispatch and cache cost from raw arithmetic.

---

### PERF-009

**Soundboard.**

A measured soundboard impulse response is 1–2 seconds — 50 000 to 100 000 taps at
48 kHz. Direct convolution is 2.4–4.8 G multiply-adds per second: impossible.

*Options, to be decided by measurement*:

- **Uniformly-partitioned FFT convolution.** Standard, well understood; the FFT
  then dominates the CPU profile and the partition size trades latency against
  cost.
- **Non-uniform partitioning.** Small blocks for the head of the IR (low latency),
  large blocks for the tail (cheap). More complex, considerably faster.
- **Modal synthesis.** A bank of resonators rather than a convolution. Much
  cheaper, fully parametric, less faithful to a specific instrument.

*Also note*: whichever is chosen, this is the component most likely to justify
running on a second thread with a lock-free handoff — with the latency
consequences that implies.

*Status (M6)*: **Implemented (modal synthesis), measured in aggregate.**
The first two convolution options above were never candidates in
practice, regardless of their engineering merit: both require possessing a
measured impulse response — a recording of a real soundboard — and this
project's own `CLAUDE.md` prohibits adding any recorded or sampled audio
asset to the repository, unconditionally. `piano_core::soundboard::
Soundboard` implements the third option: a fixed bank of
[`MODE_COUNT`](../crates/piano-core/src/soundboard.rs) = 8 two-pole
digital resonators (J. O. Smith III, *Physical Audio Signal Processing*),
fewer than a "faithful" soundboard model's dozens of resolvable low-order
modes alone — explicitly the cheaper, more parametric trade this entry
itself named as acceptable. Frequencies, decay times and gains are
literature-informed order-of-magnitude values (K. Wogram 1980; N. Suzuki,
JASA 80, 1986), not fit to any real instrument's measured response — see
the module's own honesty note. Wired into `Engine::process_chunk` as a
post-mix stage mixed additively into the direct signal (`SOUNDBOARD_MIX_
GAIN`), not a replacement — `docs/PHYSICS.md` explains why. Same aggregate
697.2 µs/block number `PERF-008` records applies here too (the soundboard
runs inside the same measured chunk); its own isolated per-sample cost has
not been measured separately, but 8 resonators × a handful of multiply-
adds each, run once per sample regardless of voice count (a post-mix
stage, not a per-voice one), is a small, bounded addition relative to the
up-to-222-string voice cost the same block also pays for — an argument,
not a number, so this stays open for M7 to close with one.

---

### PERF-010

**Cache behaviour of the delay-line working set.**

A0 at 48 kHz needs a delay line of 1 745 samples; rounded to a power of two that
is 2 048 floats, 8 KB. Two hundred forty of those is roughly **2 MB of delay lines
alone** — far past the 256 KB L2 of the target CPU and a third of its L3.

If the engine loops "for each sample, for each voice", it touches all 2 MB every
single sample and becomes memory-bandwidth bound long before it becomes
compute bound.

*Mitigation*: **loop order matters more than instruction count.** Process one
voice for a whole block of 128 samples (its delay line stays hot in L1/L2), then
move to the next voice, accumulating into a shared output block. This is the same
conclusion PERF-003 reaches from a different direction.

*Status (M5)*: **Mitigated, unmeasured in isolation.** `Engine::process_block`
already loops voice-outer, block-inner (`for voice in &mut self.voices {
... string.process_block_add(output) ... }`), the loop order this entry
recommends — in place since M2/M4, not new to M5. The same aggregate
221.9 µs/block measurement `PERF-003` records is consistent with this not
being a bottleneck at 88 voices, but does not isolate cache effects from
dispatch or raw arithmetic cost. Still scheduled for M7, as this document's
own register already said before M5 touched it: a cache-miss-isolating
measurement (e.g. comparing this loop order against the naive
sample-outer, voice-inner one) is what would actually close it.

*Update (M6)*: the loop order survives M6 unchanged in spirit — `Engine::
process_block` now chunks into `BRIDGE_BLOCK_SAMPLES`-sized pieces for
`PERF-008`'s bridge bus, but within each chunk it is still voice-outer
(now `UnisonGroup`-outer), chunk-inner, so a voice's whole working set
(now up to 3 delay lines instead of 1) still stays hot across the chunk it
is processed for. The working set per voice is larger than M5's (up to 3x
for a trichord key), which makes this entry's original cache-pressure
concern more relevant, not less — still left to M7 to actually measure.

---

### PERF-011

**WASM has no SIMD by default.**

The browser build is not just a slower native build: `wasm32-unknown-unknown`
emits scalar code unless `simd128` is enabled as a target feature, and
`AudioWorklet` runs a single thread with a fixed 128-sample quantum.

*Mitigations*: build the web target with `-C target-feature=+simd128`; keep the
engine's internal block size at 128 so it matches the worklet quantum exactly;
accept a lower voice cap in the browser and make it a parameter rather than a
constant.

*Status (M3)*: **Mitigated, unmeasured.** `.cargo/config.toml` sets
`target.wasm32-unknown-unknown.rustflags = ["-C", "target-feature=+simd128"]`,
verified locally to actually take effect (`cargo rustc ... -- --print cfg`
shows `target_feature="simd128"` for a plain build). One caveat worth
recording: the `RUSTFLAGS` environment variable is not additive with
`.cargo/config.toml`'s `target.<triple>.rustflags` — whichever one Cargo
picks, it uses *instead of* the other, never both. CI's workspace-wide
`RUSTFLAGS: -D warnings` would silently drop `simd128` for a wasm32 build
unless that job's own `env:` repeats both flags together (`-D warnings -C
target-feature=+simd128`), which is what the `wasm` job in
`.github/workflows/ci.yml` does. `piano-core` and `piano-wasm` do not write
any explicit SIMD code, so this entry stays open until someone measures
whether auto-vectorisation alone moves the needle, per this document's own
rule that a status closes only with a number.

---

### PERF-012

**Allocation at note-on.**

`PluckedString::new` allocates its delay line. That is correct for offline
rendering and *forbidden* in the realtime engine, where note-on happens on the
audio thread.

*Mitigation, as implemented*: rather than a small pool sized for the lowest note
and re-tuned per strike, `piano-audio::Engine` pre-allocates **one voice per key**
(all 88) at construction — each `PluckedString` already built and tuned for
exactly that key's frequency. `note_on` never constructs a string; it looks up
the existing voice for the struck key and calls `PluckedString::pluck`, which is
allocation-free by its own contract. This trades a larger one-time allocation
(roughly 150–300 KB across 88 delay lines, dominated by the bass) for a simpler
invariant: there is no "does this fit in the pre-sized buffer" question, because
every voice was sized for its own note from the start.

*Status (M2)*: **Closed.** `tests_no_allocation.rs` wraps the global allocator in
a guard active only while `drain_commands` and `process_block` run, drives 4 096
blocks with notes struck across the full keyboard including repeated re-strikes
of the same key, and asserts zero allocations in the guarded region. The test
was verified to actually detect a violation (a deliberately injected allocation
made it fail before being removed) rather than passing vacuously.

*Note (M5)*: stays closed. M5 added `Command::NoteOff` and
`Command::SustainPedal` — both handled by walking the existing fixed-size
`voices` array (`PianoKey`/index lookups, boolean flag writes,
`PluckedString::release` calls), none of it allocating — and
`tests_no_allocation.rs` was extended to push both new commands inside the
same guarded region rather than left to trust that by inspection alone.
Per-key voicing (`piano-audio::voicing`) only ever runs at `Engine::new`,
before the guard is active, same as the rest of voice construction.

*Update (M6)*: stays closed, verified again rather than assumed —
`tests_no_allocation.rs` was not itself changed, but it already exercises
`Engine::new` (via its `engine()` test helper) building the full 88-key
pool, now 88 `UnisonGroup`s of up to 3 delay lines each rather than 88
lone `PluckedString`s, and still passes. The one-time construction
allocation this entry already accepted as the trade grows with it: up to
222 delay lines now instead of 88 (`docs/PHYSICS.md`'s unison counts), so
the "roughly 150–300 KB" figure above is now closer to **300–600 KB**
(an estimate scaled by the same ~2.5x string-count growth `PERF-003`
measured, not a fresh measurement of its own) — still a one-time,
control-thread allocation, never on the audio thread, so this entry's
core guarantee is unaffected by the larger number.

---

### PERF-013

**`f32` precision in long bass decays.**

`f32` is the right default: half the memory traffic of `f64` and twice the SIMD
lane count, and PERF-010 says memory traffic is the real limit. But a bass string
with a 40-second decay has loop poles extremely close to the unit circle, where
`f32`'s 24-bit mantissa can produce audible quantisation in the tail or a decay
that stalls instead of reaching zero.

*Mitigation if measured*: keep `f32` as the sample and delay-line type, but use
`f64` for the **filter state** in the lowest octave only. State is a handful of
values per string; widening it costs almost no memory traffic.

*Test that would catch it*: render A0 for 60 seconds and check the envelope decays
monotonically to exact zero rather than sticking at a small constant.

---

### PERF-014

**`wasm-bindgen` call overhead at the JS↔Wasm boundary.**

`docs/REALTIME-AUDIO-RULES.md` forbids allocation inside the audio callback.
`piano-wasm::PianoVoice::render` itself honours that: its output buffer is
sized once at construction and never resized. But `AudioWorkletProcessor.
process()` — the actual per-quantum entry point, in JS — calls three
`wasm-bindgen`-generated functions every 128 samples: `render()`,
`outputPtr()` and (implicitly, via the cached `this.memory`) a fresh
`Float32Array` view constructor. Two honesty notes belong on the record
rather than in a commit message nobody rereads:

1. **`render()` and `outputPtr()` themselves are zero-copy.** Neither takes
   nor returns a slice — `render` takes no arguments and returns nothing,
   `outputPtr` returns a plain `u32` address — so `wasm-bindgen`'s glue does
   not marshal a buffer through its own allocator on these two calls. This
   is precisely why the API was designed this way instead of the more
   obvious `render(&mut self, output: &mut [f32])`: a `&mut [f32]` parameter
   would make the generated JS call `__wbindgen_malloc`/`__wbindgen_free`
   once per quantum to copy data across the boundary, which *would* be a
   real per-callback allocation and a real violation of the audio rules.
2. **The JS-side `new Float32Array(memory.buffer, ptr, quantum)` call in
   `piano-processor.js` is not free**, even though it copies no bytes.
   Constructing a typed-array view is a small object allocation in the JS
   engine's own (garbage-collected, not `wasm-bindgen`'s) heap, done once per
   `process()` call. It is unavoidable in this design because `strike()` can
   grow the Wasm heap between quanta, which detaches any cached view built
   over the old `ArrayBuffer` — see the comment at the call site. This is JS
   garbage-collector churn, not a Rust/Wasm allocation, and is the accepted
   cost of the zero-copy design; it has not been measured against the
   alternative (accepting the `wasm-bindgen` slice-copy overhead instead and
   never touching `memory.buffer` directly from JS).

*Why this is `Open` rather than `Mitigated`*: no cycle count or GC-pause
measurement exists for either path. The zero-copy design was chosen on
first-principles reasoning (avoid a guaranteed allocator round trip in
favour of a JS object allocation of unknown but probably-smaller cost), not
a benchmark — exactly the kind of claim this document's own rules say must
not be treated as settled without a number.

*What would close it*: a browser profiler trace of `process()` over a
sustained note, comparing GC pause frequency/duration against the
alternative `&mut [f32]`-parameter design, at whatever polyphony M3's
successor milestones eventually add to the browser build.
