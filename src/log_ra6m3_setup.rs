pub fn init() {
    let channels = rtt_target::rtt_init! {
        up: {
            0:{
                size: 1024,
                mode: rtt_target::ChannelMode::NoBlockTrim,
                name: "Terminal",
            }
        }
        section_cb: ".segger_rtt"
    };
    rtt_target::set_print_channel(channels.up.0);

    static LOGGER: RttLogger = RttLogger;
    // `set_logger` only fails if a logger was already installed; `init` runs once.
    let _ = log::set_logger(&LOGGER);
    log::set_max_level(log::LevelFilter::Trace);
}

/// `log::Log` backend that prepends an uptime timestamp to every record, so
/// both this crate's logs and third-party logs (smoltcp, ra-fsp-rs, picoserve)
/// share one timestamped format over RTT.
struct RttLogger;

impl log::Log for RttLogger {
    fn enabled(&self, _metadata: &log::Metadata) -> bool {
        true
    }

    fn log(&self, record: &log::Record) {
        let us = crate::mono::uptime_us();
        let secs = us / 1_000_000;
        let millis = us % 1_000_000 / 1_000;
        rtt_target::rprintln!(
            "[{:>5}.{:03}] {:<5} [{}] {}",
            secs,
            millis,
            record.level(),
            record.target(),
            record.args()
        );
    }

    fn flush(&self) {}
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    log::error!("Panic: {}", info);

    // todo: it is not ok for prod I guess? And what if low prriority task paniced?
    //       we probably need cs
    for _ in 0..1_000_000usize {
        cortex_m::asm::nop();
    }

    loop {
        cortex_m::asm::bkpt();
    }
}

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
