//! Why a MIDI connection could not be opened.

use thiserror::Error;

/// A failure connecting to, or reading from, a MIDI input device.
#[derive(Debug, Error)]
pub enum MidiError {
    /// The platform's MIDI backend could not be initialised at all — no
    /// driver, or the OS refused to open one.
    #[error("could not initialise the MIDI backend: {0}")]
    BackendInit(#[from] midir::InitError),

    /// No input port matched the requested name filter, or no ports exist
    /// at all.
    #[error("no MIDI input port matched {filter:?}; available ports: {available:?}")]
    NoMatchingPort {
        /// The filter that was requested, if any.
        filter: Option<String>,
        /// Every port name the backend could see at the time.
        available: Vec<String>,
    },

    /// A port was found but its name could not be read.
    #[error("could not read the name of a MIDI input port: {0}")]
    PortName(#[from] midir::PortInfoError),

    /// The port was found and named, but the connection itself failed.
    #[error("could not connect to the MIDI input port: {0}")]
    Connect(#[from] midir::ConnectError<midir::MidiInput>),
}
