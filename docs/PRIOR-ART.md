# Prior art and licence hygiene

This project is licensed **MIT OR Apache-2.0**. That choice constrains what we are
allowed to look at, and how.

## The rule

> **We do not read, copy, port, translate or closely paraphrase source code from
> copyleft-licensed projects.**

This is not caution for its own sake. Copying from a GPL or AGPL codebase into an
MIT/Apache-2.0 project is a licence violation that cannot be undone by rewriting
afterwards, and "I only looked at it for inspiration" is not a defence anyone has
to accept. The cost of a clean-room discipline is a few hours of reading papers
instead of reading code. The cost of getting it wrong is the project.

## OpenPiano — a benchmark, not a source

[`michele-perrone/OpenPiano`](https://github.com/michele-perrone/OpenPiano) is a
real-time piano engine in C++ with a JUCE plugin. It is **AGPL-3.0**, the
strongest copyleft licence in common use: it reaches not only distributed
binaries but software offered over a network.

It is the closest thing to a direct peer this project has, and pretending it does
not exist would be its own kind of amateurism. So it has a defined role.

**What OpenPiano is used for**

- **An auditory benchmark.** Rendering the same note on both and listening,
  comparing spectra, comparing decay envelopes. Listening to a binary creates no
  derivative work.
- **A reality check on scope.** Knowing what an open, working piano model actually
  achieves keeps this project's milestones honest.
- **A pointer to the literature.** Its documentation cites the sources (Chaigne &
  Askenfelt, Bilbao, and others). We read *those*, which are published scientific
  results and free to implement.

**What OpenPiano is never used for**

- Reading its source to learn how something is implemented.
- Copying, porting, or transliterating any function, structure, coefficient table
  or file layout.
- Taking its parameter values as our own.

**The architectural difference, stated plainly.** OpenPiano solves the string with
a **finite-difference** scheme: the string is a spatial grid and every grid point
is updated every sample. That is physically direct and produces excellent results,
and it costs `O(grid points)` per string per sample. This project uses a **digital
waveguide**, which is `O(1)` per string per sample because it exploits the fact
that the wave equation's solution is two travelling waves. That is a deliberate
divergence, chosen because efficiency is this project's stated priority — and it
means that even at the level of the core algorithm, the two projects are not doing
the same thing.

There is one place where finite differences may still earn a role here: as an
**offline oracle**. A slow, obviously-correct FD reference is a good way to
validate that the fast waveguide produces the right physics. If that is ever
built, it will be built from the published equations, in our own code.

## The literature we do build on

These are published papers and books. Implementing a published algorithm is not a
derivative work of anyone's source code.

- **J. O. Smith III**, *Physical Audio Signal Processing* — the digital waveguide
  formulation, loss and dispersion filter design, commuted synthesis. The
  foundation of this project's approach.
- **K. Karplus & A. Strong (1983)**, *Digital Synthesis of Plucked-String and Drum
  Timbres* — the algorithm milestone M1 implements.
- **D. Jaffe & J. O. Smith (1983)**, *Extensions of the Karplus-Strong
  Plucked-String Algorithm* — tuning correction, loop filter design, the
  extensions that make M1 usable.
- **A. Chaigne & A. Askenfelt (1994)**, *Numerical simulations of piano strings*
  (JASA 95) — the hammer–felt model and string parameters, in the original.
- **S. Bilbao**, *Numerical Sound Synthesis* — finite-difference schemes,
  stability analysis, and numerical hygiene that applies to any method.
- **B. Bank et al.** — efficient piano synthesis: soundboard modelling by resonator
  banks, and the beating behaviour of unison string groups.
- **J. Bensa, S. Bilbao, R. Kronland-Martinet & J. O. Smith (2003)**, *The
  simulation of piano string vibration: from physical models to finite difference
  schemes and digital waveguides* (JASA 114) — the paper that connects the
  finite-difference physics to the **waveguide** formulation this project actually
  uses. The most directly applicable source we have for improving string realism
  without abandoning the architecture.
- **G. Weinreich (1977)**, *Coupled Piano Strings* (JASA 62) — unison coupling
  through a shared bridge admittance, the fast pre-decay and slow aftersound.
  Already the cited source in `piano_core::unison`.
- **H. Fletcher (1964)**, *Normal Vibration Frequencies of a Stiff Piano String*
  (JASA 36) — inharmonicity, `f_n = n·f_1·√(1+B·n²)`. Already the cited source in
  `piano_core::dispersion`.
- **A. Stulov (1995)**, *Hysteretic model of the grand piano hammer felt* (JASA 97)
  — felt force depends on the *history* of compression, not only its current
  value. A refinement on top of a properly coupled hammer, not a replacement for
  one.
- **X. Boutillon & K. Ege (2013)**, *Vibroacoustics of the piano soundboard*
  (arXiv:1305.3057) — bridge mobility, modal density, and acoustical radiation
  regimes. The source for making the soundboard a mechanical load rather than a
  post-mix effect.

**Where the working list lives.** `MODEL-REVIEW.md` carries a literature register
mapping each paper to the specific issue it serves and whether it is already cited
in code. When a new paper arrives, add it there *and* here — the register says what
we intend to take from it, this list says we are allowed to.

## Practical policy for contributors

1. If you have read AGPL/GPL source for a component, **say so in the pull request**
   and do not write that component. Someone else will.
2. Cite the paper and equation number in a doc comment when implementing physics.
   It makes the code reviewable and it documents provenance.
3. Parameter values (string lengths, tensions, hammer stiffness) should come from
   published measurements or from our own fitting, with the source named in
   `docs/PHYSICS.md`.
4. When in doubt, the answer is the paper, not the repository.
