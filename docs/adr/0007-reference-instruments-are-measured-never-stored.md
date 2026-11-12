# 0007 — Reference instruments are measured, never stored

**Status**: Accepted

## Context

The physics fixes the form of every component and leaves numbers open: how
stiff a hammer's felt is, how fast a partial dies, how far a tuner stretches
the top octave. The literature gives ranges, often a factor of two wide, and
inside those ranges the model was reported as a guitar
([#122](https://github.com/rodolphomacedo/piano/issues/122)). Telling which
end of a range is right takes an instrument to compare with
(`docs/CALIBRATION.md`).

The instruments at hand are sampled ones: recordings somebody else made and
licensed. This repository's first rule is that it contains no recordings,
and `docs/PRIOR-ART.md` already draws the line for source code: other
projects are benchmarks and pointers to papers, never sources. The same
question had to be answered for audio before anything read a WAV
([#123](https://github.com/rodolphomacedo/piano/issues/123)).

## Decision

A reference instrument is something this project **measures**. It is never
something it **contains**.

1. Reference audio is read only by offline tools on the machine of someone
   entitled to use it. It is not committed, not fetched by CI, and no test
   needs it.
2. What enters the repository is derived numbers: a decay rate in decibels
   per second, an inharmonicity coefficient, a tuning offset in cents, the
   corner of a fitted curve. Each lives in a named constant whose doc
   comment says it was calibrated and points to `docs/CALIBRATION.md`.
3. Nothing enters from which the reference's sound could be rebuilt: no
   impulse response, no waveform, no spectrum at audio resolution. The
   soundboard is a synthesised field fitted to a handful of rise times and
   an envelope, not a convolution.
4. Where a fitted number is partly the reference's recording chain rather
   than the instrument, the constant says so (`radiation::SHORT_CIRCUIT_HZ`
   is one).
5. The gates test the model against its own invariants and against the
   derived numbers, so they pass on a machine that has never seen a
   reference.

## Consequences

**What it buys.** The parameters stop being guesses, and the repository
stays what it says it is: a model with no recordings in it. Any other
instrument can take the reference's place by repeating the measurements.

**What it costs.** The calibration cannot be reproduced from the repository
alone. A reader has to take the numbers on trust or measure a grand of
their own, and will not get the same third digit. The first pass was also
done with throwaway scripts; until
[#125](https://github.com/rodolphomacedo/piano/issues/125) brings the
measurement code in, the method is documented and the tooling is not.

**What it costs if it is wrong.** If measuring a licensed sample library
turns out to be something its licence does not allow, what has to go is a
bounded set of constants — the anchors of `voicing_scale.rs`, the knots of
`hammer.rs`, the corners of `radiation.rs` and `body.rs`, the octave types
of `voicing_stretch.rs` — each replaceable by the same measurement on an
instrument the project may use. No structure depends on where the numbers
came from.
