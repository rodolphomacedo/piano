//! Hardware denormal control — `PERF-002`.
//!
//! Every string decays toward zero and eventually produces denormal floats.
//! On x86, a denormal operation can cost 50-100 cycles instead of the usual
//! handful, which means a *quiet* voice becomes more expensive than a loud
//! one — exactly backwards. `piano-core` already flushes denormals in
//! software (`math::flush_denormal`) as a correctness backstop, but that
//! costs a compare and a select per filter per sample. Setting the hardware's
//! flush-to-zero and denormals-are-zero modes once, on the audio thread,
//! makes the CPU round denormals to zero for free instead.
//!
//! This is the one place in the project where `unsafe` is justified: setting
//! a floating-point control register cannot violate memory safety, only
//! rounding behaviour, and doing it correctly needs either a target-specific
//! intrinsic or an inline instruction. Each function below is narrowly scoped
//! to exactly that.

/// Enables flush-to-zero and denormals-are-zero for the calling thread.
///
/// Call once, on the audio thread, before the first callback. The effect is
/// per-thread and does not persist across threads or survive the thread
/// exiting.
#[cfg(target_arch = "x86_64")]
#[allow(unsafe_code)]
pub(crate) fn enable_flush_to_zero() {
    /// Bit 15 of MXCSR: flush-to-zero, for results that underflow.
    const MXCSR_FLUSH_TO_ZERO: u32 = 1 << 15;
    /// Bit 6 of MXCSR: denormals-are-zero, for denormal inputs.
    const MXCSR_DENORMALS_ARE_ZERO: u32 = 1 << 6;

    // SAFETY: reads then writes MXCSR, the SSE control/status register,
    // setting two architecturally-defined bits. x86_64 guarantees SSE2, so
    // MXCSR always exists. Raw `stmxcsr`/`ldmxcsr` are used instead of the
    // `_mm_getcsr`/`_mm_setcsr` intrinsics, which the standard library
    // deprecated because LLVM can reorder floating-point operations across
    // them; a real memory read/write is not reordered that way.
    unsafe {
        let mut mxcsr: u32 = 0;
        core::arch::asm!("stmxcsr [{0}]", in(reg) &raw mut mxcsr);
        mxcsr |= MXCSR_FLUSH_TO_ZERO | MXCSR_DENORMALS_ARE_ZERO;
        core::arch::asm!("ldmxcsr [{0}]", in(reg) &raw const mxcsr);
    }
}

/// Enables flush-to-zero for the calling thread.
///
/// AArch64 has a single FZ bit (bit 24 of FPCR) that governs both directions
/// of the x86 split between "flush to zero" and "denormals are zero".
#[cfg(target_arch = "aarch64")]
#[allow(unsafe_code)]
pub(crate) fn enable_flush_to_zero() {
    /// Bit 24 of FPCR: flush-to-zero, defined by the AArch64 architecture.
    const FPCR_FZ_BIT: u64 = 1 << 24;

    // SAFETY: FPCR is readable and writable from EL0 on every AArch64 core;
    // this reads it, sets one architecturally-defined bit, and writes it
    // back. No memory is touched, so this cannot be unsound.
    unsafe {
        let mut fpcr: u64;
        core::arch::asm!("mrs {0}, fpcr", out(reg) fpcr);
        fpcr |= FPCR_FZ_BIT;
        core::arch::asm!("msr fpcr, {0}", in(reg) fpcr);
    }
}

/// No hardware denormal control is known for this target.
///
/// `piano-core`'s software flush (`math::flush_denormal`) remains the
/// correctness backstop and still applies — this only affects speed.
#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
pub(crate) fn enable_flush_to_zero() {}
