//! Drives the engine with an arbitrary command stream decoded from raw
//! bytes, for fuzzing (issue #56).
//!
//! Each component already has its own property tests; the engine as a whole
//! has a far larger input space — notes, pedals, live parameter changes and
//! sample rates interleaved in any order — and the failure worth finding is a
//! specific ordering that leaves a `NaN` in a feedback loop or a voice that
//! never stops. [`run_command_stream`] turns any byte string into such a
//! sequence and reports what came out, so `cargo-fuzz` (`fuzz/` at the
//! repository root) can search the space with coverage guidance, and this
//! crate's own proptest can search it on stable under the allocation guard.
//!
//! Floats are read as raw bits, so `NaN`, `±∞`, subnormals and `f32::MAX`
//! all reach the engine; so do out-of-range MIDI numbers, string indices and
//! mode indices. Not a stable API: hidden from the docs, here only so the
//! fuzz target outside this crate can reach the private command type.

use piano_core::SampleRate;
use piano_core::hammer::HammerConfig;
use piano_core::soundboard::SoundboardMode;
use piano_params::Tuning;

use crate::commands::Command;
use crate::engine::Engine;

/// Sample rates a stream may pick from, spanning telephone to studio.
const SAMPLE_RATES_HZ: [f32; 6] = [8_000.0, 22_050.0, 44_100.0, 48_000.0, 96_000.0, 192_000.0];

/// Most steps one stream may take, so a long input cannot run for minutes.
const MAX_STEPS: usize = 2_048;

/// Largest block one render step asks for, the size a real device callback
/// can reach.
const MAX_BLOCK_FRAMES: usize = 512;

/// How many distinct step kinds a selector byte chooses between.
const STEP_KINDS: u8 = 39;

/// What one stream produced.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct FuzzOutcome {
    /// Steps decoded and run.
    pub steps: usize,
    /// Output samples rendered, across both channels.
    pub samples: usize,
    /// Output samples that were `NaN` or infinite.
    pub non_finite: usize,
    /// The largest magnitude among the finite ones.
    pub loudest: f32,
}

impl FuzzOutcome {
    /// Whether every sample was finite and inside the limiter's `[-1, 1]`.
    #[must_use]
    pub fn is_sound(self) -> bool {
        self.non_finite == 0 && self.loudest <= 1.0
    }

    fn record(&mut self, block: &[f32]) {
        self.samples += block.len();
        for &sample in block {
            if sample.is_finite() {
                self.loudest = self.loudest.max(sample.abs());
            } else {
                self.non_finite += 1;
            }
        }
    }
}

/// Decodes `bytes` into a sample rate and a command stream, runs it through
/// a fresh engine, and reports the output. Every call into the engine —
/// applying a command or rendering a block, the two things the audio
/// callback does — runs inside `around_engine`, so a caller can bracket
/// exactly the realtime work, as the allocation guard does.
pub fn run_command_stream(bytes: &[u8], around_engine: impl Fn(&mut dyn FnMut())) -> FuzzOutcome {
    let mut reader = ByteReader { bytes };
    let mut outcome = FuzzOutcome::default();
    let Some(sample_rate) = reader.sample_rate() else {
        return outcome;
    };
    let mut engine = Engine::new(sample_rate, Tuning::default());
    while outcome.steps < MAX_STEPS {
        let Some(step) = reader.step() else {
            break;
        };
        outcome.steps += 1;
        around_engine(&mut || run_step(&mut engine, step, &mut outcome));
    }
    outcome
}

/// One thing the stream asks the engine to do.
#[derive(Debug, Clone, Copy)]
enum Step {
    Apply(Command),
    RenderMono(usize),
    RenderStereo(usize),
}

fn run_step(engine: &mut Engine, step: Step, outcome: &mut FuzzOutcome) {
    let mut left = [0.0f32; MAX_BLOCK_FRAMES];
    let mut right = [0.0f32; MAX_BLOCK_FRAMES];
    match step {
        Step::Apply(command) => engine.apply(command),
        Step::RenderMono(frames) => {
            let block = left.get_mut(..frames).unwrap_or_default();
            engine.process_block(block);
            outcome.record(block);
        }
        Step::RenderStereo(frames) => {
            let (Some(left), Some(right)) = (left.get_mut(..frames), right.get_mut(..frames))
            else {
                return;
            };
            engine.process_block_stereo(left, right);
            outcome.record(left);
            outcome.record(right);
        }
    }
}

/// Reads fixed-width values off the front of the input; `None` once it
/// runs out, which ends the stream.
struct ByteReader<'a> {
    bytes: &'a [u8],
}

