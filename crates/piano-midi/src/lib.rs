//! MIDI input for the piano synthesiser: turns messages from a hardware
//! controller — a digital piano plugged in over USB or a MIDI cable — into
//! decoded [`MidiEvent`]s a control thread can turn into
//! `piano_audio::AudioSession` calls.
//!
//! This crate never touches `piano-audio`'s internals and never runs inside
//! its audio callback. The platform's MIDI driver calls back on its own
//! thread ([`midir`]'s), which only ever decodes one message and pushes it
//! into an SPSC ring buffer — the same lock-free pattern as `piano-audio`'s
//! command queue (ADR-0005), reused here because a driver callback thread is
//! just as unsuited to blocking as an audio callback is. [`MidiListener::poll`]
//! drains that queue from whichever thread owns the listener; under queue
//! pressure an event is dropped rather than the driver thread blocking.
//!
//! The current instrument (`piano-core`) has no release envelope: a struck
//! string rings until it decays on its own, with no way to damp it early.
//! [`MidiEvent::NoteOff`] is decoded so a future release model has
//! somewhere to plug in, but nothing acts on it yet.

use midir::{MidiInput, MidiInputPort};
use rtrb::{Consumer, Producer, RingBuffer};

mod error;
mod event;

pub use error::MidiError;
pub use event::MidiEvent;

/// How many decoded events the queue holds before new ones are dropped
/// rather than blocking the driver's callback thread.
const EVENT_QUEUE_CAPACITY: usize = 256;

/// A live connection to one MIDI input port.
///
/// Dropping this closes the connection, same as `piano_audio::AudioSession`
/// closes its stream on drop.
pub struct MidiListener {
    // Never read directly: kept alive purely so dropping `MidiListener`
    // closes the underlying connection.
    #[allow(dead_code)]
    connection: midir::MidiInputConnection<Producer<MidiEvent>>,
    consumer: Consumer<MidiEvent>,
    port_name: String,
}

impl MidiListener {
    /// Connects to a MIDI input port.
    ///
    /// `name_filter`, when given, picks the first port whose name contains
    /// it, case-insensitively; when `None`, the first port at all is used —
    /// the common case of one digital piano plugged in.
    ///
    /// # Errors
    ///
    /// Returns [`MidiError`] if the platform's MIDI backend cannot be
    /// initialised, a port's name cannot be read, no port matches, or the
    /// connection itself fails.
    pub fn connect(name_filter: Option<&str>) -> Result<Self, MidiError> {
        let input = MidiInput::new("piano-midi")?;
        let (port, port_name) = select_port(&input, name_filter)?;
        let (producer, consumer) = RingBuffer::new(EVENT_QUEUE_CAPACITY);
        let connection = input
            .connect(&port, "piano-midi-input", on_midi_message, producer)
            .map_err(|error| MidiError::Connect(error.to_string()))?;
        Ok(Self {
            connection,
            consumer,
            port_name,
        })
    }

    /// The name of the connected port, for status messages.
    #[must_use]
    pub fn port_name(&self) -> &str {
        &self.port_name
    }

    /// Every input port name currently visible to the system, in the order
    /// [`MidiListener::connect`] would consider them — for `--list`
    /// diagnostics before a port is chosen.
    ///
    /// # Errors
    ///
    /// Returns [`MidiError::BackendInit`] if the backend itself could not be
    /// started.
    pub fn available_ports() -> Result<Vec<String>, MidiError> {
        let input = MidiInput::new("piano-midi")?;
        input
            .ports()
            .iter()
            .map(|port| input.port_name(port).map_err(MidiError::from))
            .collect()
    }

    /// Pops the next queued event, if any, without blocking.
    pub fn poll(&mut self) -> Option<MidiEvent> {
        self.consumer.pop().ok()
    }
}

impl core::fmt::Debug for MidiListener {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("MidiListener")
            .field("port_name", &self.port_name)
            .finish_non_exhaustive()
    }
}

/// Whether a port's name should be selected under `filter`. `None` matches
/// the first port seen — pure and unit-tested independent of any real MIDI
/// backend, unlike [`select_port`] itself.
#[must_use]
fn matches_filter(port_name: &str, filter: Option<&str>) -> bool {
    filter.is_none_or(|filter| port_name.to_lowercase().contains(&filter.to_lowercase()))
}

/// Walks the backend's ports looking for one [`matches_filter`] accepts.
fn select_port(
    input: &MidiInput,
    name_filter: Option<&str>,
) -> Result<(MidiInputPort, String), MidiError> {
    let ports = input.ports();
    let mut available = Vec::with_capacity(ports.len());
    for port in &ports {
        let name = input.port_name(port)?;
        if matches_filter(&name, name_filter) {
            return Ok((port.clone(), name));
        }
        available.push(name);
    }
    Err(MidiError::NoMatchingPort {
        filter: name_filter.map(str::to_owned),
        available,
    })
}

/// Runs on the MIDI backend's own callback thread. Decodes one message and
/// pushes it into the queue, dropping it rather than blocking if the queue
/// is full — the same drop-not-block choice `piano-audio` makes for
/// commands, and for the same reason: a driver callback thread must return
/// promptly.
fn on_midi_message(_timestamp_micros: u64, message: &[u8], producer: &mut Producer<MidiEvent>) {
    if let Some(midi_event) = event::parse(message) {
        let _ = producer.push(midi_event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_filter_matches_the_first_port_seen() {
        assert!(matches_filter("Yamaha P-125", None));
        assert!(matches_filter("", None));
    }

    #[test]
    fn a_filter_matches_case_insensitively_by_substring() {
        assert!(matches_filter("Yamaha P-125", Some("yamaha")));
        assert!(matches_filter("Yamaha P-125", Some("P-125")));
        assert!(!matches_filter("Yamaha P-125", Some("Roland")));
    }

    // MidiError must stay Send + Sync on every platform, not just the one
    // this happens to run on: `anyhow::Context` requires it, and a variant
    // that embeds a platform backend handle (as midir's `ConnectError` does,
    // to hand the backend back to the caller) can be `Sync` on one backend
    // and not another. This caught exactly that on Linux/ALSA after passing
    // clean on macOS/CoreMIDI.
    fn assert_send_sync<T: Send + Sync>() {}

    #[test]
    fn midi_error_is_send_and_sync_on_every_platform() {
        assert_send_sync::<MidiError>();
    }
}
