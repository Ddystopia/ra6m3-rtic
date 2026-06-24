// Prepend an uptime timestamp to every defmt record, including third-party
// logs. Applies globally; defined once per program.
defmt::timestamp!("{=u64:us}", crate::mono::uptime_us());

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
