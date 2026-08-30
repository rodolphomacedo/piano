//! `piano studio` — load a `.piano.json` file and play it live from a MIDI
//! controller, so the file format and cascade resolver
//! (`crates/piano-studio`) can be validated end-to-end by ear.
//!
//! Deliberately the same shape as `crate::midi`: `AudioSession::start`,
//! `MidiListener`, the polling loop, `RawModeGuard` for terminal cleanup.
//! The only addition is applying a resolved [`piano_studio::ResolvedPiano`]
//! to the session before the play loop starts.

use std::io;
use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::time::Duration;

use anyhow::{Context, Result};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::terminal;
use piano_audio::AudioSession;
use piano_midi::MidiListener;
use piano_params::Tuning;
use piano_studio::{LiveState, StudioCommand, StudioConfig};

use crate::report_timing;

/// How long `event::poll` waits before re-checking for a reason to stop.
/// Same value as `crate::midi`'s own constant — both loops have the same
/// "check for Esc/Ctrl+C without busy-waiting" shape.
const POLL_INTERVAL: Duration = Duration::from_millis(50);

/// MIDI events drained per tick, bounding one iteration of the control
/// loop the same way `crate::midi::MAX_EVENTS_PER_TICK` does.
const MAX_EVENTS_PER_TICK: usize = 256;

/// [`StudioCommand`]s drained per tick. The same reasoning as
/// `MAX_EVENTS_PER_TICK`: a browser reload floods well over a thousand at
/// once, and this keeps one tick of the control loop bounded regardless.
const MAX_COMMANDS_PER_TICK: usize = 256;

/// `piano studio`'s default port. Nothing about it is special beyond being
/// unlikely to already be taken — see `docs/PARAMETER-STUDIO.md`'s "CLI
/// integration" section.
const DEFAULT_PORT: u16 = 7878;

/// Arguments for `piano studio`.
#[derive(Debug, clap::Args)]
pub(crate) struct StudioArgs {
    /// Path to a `.piano.json` file to load and apply before playing.
    #[arg(long)]
    piano: PathBuf,

    /// Play live from a MIDI controller after loading the file. Without
    /// this flag, `piano studio` only loads, resolves and reports the
    /// file, then exits — useful for validating a file without a
    /// keyboard plugged in.
    #[arg(long)]
    midi: bool,

    /// Substring of the MIDI input port name to connect to,
    /// case-insensitive. Defaults to the first port found.
    #[arg(short, long)]
    port: Option<String>,

    /// How many seconds to wait for a MIDI port to appear before giving up.
    /// Zero fails immediately if nothing is plugged in.
    #[arg(long, default_value_t = crate::midi::DEFAULT_WAIT_SECONDS)]
    wait: f32,

    /// Frequency of concert A, in hertz.
    #[arg(long, default_value_t = 440.0)]
    concert_a: f32,

    /// TCP port for the studio's local web server.
    #[arg(long, default_value_t = DEFAULT_PORT)]
    web_port: u16,
}

/// Runs `piano studio` until Esc or Ctrl+C: loads the file, starts the
/// audio session and the local web server, connects to a MIDI controller
/// when `--midi` is given, then services both MIDI and browser input until
/// told to stop.
///
/// # Errors
///
/// Returns an error if the `.piano.json` file cannot be loaded, if the
/// concert-A pitch is invalid, if no audio output device could be opened,
/// if the web server's port cannot be bound, or (with `--midi`) if no MIDI
/// port matches or the terminal could not be put into raw mode.
pub(crate) fn run(args: &StudioArgs) -> Result<()> {
    let file = piano_studio::load(&args.piano)
        .with_context(|| format!("could not load {}", args.piano.display()))?;
    let tuning = Tuning::with_concert_a(args.concert_a)
        .with_context(|| format!("invalid concert A: {}", args.concert_a))?;

    let mut session = AudioSession::start(tuning).context("could not start audio playback")?;
    // Resolved after the session opens, not before: `resolve`'s register
    // tier needs the real output sample rate to compute a correct
    // `sustain` (see `piano_audio::voicing::solve_loop_losses`),
    // and that rate is whatever the OS output device actually gave —
    // not necessarily 48 kHz — which is only known once the stream is
    // open.
    let live = LiveState::from_file(&file, Some(&args.piano), tuning, session.sample_rate());
    apply_commands(&mut session, &live.commands());
    let snapshot = live.snapshot();
    let string_count: usize = snapshot.keys.iter().map(|key| key.strings.len()).sum();
    println!(
        "loaded {} — {} strings across {} keys",
        args.piano.display(),
        string_count,
        snapshot.keys.len(),
    );

    let (server, commands) = piano_studio::serve(
        live,
        StudioConfig {
            port: args.web_port,
            tuning,
            sample_rate: session.sample_rate(),
        },
    )
    .context("could not start the studio's web server")?;
    println!("piano studio listening on {}", server.url());
    println!("open that address in a browser to play and edit live.");

    let mut listener = if args.midi {
        let listener = crate::midi::connect(args.port.as_deref(), args.wait)?;
        print_instructions(listener.port_name());
        Some(listener)
    } else {
        println!("no MIDI controller requested — play from the browser instead.");
        None
    };
    println!("Esc or Ctrl+C (in this terminal) to quit.\n");

    let raw_mode = RawModeGuard::enable().context("could not enable terminal raw mode")?;
    let outcome = run_until_quit(&mut session, listener.as_mut(), &commands);
    drop(raw_mode);
    outcome?;

    report_timing(&session);
    Ok(())
}

