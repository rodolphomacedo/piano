# 0004 — MIT OR Apache-2.0, with a clean-room policy

**Status**: Accepted

## Context

The most prominent open piano models are GPL or AGPL. `OpenPiano` — the closest
peer this project has — is AGPL-3.0, which reaches software offered over a network
and not only distributed binaries.

Two options were available: adopt a copyleft licence and be free to study that
prior art, or stay permissive and accept a reading restriction.

## Decision

License as **MIT OR Apache-2.0** (the Rust ecosystem default), and adopt an
explicit clean-room policy: **no copyleft source is read for a component we
intend to write.**

Prior art is used as an *auditory benchmark* and as a *pointer to the literature*.
The physics is implemented from published papers, which are free to implement
regardless of what any repository does with them.

## Consequences

**What it buys.** The crates can be depended on by anyone, including in
proprietary work, which is what makes a synthesis library useful. It also removes
an entire class of legal risk that cannot be remediated after the fact — you
cannot un-copy code by rewriting it later.

**What it costs.** Real effort. Reading a paper and deriving a filter design is
slower than reading a working implementation. Some engineering knowledge that
exists only in someone's source will have to be rediscovered.

**How it is enforced.** `docs/PRIOR-ART.md` states the rule, `CLAUDE.md` repeats it
for AI agents, and contributors are asked to disclose in the pull request if they
have read copyleft source for the component in question — in which case someone
else writes it.
