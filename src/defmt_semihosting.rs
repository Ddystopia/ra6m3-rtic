//! `defmt` global logger over semihosting
//!
//! NOTE this is meant to only be used with QEMU
//!
//! WARNING using `cortex_m_semihosting`'s `hprintln!` macro or `HStdout` API will corrupt `defmt`
//! log frames so don't use those APIs.

// nightly warns about static_mut_refs, but 1.76 (our MSRV) does not know about
// that lint yet, so we ignore it it but also ignore if it is unknown
#![allow(unknown_lints, static_mut_refs)]

use core::sync::atomic::{AtomicBool, Ordering};

use cortex_m::{interrupt, register};
use cortex_m_semihosting::hio;
use rtic_monotonics::Monotonic;

defmt::timestamp!("{=f32}", (crate::Mono::now().ticks() / 10) as f32 / 100.0);

#[macro_export]
macro_rules! trace {
    ($($arg:tt)*) => { defmt::trace!($($arg)*) };
}
#[macro_export]
macro_rules! debug {
    ($($arg:tt)*) => { defmt::debug!($($arg)*) };
}
#[macro_export]
macro_rules! info {
    ($($arg:tt)*) => { defmt::info!($($arg)*) };
}
#[macro_export]
macro_rules! warn {
    ($($arg:tt)*) => { defmt::warn!($($arg)*) };
}
#[macro_export]
macro_rules! error {
    ($($arg:tt)*) => { defmt::error!($($arg)*) };
}

#[defmt::global_logger]
struct Logger;

pub fn init() {}

static TAKEN: AtomicBool = AtomicBool::new(false);
static INTERRUPTS_ACTIVE: AtomicBool = AtomicBool::new(false);
static mut ENCODER: defmt::Encoder = defmt::Encoder::new();

#[panic_handler]
fn panic_handler(info: &core::panic::PanicInfo<'_>) -> ! {
    use cortex_m_semihosting::debug;
    use defmt::Display2Format as F;

    defmt::error!("Panic: {}", F(info));

    debug::exit(debug::EXIT_FAILURE);

    // cortex_m::asm::udf()
    loop {
        cortex_m::asm::bkpt()
    }
}

unsafe impl defmt::Logger for Logger {
    fn acquire() {
        let primask = register::primask::read();
        interrupt::disable();

        if TAKEN.load(Ordering::Relaxed) {
            panic!("defmt logger taken reentrantly")
        }

        // no need for CAS because interrupts are disabled
        TAKEN.store(true, Ordering::Relaxed);

        INTERRUPTS_ACTIVE.store(primask.is_active(), Ordering::Relaxed);

        // safety: accessing the `static mut` is OK because we have disabled interrupts.
        unsafe { ENCODER.start_frame(do_write) }
    }

    unsafe fn flush() {
        // Do nothing.
        //
        // semihosting is fundamentally blocking, and does not have I/O buffers the target can control.
        // After write returns, the host has the data, so there's nothing left to flush.
    }

    unsafe fn release() {
        unsafe {
            // safety: accessing the `static mut` is OK because we have disabled interrupts.
            ENCODER.end_frame(do_write);

            TAKEN.store(false, Ordering::Relaxed);
            if INTERRUPTS_ACTIVE.load(Ordering::Relaxed) {
                // re-enable interrupts
                interrupt::enable()
            }
        }
    }

    unsafe fn write(bytes: &[u8]) {
        unsafe {
            // safety: accessing the `static mut` is OK because we have disabled interrupts.
            ENCODER.write(bytes, do_write);
        }
    }
}

fn do_write(bytes: &[u8]) {
    // using QEMU; it shouldn't mind us opening several handles (I hope)
    if let Ok(mut hstdout) = hio::hstdout() {
        hstdout.write_all(bytes).ok();
    }
}
