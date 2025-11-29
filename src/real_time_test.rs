use core::pin::Pin;

use ra_fsp_rs::{
    gpt::{self, GptRegister},
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

This modules performs 2 tests:

1. Test of a simple periodic loop in a form likely used in production
  1. One part of the test toggles a P900 pin for oscilloscope to collect extremely accurate measurements
  2. Second part of the test occures during the "work" of loop body and manually measures jitter

2. Have a GPT timer that will periodically set a pin up, and code will try setting it down as fast as possible.
  We will do this by making GPT set P100 up and triggering interrupt. Then hardware task will set it back down.
  1. Will do this from FSP callback
  2. and the second one will do this before calling FSP ISR

Analysis later will have 3 lists of oscilloscope samples (I don't know how to collect them though, you may advice)
- A list of consisting of measurement sessions of 1.1 as `session-1-1-datetime.csv` files
- A list of consisting of measurement sessions of 2.1 as `session-2-1-datetime.csv` files
- A list of consisting of measurement sessions of 2.2 as `session-2-2-datetime.csv` files

Measurement sessions for 1.2 would be collected to `session-1-2-datetime.log` files.

*/

static GPT: StaticCell<gpt::Gpt<Opened>> = StaticCell::new();
static CALLBACK: StaticCell<Gpt5Context> = StaticCell::new();

// 2. it during callback or before calling into fsp isr and compare differences
pub struct Gpt5Context(pac::PORT1);
impl ra_fsp_rs::Callback<sys::generated::timer_event_t, gpt::Gpt<'_, Opened>> for Gpt5Context {
    fn call_with_block(
        _context: &Self,
        block: Pin<&mut gpt::Gpt<'_, Opened>>,
        event: sys::generated::timer_event_t,
    ) {
        if event == sys::generated::e_timer_event::TIMER_EVENT_CYCLE_END {
            let GptRegister::GPT32E5(gpt5) = block.regs_full() else {
                unreachable!()
            };
            gpt5.gtstp().write(|w| w.cstop5()._1());
            // gpt5.gtclr().write(|w| w.cclr5()._1());
            for _ in 0..10 {
                cortex_m::asm::nop();
            }

            gpt5.gtstr().write(|w| w.cstrt5()._1());
        }
    }
}
// impl ra_fsp_rs::Callback<sys::generated::timer_event_t> for Gpt5Context {
//     fn call(_context: &Self, event: sys::generated::timer_event_t) {
//         if event == sys::generated::e_timer_event::TIMER_EVENT_CYCLE_END {
//             for _ in 0..10 {
//                 cortex_m::asm::nop();
//             }
//         }
//     }
// }

pub fn setup_gpt5(
    gpt: GPT32E5,
    port1: pac::PORT1,
    cycle_end: Interrupt,
) -> Pin<&'static mut gpt::Gpt<'static, Opened>> {
    // gtiob 01000
    // obdflt 0 -- set down when stopped
    // obhld 0 -- set down when stopped
    // obe 1
    // obdf  00
    // nfben 0
    // nfcsb 00

    /*
    let gtior_inner = unsafe {
        let mut gtior_inner = sys::generated::s_gpt_gtior_setting__bindgen_ty_1::default();
        gtior_inner.gtior_b.set_gtiob(0b01000);
        gtior_inner.gtior_b.set_obdflt(0);
        gtior_inner.gtior_b.set_obhld(1);
        gtior_inner.gtior_b.set_obe(1);
        gtior_inner.gtior_b.set_obdf(0);
        gtior_inner.gtior_b.set_nfben(0);
        gtior_inner
    };

    let gpt5_cfg = ra_fsp_rs::timer_api::TimerConf {
        channel: 5,
        mode: sys::generated::e_timer_mode::TIMER_MODE_PERIODIC,
        period_counts: 187500, // 100ms
        source_div: sys::generated::e_timer_source_div::TIMER_SOURCE_DIV_64,
        cycle_end,
        duty_cycle_counts: 0,
        extend: gpt::GptExtendedConfig {
            gtiocb: sys::generated::gpt_output_pin_t {
                output_enabled: true,
                stop_level: sys::generated::gpt_pin_level_t::GPT_PIN_LEVEL_LOW,
            },
            gtior_setting: sys::generated::gpt_gtior_setting_t {
                __bindgen_anon_1: gtior_inner,
            },
            ..Default::default()
        },
    };
    */
    let gpt5_cfg = ra_fsp_rs::timer_api::TimerConf {
        channel: 5,
        mode: sys::generated::e_timer_mode::TIMER_MODE_PWM,
        period_counts: 1875, // 1ms
        source_div: sys::generated::e_timer_source_div::TIMER_SOURCE_DIV_64,
        cycle_end: Some(cycle_end),
        duty_cycle_counts: 1875 / 2,
        extend: gpt::GptExtendedConfig {
            gtiocb: sys::generated::gpt_output_pin_t {
                output_enabled: true,
                stop_level: sys::generated::gpt_pin_level_t::GPT_PIN_LEVEL_LOW,
            },
            ..Default::default()
        },
    };

    let mut gpt5 = GPT
        .uninit()
        .write_pin_init(gpt::Gpt::new_open(GptRegister::GPT32E5(gpt), gpt5_cfg))
        .expect("Error creating GPT5");

    gpt5.as_mut()
        .callback_set(CALLBACK.init(Gpt5Context(port1)))
        .expect("Failed to set GP5 callback");

    gpt5.as_mut().start().expect("Failed to start GPT5");

    gpt5
}

pub async fn high_priority_task(
    _ctx: crate::app::hight_prioriority_task::Context<'_>,
    port9: ra_fsp_rs::pac::PORT9,
) -> ! {
    port9.pcntr3().write(|w| w.posr()._0());

    // We can't get exactly 300us on a 12kHz clock, so choose the closest one down
    // Print in us if it can be accurately represented in us

    // 1 tick = 1/freq s = 1_000_000 / freq us
    // 1 us = freq / 1_000_000 ticks
    // 300 us = 300 * freq / 1_000_000
    let freq = crate::CLOCK_HZ as u64;
    let duration = crate::Duration::from_ticks(300 * freq / 1_000_000);

    if let Some(us_duration) =
        fugit::Duration::<crate::Ticks, 1, 1_000_000>::const_try_from(duration)
        && crate::Duration::const_try_from(us_duration) == Some(duration)
    {
        log::info!("Control loop with period {us_duration}");
    } else {
        log::info!("Control loop with period {duration}");
    }

    let mut next = Mono::now();
    loop {
        next += duration;
        Mono::delay_until(next).await;

        port9.pcntr3().write(|w| w.posr()._1());
        port9.pcntr3().write(|w| w.porr()._1());
    }
}
