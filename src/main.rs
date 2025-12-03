#![no_main]
#![no_std]
#![feature(never_type)]
#![feature(try_blocks)]
#![feature(type_alias_impl_trait)]

/*

IMPORTANT: Run with debugger attached. If not, set logger channel to `NoBlockTrim`
instead of `BlockIfFull`, or it won't work.

todo:
  R_BSP_GroupIrqWrite(BSP_GRP_IRQ_MPU_STACK, handle_stack_overflow);

  And the stack we will be overflowing is interrupt stack, then where
    `handle_stack_overflow` gets to run?

*/

use ra_fsp_rs::pac;

// fixme: if link is down for long, needs reset for ping to work

mod io_ports;
#[cfg(feature = "log")]
mod log_ra6m3_setup;
mod net_ra6m3;
// mod rand;

mod log {
    #![allow(unused_imports)]

    pub use crate::debug;
    pub use crate::error;
    pub use crate::info;
    pub use crate::trace;
    pub use crate::warn;
}

mod conf;
mod http;
mod mqtt;
mod network;
mod poll_share;
#[cfg(feature = "real-time-tests")]
mod real_time_test;
mod socket;
mod socket_storage;
mod util;

use ra_fsp_rs::ioport::IoPort;

use conf::CLOCK_HZ;
use ra_fsp_rs::pin_init::InPlaceWrite;
#[allow(unused_imports)]
use rtic::mutex_prelude::*;
use rtic_monotonics::systick::prelude::*;
use smoltcp::iface::SocketHandle;

use socket_storage::SocketStorage;

use log_ra6m3_setup as logger_setup;
use net_ra6m3 as net_device;
use network::Net;

type Ticks = u64;
use fugit::ExtU64 as TimeExt;
type Instant = fugit::Instant<Ticks, 1, CLOCK_HZ>;
type Duration = fugit::Duration<Ticks, 1, CLOCK_HZ>;

ra_fsp_rs::event_link_select! {
    ra_fsp_rs::e_elc_event::ELC_EVENT_EDMAC0_EINT => pac::Interrupt::IEL0,
    ra_fsp_rs::e_elc_event::ELC_EVENT_GPT5_COUNTER_OVERFLOW => pac::Interrupt::IEL10,
    ra_fsp_rs::e_elc_event::ELC_EVENT_GPT7_COUNTER_OVERFLOW => pac::Interrupt::IEL12,
}

systick_monotonic!(Mono, CLOCK_HZ);

const POLL_NETWORK: fn() = network::request_network_poll;

fn init(mut ctx: app::init::Context) -> (app::Shared, app::Local) {
    logger_setup::init();

    info!("Init start");
    info!("Size of tasks: {}b", ctx.executors_size);

    ctx.core.SCB.set_sleepdeep();

    let io_port = IoPort::new(ctx.device.PORT0, io_ports::BSP_PIN_CFG);
    let Ok(mut io_port) = ctx.local.io_port.write_pin_init(io_port);

    io_port.as_mut().open().expect("Failed to open ioports");

    Mono::start(ctx.core.SYST, ra_fsp_rs::systick::system_core_clock(ctx.cs));

    unsafe {
        ctx.core.NVIC.set_priority(
            pac::Interrupt::IEL12,
            ra_fsp_rs::utils::fsp_prio_to_hw(1, pac::NVIC_PRIO_BITS),
        );
    }

    let mut gpt5 = real_time_test::setup_gpt5(ctx.device.GPT32E5, pac::Interrupt::IEL10);
    let mut gpt7 = real_time_test::setup_gpt7(ctx.device.GPT32E7, pac::Interrupt::IEL12);

    app::pricise_clear::spawn().unwrap();

    let (net, device, sockets) = network::init_network(
        ctx.cs,
        &mut ctx.core.NVIC,
        ctx.device.EDMAC0,
        ctx.device.ETHERC0,
        ctx.local.sockets,
    );

    let [mqtt_socket_handle, http_socket_handle] = sockets;

    app::blinky::spawn(ctx.device.PORT1, ctx.device.PORT4).unwrap();
    app::http_task::spawn(http_socket_handle).unwrap();
    app::mqtt_task::spawn(mqtt_socket_handle).unwrap();
    app::network_link_poll::spawn().unwrap();
    app::network_poller::spawn().unwrap();

    // interferes with measurements
    app::hight_prioriority_task::spawn(ctx.device.PORT9).unwrap();

    info!("Init done");

    (app::Shared { net, device }, app::Local { gpt5, gpt7 })
}

#[rtic::app(
  device = pac,
  dispatchers = [IEL95, IEL94, IEL93, IEL92, IEL91, IEL90, IEL89, IEL88],
  peripherals = true
)]
mod app {
    use core::mem::MaybeUninit;

    use super::*;

    #[shared]
    pub struct Shared {
        // Safety: That section is provided the the linker script and is valid for this purpose.
        #[unsafe(link_section = ".noinit")]
        pub net: network::Net,
        // Safety: That section is provided the the linker script and is valid for this purpose.
        #[unsafe(link_section = ".noinit")]
        pub device: net_device::Dev,
    }

    #[local]
    pub struct Local {
        pub gpt5: core::pin::Pin<&'static mut real_time_test::Gpt5>,
        pub gpt7: core::pin::Pin<&'static mut real_time_test::Gpt7>,
    }

