//! Errors from loading or saving a `.piano.json` file, or from starting
//! the studio's local web server.

use std::net::SocketAddr;
use std::path::PathBuf;

/// Something went wrong reading or writing a `.piano.json` file, or
/// opening the studio's listening socket.
#[derive(Debug, thiserror::Error)]
pub enum StudioError {
    /// The file could not be read or written.
    #[error("could not access {path}: {source}")]
    Io {
        /// The file that could not be accessed.
        path: PathBuf,
        /// The underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// The file's contents were not valid `.piano.json`.
    #[error("{path} is not a valid .piano.json file: {source}")]
    Parse {
        /// The file that failed to parse.
        path: PathBuf,
        /// The underlying JSON error.
        #[source]
        source: serde_json::Error,
    },
    /// The studio's web server could not take the address it was asked
    /// for. Almost always another studio already running on that port.
    #[error("could not start the studio on {address}: {source}")]
    Bind {
        /// The address that could not be bound.
        address: SocketAddr,
        /// The underlying I/O error.
        #[source]
        source: std::io::Error,
    },
}
