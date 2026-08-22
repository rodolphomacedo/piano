# The physics, and how it becomes code

Every component in `piano-core` corresponds to something a real string does. This
document is the map between the two. It is deliberately intuitive first and
mathematical second — the equations are in the papers listed in
`docs/PRIOR-ART.md`.

## Why a delay line is a string

The ideal string obeys the wave equation, and its solution is not a single
vibration but **two travelling waves**, one going left and one going right,
bouncing between the two fixed ends.

That is the whole insight behind digital waveguides. Instead of simulating the
string's shape at every point (which costs one update per grid point per sample),
you simulate the two travelling waves — and a travelling wave in a computer is
just a **delay line**. One round trip takes

```
period = sample_rate / fundamental_frequency
```

samples, and that is the delay length. The cost is `O(1)` per string per sample
regardless of how long the string is, which is why this project chose waveguides
over finite differences.

## Why there is a lowpass filter in the loop

If the reflections were perfect, the note would ring forever. They are not: energy
leaves through the bridge, and the string loses energy to air and to internal
friction. Crucially, **it does not lose it uniformly** — high partials die much
faster than low ones.

That is exactly the behaviour of a lowpass filter, and one pole is enough to
capture the essential character. It is why a piano note goes bright, then mellow,
then quiet, in that order.

Implemented as `filter::OnePoleLowpass`. The `damping` parameter is that pole.

## Why the loop is shorter than the period

The loss filter is *inside* the feedback loop, so it contributes its own delay.
A one-pole lowpass `H(z) = (1-a)/(1 - a·z⁻¹)` has phase delay `a/(1-a)` samples at
low frequency. There is also one sample of delay from the feedback path itself.

So the delay line must be

```
loop_delay = period − filter_phase_delay − 1
```

Skipping this term detunes the instrument, mildly in the bass and badly in the
treble. It is implemented in `PluckedString::new`, and the test
`loop_delay_is_close_to_the_period` guards it. Measured output for A4 lands within
about 1.5 cents of 440 Hz.

## Why the excitation is noise (for now)

Karplus–Strong fills the delay line with noise. Physically this is a very crude
model of a pluck: it says "give the string a random initial displacement". It is
wrong in an interesting way — noise contains every partial equally, and the loop
then filters it into a harmonic series. It sounds like a plucked string because it
*is* a plucked string, badly excited.

A piano is not plucked, it is struck by a felt hammer, and the hammer is what makes
a piano sound like a piano. That is milestone M4.

## What the current model does not do

Stated plainly, because these are the gaps that later milestones close:

| Missing | Consequence | Milestone |
|---|---|---|
| **Stiffness / inharmonicity** | Partials sit at exact integer multiples. Real piano partials are progressively sharp; this is a large part of why a piano sounds unlike a guitar. | M4 |
| **Hammer excitation** | Noise burst instead of a felt strike. No velocity-dependent brightness — a real hammer gets harder the faster it hits. | M4 |
| **Multiple strings per note** | No beating, no aftersound. Real notes have 2–3 slightly detuned strings whose interference produces the characteristic two-stage decay. | M6 |
| **Sympathetic resonance** | Sustain pedal does nothing. | M6 |
| **Soundboard** | The string signal is heard raw; a real piano is heard through a radiating wooden plate. | M6 |
| **Longitudinal modes** | No metallic "phantom partials" of the low bass. | Backlog |

## Numbers worth having

At 48 kHz on an 88-key piano:

| | A0 (27.5 Hz) | A4 (440 Hz) | C8 (4186 Hz) |
|---|---|---|---|
| Period, samples | 1745 | 109 | 11.5 |
| Delay buffer (pow-2) | 2048 | 128 | 16 |
| Memory per string | 8 KB | 512 B | 64 B |
| Typical decay | 30–40 s | 8–15 s | 1–2 s |
| Dispersion sections needed | ~8 | ~2 | ~0–1 |

The last row is why `PERF-005` insists that dispersion order be scaled per
register rather than fixed: paying for eight allpass sections on C8 would be
half the treble's CPU budget spent on an effect nobody can hear.
