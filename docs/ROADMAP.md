# Roadmap

Milestones are **demonstrable states of the instrument**, not lists of tasks. Each
one ends with something you can hear or measure. The order is chosen so that the
project is audible from day one and stays audible.

Every milestone here maps to a GitHub milestone of the same name.

---

## M0 — Foundations ✅

Cargo workspace, lint gate, CI, and the documents that define how the project
works. No sound yet, but the rules that make the rest possible.

**Done when**: `cargo clippy --workspace --all-targets -- -D warnings` is clean and
CI runs on every push.

---

## M1 — First sound ✅

Karplus–Strong plucked string, rendered offline to a WAV file through a CLI.

**Done when**: `piano render --note A4 --seconds 2` produces an audible file whose
measured fundamental is within a few cents of 440 Hz.

*Achieved: 1.4 cents.*

---

## M2 — Real-time playback ✅

Live audio output on macOS through `cpal`, driven by a lock-free command queue.
This is where the real-time rules stopped being theory.

**Includes**: the SPSC ring buffer (`rtrb`), the audio-thread contract, hardware
denormal control (`PERF-002`), a callback timing histogram (p50–p99.9), and a
no-allocation proof test. `piano play --note A4` plays one note through the
default output device; `piano keyboard` plays live from the computer keyboard;
`piano midi` plays live from a hardware MIDI controller via `midir`, nothing
written to disk in any case. Damping, sustain and the excitation seed are live
setters reachable from any of the three (`--damping`/`--sustain` flags,
`[`/`]`/`-`/`=` on the keyboard, CC74/CC1 over MIDI), not build-time constants
— the instrument is fully parametrised, ready for both manual voicing and a
future automated one (see "Ideas beyond M8" below).

**Done when**: `piano play --note A4` makes sound with no allocation, no locking
and no missed buffers over a ten-minute run.

*Achieved, with one open item*: the audio path is proven allocation-free by a
custom-allocator test (`piano-audio/src/tests_no_allocation.rs`, `PERF-012`
closed), and callback timing is measured and reported. Not yet done: a
ten-minute unattended soak run, and full device-hot-swap recovery (issue #20
covers panic-free stream error reporting; retuning voices after a live
sample-rate change is deferred).

---

## M3 — Web playground ✅

The core compiled to WASM and driven from an `AudioWorklet`, with a page that has
a button and a frequency slider. Ugly is fine. The point is a fast feedback loop
for anyone, on any machine, with no toolchain.

**Includes**: `simd128` build flags (`PERF-011`) and a 128-sample block size
matching the worklet quantum.

**Done when**: clicking a button in a browser plays the same string model.

