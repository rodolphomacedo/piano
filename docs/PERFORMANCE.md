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
| [PERF-003](#perf-003) | Per-sample dispatch in the voice loop | M5 | Open |
| [PERF-004](#perf-004) | Linear interpolation in the fractional delay | M4 | Open |
| [PERF-005](#perf-005) | Dispersion allpass cascade | M4 | Open |
| [PERF-006](#perf-006) | Polyphony and voice management | M5 | Open |
| [PERF-007](#perf-007) | Hammer–string contact solver | M4 | Open |
| [PERF-008](#perf-008) | Sympathetic resonance coupling | M6 | Open |
| [PERF-009](#perf-009) | Soundboard convolution | M6 | Open |
| [PERF-010](#perf-010) | Cache behaviour of the delay-line working set | M7 | Open |
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
