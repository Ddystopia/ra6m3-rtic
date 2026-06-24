//! `rtic-monotonics` monotonic backed by a GPT timer channel, defined with the
//! `gpt_monotonic!` macro from `ra-fsp-rs`.
//!
//! # Why a GPT monotonic
//!
//! This gives RTIC its own free-running hardware time source -- GPT channel 9
//! (`GPT329`) -- independent of `embassy-time`, which runs on GPT channel 8 (see
//! `embassy_time_driver`). The two timekeepers are independent.
//!
//! # Clocking
//!
//! GPT counts from PCLKD (120 MHz on this board). With a `/4` source divider the
//! counter advances at 30 MHz, which is the monotonic tick rate (`conf::CLOCK_HZ`,
//! passed to `gpt_monotonic!`). The 32-bit counter runs free (`period_counts =
//! 0xFFFF_FFFF`); `ra-fsp-rs` extends it to 64 bits in software via the cycle-end
//! and compare-A "period" events.
//!
//! # Interrupts
//!
//! Three GPT events drive the timekeeping; they are linked to IEL slots by the
//! `event_link_select!` table in `main.rs` and serviced by RTIC hardware tasks
//! that forward into [`handle_isr`]:
//!
//! * counter overflow (cycle end) -> [`IsrPrototype::Overflow`]
//! * capture/compare A (period midpoint) -> [`IsrPrototype::CompareA`]
//! * capture/compare B (alarm) -> [`IsrPrototype::CompareB`]
//!
//! Their NVIC priorities are owned by RTIC: the GPT `open` path reads the
//! priority RTIC has already programmed and feeds it back to FSP so FSP leaves
//! it untouched.

use core::sync::atomic::{AtomicBool, Ordering};

use ra_fsp_rs::DriverPlace;
use ra_fsp_rs::gpt::{Channel, Gpt, GptExtendedConfig, IsrPrototype};
use ra_fsp_rs::pac::{GPT329, Interrupt};
use ra_fsp_rs::state_markers::Opened;
use ra_fsp_rs::sys::generated::{e_timer_mode, e_timer_source_div, gpt_source_t};
use ra_fsp_rs::timer_api::TimerConf;
use rtic_monotonics::Monotonic;

use crate::conf::CLOCK_HZ;

ra_fsp_rs::gpt_monotonic!(Mono<GPT329>, CLOCK_HZ);

/// Set once [`start`] has run. `Mono::now()` panics until the GPT driver is
/// initialized, so [`uptime_us`] checks this before reading the clock.
static STARTED: AtomicBool = AtomicBool::new(false);

/// Microseconds elapsed since [`start`] was called.
///
/// Returns 0 before the monotonic is up, so it is safe to call from the logger
/// during early `init` (the first few log lines will read 0).
pub fn uptime_us() -> u64 {
    if !STARTED.load(Ordering::Relaxed) {
        return 0;
    }
    Mono::now().duration_since_epoch().to_micros()
}

/// IEL slots the GPT9 events are linked to. These must match the
/// `event_link_select!` mapping and the `#[task(binds = ..)]` handlers in
/// `main.rs`.
pub const CYCLE_END_IRQ: Interrupt = Interrupt::IEL4;
pub const CAPTURE_A_IRQ: Interrupt = Interrupt::IEL5;
pub const CAPTURE_B_IRQ: Interrupt = Interrupt::IEL6;

/// Open GPT channel 9 and start the [`Mono`] monotonic.
///
/// Call once from `init`, before any `Mono` timer is awaited.
pub fn start(gpt_reg: GPT329) {
    static GPT: DriverPlace<Gpt<'static, GPT329, Opened>> = DriverPlace::new();

    let cfg = TimerConf {
        channel: GPT329::N as u8,
        mode: e_timer_mode::TIMER_MODE_PERIODIC,
        period_counts: 0xFFFF_FFFF,
        // PCLKD (120 MHz) / 4 = 30 MHz == the monotonic tick rate (CLOCK_HZ).
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
        .expect("Failed to open GPT timer for the rtic monotonic");

    Mono::start(gpt).expect("Failed to start the rtic GPT monotonic");

    STARTED.store(true, Ordering::Relaxed);
}

/// Dispatch a GPT9 ISR event from its bound RTIC task. `handle_isr` no-ops
/// unless it runs in the channel's configured IRQ, so this is safe to call
/// directly.
#[inline]
pub fn handle_isr(isr_prototype: IsrPrototype) {
    Mono::handle_isr(isr_prototype);
}
