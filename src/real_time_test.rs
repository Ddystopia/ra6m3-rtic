use core::pin::Pin;

use ra_fsp_rs::{
    gpt,
    pac::{self, GPT32E5, Interrupt},
    pin_init::InPlaceWrite,
    ra_fsp_sys as sys,
    state_markers::Opened,
    timer_api::TimerApi,
};
use rtic_monotonics::systick::prelude::*;
use static_cell::StaticCell;

use crate::Mono;

/*

GPT5 -> rtic + P600 + IEL10
GPT7 -> raw + P900 + IEL12

Generally I measured that
- Calling callback directly:
  49 with inline always, unreachables, count at the start
  64 with no inline alywas
  71 with both call and unreachable
- Thus 17 for a function call, 2-5 for unreachable, check for event optimized out

- Calling FSP that will call Rust wrapper, modifications in FSP:
   116 with inline and unreachable without many ifs
   123 with check for event
   147 with check for event and their ifs (+ 24)
   192 with check for event and their ifs and R_BSP_IrqStatusClear at the start, with/without inline always (+ 45)
   200 with all above and move count getter after stop
- Thus
   116 ticks was try to minimize FSP overhead, 116 - 49 = 57, maximum possible overhead of Rust wrapper
   Lets assume half of it goes to cuts of FSP and half to Rust wrappers, thus
   Rust Wrappers would eat up 28 ticks

*/

pub type Gpt5 = gpt::Gpt<'static, pac::GPT32E5, Opened>;
pub type Gpt7 = gpt::Gpt<'static, pac::GPT32E7, Opened>;

static GPT5: StaticCell<Gpt5> = StaticCell::new();
static GPT7: StaticCell<Gpt7> = StaticCell::new();

pub fn setup_gpt5(gpt: GPT32E5, cycle_end: Interrupt) -> Pin<&'static mut Gpt5> {
    use sys::generated::{
        s_gpt_gtior_setting__bindgen_ty_1 as GtiorInner,
        s_gpt_gtior_setting__bindgen_ty_1__bindgen_ty_1 as GtiorB,
    };

    let gtior_inner = GtiorInner {
        gtior_b: {
            let mut gtior_b = GtiorB::default();
            gtior_b.set_gtiob(0b01001);
            gtior_b.set_obdflt(0);
            gtior_b.set_obhld(0);
            gtior_b.set_obe(1);
            gtior_b.set_obdf(0);
            gtior_b.set_nfben(0);
            gtior_b
        },
    };

    let gpt5_cfg = ra_fsp_rs::timer_api::TimerConf {
        channel: 5,
        mode: sys::generated::e_timer_mode::TIMER_MODE_PERIODIC,
        period_counts: 1200_000, // 10ms
        source_div: sys::generated::e_timer_source_div::TIMER_SOURCE_DIV_1,
        cycle_end: Some(cycle_end),
        duty_cycle_counts: 0,
        extend: gpt::GptExtendedConfig {
            gtiocb: sys::generated::gpt_output_pin_t {
                output_enabled: true,
                stop_level: sys::generated::gpt_pin_level_t::GPT_PIN_LEVEL_LOW,
            },
            compare_match_value: [0, 50],
            compare_match_status: 0b10, // enable compare match b
            gtior_setting: sys::generated::gpt_gtior_setting_t {
                __bindgen_anon_1: gtior_inner,
            },
            ..Default::default()
        },
    };

    let mut gpt5 = GPT5
        .uninit()
        .write_pin_init(Gpt5::new_open(gpt, gpt5_cfg))
        .expect("Error creating GPT5");

    gpt5.as_mut().start().expect("Failed to start GPT5");

    gpt5
}

pub fn setup_gpt7(gpt: pac::GPT32E7, cycle_end: Interrupt) -> Pin<&'static mut Gpt7> {
    use sys::generated::{
        s_gpt_gtior_setting__bindgen_ty_1 as GtiorInner,
        s_gpt_gtior_setting__bindgen_ty_1__bindgen_ty_1 as GtiorB,
    };

    let gtior_inner = GtiorInner {
        gtior_b: {
            let mut gtior_b = GtiorB::default();
            gtior_b.set_gtiob(0b01001);
            gtior_b.set_obdflt(0);
            gtior_b.set_obhld(0);
            gtior_b.set_obe(1);
            gtior_b.set_obdf(0);
            gtior_b.set_nfben(0);
            gtior_b
        },
    };

    let gpt7_cfg = ra_fsp_rs::timer_api::TimerConf {
        channel: 7,
        mode: sys::generated::e_timer_mode::TIMER_MODE_PERIODIC,
        period_counts: 1200_000, // 10ms
        source_div: sys::generated::e_timer_source_div::TIMER_SOURCE_DIV_1,
        cycle_end: Some(cycle_end),
        duty_cycle_counts: 0,
        extend: gpt::GptExtendedConfig {
            gtiocb: sys::generated::gpt_output_pin_t {
                output_enabled: true,
                stop_level: sys::generated::gpt_pin_level_t::GPT_PIN_LEVEL_LOW,
            },
            compare_match_value: [0, 50],
            compare_match_status: 0b10, // enable compare match b
            gtior_setting: sys::generated::gpt_gtior_setting_t {
                __bindgen_anon_1: gtior_inner,
            },
            ..Default::default()
        },
    };

    let mut gpt7 = GPT7
        .uninit()
        .write_pin_init(Gpt7::new_open(gpt, gpt7_cfg))
        .expect("Error creating GPT7");

    gpt7.as_mut().start().expect("Failed to start GPT7");

    gpt7
}

// We can't get exactly 300us on a 12kHz clock, so choose the closest one down
// Print in us if it can be accurately represented in us
const DURATION: crate::Duration = crate::Duration::micros(300);
const DURATION_US: fugit::Duration<crate::Ticks, 1, 1_000_000> =
    match fugit::Duration::<crate::Ticks, 1, 1_000_000>::const_try_from(DURATION) {
        Some(d) => d,
        None => panic!(),
    };

#[unsafe(link_section = ".code_in_ram")]
pub async fn high_priority_task(
    _ctx: crate::app::hight_prioriority_task::Context<'_>,
    port9: pac::PORT9,
) -> ! {
    log::info!("Control loop with period {DURATION_US}");

    let mut next = Mono::now();
    loop {
        next += DURATION;
        Mono::delay_until(next).await;

        port9.pcntr3().write(|w| w.posr()._1());
        port9.pcntr3().write(|w| w.porr()._1());
    }
}


#[unsafe(no_mangle)]
#[unsafe(link_section = ".code_in_ram")]
extern "C" fn IEL12() {
    use core::sync::atomic::{Ordering::SeqCst, compiler_fence};
    use ra_fsp_rs::sys::generated as sys;

    let port = unsafe { pac::PORT9::steal() };
    port.pcntr3().write(|w| w.posr()._1());
    compiler_fence(core::sync::atomic::Ordering::SeqCst);
    cortex_m::asm::nop();
    cortex_m::asm::nop();
    cortex_m::asm::nop();
    cortex_m::asm::nop();
    cortex_m::asm::nop();
    cortex_m::asm::nop();
    cortex_m::asm::nop();
    cortex_m::asm::nop();
    cortex_m::asm::nop();
    cortex_m::asm::nop();
    compiler_fence(core::sync::atomic::Ordering::SeqCst);
    port.pcntr3().write(|w| w.porr()._1());
    compiler_fence(core::sync::atomic::Ordering::SeqCst);

    ra_fsp_rs::icu::irq_status_clear(pac::Interrupt::IEL12);
    compiler_fence(core::sync::atomic::Ordering::SeqCst);
}
