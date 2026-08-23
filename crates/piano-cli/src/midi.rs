//! `piano midi` — play notes live from a MIDI controller: a digital piano
//! plugged in over USB or a MIDI cable.
//!
//! No file is written. Same real-time queue as `piano keyboard`, just fed by
//! `piano-midi` instead of the computer keyboard.

use std::io;
use std::time::Duration;

use anyhow::{Context, Result};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::terminal;
use piano_audio::AudioSession;
use piano_midi::{MidiEvent, MidiListener};
use piano_params::Tuning;

use crate::report_timing;

/// How long `event::poll` waits before re-checking for a reason to stop.
const POLL_INTERVAL: Duration = Duration::from_millis(50);

/// MIDI events drained per tick: bounded so a flooded queue cannot make one
/// iteration of the control loop run unboundedly long.
const MAX_EVENTS_PER_TICK: usize = 256;

/// Control-change controller number mapped to the damping knob. CC74 is
/// General MIDI 2's "brightness" — the closest standard controller to this
/// instrument's damping parameter, and the one most controllers expose on a
/// physical knob or slider. Turning it up brightens a real synth, so it is
/// inverted here: `damping = 1 - cc74`.
const DAMPING_CC: u8 = 74;

/// Control-change controller number mapped to the sustain knob. CC1 is the
/// modulation wheel, present on nearly every controller, standing in for a
/// dedicated "sustain amount" control that General MIDI does not define.
const SUSTAIN_CC: u8 = 1;

/// Arguments for `piano midi`.
#[derive(Debug, clap::Args)]
pub(crate) struct MidiArgs {
    /// Substring of the MIDI input port name to connect to,
    /// case-insensitive. Defaults to the first port found.
    #[arg(short, long)]
    port: Option<String>,

    /// List available MIDI input ports and exit, without opening audio
    /// output.
    #[arg(long)]
    list: bool,

    /// Frequency of concert A, in hertz.
    #[arg(long, default_value_t = 440.0)]
    concert_a: f32,
}

/// Runs `piano midi` until Esc or Ctrl+C.
///
/// # Errors
///
/// Returns an error if `--list` fails to enumerate ports, if no MIDI port
/// matches, if the concert-A pitch is invalid, if no audio output device
/// could be opened, or if the terminal could not be put into raw mode.
pub(crate) fn run(args: &MidiArgs) -> Result<()> {
    if args.list {
        return list_ports();
    }

    let tuning = Tuning::with_concert_a(args.concert_a)
        .with_context(|| format!("invalid concert A: {}", args.concert_a))?;
    let mut session = AudioSession::start(tuning).context("could not start audio playback")?;
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

fn list_ports() -> Result<()> {
    let ports = MidiListener::available_ports().context("could not list MIDI input ports")?;
    if ports.is_empty() {
        println!("no MIDI input ports found");
        return Ok(());
    }
    println!("available MIDI input ports:");
    for name in ports {
        println!("  {name}");
    }
    Ok(())
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
        apply(session, midi_event);
    }
}

fn apply(session: &mut AudioSession, event: MidiEvent) {
    match event {
        MidiEvent::NoteOn { note, velocity } => {
            session.note_on(note, velocity);
        }
        MidiEvent::ControlChange { controller, value } if controller == DAMPING_CC => {
            session.set_damping(1.0 - value);
        }
        MidiEvent::ControlChange { controller, value } if controller == SUSTAIN_CC => {
            session.set_sustain(value);
        }
        // No release envelope yet — see the piano-midi crate docs. Every
        // other control change is not one this instrument understands.
        MidiEvent::NoteOff { .. } | MidiEvent::ControlChange { .. } => {}
    }
}

fn print_instructions(port_name: &str) {
    println!("piano midi — connected to \"{port_name}\".");
    println!("play your MIDI controller; nothing is written to disk.");
    println!(
        "CC{DAMPING_CC} (brightness) -> damping, inverted   CC{SUSTAIN_CC} (mod wheel) -> sustain"
    );
    println!("Esc or Ctrl+C (in this terminal) to quit.\n");
}

/// Ensures raw mode is always disabled on the way out, including on error —
/// otherwise an early return would leave the user's terminal broken.
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