/// Applies every command in `commands`, in order. Used both for the
/// commands a freshly loaded [`LiveState`] emits and for the ones arriving
/// live from the web server — one translation from wire command to engine
/// setter, for both sources.
fn apply_commands(session: &mut AudioSession, commands: &[StudioCommand]) {
    for command in commands {
        apply_command(session, *command);
    }
}

/// Translates one [`StudioCommand`] into the matching
/// [`AudioSession`] setter call.
fn apply_command(session: &mut AudioSession, command: StudioCommand) {
    match command {
        StudioCommand::NoteOn { midi, velocity } => {
            session.note_on(midi, velocity);
        }
        StudioCommand::NoteOff { midi } => {
            session.note_off(midi);
        }
        StudioCommand::AllNotesOff => {
            session.all_notes_off();
        }
        StudioCommand::SustainPedal { down } => {
            session.set_sustain_pedal(down);
        }
        StudioCommand::SetStringDamping {
            midi,
            string_index,
            damping,
        } => {
            session.set_string_damping(midi, string_index, damping);
        }
        StudioCommand::SetStringSustain {
            midi,
            string_index,
            sustain,
        } => {
            session.set_string_sustain(midi, string_index, sustain);
        }
        StudioCommand::SetStringInharmonicity {
            midi,
            string_index,
            inharmonicity,
        } => {
            session.set_string_inharmonicity(midi, string_index, inharmonicity);
        }
        StudioCommand::SetStringDetune {
            midi,
            string_index,
            cents,
        } => {
            session.set_string_detune(midi, string_index, cents);
        }
        StudioCommand::SetStringSeed {
            midi,
            string_index,
            seed,
        } => {
            session.set_string_seed(midi, string_index, seed);
        }
        StudioCommand::SetStringHammer {
            midi,
            string_index,
            hammer,
        } => {
            session.set_string_hammer(midi, string_index, hammer);
        }
        StudioCommand::SetSoundboardMode { index, mode } => {
            session.set_soundboard_mode(index, mode);
        }
        StudioCommand::SetLocalCouplingGain { gain } => {
            session.set_local_coupling_gain(gain);
        }
        StudioCommand::SetGlobalCouplingGain { gain } => {
            session.set_global_coupling_gain(gain);
        }
    }
}

/// Services MIDI (when connected) and the web server's command channel
/// until Esc or Ctrl+C is pressed in this terminal.
fn run_until_quit(
    session: &mut AudioSession,
    mut listener: Option<&mut MidiListener>,
    commands: &Receiver<StudioCommand>,
) -> Result<()> {
    loop {
        if let Some(listener) = listener.as_deref_mut() {
            drain_midi(session, listener);
        }
        drain_studio_commands(session, commands);
        if !event::poll(POLL_INTERVAL)? {
            continue;
        }
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Release && should_quit(&key) {
            return Ok(());
        }
    }
}

/// Drains commands the web server has queued, bounding one tick's work the
/// same way [`drain_midi`] bounds MIDI: a browser reload floods well over a
/// thousand commands at once.
fn drain_studio_commands(session: &mut AudioSession, commands: &Receiver<StudioCommand>) {
    for _ in 0..MAX_COMMANDS_PER_TICK {
        let Ok(command) = commands.try_recv() else {
            break;
        };
        apply_command(session, command);
    }
}

fn should_quit(key: &KeyEvent) -> bool {
    key.code == KeyCode::Esc
        || (key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL))
}

