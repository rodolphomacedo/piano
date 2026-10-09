# The soundboard as a load on the strings (issues #90, #91) — design

Status: implemented (per-key bank; the shared-board prediction below was tried and rejected). Implements P4.1 and P4.2 of `docs/MODEL-REVIEW.md` and
resolves N2 there. Validated against the full-engine harness (#87).

## Context

`Engine::process_stereo_chunk` runs strings → mix → soundboard → limiter. The
soundboard colours the output but never acts back on a string, and the only
path back into a string through the bridge is a **scalar**: the cross-key
bus readback, scaled by `1 − loop_gain` in
`PluckedString::write_mixed_feedback`. A real bridge's mobility `Y_b(ω)` is
neither scalar nor one-way: its resonances are the board's modes, and the
velocity they impose on the bridge is a boundary condition on every string
resting on it (Weinreich 1977; Boutillon & Ege 2013).

## The physical decomposition

Boutillon & Ege (2013, "Vibroacoustics of the piano soundboard",
arXiv:1305.3057) separate the bridge mobility into two regimes:

- **Low frequencies — modal.** Below a few hundred hertz to ~1 kHz the
  mobility is a sum of resolved board modes: peaks at the mode frequencies,
  valleys between.
- **High frequencies — smooth.** Modal density grows until the mobility
  approaches that of an infinite plate (stiffened by ribs): a nearly real,
  frequency-flat conductance.

This project already has one piece of each. The 28-mode table (#78) is the
modal part, but only radiates. The cross-key `BridgeBus` is a flat
conductance shared by every key, i.e. the high-frequency asymptote. This
design makes the modal part a load and keeps the bus as the flat part:

```
Y_b(z) = Σ_k a_k · BP_k(z)   +   Y_flat
         └── BridgeLoad, new ──┘  └── BridgeBus, existing ──┘
```

`BP_k` is the constant-peak bandpass form of mode `k`'s resonator,
`(1 − z⁻²)·R_k(z)/(2 sin θ_k)`. It has the same poles as the radiating
mode, unit gain and zero phase at resonance, and a non-negative real part at
every frequency. Fed back negatively, it therefore only removes energy:
most at the board's resonances, little between them. `a_k` is
`SoundboardMode::bridge_coupling`, carried since #78 and read for the first
time here.

## Rejected: one shared board, predicted a block ahead

The engine renders voice-outer and sample-inner (`PERF-010`). A voice
renders its whole block before the next voice starts, so no string can read
*this* sample's velocity from a board shared by every key. The first design
got around that by splitting each mode's response in two. The **free
response** from the block's starting state is known before any voice runs,
so it was predicted over the block and read sample-accurately. The
**forced response** to the block's own forces reached the strings one block
later.

That load measured as **not passive**. With one string per note and load
gain 4, some partials decayed *slower* with the load on than off: ratio
0.67 at MIDI 45 H3, 0.75 at MIDI 57 H3, 0.73 at MIDI 81 H2. The free
continuation of a mode driven off resonance rings at the *mode's* frequency,
not the string's, so its phase against the string drifts across the block
and part of the time it pushes energy in. Since the bandpass's non-negative
real part only holds for the exact, causal response, the prediction broke
the passivity argument.

## Adopted: every key carries its own bank

Each `UnisonGroup` owns a `BridgeLoad`, driven sample by sample by its own
strings' mean bridge signal with no latency and no algebraic loop. The
velocity it returns this sample comes from the resonators' past state plus
this sample's force, through the `1 − z⁻²` numerator. The bandpass therefore
stays exact and passive. What this gives up is the board's cross-key modal
path: key A no longer drives key B through a board *mode*. That path is the
sympathetic resonance the flat `BridgeBus` already carries, with its
inaudible block of latency, so cross-key coupling keeps its flat model and
the frequency-dependent part is per key.

A key only carries modes that can take its energy, those between
`0.8 × f0` and 1 kHz (`PERF-015`, `bridge_load` module docs). That cuts the
full-polyphony cost from +0.80 ms to +0.30 ms per block.

Measured (`crates/piano-audio/tests/board_load.rs`): with the load on,
every key loses energy every second and never gains any, at most about 3 dB
after five seconds in the bass at the default gain. No single-string
partial decays slower at the maximum gain. A0's fundamental decays 2.2×
faster where board modes cluster, and partials far from a mode change by a
few percent. That is the frequency dependence #91 asks for.

## N2: deferred, with the measurement that justifies it

`voicing.rs` solves each key's loop losses for a string in isolation, and
the load removes a little more. At the default gain that is at most about
3 dB after five seconds (bass), 0.5 dB through the middle and 2.6 dB at C7.
That is small next to the spread the scale table itself allows. A per-key
decay correction regenerated like `LEVEL_CORRECTION_DB` stays the right fix
once the load gain is tuned by ear. Until then it would pin a
target that is still moving.

## Totality and safety

- `BridgeLoad::process` clamps its input and flushes denormals, as
  `Soundboard::process` does. Every coefficient comes from
  `Resonator::new`, which is already total.
- `board_load_gain` is clamped into `[0, MAX_BOARD_LOAD_GAIN]` engine-side.
- Stability is not argued. It is tested by: a full sustain-pedal glissando
  at maximum load gain staying bounded and decaying; the command-stream fuzz
  covering the new command; and a proptest over `f32::ANY` on every new
  entry point.

## Non-goals

- Restructuring the engine to sample-outer order.
- Making the bus frequency-dependent. It is the flat asymptote on purpose.
- Changing the radiating soundboard's drive, gains or stereo image.
