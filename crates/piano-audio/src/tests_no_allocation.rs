//! The no-allocation proof — issue #19.
//!
//! Every other test in this crate checks behaviour. This one checks a
//! structural claim: that draining commands and rendering a block never
//! touches the global allocator, no matter how many notes are struck. A
//! custom allocator records whether an allocation happened while a guard
//! flag is set; the flag brackets exactly the two calls the real audio
//! callback makes every time it runs (see `stream::AudioCallback::run`).
//!
//! This is the single most valuable test in the project: the failure it
//! catches — an allocation buried a few calls deep in the audio path — is
//! intermittent, environment-dependent, and nearly impossible to reproduce
//! by ear or by reading the diff. An earlier version of `Engine::note_on`
//! built a fresh `PluckedString` per strike from a small dynamic pool, which
//! allocated on the audio thread on every note-on; this test is what a
//! correct implementation must pass.

// A custom `GlobalAlloc` is unavoidably `unsafe`; this module exists only
// under `#[cfg(test)]` and never ships. See CONTRIBUTING.md: test modules
// opt out of the `unsafe_code` gate, production code does not.
#![allow(unsafe_code, clippy::unwrap_used, clippy::expect_used)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicUsize, Ordering};

use piano_core::SampleRate;
use piano_params::{HIGHEST_PIANO_KEY, LOWEST_PIANO_KEY, Tuning};

use crate::commands::Command;
use crate::engine::Engine;

thread_local! {
    static GUARD_ACTIVE: Cell<bool> = const { Cell::new(false) };
}

/// Allocations observed while the guard was active.
static VIOLATIONS: AtomicUsize = AtomicUsize::new(0);

struct GuardedAllocator;

// SAFETY: every call is forwarded unchanged to `System`, which is itself a
// sound `GlobalAlloc`. The only behaviour added here is a counter
// increment, which cannot affect the validity of an allocation.
unsafe impl GlobalAlloc for GuardedAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if GUARD_ACTIVE.with(Cell::get) {
            VIOLATIONS.fetch_add(1, Ordering::SeqCst);
        }
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: GuardedAllocator = GuardedAllocator;

fn guarded<R>(work: impl FnOnce() -> R) -> R {
    GUARD_ACTIVE.with(|flag| flag.set(true));
    let result = work();
    GUARD_ACTIVE.with(|flag| flag.set(false));
    result
}

#[test]
fn draining_commands_and_processing_blocks_never_allocates() {
    let rate = SampleRate::new(48_000.0).expect("48 kHz is valid");
    let mut engine = Engine::new(rate, Tuning::default());
    let (mut producer, mut consumer) = rtrb::RingBuffer::new(256);
    let key_span = u32::from(HIGHEST_PIANO_KEY - LOWEST_PIANO_KEY);

    let before = VIOLATIONS.load(Ordering::SeqCst);
    guarded(|| {
        for block in 0..4_096u32 {
            if block % 100 == 0 {
                let midi = LOWEST_PIANO_KEY + ((block / 100) % key_span) as u8;
                let _ = producer.push(Command::NoteOn {
                    midi,
                    velocity: 0.75,
                });
            }
            if block % 733 == 0 {
                let _ = producer.push(Command::AllNotesOff);
            }
            if block % 211 == 0 {
                let _ = producer.push(Command::SetDamping {
                    damping: 0.4 + 0.01 * (block % 50) as f32,
                });
            }
            if block % 317 == 0 {
                let _ = producer.push(Command::SetSustain {
                    sustain: 0.9 + 0.001 * (block % 90) as f32,
                });
            }
            engine.drain_commands(&mut consumer);
            let mut buffer = [0.0f32; 128];
            engine.process_block(&mut buffer);
        }
    });
    let after = VIOLATIONS.load(Ordering::SeqCst);

    assert_eq!(
        after - before,
        0,
        "the guarded region allocated {} time(s) — the audio callback must never allocate",
        after - before
    );
}
