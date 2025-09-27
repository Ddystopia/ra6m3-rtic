pub fn init() {
    let channels = rtt_target::rtt_init! {
        up: {
            0:{
                size: 1024,
                mode: rtt_target::ChannelMode::BlockIfFull,
                name: "Terminal",
            }
        }
        section_cb: ".code_in_ram.segger_rtt"
    };
    rtt_target::set_print_channel(channels.up.0);
    rtt_target::init_logger_with_level(log::LevelFilter::Trace);
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
