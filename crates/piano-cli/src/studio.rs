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
use std::time::Duration;

use anyhow::{Context, Result};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::terminal;
use piano_audio::AudioSession;
use piano_midi::{MidiEvent, MidiListener};
use piano_params::Tuning;
use piano_studio::ResolvedPiano;

use crate::report_timing;

/// How long `event::poll` waits before re-checking for a reason to stop.
/// Same value as `crate::midi`'s own constant — both loops have the same
/// "check for Esc/Ctrl+C without busy-waiting" shape.
const POLL_INTERVAL: Duration = Duration::from_millis(50);

/// MIDI events drained per tick, bounding one iteration of the control
/// loop the same way `crate::midi::MAX_EVENTS_PER_TICK` does.
const MAX_EVENTS_PER_TICK: usize = 256;

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

    /// Frequency of concert A, in hertz.
    #[arg(long, default_value_t = 440.0)]
    concert_a: f32,
}

/// Runs `piano studio` until Esc or Ctrl+C (when `--midi` is given), or
/// until the file has been loaded, resolved and applied (otherwise).
///
/// # Errors
///
/// Returns an error if the `.piano.json` file cannot be loaded, if the
/// concert-A pitch is invalid, if no audio output device could be opened,
/// or (with `--midi`) if no MIDI port matches or the terminal could not be
/// put into raw mode.
pub(crate) fn run(args: &StudioArgs) -> Result<()> {
    let file = piano_studio::load(&args.piano)
        .with_context(|| format!("could not load {}", args.piano.display()))?;
    let tuning = Tuning::with_concert_a(args.concert_a)
        .with_context(|| format!("invalid concert A: {}", args.concert_a))?;
    let resolved = piano_studio::resolve(&file, tuning);

    let mut session = AudioSession::start(tuning).context("could not start audio playback")?;
    apply_resolved_piano(&mut session, &resolved);
    println!(
        "loaded {} — {} strings, {} soundboard mode override(s)",
        args.piano.display(),
        resolved.strings.len(),
        resolved.soundboard_modes.len(),
    );

    if !args.midi {
        report_timing(&session);
        return Ok(());
    }

    let mut listener = MidiListener::connect(args.port.as_deref())
        .context("could not connect to a MIDI input port")?;
    print_instructions(listener.port_name());
    let raw_mode = RawModeGuard::enable().context("could not enable terminal raw mode")?;
    let outcome = play_until_quit(&mut session, &mut listener);
    drop(raw_mode);
    outcome?;

    report_timing(&session);
    Ok(())
}

/// Queues every one of `resolved`'s per-string and instrument-wide
/// settings onto `session`, applied before the first note plays.
fn apply_resolved_piano(session: &mut AudioSession, resolved: &ResolvedPiano) {
    for string in &resolved.strings {
        session.set_string_damping(string.midi, string.string_index, string.damping);
        session.set_string_sustain(string.midi, string.string_index, string.sustain);
        session.set_string_inharmonicity(string.midi, string.string_index, string.inharmonicity);
        session.set_string_detune(string.midi, string.string_index, string.detune_cents);
        session.set_string_seed(string.midi, string.string_index, string.seed);
        session.set_string_hammer(string.midi, string.string_index, string.hammer);
    }
    for (index, mode) in resolved.soundboard_modes.iter().enumerate() {
        session.set_soundboard_mode(index, *mode);
    }
    session.set_local_coupling_gain(resolved.local_coupling_gain);
    session.set_global_coupling_gain(resolved.global_coupling_gain);
}

fn play_until_quit(session: &mut AudioSession, listener: &mut MidiListener) -> Result<()> {
    loop {
        drain_midi(session, listener);
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

fn should_quit(key: &KeyEvent) -> bool {
    key.code == KeyCode::Esc
        || (key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL))
}

fn drain_midi(session: &mut AudioSession, listener: &mut MidiListener) {
    for _ in 0..MAX_EVENTS_PER_TICK {
        let Some(midi_event) = listener.poll() else {
            break;
        };
        apply_midi_event(session, midi_event);
    }
}

/// Handles the same three MIDI events `crate::midi` does — this
/// subcommand adds a loaded file's baseline, not new live controls.
fn apply_midi_event(session: &mut AudioSession, event: MidiEvent) {
    match event {
        MidiEvent::NoteOn { note, velocity } => {
            session.note_on(note, velocity);
        }
        MidiEvent::NoteOff { note } => {
            session.note_off(note);
        }
        MidiEvent::ControlChange { .. } => {}
    }
}

fn print_instructions(port_name: &str) {
    println!("piano studio — connected to \"{port_name}\".");
    println!("play your MIDI controller; nothing is written to disk.");
    println!("Esc or Ctrl+C (in this terminal) to quit.\n");
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
