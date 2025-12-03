use core::pin::Pin;

use ra_fsp_rs::{
    gpt::{self},
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

pub type Gpt5 = gpt::Gpt<'static, gpt::Channel5, Opened>;
pub type Gpt6 = gpt::Gpt<'static, gpt::Channel6, Opened>;
pub type Gpt7 = gpt::Gpt<'static, gpt::Channel7, Opened>;

static GPT5: StaticCell<Gpt5> = StaticCell::new();
static GPT6: StaticCell<Gpt6> = StaticCell::new();
static GPT7: StaticCell<Gpt7> = StaticCell::new();
static CALLBACK: StaticCell<Gpt5Context> = StaticCell::new();

// 2. it during callback or before calling into fsp isr and compare differences
pub struct Gpt5Context();
impl ra_fsp_rs::Callback<sys::generated::timer_event_t, Gpt5> for Gpt5Context {
    #[inline(always)]
    fn call_with_block(
        _context: &Self,
        _block: Pin<&mut Gpt5>,
        _event: sys::generated::timer_event_t,
    ) {
        let port = unsafe { pac::PORT1::steal() };
        port.pcntr3().write(|w| w.posr()._1());
        port.pcntr3().write(|w| w.porr()._1());
    }
}

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

    gpt5.as_mut()
        .callback_set(CALLBACK.init(Gpt5Context()))
        .expect("Failed to set GP5 callback");

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
