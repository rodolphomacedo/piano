# Architecture

## The one idea

**The synthesis engine knows nothing about the outside world.** It does not know
whether it is being driven by a CLI, a desktop app, a browser, a MIDI keyboard or
a test harness. Everything platform-specific lives in a crate that depends on the
engine, never the other way round.

This is what makes the same code run natively, in a browser and in a unit test
without a single `#[cfg]` in the DSP.

## Crate layout

```
piano-core     pure DSP. no_std + alloc. forbid(unsafe_code). No I/O, no threads,
               no time, no allocation while processing.
                 │
                 ├── piano-params    notes, MIDI numbers, tuning. no_std.
                 │        │
                 │        ├── piano-render   offline rendering + WAV files (std)
                 │        │        │
                 │        │        └── piano-cli   the `piano` binary
                 │        │
                 │        ├── piano-audio    [M2] realtime output via cpal
                 │        └── piano-wasm     [M3] browser bindings + AudioWorklet
```

Dependencies point in one direction only. A change in how audio reaches the
speakers cannot affect the physics, and a change in the physics cannot break the
browser build in a way the native build would not catch.

### `piano-core`

The whole point of the project. Contains delay lines, filters, excitation and
string models, and nothing else.

Its contract is written at the top of `src/lib.rs` and enforced by
`docs/REALTIME-AUDIO-RULES.md`: no allocation while processing, no panics, no
platform coupling. It is `no_std` not because anyone wants to run this on a
microcontroller today, but because `no_std` makes most real-time violations
*impossible to write* rather than merely discouraged.

### `piano-params`

The musical layer: note names, MIDI numbers, the 88-key range, tuning. Separated
because the DSP genuinely does not need it, and mixing "what is A4" with "how does
a string vibrate" is how synthesisers become impossible to test.

### `piano-render`

The slow, allocating, file-touching half: render a note to a buffer, write a WAV.
Deliberately a different crate from `piano-core` so that nothing in it can ever be
called from an audio callback by accident.

### `piano-cli`

The thinnest possible shell: parse arguments, call one library function, print
what happened. If a decision matters, it does not live here.

### `piano-audio` (M2) and `piano-wasm` (M3)

Not yet written. `piano-audio` owns the `cpal` stream, the lock-free command ring,
and the platform-specific denormal control (`PERF-002`) — which is also where the
first `unsafe` in the project will live, if any does. `piano-wasm` owns the
`wasm-bindgen` surface and the `AudioWorklet` glue.

## How a note becomes sound

```
  "A4"                            piano-params    parse + tune
    │
    ▼
  440 Hz  ──────────────────────  piano-core      StringConfig
    │
    ▼
  loop delay = 48000/440 - filter delay - 1  =  107.9 samples
    │
    ▼
  ┌──────────────────────────────────────────────┐
  │  noise burst ──▶ [ delay line 107.9 ] ──┬──▶ │ output
  │                        ▲                │    │
  │                        │                ▼    │
  │                   [ × sustain ] ◀── [ lowpass ]
  └──────────────────────────────────────────────┘
    │
    ▼
  DC blocker ──▶ envelope follower ──▶ samples
```

The delay line is the string's length. The lowpass is the fact that a real
reflection loses high frequencies faster than low ones. The sustain gain is
broadband loss. That is the whole of milestone M1, and every later milestone adds
elements *inside that loop* rather than replacing it.

## Where the design deliberately leaves room

These are the extension points that later milestones plug into, listed so that
nobody has to reverse-engineer the intent:

| Extension | Where it goes | Milestone |
|---|---|---|
| Dispersion (inharmonicity) | An allpass cascade inside the loop, after the loss filter | M4 |
| Hammer excitation | Replaces the noise burst; couples to the loop at a strike position | M4 |
| Better fractional delay | Swaps `read_interpolated` for allpass or Lagrange | M4 |
| Multiple strings per note | A voice owns 2–3 `PluckedString`s with detuned frequencies | M6 |
| Sympathetic resonance | A shared bridge bus every voice reads and writes — **not** all-to-all coupling (`PERF-008`) | M6 |
| Soundboard | A single post-mix stage, after all voices are summed | M6 |
| Block processing | `process_block_add` already has the signature; the engine loop order is what changes (`PERF-010`) | M5 |

## Testing strategy

Three layers, each catching what the others cannot:

1. **Unit tests** on every DSP primitive, checking behaviour rather than
   implementation: does the step response converge, does the DC blocker remove DC,
   does capacity round to a power of two.
2. **Property tests** (`proptest`) for the guarantees that must hold for *all*
   inputs: reading any delay never panics, the loss filter never diverges for any
   coefficient. These are the tests that make "never crashes" a checked claim
   rather than a hope.
3. **Physical tests** on the assembled model: a plucked string decays
   monotonically, eventually reaches silence, and is in tune to within a few cents
   of its target frequency.

Determinism makes all of this possible — the excitation is a seeded xorshift, so
the same note renders identically every time.