    #[init(local = [
        sockets: SocketStorage = SocketStorage::new(),
        io_port: MaybeUninit<IoPort> = MaybeUninit::uninit(),
    ])]
    fn init(ctx: init::Context) -> (Shared, Local) {
        super::init(ctx)
    }

    #[task(priority = 3)]
    async fn pricise_clear(_ctx: pricise_clear::Context<'_>) {
        let gpt5 = unsafe { pac::GPT32E5::steal() };
        gpt5.gtclr().write(|w| w.cclr5()._1());
        for _ in 0..20 {
            cortex_m::asm::nop();
        }
        gpt5.gtclr().write(|w| w.cclr7()._1());
    }

    #[task(priority = 3)]
    async fn hight_prioriority_task(
        ctx: hight_prioriority_task::Context<'_>,
        port9: pac::PORT9,
    ) -> ! {
        super::real_time_test::high_priority_task(ctx, port9).await
    }

    #[task(priority = 1, shared = [net], local = [storage: mqtt::Storage = mqtt::Storage::new()])]
    async fn mqtt_task(ctx: mqtt_task::Context, socket: SocketHandle) -> ! {
        mqtt::mqtt(ctx, socket).await
    }

    #[task(priority = 1, shared = [net], local = [storage: http::Storage = http::Storage::new()])]
    async fn http_task(ctx: http_task::Context, socket: SocketHandle) -> ! {
        http::http(ctx, socket).await
    }

    // todo: is there a reason to give this task higher priority?
    #[task(binds = IEL0, priority = 2, shared = [device])]
    #[unsafe(link_section = ".code_in_ram")]
    fn ethernet_isr(mut ctx: ethernet_isr::Context) {
        ctx.shared.device.lock(|d| d.eth().handle_isr());
    }

    #[task(priority = 2, shared = [net, device])]
    #[unsafe(link_section = ".code_in_ram")]
    async fn network_poller(ctx: network_poller::Context) -> ! {
        network::network_poller_task(ctx).await
    }

    // fixme: this code was in NetxDuo. But I personally don't like polling
    //        every 10ms. Maybe we can somehow trigger something etc.
    //        I don't even rememeber teh point of that whole thing.
    #[task(priority = 1, shared = [device])]
    async fn network_link_poll(mut ctx: network_link_poll::Context) {
        let mut next = Mono::now();
        loop {
            next += 10.millis();
            Mono::delay_until(next).await;
            ctx.shared.device.lock(|dev| dev.poll_link());
        }
    }

    #[task(priority = 1)]
    async fn blinky(_ctx: blinky::Context, port1: pac::PORT1, port4: pac::PORT4) {
        const PERIOD_MS: Ticks = 100;

        let mut next = Mono::now();
        let mut i: usize = 0;

        // port1.pcntr3().write(|w| w.posr()._1());

        loop {
            let phase = i % 8;

            if phase == 0 || phase == 2 {
                port4.pcntr3().write(|w| w.posr()._1());
            } else {
                port4.pcntr3().write(|w| w.porr()._1());
            }

            i = i.wrapping_add(1);
            next += PERIOD_MS.millis();
            Mono::delay_until(next).await;
        }
    }

    #[task(binds = IEL10, priority = 3, local = [gpt5])]
    #[unsafe(link_section = ".code_in_ram")]
    fn gpt5_overflow_isr(ctx: gpt5_overflow_isr::Context) {
        use core::sync::atomic::{compiler_fence, Ordering::SeqCst};
        let port = unsafe { pac::PORT1::steal() };
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
        cortex_m::asm::nop();
        cortex_m::asm::nop();
        cortex_m::asm::nop();
        cortex_m::asm::nop();
        cortex_m::asm::nop();
        cortex_m::asm::nop();
        compiler_fence(core::sync::atomic::Ordering::SeqCst);
        port.pcntr3().write(|w| w.porr()._1());
        compiler_fence(core::sync::atomic::Ordering::SeqCst);
        ra_fsp_rs::icu::irq_status_clear(pac::Interrupt::IEL10);
        compiler_fence(core::sync::atomic::Ordering::SeqCst);
    }

    /*
    #[task(binds = IEL10, priority = 3, local = [gpt5])]
    fn gpt5_overflow_isr(ctx: gpt5_overflow_isr::Context) {
        use ra_fsp_rs::gpt::IsrPrototype;
        ctx.local.gpt5.as_mut().handle_isr(IsrPrototype::Overflow);

        // Generally I measured that
        // - Calling callback directly:
        //   49 with inline always, unreachables, count at the start
        //   64 with no inline alywas
        //   71 with both call and unreachable
        // - Thus 17 for a function call, 2-5 for unreachable, check for event optimized out
        //
        // - Calling FSP that will call Rust wrapper, modifications in FSP:
        //    116 with inline and unreachable without many ifs
        //    123 with check for event
        //    147 with check for event and their ifs (+ 24)
        //    192 with check for event and their ifs and R_BSP_IrqStatusClear at the start, with/without inline always
        //    200 with all above and move count getter after stop
        // - Thus
        //    116 ticks was try to minimize FSP overhead, 116 - 49 = 57, maximum possible overhead of Rust wrapper
        //    Lets assume half of it goes to cuts of FSP and half to Rust wrappers, thus
        //    Rust Wrappers would eat up 28 ticks

        // use ra_fsp_rs::ra_fsp_sys::generated::e_timer_event;
        // <real_time_test::Gpt5Context as ra_fsp_rs::Callback<e_timer_event, _>>::call_with_block(
        //     &real_time_test::Gpt5Context(),
        //     gpt5.as_mut(),
        //     e_timer_event::TIMER_EVENT_CYCLE_END,
        // );

        // I moved this R_BSP_IrqStatusClear inside FSP after the callback and latency
        // reduced by 41 cycles (120MHz)
    }
    */

    #[idle]
    fn idle(_: idle::Context) -> ! {
        loop {
            rtic::export::wfi();
        }
    }
}

#[unsafe(no_mangle)]
#[unsafe(link_section = ".code_in_ram")]
extern "C" fn IEL12() {
    use core::sync::atomic::{compiler_fence, Ordering::SeqCst};
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
