//! Why a MIDI connection could not be opened.

use thiserror::Error;

/// A failure connecting to, or reading from, a MIDI input device.
#[derive(Debug, Error)]
pub enum MidiError {
    /// The platform's MIDI backend could not be initialised at all — no
    /// driver, or the OS refused to open one.
    #[error("could not initialise the MIDI backend: {0}")]
    BackendInit(#[from] midir::InitError),

    /// The backend started, but it can see no input port at all.
    ///
    /// Kept separate from [`MidiError::NoMatchingPort`] because the two mean
    /// opposite things to whoever is holding the instrument: this one says
    /// nothing is there to connect to, so the message names what to check on
    /// the hardware. `available ports: []` — what the single combined
    /// variant used to print — told the player nothing they could act on.
    #[error(
        "no MIDI input port exists; check that the instrument is powered on, \
         that its cable is connected, and that its USB port is set to send \
         MIDI rather than audio only"
    )]
    NoPortsAvailable,

    /// Input ports exist, but no name contains the requested filter.
    ///
    /// Only reachable with a filter: without one the first port always
    /// matches, so `filter` is a `String` rather than an `Option<String>` —
    /// the filterless-yet-unmatched combination is not representable.
    #[error("no MIDI input port name contains {filter:?}; available ports: {available:?}")]
    NoMatchingPort {
        /// The filter that was requested.
        filter: String,
        /// Every port name the backend could see at the time.
        available: Vec<String>,
    },

    /// A port was found but its name could not be read.
    #[error("could not read the name of a MIDI input port: {0}")]
    PortName(#[from] midir::PortInfoError),

    /// The port was found and named, but the connection itself failed.
    ///
    /// Formatted to a `String` at the boundary rather than wrapping
    /// [`midir::ConnectError`] directly: that type carries the whole
    /// `MidiInput` back to the caller (so a failed connection can be
    /// retried with the same backend), and on at least one platform that
    /// embeds a handle that is not `Sync` — which would make `MidiError`
    /// itself not `Sync` and break every `anyhow::Context` call site.
    #[error("could not connect to the MIDI input port: {0}")]
    Connect(String),
}

impl MidiError {
    /// Whether this failure only means the requested port is not there
    /// *yet* — the one class of failure that waiting can fix, because a
    /// player can still switch the instrument on or plug the cable in.
    ///
    /// A backend that would not start, or a port that was found and then
    /// refused the connection, will not fix itself by being asked again, so
    /// [`MidiListener::connect_within`] fails on those immediately instead
    /// of making the player wait out a timeout for a foregone answer.
    ///
    /// [`MidiListener::connect_within`]: crate::MidiListener::connect_within
    #[must_use]
    pub fn is_port_absent(&self) -> bool {
        matches!(self, Self::NoPortsAvailable | Self::NoMatchingPort { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_absent_port_is_worth_waiting_for() {
        assert!(MidiError::NoPortsAvailable.is_port_absent());
        assert!(
            MidiError::NoMatchingPort {
                filter: "yamaha".to_owned(),
                available: alloc_names(&["Digital Piano"]),
            }
            .is_port_absent()
        );
    }

    #[test]
    fn a_refused_connection_is_not_worth_waiting_for() {
        assert!(!MidiError::Connect("port busy".to_owned()).is_port_absent());
    }

    #[test]
    fn an_empty_port_list_says_what_to_check_rather_than_printing_nothing() {
        let message = MidiError::NoPortsAvailable.to_string();
        assert!(message.contains("powered on"), "unhelpful: {message}");
        assert!(message.contains("cable"), "unhelpful: {message}");
        assert!(
            !message.contains("[]"),
            "still printing an empty list at the player: {message}"
        );
    }

    #[test]
    fn an_unmatched_filter_names_the_filter_and_what_was_there_instead() {
        let message = MidiError::NoMatchingPort {
            filter: "roland".to_owned(),
            available: alloc_names(&["Digital Piano", "IAC Driver Bus 1"]),
        }
        .to_string();
        assert!(message.contains("roland"), "lost the filter: {message}");
        assert!(
            message.contains("Digital Piano"),
            "lost what was available: {message}"
        );
    }

    fn alloc_names(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }
}
