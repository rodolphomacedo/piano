//! `piano keyboard` — play notes live by typing on the computer keyboard.
//!
//! No file is written. Every keypress becomes a `NoteOn` command crossing
//! from this thread to the audio thread through the lock-free queue in
//! `piano-audio` — the everyday use case ADR-0005 exists for.

use std::io;
use std::time::Duration;

use anyhow::{Context, Result};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::terminal;
use piano_audio::AudioSession;
use piano_params::Tuning;

use crate::{parse_key, report_timing};

/// How long `event::poll` waits before re-checking for a reason to stop.
const POLL_INTERVAL: Duration = Duration::from_millis(50);

/// Strike strength used for every key: a QWERTY keyboard has no notion of
/// how hard a key was pressed.
const DEFAULT_VELOCITY: f32 = 0.85;

/// QWERTY key to semitone-offset table for the "typing keyboard as piano"
/// layout common to software synths: the bottom row is one octave, the top
/// row continues a fourth above that, overlapping by one note (`,` and `q`
/// both land on the twelfth semitone) so the hand-over between rows is
/// smooth.
const KEY_LAYOUT: [(char, u8); 30] = [
    ('z', 0),
    ('s', 1),
    ('x', 2),
    ('d', 3),
    ('c', 4),
    ('v', 5),
    ('g', 6),
    ('b', 7),
    ('h', 8),
    ('n', 9),
    ('j', 10),
    ('m', 11),
    (',', 12),
    ('q', 12),
    ('2', 13),
    ('w', 14),
    ('3', 15),
    ('e', 16),
    ('r', 17),
    ('5', 18),
    ('t', 19),
    ('6', 20),
    ('y', 21),
    ('7', 22),
    ('u', 23),
    ('i', 24),
    ('9', 25),
    ('o', 26),
    ('0', 27),
    ('p', 28),
];

/// Arguments for `piano keyboard`.
#[derive(Debug, clap::Args)]
pub(crate) struct KeyboardArgs {
    /// Note the bottom-left key (Z) plays; the layout spans about two
    /// octaves upward from here.
    #[arg(short, long, default_value = "C4")]
    base_note: String,

    /// Frequency of concert A, in hertz.
    #[arg(long, default_value_t = 440.0)]
    concert_a: f32,
}

/// Runs `piano keyboard` until Esc or Ctrl+C.
///
/// # Errors
///
/// Returns an error if the base note or concert-A pitch is invalid, if no
/// audio output device could be opened, or if the terminal could not be put
/// into raw mode.
pub(crate) fn run(args: &KeyboardArgs) -> Result<()> {
    let base_key = parse_key(&args.base_note)?;
    let tuning = Tuning::with_concert_a(args.concert_a)
        .with_context(|| format!("invalid concert A: {}", args.concert_a))?;
    let mut session = AudioSession::start(tuning).context("could not start audio playback")?;

    print_instructions(base_key.midi_number());
    let raw_mode = RawModeGuard::enable().context("could not enable terminal raw mode")?;
    let outcome = play_until_quit(&mut session, base_key.midi_number());
    drop(raw_mode);
    outcome?;

    report_timing(&session);
    Ok(())
}

fn play_until_quit(session: &mut AudioSession, base_note: u8) -> Result<()> {
    loop {
        if !event::poll(POLL_INTERVAL)? {
            continue;
        }
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind == KeyEventKind::Release {
            continue;
        }
        if should_quit(&key) {
            return Ok(());
        }
        if let KeyCode::Char(letter) = key.code {
            strike(session, base_note, letter);
        }
    }
}

fn should_quit(key: &KeyEvent) -> bool {
    key.code == KeyCode::Esc
        || (key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL))
}

fn strike(session: &mut AudioSession, base_note: u8, letter: char) {
    let Some(offset) = semitone_offset(letter) else {
        return;
    };
    session.note_on(base_note.saturating_add(offset), DEFAULT_VELOCITY);
}

fn semitone_offset(letter: char) -> Option<u8> {
    let lower = letter.to_ascii_lowercase();
    KEY_LAYOUT
        .iter()
        .find_map(|&(key, offset)| (key == lower).then_some(offset))
}

fn print_instructions(base_midi: u8) {
    println!("piano keyboard — live playback, nothing is written to disk.");
    println!("bottom row  z s x d c v g b h n j m ,   -> one octave from MIDI {base_midi}");
    println!("top row     q 2 w 3 e r 5 t 6 y 7 u i 9 o 0 p -> continues upward");
    println!("Esc or Ctrl+C to quit.\n");
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
