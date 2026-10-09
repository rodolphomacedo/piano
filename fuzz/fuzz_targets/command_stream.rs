//! Arbitrary command streams into the whole engine (issue #56): whatever
//! the order of notes, pedals, live settings and sample rate, every output
//! sample must be finite and inside the limiter's `[-1, 1]`.
//!
//! ```sh
//! cargo +nightly fuzz run command_stream fuzz/corpus/command_stream
//! ```

#![no_main]

use libfuzzer_sys::fuzz_target;
use piano_audio::fuzzing::run_command_stream;

fuzz_target!(|bytes: &[u8]| {
    let outcome = run_command_stream(bytes, |work| work());
    assert!(outcome.is_sound(), "{outcome:?}");
});