*Achieved, with one open item*: `piano-wasm` wraps a single `PluckedString` in
a `wasm-bindgen` `PianoVoice`, built and reviewed to the same no-allocation
standard as `piano-audio::Engine` (`PianoVoice::render` is the per-quantum
entry point; `PianoVoice::strike`, the one allocating call, is documented to
run only between quanta — see `PERF-014`). The plain HTML/JS page and
`AudioWorkletProcessor` in `crates/piano-wasm/www/` were written against, and
verified against, the actual `wasm-bindgen 0.2.100 --target web` glue —
generating that glue and reading it caught a real bug before it could reach
a browser (`initSync` is a named export, not the default one, in this
`wasm-bindgen` version; the worklet's import statement had it backwards).
`cargo build -p piano-wasm --release --target wasm32-unknown-unknown`
succeeds with `simd128` enabled, and `wasm-bindgen` turns that binary into
loadable JS/Wasm without errors. **Not yet done**: no human has opened a
browser tab and clicked the button — the one verification this milestone's
own "done when" line names cannot be claimed from a terminal, and was not
available in the environment this milestone was built in. Whoever confirms
that should also delete this sentence.

---

## M4 — Real piano physics ✅

The milestone that turns a plucked string into a struck one. The largest
single jump in sound quality in the whole roadmap.

**Includes**: a proper digital waveguide, a frequency-dependent loss filter, an
allpass dispersion cascade for inharmonicity (`PERF-005`), a better fractional
delay (`PERF-004`), and a nonlinear felt hammer model (`PERF-007`).

**Done when**: a struck note is audibly a piano rather than a guitar, its partials
are measurably sharp in the way a real piano's are, and hitting harder makes it
brighter rather than merely louder.

*Achieved, with one open item*: all four pieces landed in `piano-core` —
`DelayLine::read_allpass` (first-order allpass fractional delay, `PERF-004`),
`piano_core::dispersion::DispersionCascade` (a register-scaled allpass cascade
implementing Fletcher's stiff-string inharmonicity formula, `PERF-005`,
exposed live via `StringConfig::inharmonicity`/`PluckedString::set_inharmonicity`
the same way `damping`/`sustain` already were), `filter::LoopFilter` (a
one-pole-one-zero loss filter so upper partials measurably decay faster than
the fundamental, replacing the single-pole design), and
`piano_core::hammer::simulate_contact` (a bounded nonlinear
Hertzian-contact hammer model, `PERF-007`, shaping the excitation from the
existing `velocity` parameter rather than merely scaling it). Both objective
"done when" criteria have real measured numbers, not assertions
(`crates/piano-render/tests/m4_spectral.rs`, run as part of
`cargo test --workspace`): rendering A4 shows partials 1 through 7 sharpening
with increasing partial number (roughly -2, +0.7, -0.2, +0.7, +1.3, +3.6,
+6.1 cents of deviation from an exact harmonic series), the qualitative
signature a pure Karplus-Strong loop cannot produce; and a hard strike
(velocity 0.95) measures a spectral centroid of about 910 Hz against a soft
strike's (velocity 0.15) 850 Hz for the same note, with neither render
loudness-normalised, confirming brightness comes from shape, not level.
**Not yet done**: the third criterion, "audibly a piano rather than a
guitar", is a human-hearing judgement no test suite can make. Sample renders
at A2, A4, A6, and a soft/hard A4 pair for a direct brightness comparison,
were written to `/tmp/piano-m4-samples/` for a person to listen to — nobody
had done so as of this milestone landing. Whoever confirms that should also
delete this sentence. Also open: every `PERF-004`/`PERF-005`/`PERF-007` entry
in `docs/PERFORMANCE.md` is "Implemented, unmeasured" rather than "Closed" —
no cycle counts exist yet for the new per-sample cost this milestone added,
per that document's own rule that a status closes only with a number.

---

## M5 — The whole keyboard ✅

88 keys, real polyphony, and the voice management that makes it survivable.
Basic MIDI note input already landed in M2 (`piano midi`, one permanent voice
per key means no stealing is needed yet); what M5 still owed was the sustain
pedal and a release/damping model — the instrument had no way to stop a note
early, MIDI or otherwise.

**Includes**: a pre-allocated voice pool (`PERF-012`), energy-gated voice skipping
and voice stealing (`PERF-006`), block-based processing (`PERF-003`, `PERF-010`),
and per-key parameter tables.

**Done when**: a MIDI keyboard plays chords, the sustain pedal works, and the
callback holds its deadline at the documented voice count.

*Achieved, with open items.* A release/damping model landed in
`piano-core`: `PluckedString::release` (`crates/piano-core/src/string.rs`)
engages a damper flag that multiplies the loop's broadband gain by
`RELEASE_LOSS_MULTIPLIER` (0.4, Chaigne & Askenfelt 1994's per-round-trip
loss model) on every following round trip, reaching
`SILENCE_THRESHOLD` in roughly 10 round trips regardless of register — and,
since the envelope follower that gates energy-based voice skipping has its
own independent forgetting rate, a second constant
(`RELEASED_ENVELOPE_DECAY`) was needed so `is_silent` actually reflects that
fast decay instead of the ~380 ms floor the held-note follower constant
would otherwise impose; a `proptest` (`release_any_number_of_times_never_
breaks_the_string`) checks totality across any number of `release` calls at
any point relative to plucking. `piano-audio::Engine` (`engine.rs`) wires
this to a new `Command::NoteOff` and a `Command::SustainPedal` on the same
SPSC command-queue pattern `SetDamping`/`SetSustain` already used, with a
per-voice `pending_pedal_release` flag: a `NoteOff` while the pedal is down
marks the voice instead of releasing it, and `SustainPedal { down: false }`
walks all 88 voices (a compile-time-bounded loop, not unbounded) releasing
everything that was pending — naming throughout keeps this unambiguous
against the pre-existing, unrelated CC1→`set_sustain` decay-rate control
(`AudioSession::set_sustain_pedal`, never a bare `set_sustain`-adjacent
name). `piano-midi` needed no new parser variant — CC64 falls out of the
existing generic `ControlChange` decoding — so only `piano-cli`'s `midi.rs`
changed, mapping CC64 ≥ 64/127 to pedal-down and wiring `NoteOff` to
`AudioSession::note_off`. Per-key parameter tables landed as
`piano-audio::voicing` (`voicing.rs`): each of the 88 keys gets its own
`damping`/`sustain`/`inharmonicity` baseline, `sustain` derived by a closed
form from this project's own already-documented per-register decay times
(`docs/PHYSICS.md`'s "Typical decay" row) and `inharmonicity` interpolated
between the bass/treble figures `piano_core::dispersion` already cites from
Fletcher & Rossing; `damping`'s bass/treble anchors are this project's own
reasoned (not measured) interpolation, honestly labelled as such in the
module doc comment. Energy-gated voice skipping (`PERF-006`'s cheap half)
turned out to already exist from M2 — `Engine::process_block`'s `if
string.is_silent() { continue; }` — verified by `git log -p`, not assumed;
what M5 actually added on that front was making a *released* voice reach
`is_silent` promptly (above) so that skip has something to bite on soon
after a note-off, not just after a long natural decay. True voice
*stealing* was not built, per this milestone's own scope: one permanent
voice per key (`PERF-012`, already closed in M2) means slots never run
out, so there is nothing to steal from. A real measurement exists for the
"holds its deadline at the documented voice count" criterion:
`piano-audio::engine::tests::callback_time_at_full_88_voice_polyphony_
clears_the_deadline` (run manually, `#[ignore]`d in CI because wall-clock
timing on a shared runner is not a fair pass/fail gate) measured 221.9 µs
per 128-sample block with all 88 voices freshly struck, against a 2.67 ms
deadline, on the exact reference machine `docs/PERFORMANCE.md`'s budget
section names (a 2.3 GHz Intel Core i5-8259U) — about 8% of the deadline,
comfortably inside it. **Not yet done**: the computer keyboard's key-release
was investigated, not assumed — most terminals, including macOS's default
Terminal.app, only ever deliver key-*press* events in raw mode; genuine
key-up needs the terminal to implement the Kitty keyboard protocol (kitty,
WezTerm and a still-minority list of others). `piano keyboard` now queries
`crossterm::terminal::supports_keyboard_enhancement()` and only claims
early release when the terminal actually reports it, printing an honest
message either way — this was not run against a real interactive terminal
in the environment this milestone was built in (no TTY), so the *enabled*
path (uncommon terminals) is implemented against the documented crossterm
0.28 API but unverified by an actual keypress; the *disabled* path (the
common case, unchanged ring-to-completion behaviour) is exactly what
shipped before this milestone and was already exercised. No human has
listened to a chord or a pedal-held note yet — nobody had done so as of
this milestone landing. Whoever confirms either of those should delete
this sentence.

---

## M6 — Instrument realism ✅

The things that separate "a piano model" from "an instrument": unison string
groups and their beating, sympathetic resonance through a shared bridge bus
(`PERF-008`), and a soundboard (`PERF-009`).

**Done when**: holding the sustain pedal and playing a note makes the rest of the
instrument ring, and a note has the two-stage decay real pianos have.

*Achieved, with open items.* All three pieces named above landed, each
reusing rather than forking the existing model per the milestone's own
scope:

- **Unison strings.** `piano_core::unison::UnisonGroup` gives each key 1,
  2 or 3 `PluckedString`s (never a new DSP primitive) — 12 single-strung
  bass keys, 18 double-strung tenor keys, 58 triple-strung treble keys
  (`piano_core::unison`'s module docs cite A. Reblitz's *Piano Servicing,
  Tuning, and Rebuilding* for the standard modern-piano layout this
  boundary choice represents; exact break points vary by instrument, so
  this is this project's own representative choice within that
  convention, stated as such). That raises the engine's effective string
  count from 88 to **222** (`12·1 + 18·2 + 58·3`), close to `PERF-008`'s
  own illustrative `N = 240` estimate — `docs/PERFORMANCE.md`'s
  `PERF-003`/`PERF-006`/`PERF-010`/`PERF-012` entries are all updated with
  what that costs (a new **697.2 µs/128-sample block** measurement on the
  documented reference machine, up from M5's 221.9 µs, still comfortably
  inside the 2.67 ms deadline at about 26 %).
- **Sympathetic resonance (`PERF-008`).** `piano_core::bridge::BridgeBus`
  is the shared, `O(N)` bus the entry demanded rather than the infeasible
  `O(N²)` mesh: every voice writes its own bridge-end signal into one
  running **average** (not sum — an earlier, unnormalised-sum version
  diverged to infinity within a few blocks of holding the pedal down,
  caught by a test disagreeing with expectation, not by inspection — see
  the module's own honesty note) and reads back everyone else's, one
  block of latency (~2.7 ms at 48 kHz). `piano_audio::engine::Engine::
  set_sustain_pedal` lifts every idle voice's damper while the pedal is
  down, not just a held key's, so a struck note's energy genuinely
  reaches other, unstruck strings and they audibly ring — the pedal
  behaviour this milestone's own "done when" line names.
- **Soundboard (`PERF-009`).** `piano_core::soundboard::Soundboard`, a
  bank of 8 two-pole modal resonators (frequency, decay time, gain, all
  literature-informed, not measured from a specific instrument), mixed
  additively into `Engine`'s post-mix output. **Modal synthesis, not
  convolution against a measured impulse response, was the only option
  this project could take**: the other two options `PERF-009` originally
  listed both require possessing a recorded impulse response from a real
  soundboard, and this repository's own `CLAUDE.md` prohibits adding any
  recorded or sampled audio asset, unconditionally, regardless of how the
  performance register's own phrasing reads. **No audio recording,
  sampled impulse response, or other captured-audio asset was added
  anywhere in building this milestone.**
- **Two-stage decay.** Measured, not hand-tuned on top:
  `crates/piano-render/tests/m6_spectral.rs` renders a trichord A4 and
  shows its early decay rate (just after the attack transient) is
  measurably faster than its settled rate, and that the settled rate then
  sits close to a monochord control's own natural decay — the qualitative
  and quantitative signature G. Weinreich's "Coupled Piano Strings" (JASA
  62(6), 1977) predicts for near-unison strings coupled through a shared
  bridge. This genuinely emerged from the coupled-string model rather
  than being asserted or faked: two real implementation bugs were found
  and fixed by this measurement disagreeing with expectation during
  development — an additive (rather than convex) local-coupling term that
  diverged for near-lossless strings, and every unison string within a
  group sharing one excitation noise seed, which suppressed the beating
  this whole mechanism exists to produce. Both are documented in full in
  `piano_core::string`'s and `piano_core::unison`'s module docs.

**Not yet done**: no human has held the sustain pedal and played a note,
or listened for the beating/two-stage decay, on real hardware — every
claim above is a passing automated test or a measured number, not a
listening confirmation, the same open item M3/M4/M5 each left for
whoever next has access to real audio hardware. The coupling gains
(`LOCAL_COUPLING_GAIN`, `GLOBAL_COUPLING_GAIN`) and the soundboard's mode
table are honestly labelled as literature-order-of-magnitude, reasoned
choices, not fit to any measured instrument — a future milestone could
calibrate them against a real recording (an *offline*, non-shipped
calibration tool, the same "Ideas beyond M8" pattern already described
below for Bayesian parameter estimation), but none of that landed here.
`piano play`, `piano keyboard` and `piano midi` all go through
`piano_audio::Engine`, so they already carry every M6 piece described
above with no further wiring needed. `piano render` — the offline WAV
export, via `piano_render::render_note` — was deliberately **not**
changed to use unison strings, the bridge bus or the soundboard: that
function is exactly what M1's and M4's already-measured tuning and
inharmonicity tests are calibrated against, and risking a regression in
an already-closed measurement for the sake of also covering M6 there was
judged not worth it. `piano_render::render_unison_note` is the M6-aware
sibling the new spectral test uses instead, not yet wired into the CLI.
`piano-wasm`'s single-voice browser demo was also left untouched — M3
asked for a button and a slider, not polyphony, so it has no second voice
for sympathetic resonance to reach anyway. Whoever confirms the listening
test should also delete this sentence.

---

## M7 — Performance engineering

A dedicated pass with the benchmark harness in place: profile, measure, and close
`PERF` register entries with numbers.

**Includes**: criterion benchmarks per component, a callback-timing harness that
reports p99.9 rather than mean, SIMD where measurement justifies it, and
resolution of `PERF-001`, `PERF-010` and `PERF-013`.

**Done when**: every open `PERF` entry has either a measurement showing it does not
matter or a fix with a before/after number.

---

## M8 — Release and packaging

Reproducible builds for macOS Intel, macOS Apple Silicon and Linux; published
crates; a documented plugin path if it is worth taking.

**Done when**: someone who is not the author can install it and play it.

---

## Backlog — future optimisations

Items that are real but not scheduled. The `PERF` register in
`docs/PERFORMANCE.md` is the authoritative list; anything there without a
milestone lives here.

---

## Ideas beyond M8

Not scheduled, not sized, and not started — recorded so the reasoning behind
them is not lost between conversations.

**Bayesian parameter estimation from a recorded instrument.** Every parameter
this model exposes (damping, sustain, dispersion once M4 lands, hammer
hardness) is a live, adjustable control as of M2 — the prerequisite for
fitting them automatically rather than only by ear. The idea: record a real
instrument's note, extract features (partial decay rates, inharmonicity
coefficients, attack shape), and use Bayesian inference (Stan, or a
Rust-native sampler such as `nuts-rs`/`bridgestan` bindings to avoid a
non-Rust runtime dependency) to fit the physical model's parameters against
that recording — a calibration problem, not a real-time one, so none of the
audio-thread rules apply to it. Plausible in principle for a Karplus-Strong
model's handful of scalar parameters; substantially harder once M4's full
waveguide and M6's coupled strings add dozens of interacting ones. The
recordings this needs are training data for an *offline* fitting tool, never
compiled into the instrument itself, so they cannot live in this repository —
its "no samples, ever" rule (see the project's `CLAUDE.md`) is about what
ships, not about what a separate calibration tool may read from disk. This
belongs in its own repository that depends on `piano-core` for the model to
fit, not the other way around.
