//! `embassy-time` time driver backed by a GPT timer channel, using the
//! `EmbassyGptTimerDriver` from `ra-fsp-rs`.
//!
//! # Why a dedicated timer
//!
//! `embassy-time` requires exactly one global time driver, registered through
//! the `embassy_time_driver` linker singleton. This driver gives embassy its
//! own hardware time source -- GPT channel 8 (`GPT328`) -- leaving SysTick to
//! the `rtic-monotonics` `Mono` monotonic. The two timekeepers are independent.
//!
//! # Clocking
//!
//! GPT counts from PCLKD (120 MHz on this board). With a `/4` source divider
//! the counter advances at 30 MHz, which must equal embassy's `TICK_HZ`
//! (`tick-hz-30_000_000` feature) -- [`EmbassyGptTimerDriver::start`] asserts
//! this against `R_GPT_InfoGet`. The 32-bit counter runs free
//! (`period_counts = 0xFFFF_FFFF`); the driver extends it to 64 bits in
//! software via the cycle-end and compare-A "period" events.
//!
//! # Interrupts
//!
//! Three GPT events drive the timekeeping; they are linked to IEL slots by the
//! `event_link_select!` table in `main.rs` and serviced by RTIC hardware tasks
//! that forward into FSP's ISRs through [`IsrPrototype::call_fsp_isr_handler`]:
//!
//! * counter overflow (cycle end) -> [`on_cycle_end`]
//! * capture/compare A (period midpoint) -> [`on_capture_a`]
//! * capture/compare B (alarm) -> [`on_capture_b`]
//!
//! Their NVIC priorities are owned by RTIC: the GPT `open` path reads the
//! priority RTIC has already programmed and feeds it back to FSP so FSP leaves
//! it untouched.

use ra_fsp_rs::DriverPlace;
use ra_fsp_rs::embassy::gpt::{EmbassyGptTimerDriver, time_driver_impl};
use ra_fsp_rs::gpt::{Channel, Gpt, GptExtendedConfig, IsrPrototype};
use ra_fsp_rs::pac::{GPT328, Interrupt};
use ra_fsp_rs::state_markers::Opened;
use ra_fsp_rs::sys::generated::{e_timer_mode, e_timer_source_div, gpt_source_t};
use ra_fsp_rs::timer_api::TimerConf;

time_driver_impl!(
    static DRIVER: EmbassyGptTimerDriver<GPT328> = EmbassyGptTimerDriver::new()
);

/// IEL slots the GPT8 events are linked to. These must match the
/// `event_link_select!` mapping and the `#[task(binds = ..)]` handlers in
/// `main.rs`.
pub const CYCLE_END_IRQ: Interrupt = Interrupt::IEL1;
pub const CAPTURE_A_IRQ: Interrupt = Interrupt::IEL2;
pub const CAPTURE_B_IRQ: Interrupt = Interrupt::IEL3;

/// Open GPT channel 8 and hand it to the embassy-time driver.
///
/// Call once from `init`, before any `embassy-time` timer is awaited.
pub fn start(gpt_reg: GPT328) {
    static GPT: DriverPlace<Gpt<'static, GPT328, Opened>> = DriverPlace::new();

    let cfg = TimerConf {
        channel: GPT328::N as u8,
        mode: e_timer_mode::TIMER_MODE_PERIODIC,
        period_counts: 0xFFFF_FFFF,
        // PCLKD (120 MHz) / 4 = 30 MHz == embassy TICK_HZ.
        source_div: e_timer_source_div::TIMER_SOURCE_DIV_4,
        duty_cycle_counts: 0,
        cycle_end: Some(CYCLE_END_IRQ),
        extend: GptExtendedConfig {
            capture_a: Some(CAPTURE_A_IRQ),
            capture_b: Some(CAPTURE_B_IRQ),
            count_up_source: gpt_source_t::GPT_SOURCE_NONE,
            count_down_source: gpt_source_t::GPT_SOURCE_NONE,
            // Enable compare-match channels A and B.
            compare_match_status: 0b11,
            ..Default::default()
        },
    };

    let gpt = GPT
        .write_pin_init(Gpt::new_open(gpt_reg, cfg))
        .expect("Failed to open GPT timer for embassy-time");

    DRIVER
        .start(gpt)
        .expect("Failed to start embassy-time GPT driver");
}

/// Dispatch a GPT ISR event from its bound RTIC task. `handle_isr` no-ops unless
/// it runs in the channel's configured IRQ, so this is safe to call directly.
#[inline]
pub fn handle_isr(isr_prototype: IsrPrototype) {
    DRIVER.handle_isr(isr_prototype);
}
