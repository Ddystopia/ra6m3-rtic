pub fn init() {
    //
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {
        cortex_m::asm::bkpt();
    }
}

#[macro_export]
macro_rules! trace {
    ($($arg:tt)*) => { { _ = format_args!($($arg)*); } };
}
#[macro_export]
macro_rules! debug {
    ($($arg:tt)*) => { { _ = format_args!($($arg)*); } };
}
#[macro_export]
macro_rules! info {
    ($($arg:tt)*) => { { _ = format_args!($($arg)*); } };
}
#[macro_export]
macro_rules! warn {
    ($($arg:tt)*) => { { _ = format_args!($($arg)*); } };
}
#[macro_export]
macro_rules! error {
    ($($arg:tt)*) => { { _ = format_args!($($arg)*); } };
}