impl ByteReader<'_> {
    fn take<const N: usize>(&mut self) -> Option<[u8; N]> {
        let (head, rest) = self.bytes.split_first_chunk::<N>()?;
        self.bytes = rest;
        Some(*head)
    }

    fn byte(&mut self) -> Option<u8> {
        self.take::<1>().map(|[byte]| byte)
    }

    fn flag(&mut self) -> Option<bool> {
        self.byte().map(|byte| byte % 2 == 1)
    }

    fn float(&mut self) -> Option<f32> {
        self.take::<4>().map(f32::from_le_bytes)
    }

    fn word(&mut self) -> Option<u32> {
        self.take::<4>().map(u32::from_le_bytes)
    }

    fn index(&mut self) -> Option<usize> {
        self.take::<8>()
            .map(|raw| usize::try_from(u64::from_le_bytes(raw)).unwrap_or(usize::MAX))
    }

    fn sample_rate(&mut self) -> Option<SampleRate> {
        let choice = usize::from(self.byte().unwrap_or(0)) % SAMPLE_RATES_HZ.len();
        SampleRate::new(*SAMPLE_RATES_HZ.get(choice)?).ok()
    }

    fn frames(&mut self) -> Option<usize> {
        let raw = u16::from_le_bytes(self.take::<2>()?);
        Some(usize::from(raw) % (MAX_BLOCK_FRAMES + 1))
    }

    fn step(&mut self) -> Option<Step> {
        let kind = self.byte()? % STEP_KINDS;
        match kind {
            0..=7 => self.performance(kind).map(Step::Apply),
            8..=18 | 32..=38 => self.global_setting(kind).map(Step::Apply),
            19..=29 => self.per_string_setting(kind).map(Step::Apply),
            30 => self.frames().map(Step::RenderMono),
            _ => self.frames().map(Step::RenderStereo),
        }
    }

    fn performance(&mut self, kind: u8) -> Option<Command> {
        Some(match kind {
            0 => Command::NoteOn {
                midi: self.byte()?,
                velocity: self.float()?,
            },
            1 => Command::NoteOff { midi: self.byte()? },
            2 => Command::AllNotesOff,
            3 => Command::SustainPedal { down: self.flag()? },
            4 => Command::SustainPedalPosition {
                position: self.float()?,
            },
            5 => Command::SostenutoPedal { down: self.flag()? },
            6 => Command::SoftPedal { down: self.flag()? },
            _ => Command::SetSoundboardMode {
                index: self.index()?,
                mode: self.mode()?,
            },
        })
    }

    fn global_setting(&mut self, kind: u8) -> Option<Command> {
        let value = self.float()?;
        Some(match kind {
            8 => Command::SetDamping { damping: value },
            9 => Command::SetSustain { sustain: value },
            10 => Command::SetActionNoiseGain { gain: value },
            11 => Command::SetPhantomGain { gain: value },
            12 => Command::SetDuplexGain { gain: value },
            13 => Command::SetRoomMix { mix: value },
            14 => Command::SetSoundboardMixGain { gain: value },
            15 => Command::SetMasterGain { gain: value },
            16 => Command::SetVelocityCurve { exponent: value },
            17 => Command::SetLocalCouplingGain { gain: value },
            32 => Command::SetDamperStrength { strength: value },
            33 => Command::SetRoomSize { size: value },
            34 => Command::SetRoomReverbSeconds { seconds: value },
            35 => Command::SetRoomTrebleReverbSeconds { seconds: value },
            36 => Command::SetRoomPredelay {
                milliseconds: value,
            },
            37 => Command::SetLimiterThreshold { threshold: value },
            38 => Command::SetBoardLoadGain { gain: value },
            _ => Command::SetGlobalCouplingGain { gain: value },
        })
    }

    fn per_string_setting(&mut self, kind: u8) -> Option<Command> {
        let midi = self.byte()?;
        let string_index = self.byte()?;
        Some(match kind {
            19 => Command::SetStringDamping {
                midi,
                string_index,
                damping: self.float()?,
            },
            20 => Command::SetStringSustain {
                midi,
                string_index,
                sustain: self.float()?,
            },
            21 => Command::SetStringInharmonicity {
                midi,
                string_index,
                inharmonicity: self.float()?,
            },
            22 => Command::SetStringDetune {
                midi,
                string_index,
                cents: self.float()?,
            },
            25 => Command::SetStringLoopZeroMix {
                midi,
                string_index,
                zero_mix: self.float()?,
            },
            26 => Command::SetStringStrikePosition {
                midi,
                string_index,
                position: self.float()?,
            },
            27 => Command::SetStringExcitationNoiseMix {
                midi,
                string_index,
                mix: self.float()?,
            },
            23 => Command::SetStringSeed {
                midi,
                string_index,
                seed: self.word()?,
            },
            _ => Command::SetStringHammer {
                midi,
                string_index,
                hammer: self.hammer()?,
            },
        })
    }

    fn mode(&mut self) -> Option<SoundboardMode> {
        Some(SoundboardMode {
            frequency_hz: self.float()?,
            decay_seconds: self.float()?,
            gain: self.float()?,
            bridge_coupling: self.float()?,
        })
    }

    fn hammer(&mut self) -> Option<HammerConfig> {
        Some(HammerConfig {
            contact_exponent: self.float()?,
            stiffness: self.float()?,
            mass: self.float()?,
            string_impedance: self.float()?,
            felt_bandwidth: self.float()?,
        })
    }
}
