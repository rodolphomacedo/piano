//! Real-time-safe DSP primitives for physically modelled piano strings.
//!
//! # Design contract
//!
//! Every type in this crate obeys three rules that make it safe to call from an
//! audio callback:
//!
//! 1. **No allocation while processing.** Buffers are sized and allocated in
//!    constructors. `process` and `process_block` only touch memory they
//!    already own.
//! 2. **No panics.** Indices are masked into range, divisors are validated at
//!    construction, and every cast saturates. Invalid parameters are rejected
//!    by [`ParamError`] before they reach the hot path.
//! 3. **No platform coupling.** The crate is `no_std` + `alloc` and knows
//!    nothing about audio devices, files, threads or time.
//!
//! See `docs/REALTIME-AUDIO-RULES.md` for the rules these guarantees come from
//! and `docs/PERFORMANCE.md` for the known bottlenecks.

#![cfg_attr(not(feature = "std"), no_std)]
#![forbid(unsafe_code)]

extern crate alloc;

pub mod bridge;
pub mod delay;
pub mod dispersion;
pub mod filter;
pub mod hammer;
pub mod math;
pub mod noise;
pub mod soundboard;
pub mod string;
pub mod unison;

mod error;
mod units;

pub use bridge::BridgeBus;
pub use error::ParamError;
pub use soundboard::Soundboard;
pub use string::PluckedString;
pub use unison::UnisonGroup;
pub use units::{Hz, SampleRate};