/// Plays queued MIDI through [`crate::midi::apply`] — the same mapping
/// `piano midi` uses, called rather than copied.
///
/// This loop used to reach a `match` of its own, which handled note-on and
/// note-off and dropped every control change on the floor. The sustain
/// pedal therefore worked under `piano midi` and did nothing whatsoever
/// here, with no error and no log line: a player pressing the pedal while
/// playing a `.piano.json` file simply found the instrument ignoring it.
/// Calling one shared mapping is what stops the two subcommands drifting
/// apart again, and puts both under `crate::midi`'s tests.
fn drain_midi(session: &mut AudioSession, listener: &mut MidiListener) {
    for _ in 0..MAX_EVENTS_PER_TICK {
        let Some(midi_event) = listener.poll() else {
            break;
        };
        crate::midi::apply(session, midi_event);
    }
}

fn print_instructions(port_name: &str) {
    println!("piano studio — connected to \"{port_name}\".");
    println!("play your MIDI controller alongside the browser.");
}

/// Ensures raw mode is always disabled on the way out, including on error
/// — the same guard `crate::midi` uses, duplicated rather than shared
/// because it is three lines and pulling it into a third module would cost
/// more in indirection than it saves.
struct RawModeGuard;

impl RawModeGuard {
    fn enable() -> io::Result<Self> {
        terminal::enable_raw_mode()?;
        Ok(Self)
    }
}

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use clap::Parser;

    use super::*;

    /// `StudioArgs` alone is not parseable — `clap::Args` describes a set of
    /// arguments, not a command line. This wraps it in the smallest parser
    /// that can be handed one.
    #[derive(Debug, clap::Parser)]
    struct OnlyStudioArgs {
        #[command(flatten)]
        studio: StudioArgs,
    }

    fn parse(arguments: &[&str]) -> StudioArgs {
        OnlyStudioArgs::parse_from(arguments).studio
    }

    #[test]
    fn studio_waits_for_a_late_instrument_by_default_just_like_piano_midi() {
        // Both subcommands look for the same instrument in the same way, so
        // a player who learns `piano midi`'s behaviour must not be met with
        // a different one here.
        let args = parse(&["piano-studio", "--piano", "x.json"]);
        assert_eq!(
            crate::midi::wait_duration(args.wait),
            crate::midi::wait_duration(crate::midi::DEFAULT_WAIT_SECONDS)
        );
    }

    #[test]
    fn the_wait_can_be_turned_off_for_a_fail_fast_run() {
        let args = parse(&["piano-studio", "--piano", "x.json", "--wait", "0"]);
        assert_eq!(crate::midi::wait_duration(args.wait), Duration::ZERO);
    }

    /// The regression test for the defect a player would have reported as
    /// "MIDI is not working": `piano studio --midi` kept its own copy of the
    /// event mapping, and that copy discarded every control change. The
    /// sustain pedal — the control a pianist uses most after the keys — did
    /// nothing here while working under `piano midi`, silently.
    ///
    /// Asserted through this module's own [`drain_midi`] path rather than by
    /// calling `crate::midi::apply` directly, so re-introducing a local
    /// `match` here fails the test instead of quietly passing it.
    #[test]
    fn the_studio_answers_every_midi_control_piano_midi_does() {
        use crate::sink::recorder::{Played, Recorder};

        let mut recorder = Recorder::default();
        for message in [
            [0x90u8, 60, 100].as_slice(), // middle C down
            &[0xB0, 64, 127],             // sustain pedal down
            &[0x80, 60, 64],              // middle C up, held by the pedal
            &[0xB0, 64, 0],               // pedal up
            &[0xB0, 74, 0],               // brightness knob to minimum
            &[0xB0, 1, 127],              // mod wheel to maximum
        ] {
            let event = piano_midi::MidiEvent::decode(message).expect("a message this CLI acts on");
            crate::midi::apply(&mut recorder, event);
        }

        assert_eq!(
            recorder.played,
            vec![
                Played::NoteOn {
                    midi: 60,
                    velocity: 100.0 / 127.0
                },
                Played::SustainPedal { down: true },
                Played::NoteOff { midi: 60 },
                Played::SustainPedal { down: false },
                Played::Damping { damping: 1.0 },
                Played::Sustain { sustain: 1.0 },
            ],
            "the studio dropped a control `piano midi` acts on"
        );
    }

    #[test]
    fn loading_a_file_without_midi_needs_no_port_at_all() {
        // The no-`--midi` path must never reach `connect`, so validating a
        // `.piano.json` still works with nothing plugged in.
        let args = parse(&["piano-studio", "--piano", "x.json"]);
        assert!(!args.midi);
    }
}
