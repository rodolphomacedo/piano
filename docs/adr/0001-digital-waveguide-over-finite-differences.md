# 0001 — Digital waveguide, not finite differences

**Status**: Accepted

## Context

There are two established ways to simulate a vibrating string.

A **finite-difference** scheme discretises the string in space and time: the
string becomes a grid of points and every point is updated every sample. It is
physically direct, it extends naturally to stiffness and nonlinearity, and its
cost is `O(grid points)` per string per sample. A realistic bass string needs
hundreds of grid points. Several respected open piano models take this route.

A **digital waveguide** exploits d'Alembert's solution: the wave equation's
solution is two travelling waves, so the string can be simulated as two delay
lines with filters at the ends. Cost is `O(1)` per string per sample, regardless
of the string's length.

A full piano is up to 240 simultaneously ringing strings. The difference between
`O(1)` and `O(hundreds)` per string per sample decides whether that is possible at
all on a laptop.

## Decision

Use a digital waveguide as the string model.

## Consequences

**What it buys.** Roughly two orders of magnitude less work per string. A
polyphonic piano becomes feasible on one core, which is the entire premise of the
project.

**What it costs.** Effects that are natural in a finite-difference grid have to be
engineered into filters:

- Stiffness becomes an allpass cascade whose coefficients must be designed
  (`PERF-005`), rather than falling out of the equations.
- Nonlinear behaviour — the hammer, tension modulation, longitudinal modes — is
  harder, because a waveguide assumes linearity and superposition.
- The excitation point and pickup point must be modelled explicitly rather than
  being "wherever you touch the grid".

**The exit if it is wrong.** A finite-difference implementation would be a new
module behind the same voice interface, not a rewrite. It may also earn a place as
a slow, obviously-correct **offline oracle** for validating the waveguide's
physics — see `docs/PRIOR-ART.md`.
