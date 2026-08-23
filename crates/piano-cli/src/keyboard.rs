//! `piano keyboard` — play notes live by typing on the computer keyboard.
//!
//! No file is written. Every keypress becomes a `NoteOn` command crossing
//! from this thread to the audio thread through the lock-free queue in
//! `piano-audio` — the everyday use case ADR-0005 exists for.

use std::io::{self, stdout};
use std::time::Duration;

use anyhow::{Context, Result};
use crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, KeyboardEnhancementFlags,
    PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::execute;
use crossterm::terminal;
use piano_audio::AudioSession;
use piano_core::string::{DEFAULT_DAMPING, DEFAULT_SUSTAIN};
use piano_params::Tuning;

use crate::{parse_key, report_timing};

/// How long `event::poll` waits before re-checking for a reason to stop.
const POLL_INTERVAL: Duration = Duration::from_millis(50);

/// Strike strength used for every key: a QWERTY keyboard has no notion of
/// how hard a key was pressed.
const DEFAULT_VELOCITY: f32 = 0.85;

/// How much `[`/`]` change damping per press.
const DAMPING_STEP: f32 = 0.02;

/// How much `-`/`=` change sustain per press. Smaller than
/// [`DAMPING_STEP`] because the audible range of sustain that does not
/// either die instantly or ring forever is narrow, close to 1.0.
const SUSTAIN_STEP: f32 = 0.002;

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

    let raw_mode = RawModeGuard::enable().context("could not enable terminal raw mode")?;
    // Most terminals (including macOS's Terminal.app) only ever deliver key
    // *press* events in raw mode; genuine key-up requires the terminal to
    // implement the Kitty keyboard protocol (kitty, WezTerm, and a growing
    // but still-minority list of others). Query rather than assume, and be
    // honest in the instructions about which case this run landed in —
    // never claim a release feature the terminal cannot actually deliver.
    let key_release_supported = terminal::supports_keyboard_enhancement().unwrap_or(false);
    let enhancement_guard = key_release_supported
        .then(KeyboardEnhancementGuard::enable)
        .transpose()
        .context("could not enable terminal key-release reporting")?;

    print_instructions(base_key.midi_number(), key_release_supported);
    let outcome = play_until_quit(&mut session, base_key.midi_number(), key_release_supported);
    drop(enhancement_guard);
    drop(raw_mode);
    outcome?;

    report_timing(&session);
    Ok(())
}

/// Live-adjustable voicing, mirrored locally so the terminal can print the
/// current value — `AudioSession` only accepts new settings, it does not
/// report the current one back.
struct Voicing {
    damping: f32,
    sustain: f32,
}

impl Voicing {
    fn defaults() -> Self {
        Self {
            damping: DEFAULT_DAMPING,
            sustain: DEFAULT_SUSTAIN,
        }
    }
}

fn play_until_quit(
    session: &mut AudioSession,
    base_note: u8,
    key_release_supported: bool,
) -> Result<()> {
    let mut voicing = Voicing::defaults();
    loop {
        if !event::poll(POLL_INTERVAL)? {
            continue;
        }
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind == KeyEventKind::Release {
            if key_release_supported {
                release_key(session, base_note, key.code);
            }
            continue;
        }
        if should_quit(&key) {
            return Ok(());
        }
        if let KeyCode::Char(letter) = key.code {
            handle_key(session, &mut voicing, base_note, letter);
        }
    }
}

/// Releases the note a key-up maps to. Only ever called when
/// `key_release_supported` was confirmed true, since most terminals never
/// deliver a `Release` kind at all — see [`run`].
fn release_key(session: &mut AudioSession, base_note: u8, code: KeyCode) {
    let KeyCode::Char(letter) = code else {
        return;
    };
    let Some(offset) = semitone_offset(letter) else {
        return;
    };
    session.note_off(base_note.saturating_add(offset));
}

fn handle_key(session: &mut AudioSession, voicing: &mut Voicing, base_note: u8, letter: char) {
    match letter {
        '[' => nudge_damping(session, voicing, -DAMPING_STEP),
        ']' => nudge_damping(session, voicing, DAMPING_STEP),
        '-' => nudge_sustain(session, voicing, -SUSTAIN_STEP),
        '=' => nudge_sustain(session, voicing, SUSTAIN_STEP),
        _ => strike(session, base_note, letter),
    }
}

fn nudge_damping(session: &mut AudioSession, voicing: &mut Voicing, delta: f32) {
    voicing.damping = (voicing.damping + delta).clamp(0.0, 1.0);
    session.set_damping(voicing.damping);
    println!("damping -> {:.2}", voicing.damping);
}

fn nudge_sustain(session: &mut AudioSession, voicing: &mut Voicing, delta: f32) {
    voicing.sustain = (voicing.sustain + delta).clamp(0.0, 1.0);
    session.set_sustain(voicing.sustain);
    println!("sustain -> {:.3}", voicing.sustain);
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

fn print_instructions(base_midi: u8, key_release_supported: bool) {
    println!("piano keyboard — live playback, nothing is written to disk.");
    println!("bottom row  z s x d c v g b h n j m ,   -> one octave from MIDI {base_midi}");
    println!("top row     q 2 w 3 e r 5 t 6 y 7 u i 9 o 0 p -> continues upward");
    println!("[ ]  -> damping down / up (brighter <-> duller)");
    println!("- =  -> sustain down / up (shorter <-> longer ring)");
    if key_release_supported {
        println!("this terminal reports key-up: releasing a note key damps it early.");
    } else {
        println!(
            "this terminal does not report key-up (most don't outside kitty/WezTerm-style \
             terminals) — notes always ring out on their own; use `piano midi` for real \
             note-off and a sustain pedal."
        );
    }
    println!("Esc or Ctrl+C to quit.\n");
}

/// Enables the terminal's progressive keyboard enhancement protocol so
/// `event::read` reports key-*release*, not only key-press — see [`run`].
/// Pushed and popped the same bracketed way [`RawModeGuard`] handles raw
/// mode, so an early return still leaves the terminal in its original
/// state.
struct KeyboardEnhancementGuard;

impl KeyboardEnhancementGuard {
    fn enable() -> io::Result<Self> {
        execute!(
            stdout(),
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::REPORT_EVENT_TYPES)
        )?;
        Ok(Self)
    }
}

impl Drop for KeyboardEnhancementGuard {
    fn drop(&mut self) {
        let _ = execute!(stdout(), PopKeyboardEnhancementFlags);
    }
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
