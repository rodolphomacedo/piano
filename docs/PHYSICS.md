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

That is exactly the behaviour of a lowpass filter. It is why a piano note goes
bright, then mellow, then quiet, in that order.

Implemented as `filter::LoopFilter`, a one-pole, one-zero design (M4; see "Why
upper partials sit sharp", below, for the allpass dispersion cascade this
composes with). The `damping` parameter is the pole; `zero_mix` is the second
parameter the section below exists to explain.

### Why the zero had to become a second, per-string parameter

M4 added a fixed zero at Nyquist on top of the pole, for extra high-frequency
rolloff a bare pole cannot give (D. Jaffe & J. O. Smith, "Extensions of the
Karplus-Strong Plucked-String Algorithm", 1983). That zero costs real
amplitude at any fundamental that is not close to DC, independent of the
pole — even `pole = 0` still carries the zero's own loss. For a bass string
that loss is negligible (A0's fundamental sits far below Nyquist), but for a
treble string completing thousands of round trips a second it compounds
catastrophically: measured with the zero fixed at Nyquist and `damping`
applied uncorrected, a C8 string decayed to silence in about **12
milliseconds** against this document's own 1-2 s target — the zero alone was
costing about 16% of the fundamental's amplitude every round trip, and no
value `sustain` can take (bounded to `[0, 1]`) can put amplitude back that
the filter already removed.

The fix was to make `filter::LoopFilter` take `zero_mix` as well as `pole`,
and to have `piano_audio::voicing` choose both per key rather than fixing
them globally. Verified two ways: analytically, against `LoopFilter::
magnitude_at`'s own closed-form gain; and empirically, by seeding a minimal
delay-line-plus-filter loop (no dispersion, no hammer) with a plain sine
wave and measuring its actual per-cycle decay, which matched the analytic
prediction to within `0.03%`.

*How* those two coefficients get chosen changed afterwards, for the reason
the next section is about: the calibration described here scaled them down
together against a single constraint at the fundamental, and that was the
wrong constraint.

The hammer this was first measured with has since been replaced (see "Why
the hammer is solved against the string", below); the correction to the
filter stands on its own.

### Why the loop is solved against three decay times, not one

Tuning the loop so that the **fundamental** rings for the right time is one
equation. The filter has two free coefficients, and a third quantity —
`sustain`, the frequency-independent gain per round trip — feeds the same
loop. Two unknowns spent on one equation leave the filter's *slope* to
chance, and the slope is what sets every upper partial's decay.

`piano_audio::voicing::solve_loop_losses` gives each key three decay
targets — its fundamental's, a middle partial's and a bright partial's —
and solves `pole`, `zero_mix` and `sustain` against them. `sustain` has to
be in the solve: the loop filter's gain at DC is exactly `1` by
construction, so it cannot attenuate a fundamental, and a solve without
`sustain` either drags the corner down far enough to take the harmonics
with it or leaves every fundamental ten times too long.

The fit is a weighted least squares, not an exact solve: near DC a pole and
a zero both attenuate as `f²`, so the two coefficients are nearly collinear
there. The fundamental carries double weight. At 48 kHz, from
`report_the_solved_voicing_at_each_anchor` (seconds to fall 80 dB, achieved
against target):

| key | pole | zero_mix | sustain | fundamental | middle | bright |
|---|---|---|---|---|---|---|
| A0 | 0.80153 | 0.47529 | 0.987581 | 26.5 / 26.7 s | 24.7 / 24.4 s | 16.0 / 16.0 s |
| A2 | 0.00001 | 0.38397 | 0.992169 | 10.6 / 10.7 s | 10.4 / 10.3 s | 8.9 / 8.9 s |
| A4 | 0.00214 | 0.00787 | 0.995541 | 4.67 / 4.71 s | 4.55 / 4.46 s | 3.80 / 3.81 s |
| A6 | 0.00243 | 0.00196 | 0.996759 | 1.56 / 1.54 s | 1.26 / 1.29 s | 0.59 / 0.59 s |
| C8 | 0.00259 | 0.00012 | 0.996913 | 0.63 / 0.62 s | 0.45 / 0.48 s | 0.26 / 0.25 s |

The middle partial is the 2.8th and the bright one the 8th, except at C8,
whose 8th would sit at 33 kHz: there they are the 2.3rd and the 5.2nd, the
highest that stays under Nyquist.

**Where the targets come from.** They were first drawn from the rule of
thumb that partial `n` dies about `n` times faster than the fundamental,
and measured against a concert grand that was wrong where it matters most.
Resolved into decaying modes, the grand's partials through the middle of
the keyboard all fall about as fast as the fundamental, 15-25 dB/s for the
first dozen: the loss is the bridge's, and the bridge takes every partial
alike. The earlier table kept the fundamental four times longer than that
and killed the eighth partial seven times faster — a tone that goes dull
while it is still loud, which is a plucked string. The bass rings long at
every partial, a few dB/s for the first twenty, and the top dies in well
under a second. The rates are `voicing_scale.rs`'s anchors; only the
derived numbers are kept (`docs/CALIBRATION.md`).

These are the rates of the **prompt sound**, the motion in which a key's
strings push the bridge together. The slower aftersound is not in the
table: it comes out of the unison (see "Why a note has a two-stage decay").

### Why the mix bus needed a limiter too

A separate, unrelated gap surfaced alongside the one above: nothing summed
every ringing voice plus the soundboard against any ceiling before handing
samples to the audio device, contradicting this project's own stated
invariant ("Output is bounded by construction",
`docs/REALTIME-AUDIO-RULES.md`) once more than one voice could be ringing —
a full chord, or the sustain pedal's sympathetic resonance
(`piano_core::bridge`), could sum past `±1.0` and hit whatever hard-clipping
the host's `f32 -> PCM` conversion does. `piano_audio::engine`'s
`process_stereo_chunk` now runs every sample through `limiter::soft_limit`: the
identity function below `0.9`, so a single voice's own render is bit-for-bit
unaffected, and a smooth `tanh` saturation into the remaining headroom above
it — a standard soft-knee limiter, not a physical model, so (unlike
everything else in this document) it carries no literature citation.

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

## Why the hammer is solved against the string, every sample

Karplus-Strong fills the delay line with noise, which says "give the string
a random shape". A piano string is struck by a felt hammer, and the hammer
is most of what makes a piano sound like one. For several milestones this
model shaped a noise burst with a computed contact force and a lowpass; it
was heard as a knock in front of a guitar, and measured as an attack whose
second and third partials sat 57 and 76 dB under the fundamental.

`piano_core::hammer` is two statements, both in SI units:

- **The felt is a nonlinear spring.** Compressed by `ξ`, it pushes back
  with `F = K·ξ^p`, `p` between 2 and 3 (A. Chaigne & A. Askenfelt,
  "Numerical simulations of piano strings. I", JASA 95(2), 1994).
- **The string is a waveguide.** A force `F` at one point moves that point
  at `F / 2Z` on top of the two waves already passing through it, `Z` being
  the string's characteristic impedance (J. O. Smith, *Physical Audio
  Signal Processing*).

Nothing is written into the delay line in advance. Each sample the hammer's
compression is solved against the wave arriving at the strike point, the
force goes into the string there, and the hammer is decelerated by the same
force. What a listener calls touch follows instead of being added: a harder
blow compresses the felt further, the stiffer spring throws the hammer back
sooner, and the shorter pulse carries more high partials. Reflections that
return from the agraffe while the felt is still on the string push the
hammer off early. A string struck again is struck while it is moving.

One step is implicit — the force at the end of the sample depends on the
compression at the end of the sample — and the trapezoidal rule turns it
into `ξ + c·ξ^p = a`, whose left side is convex and increasing. Newton's
method started above the root of such a function never overshoots, so four
steps are enough and there is no `while !converged` (`PERF-007`,
`docs/REALTIME-AUDIO-RULES.md`). Contact is bounded at 30 ms.

Key velocity `v` in `[0, 1]` becomes a hammer speed of `4.7·v^1.25` m/s.
What the string reports is the force it puts on the bridge, `2Z·v`, over a
full scale of 50 N.

### Why every register has its own hammer

A. Askenfelt & E. Jansson ("From touch to string vibrations", JASA 1990-91)
measure contact falling from about 4 ms in the bass to under 1 ms at the
top: treble hammers are lighter and their felt is harder.
`hammer::hammer_for_frequency` gives each key one string's share of such a
hammer:

- **Mass** falls from 10.5 g at A0 to 7.2 g at F♯4, the usual trend of a
  concert grand's set, then faster to 1.9 g at C8.
- **Felt stiffness** rises from `1.6e8` to `8e13` N/m^p and the exponent
  from 2.2 to 3.2, fitted so that the spectrum changes between soft, medium
  and loud notes the way the reference instrument's does. Up to F♯4 the fit
  lands within a factor of two of Chaigne & Askenfelt's own values.
- **String impedance** is `2·L·f·μ` for a generic concert-grand scale: 13.7
  kg/s at A0, about 2.2 through the middle, 1.6 at C8.

The treble departs from the bass trend for one reason. A hammer that stays
on its string for about a period and a half delivers a pulse with a
spectral null at the fundamental, and the note all but vanishes. With the
bass trend carried to the top, that null crossed the keyboard between A6
and F♯7 as the touch went from soft to loud: holes 30 dB deep and one key
wide, at a different key for each dynamic. A lighter hammer on stiffer felt
leaves sooner.

`voicing::hammer_for_key` then moves each key's felt stiffness by a fixed
pseudo-random amount (`hammer_unevenness`, 0.25 by default), because no set
of hammers is voiced perfectly evenly.

### What the hammer still gets wrong

- **Below a key velocity of 0.25 the top two octaves can still lose their
  fundamental.** A power-law felt has no stiffness left at so small a
  compression, so the hammer lingers. Real felt is stiffer at rest than
  `K·ξ^p` says.
- **The felt has no hysteresis.** Real felt pushes back less on the way out
  than on the way in (A. Stulov, JASA 97(4), 1995). A fit with it as a free
  parameter matched the reference only slightly better than one without,
  so it is left out.
- **The hammer is a point.** Its width along the string, which rounds off
  the highest partials, is not modelled.

### Why each key has a regulation gain

Nothing in the string model makes neighbouring keys equally loud: the
unison count steps from one string to two and then to three, each step
adding several decibels, and the per-register hammer moves the top octaves
by several more. A technician evens a real piano out after the physics, by
needling and hardening hammers; `voicing::level_for_key` does the same with
a measured gain per key (`voicing_level.rs`), regulated to a curve that is
flat from C3 to C6, lower on the meter in the bass (A-weighting discounts
50-200 Hz 10-15 dB more than the ear does at playing level, ISO 226) and
8 dB down from C6 to C8.

What is regulated is the **trend along each unison register**, not each key
on its own. The board's diffuse field (below) answers a single partial
several decibels louder or softer depending on where it falls among the
field's resonances, as a real board does, and that pattern moves with the
sample rate and the tuning. A table that cancelled it key by key would be
right at one sample rate and twice as uneven at any other. So each key's
deviation is averaged with five neighbours either side, never across a
change of unison count; `tests/keyboard_loudness.rs` holds the trend within
2 dB and any single key within 8, and regenerates the table.

The gain is applied after the phantom partials and the duplex ring, which
grow with the string's own motion and must not be rescaled by it.

## Why the strike position notches out a partial

A hammer lands at one point, roughly 1/7 to 1/9 of the way along the string
(Fletcher & Rossing, *The Physics of Musical Instruments*, 2nd ed., §12.3).
A string takes no energy in a mode with a node where it is struck, so
striking at a fraction `β` leaves partials `1/β, 2/β, …` silent. Builders
choose `β ≈ 1/8` to put the first notch on the 8th partial, the most
dissonant of the low harmonics.

In this model the notch is geometry, not a filter. The hammer's force
enters the delay line at a tap `β` of the way round the loop
(`PluckedString::place_strike_tap`), and the wave it launches towards the
agraffe comes back inverted and cancels the outgoing one at exactly those
partials. `β · L` is seldom a whole number of samples — at C8 the whole
loop is 11 samples — so the force is shared between the two neighbouring
taps. Without that, the notch jumped from key to key and the top octave
was audibly uneven.

`β` is `StringConfig::strike_position`, set per key by
`piano_audio::voicing`: 1/8 in the bass, drifting to 1/10 at the top, where
a note has too few partials to spend one on a notch.

## Why upper partials sit sharp (M4)

A real string is stiff, not an idealised flexible one, so its restoring force
includes a bending-stiffness term the wave equation for an ideal string does
not. The consequence, first derived by Fletcher (1964): partial `n` sits at
`f_n ≈ n·f_1·sqrt(1 + B·n²)` rather than exactly `n·f_1`, where `B` is the
string's inharmonicity coefficient. `piano_core::dispersion::DispersionCascade`
reproduces this with a cascade of first-order allpass sections inside the
loop, after the loss filter (Jaffe & Smith 1983's extension to
Karplus-Strong): each section is flat in magnitude but adds a
frequency-dependent phase delay, and enough of them approximate the stretched
dispersion curve. Section count and the shared coefficient are *fitted* per
string (issue #96): the fewest sections that place partials 2-8 within two
cents of Fletcher's curve, which in practice is 1 across most of the bass
and 8 from E4 to B6. The loop is tuned by every element's phase delay *at
the fundamental*, not at DC — tuning at DC left A5 22 cents sharp.

The default `B` curve (`piano_audio::voicing`, `voicing_inharmonicity.rs`)
is the two-asymptote shape of Rigaud, David & Daudet (JASA 133(5), 2013) —
`B` falling from ~`3·10⁻⁴` at A0 to a tenor minimum near `10⁻⁴`, then rising
exponentially to ~`5·10⁻⁴` at A4 and `1.5·10⁻²` at C8 — anchored inside
Fletcher & Rossing's ranges, not measured on one instrument.
`crates/piano-audio/tests/keyboard_stiffness_and_tuning.rs` measures every
key's realised `B` and fundamental from its rendered sound.

## Why most notes are more than one string (M6)

A real piano key strikes more than one physical string for most of the
keyboard: a standard modern instrument is single-strung (monochord) in the
bass, double-strung (bichord) through the tenor, and triple-strung
(trichord) for the rest of the treble (A. Reblitz, *Piano Servicing,
Tuning, and Rebuilding*, 1993 — the layout piano technicians work to;
qualitatively consistent with Fletcher & Rossing's own bass/tenor/treble
description, already cited above for inharmonicity). This project uses 12
single-strung keys, 18 double-strung and 58 triple-strung — a
representative choice of break points within that convention, not a
measurement of one specific instrument (exact break points vary by piano
model and scale design) — implemented as
`piano_core::unison::unison_count_for_key_index` and looked up per key by
`piano_audio::voicing::unison_count_for_key`. That raises the *effective*
number of strings the engine ever processes simultaneously from 88 to
`12·1 + 18·2 + 58·3 = 222`, close to `PERF-008`'s own illustrative `N =
240` estimate for "up to three strings on all 88 keys."

`piano_core::unison::UnisonGroup` reuses [`piano_core::PluckedString`]
rather than forking it: a unison group is 1-3 strings struck together by
`UnisonGroup::pluck`, the way one hammer strikes every string of a note.

How far apart the strings are tuned is not a fixed number of cents
(`voicing_unison.rs`). Weinreich (below) shows that what matters is the
mistuning *measured against how fast the note decays*: far apart on that
scale each string rings alone and the note beats; close together they act
as one. Each key's strings sit 0.07 of the fundamental's own prompt decay
rate apart — about 0.17 cents at A4, a unison a good tuner would call dead
on — which keeps the first half-dozen partials on the "one string" side, as
the reference's are: each of its partials resolves into one fast mode with
slower companions 10-30 dB under it, not two equals beating.

## Why a note has a two-stage decay (M6)

G. Weinreich, "Coupled Piano Strings" (JASA 62(6), 1977) is the seminal
model and measurement of this: near-unison strings are not independent —
they share one mechanical connection, the bridge, and that shared,
slightly-yielding contact point is what makes a real piano note's envelope
bend rather than follow one clean exponential. Struck together, a note's
strings start in phase: that common mode drives the bridge, drains into the
soundboard and decays fast (the *prompt sound*). A cent or so of mistuning
rotates energy into differential modes, where the strings' forces on the
bridge cancel; the bridge barely moves, and that energy rings on several
times longer (the *aftersound*).

This project reproduces that mechanism, not a hand-tuned envelope shape,
with two coupling tiers:

- **Local** (`piano_core::unison`): every round trip, the part of each
  string equal to its group's mean loses the string's full `sustain` loss,
  and the part that differs from the mean is spared
  `DEFAULT_LOCAL_COUPLING_GAIN` (0.87) of it — a symmetric scattering with
  eigenvalues `sustain` and `sustain + 0.87·(1 − sustain)`, passive for any
  `sustain`. At 0.87 the slow modes lose about an eighth of what the prompt
  mode does, the ratio measured on the reference from the tenor to the top
  octave. The strings also load the bridge unequally (`BRIDGE_LOAD_*`):
  with equal loads a hammer that strikes them alike excites only the prompt
  mode, and the aftersound is reached through a dead notch between the two
  stages. A single string is its own mean and decays exactly as voiced.
  The earlier convex blend `(1−w)·own + w·mean(others)` did the reverse —
  it damped the differential modes — and treble trichords collapsed
  (issue #96).
- **Global** (`piano_core::bridge::BridgeBus`, `PERF-008`): cross-*key*
  coupling, e.g. from the sustain pedal, blends with one block
  (~2.7 ms at 48 kHz) of latency, because summing all 88 keys' contributions
  before any one of them can read the total back is not representable in a
  single per-voice, per-sample call without sacrificing the cache-friendly
  loop order `PERF-010` already established.

Measured, not asserted: `crates/piano-render/tests/m6_spectral.rs` renders
a trichord A4 and shows its early decay rate (nepers per 0.1 s block, just
after the attack transient) is measurably faster than its settled rate
once beating has died down, and that the settled rate then sits close to
(within a factor of two of) the same note rendered as a monochord control
— the qualitative and quantitative signature Weinreich's model predicts.
This measurement also caught two real implementation bugs during
development, not by inspection: an earlier *additive* coupling term
diverged to infinity for near-lossless (high-`sustain`) strings once
enough voices became simultaneously receptive (fixed by the convex-blend
redesign above and by `BridgeBus` averaging rather than summing
contributions), and every unison string sharing one excitation suppressed
the very beating the model exists to produce.

## Why the sustain pedal makes the rest of the instrument ring (M6, `PERF-008`)

Before M6 the sustain pedal only changed *when* a struck voice's own
damper engaged — it had no effect on any other string, because nothing
coupled voices together. `piano_core::bridge::BridgeBus` is the fix: every
voice writes its own bridge-end signal into one shared running total and
reads back everyone else's, so a string whose damper the pedal has lifted
(even if it was never struck) picks up a little energy from whatever *is*
ringing and starts to audibly resonate — a real piano's sustain pedal
lifts every damper on the instrument, not just the one under a held key.
`piano_audio::engine::Engine::set_sustain_pedal` is what actually lifts
those idle dampers; `piano_core::unison::UnisonGroup::is_receptive`
distinguishes a genuinely-damped, safely-skippable voice
(`PERF-006`) from a silent-but-undamped one that must still be processed
so it can wake up.

The bridge deliberately does not model per-pair coupling (`O(N²)`,
infeasible at this string count — see `PERF-008` in
`docs/PERFORMANCE.md`) or per-string admittance (this project has no
per-instrument measurement to derive one from): every voice shares the
same coupling gain and the same bus, an engineering simplification stated
plainly rather than presented as more faithful than it is.

The bus was first read back as the *mean* of every receptive key's
contribution, which kept it bounded but diluted every sounding key's drive
by the number of listeners — so pressing the sustain pedal, which makes all
88 keys listen, cut each one's share by up to 39 dB: the pedal made the
instrument *less* resonant (#62). The bridge physically moves with the
*sum* of the strings' forces, so the drive is now `g · Σ`, capped at
`0.9 / N` per key only when `N` receptive keys would otherwise break
passivity (`unison::bridge_drive_for`). At the default gain (0.2) a C5 held
silently rings 37 dB under a struck C4, a fifth 44 dB under and a semitone
62 dB under, so the harmonic selectivity comes out of the strings
themselves. The pedal's bloom on a held C4 sits 9 dB under what is left of
the note one to three seconds after the strike — nearer the note than it
was, because the note itself now decays as fast as a piano's does — and a
full keyboard struck *fortissimo* with the pedal down still decays
(`piano-audio/tests/sympathetic.rs`).

## Why the pedals are three damper rules, not three switches (#59-#61)

Every key's damper is recomputed from one rule whenever a key or a pedal
moves: a key held by a finger, or caught by the sostenuto, has its damper
lifted; every other key's damper rests with the pressure the sustain pedal's
position leaves it. That single rule is the release, the pedal-held release,
the sympathetic lift with the pedal down, half-pedalling and the sostenuto.

- **Half-pedalling.** CC64 is a position, not a switch. Pressure falls along a
  smoothstep between 25 % and 70 % of the stroke; a damper's per-round-trip
  loss is `−ln(0.4)·pressure⁵`, because felt that only grazes the string damps
  a sliver as hard as felt seated on it — linear in pressure, a half-damped A3
  would still stop inside 50 ms. Measured: A3 released at mid-stroke rings
  between the damped and the held note, more than 6 dB from each.
- **Sostenuto** (CC66). Pressing it catches exactly the keys held at that
  instant.
- **Una corda** (CC67). The shifted action misses one string of every bichord
  and trichord; the missed string's damper still lifts, so the struck ones
  drive it through the coupling and it rings against them: less prompt
  sound, more aftersound, which is the veiled colour the pedal is played for
  (Weinreich 1977, §V). The string missed is the one that loads the bridge
  hardest, so what is struck leans towards the slow modes. Single-string
  bass keys are struck as usual; the softer felt a real shifted hammer
  presents is not modelled.

## Why a note starts with a knock that never touches a string (#64)

A. Askenfelt ("Observations on the transient components of the piano tone",
STL-QPSR 34(4), 1993) separates the onset of a real note into the string's
tone and mechanical transients that never pass through a string, the largest
being the key bottoming on the keybed a few milliseconds after the hammer
meets the string. It drives the case and board directly and is heard as a
dull knock under the tone — plainly in the treble, where the tone is thin.
`piano_core::action::ActionNoise` synthesises it as a half-sine force pulse
(4 ms at *pianissimo*, 1.5 ms at *fortissimo*, amplitude ∝ velocity^1.5)
fed into the soundboard only, so the board's modes give it the body's
colour. Measured over a note's first 50 ms: 25 dB under a
*fortissimo* A4, 10 dB quieter at velocity 0.3, and 8 dB more prominent at
C7 than at C4. `AudioSession::set_action_noise_gain` sets it, `0` removes it.

The action makes one more sound, at the other end of the note: when a key
comes up (or the sustain pedal lets go), each damper's felt drops onto a
string that is still moving. `ActionNoise::damp` gives that landing its own
soft pulse — 6 ms long, a twentieth of a full-velocity knock, and the same
however the key was played, because the felt falls under its own spring and
weight. It sounds only when a damper lands on a string that is still
sounding, never under the pedal; measured on a released *mezzo-forte* C4,
it sits about 28 dB under the note as it is cut off.

## Why every note is coloured by a soundboard (M6, `PERF-009`)

A real piano's strings barely radiate sound on their own — their thin
cross-section couples poorly to air. Almost everything a listener hears
comes from the soundboard, the large wooden plate the bridge drives.
`piano_core::soundboard::Soundboard` models that as a bank of
[`MODE_COUNT`](../crates/piano-core/src/soundboard.rs) damped resonant
filters (modal synthesis — B. Bank et al., already cited in
`docs/PRIOR-ART.md` for exactly this architecture) rather than convolving
against a measured impulse response. That second option is not a choice
this project can make regardless of engineering merit: an impulse response
is, by definition, recorded from a real instrument, and this repository's
own rule (see the root `CLAUDE.md`) is that no recorded or sampled audio
asset may be added, ever, for any reason. Modal synthesis needs no such
recording — each mode is a frequency, a decay time and a gain, all
literature-informed order-of-magnitude values (see the module's own
honesty note on where they come from and what they are not), not fit to
any instrument's measured response.

The bank is **28 modes** from 52 Hz to 6.3 kHz (issue #78; it was 8,
topping out at 1.4 kHz, which left the whole treble a bare wire). The
table is deliberately irregular in frequency spacing and in `Q` — a real
board is neither a smooth curve nor a single damping value — and its
`gain` column is shaped like a soundboard's **radiation efficiency**, not
its raw modal amplitude: the lowest modes move a lot of wood but push
little air (dipole cancellation across the panel), so they are held down;
efficiency climbs through the low mid-range and is near unity by 1–3 kHz
(the soundboard's critical band), so the peak sits around 200–550 Hz and
the treble tail only falls ~13 dB below it, where the old smooth curve
fell ~34 dB. `Q` stays in the 16–44 range a ribbed spruce board occupies,
so every mode decays in tens of milliseconds — a damped wooden box, not a
struck bar.

The soundboard's output is **mixed in**, not substituted for the direct
signal (`piano_audio::engine::Engine`'s `soundboard_mix_gain`, default
`0.5`, live-adjustable via `AudioSession::set_soundboard_mix_gain` and
`.piano.json`'s `instrument.soundboard_mix_gain`): replacing the direct
signal entirely would be more faithful to how a real piano is *only* ever
heard through its soundboard, but would also change the level and shape of
the fundamental partial in every already-measured tuning and inharmonicity
test this project has (M1's cents figure, M4's partial-sharpening and
brightness measurements) — an honest trade stated here rather than
silently made. At that mix level the board colours a note's attack and
adds sustained body without dominating its spectrum: it shifts a rendered
note's spectral centroid only a few percent (a spectral centroid is a
poor detector of a fast-decaying body under a broadband note), but changes
its radiated energy by 3–10% in the low and mid register.

## Why a piano's attack is not a plucked string's (the body)

The modal bank above stops being a good description at a few hundred
hertz. Above that a soundboard's modes overlap: bending waves leave the
bridge, reflect off the rim and the ribs, and within a few milliseconds the
plate is full of them. A force at the bridge is not heard as the
sharp-edged wave the string delivered but smeared over tens of
milliseconds. Measured on the reference, the band around 1 kHz takes about
12 ms to rise to full level; the bridge force alone takes under 2. That
smear is the difference between a piano's attack and a plucked string's,
and without it this model was reported, correctly, as a guitar.

`piano_core::body::BoardBody` is the feedback delay network B. Bank
proposed for the job (Helsinki University of Technology, Report 54, 2000;
Bank et al., EURASIP JASP 2003:10): the network of `piano_core::fdn`, with
lines a few milliseconds long — the time a bending wave takes to cross the
board and return — where a room's are tens. Two of its numbers are physical:
its lines add up to 43.7 ms, a mode every 23 Hz, the density modal analyses
of soundboards count below 1 kHz (Ege, Boutillon & Rébillat, JSV 332,
2013); and its absorption follows a loss factor of one to two per cent,
spruce's — about 100 ms at 1 kHz, 26 ms at 8 kHz.

The engine sums three things (`engine_render.rs`): the bridge force itself
at 0.3, heard at once, as the board next to the bridge radiates before the
waves have spread; the diffuse field; and the modal bank at
`soundboard_mix_gain`.

`piano_core::radiation::Radiation` then makes sound of that motion, which a
board does unevenly: almost nothing below its lowest resonances, where the
air flows round the edge of the plate instead of being compressed (the
fundamental of a grand's lowest notes is 20-35 dB under its partials three
to six); two strong resonances near 85 and 128 Hz; and a treble radiated
about 10 dB more efficiently than the middle, as the bending waves approach
the speed of sound in air (N. Suzuki, JASA 80, 1986; N. Giordano, JASA 104,
1998). The papers give the shape; the corners and gains are fitted to the
reference's spectral envelope. Two of them are not the instrument's alone
and say so in the code: the slope below the first resonance, part of which
may be the reference's recording chain, and a 3.5 dB cut at 560 Hz that
takes back a hump of the delay network's own.

No impulse response is measured, stored or convolved: the field is
synthesised, like everything else here.

## Why the piano is not tuned to equal temperament (stretch)

A piano with every fundamental on its equal-tempered frequency would sound
out of tune with itself. Its partials run sharp of whole multiples, so an
octave whose fundamentals are exactly 2:1 apart has the upper note beating
against the lower note's second partial. A tuner removes that beat, and in
doing so tunes the treble progressively sharp and the bass flat — the curve
O. L. Railsback measured (JASA 9(3), 1938).

The stretch is therefore not a table but a consequence of the scale's own
`B`, and `voicing_stretch.rs` computes it that way: a middle octave is left
in equal temperament and every other key is tuned in octaves from it, each
made beatless between partial `2ρ` of its lower note and partial `ρ` of its
upper. `ρ` is the octave type of Rigaud, David & Daudet's tuning model
(JASA 133(5), 2013): the 2:1 octave tuners use in the treble, 4:2 in the
middle, 6:3 in the bass. Where the types change is fitted to the reference,
whose offsets the rule reproduces to about 2 cents: 15 cents flat at A0,
10 sharp at C7, 30 at C8. Concert A stays where the tuning puts it.

## Why the bass has partials no string harmonic explains (#54)

Transverse vibration stretches a string, and the tension change goes with
the *square* of the displacement. That square holds every sum `f_m + f_n`
of the string's partials, and it reaches the bridge longitudinally. Conklin
(JASA 105(1), 1999) measured these "phantom partials" in real pianos; Bank
and Sujbert (JASA 117(4), 2005) traced them to the longitudinal motion and
found them strong in the bass and tenor, negligible in the short treble
strings. They are the metallic edge of a hard-struck low note.

`piano_core::phantom::PhantomPartials` squares each voice's output,
highpasses away the DC the square carries (20 Hz corner), and adds it back
at a per-key gain (`voicing::phantom_gain_for_key`): full up to A2, tapering
to nothing by E5. Being quadratic, the phantoms grow 2 dB for every 1 dB the
note grows, so they are near-absent at *piano* and plainly there at
*fortissimo*. Measured (`piano-audio/tests/phantom_partials.rs`): at *ff*
they sit 24-28 dB under the note across A0-A2 and rise 14 dB relative to it
from *p* to *ff*. The string's free longitudinal
resonances are not modelled: their frequencies are scale-design data with
no published source this project can use yet (#65).

## Why the instrument is heard in stereo

A piano heard from the bench is wide: the bass strings fan out to the
player's left, the treble to the right, and the soundboard's modes each
have their own shape over the board, so two ears (or a stereo microphone
pair) receive every mode at a different strength. A mono render collapses
all of that into a point.

`engine_render.rs` places every key at a pan of `+0.5` (A0, left) to `-0.5`
(C8, right), linear in key number, and `Soundboard::process_stereo` weighs
each mode `1 ± spread` between the two channels, with spreads stepped by
the golden angle so neighbouring modes land on unrelated sides. Every
weight is `1 ± x`, so the mean of the two channels is *exactly* the mono
instrument: every level, decay and tuning measurement in this repository
still holds on the mono mean (`tests/stereo.rs` checks that, and that A0
sits 7 dB left and C8 15 dB right).

## Why the instrument is heard in a room

Nobody hears a piano dry. Every recording a listener compares this
instrument with was made in a hall or a studio, and the room is a large part
of what makes a recorded concert grand sound rich: a few milliseconds of
silence, then a dense, decorrelated tail that dies faster in the treble
than in the bass.

`piano_core::room::Room` is the feedback delay network of Jot and Chaigne
(AES 1991): eight delay lines of 30-73 ms, fed back through an orthonormal
(lossless) Hadamard matrix, each followed by a one-pole absorption filter
solved so the tail reverberates 1.8 s at low frequencies and 0.5 s at
Nyquist, after a 12 ms predelay. Left and right read the lines through
orthogonal sign patterns, so the two channels are uncorrelated. It is fed
the dry instrument after the soundboard and added back in stereo before the
limiter.

The live engine starts at `DEFAULT_ROOM_MIX` (0.25), which puts the room
about 11-14 dB under the direct sound over a sustained note
(`tests/room.rs`); `.piano.json`'s `instrument.room_mix` and the studio's
room slider move it, and `0` is dry. Offline renders start dry, so every
level, decay and tuning measurement in this repository sees the instrument
alone.

## Why the treble rings on after its strings (duplex scale)

On a concert grand the short string lengths beyond the bridge are not all
muted. Steinway's duplex scale (US patent 126,848, 1872) leaves them free
from about the middle of the keyboard up, with lengths set to resonate at a
harmonic of the speaking length; Conklin (JASA 100(3), 1996, Part III)
describes the result as a shimmering after-ring in the treble.

A segment set to the *exact* multiple would hardly ring: a stiff treble
string's fourth partial sits tens of hertz sharp of `4·f0`, far outside a
high-Q resonance — which is why technicians set duplex bars by ear.
`piano_core::duplex::DuplexResonance` therefore tunes each key's segment to
its string's real partial, `n·f0·√((1 + B·n²)/(1 + B))`, then 1.5 cents off
it, the residual of a by-ear setting, heard as a slow beat. It is one
two-pole resonator per key from C4 up (the double octave below C6, the
octave from C6, where the fourth partial is too weak to drive it), driven by
the key's own bridge signal. Measured (`piano-audio/tests/duplex.rs`) it
rings 25-56 dB under the note from C4 up, 35 dB in the median — it depends
on how strong the partial it is tuned to happens to be on each key — and
the bass has none. Tuning to the
exact multiple instead put it 45-95 dB under — inaudible.

## What the current model still does not do

Stated plainly, because these are the gaps a later milestone would close:

| Missing | Consequence | Milestone |
|---|---|---|
| **Free longitudinal modes** | The phantom partials' nonlinear mixing is modelled (above); the string's own longitudinal resonances, at frequencies set by scale design, are not (#65). | Backlog |
| **Per-string bridge admittance** | Every voice couples to the shared bridge bus at the same fixed gain; a real bridge's admittance varies with frequency and string position. | Backlog |
| **Soft notes at the top** | Below a key velocity of 0.25 the top two octaves can lose their fundamental (see "What the hammer still gets wrong"). | Backlog |
| **The board rings on after a release** | Near 132 Hz the modal bank keeps sounding for a fraction of a second after the dampers land. | Backlog |
| **Evenness key to key** | A single key can sit several decibels off its neighbours, and the duplex ring varies by tens of decibels, as the diffuse field and the stiff partials fall where they fall. | Backlog |
| **The 4-8 kHz partials die slowly** | Partials in that band outlast the reference's. | Backlog |

## Numbers worth having

At 48 kHz on an 88-key piano:

| | A0 (27.5 Hz) | A4 (440 Hz) | C8 (4186 Hz) |
|---|---|---|---|
| Period, samples | 1745 | 109 | 11.5 |
| Delay buffer (pow-2) | 2048 | 128 | 16 |
| Memory per string | 8 KB | 512 B | 64 B |
| Prompt sound, fundamental (80 dB) | 26.5 s | 4.7 s | 0.63 s |
| Prompt sound, 8th partial (80 dB) | 16.0 s | 3.8 s | 0.26 s (5.2nd) |
| Dispersion sections needed | ~8 | ~2 | ~0–1 |

The last row is why `PERF-005` insists that dispersion order be scaled per
register rather than fixed: paying for eight allpass sections on C8 would be
half the treble's CPU budget spent on an effect nobody can hear.
