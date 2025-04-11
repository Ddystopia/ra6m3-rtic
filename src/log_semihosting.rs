// nightly warns about static_mut_refs, but 1.76 (our MSRV) does not know about
// that lint yet, so we ignore it it but also ignore if it is unknown
#![allow(unknown_lints, static_mut_refs)]

use rtic_monotonics::Monotonic;

#[macro_export]
macro_rules! trace {
    ($($arg:tt)*) => { ::log::trace!($($arg)*) };
}
#[macro_export]
macro_rules! debug {
    ($($arg:tt)*) => { ::log::debug!($($arg)*) };
}
#[macro_export]
macro_rules! info {
    ($($arg:tt)*) => { ::log::info!($($arg)*) };
}
#[macro_export]
macro_rules! warn {
    ($($arg:tt)*) => { ::log::warn!($($arg)*) };
}
#[macro_export]
macro_rules! error {
    ($($arg:tt)*) => { ::log::error!($($arg)*) };
}

static LOGGER: Logger = Logger;

struct Logger;

pub fn init() {
    log::set_logger(&LOGGER).unwrap();
    if cfg!(debug_assertions) {
        log::set_max_level(log::LevelFilter::Trace);
    } else {
        log::set_max_level(log::LevelFilter::Info);
    }
}

impl log::Log for Logger {
    fn enabled(&self, _metadata: &log::Metadata) -> bool {
        true
    }

    fn log(&self, record: &log::Record) {
        cortex_m_semihosting::hprintln!(
            "{} {}: {}",
            crate::Mono::now().ticks(),
            record.level(),
            record.args()
        )
    }

    fn flush(&self) {}
}

#[panic_handler]
fn panic_handler(info: &core::panic::PanicInfo<'_>) -> ! {
    use cortex_m_semihosting::debug;

    log::error!("Panic: {}", info);

    debug::exit(debug::EXIT_FAILURE);

    // cortex_m::asm::udf()
    loop {
        cortex_m::asm::bkpt()
    }
}
