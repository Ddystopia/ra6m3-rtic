#![no_main]
#![no_std]
#![feature(never_type)]
#![feature(try_blocks)]
#![feature(type_alias_impl_trait)]

/*

IMPORTANT: Run with debugger attached. If not, set logger channel to `NoBlockTrim`
instead of `BlockIfFull`, or it won't work.

*/

use ra_fsp_rs::gpt_timer_monotonic;
#[cfg(feature = "ra6m3")]
use ra_fsp_rs::pac;

#[cfg(all(feature = "qemu", feature = "defmt"))]
#[path = "defmt_semihosting.rs"]
mod logger_setup;
#[cfg(all(feature = "qemu", feature = "log"))]
#[path = "log_semihosting.rs"]
mod logger_setup;
#[cfg(feature = "qemu")]
#[path = "net_semihosting.rs"]
mod net_device;

// fixme: if link is down for long, needs reset for ping to work

#[cfg(feature = "ra6m3")]
mod io_ports;
#[cfg(all(feature = "ra6m3", feature = "log"))]
#[path = "log_ra6m3.rs"]
mod logger_setup;
#[cfg(feature = "ra6m3")]
#[path = "net_ra6m3.rs"]
mod net_device;
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
mod socket;
mod socket_storage;
mod util;

#[cfg(feature = "ra6m3")]
use ra_fsp_rs::ioport::IoPortInstance;

use conf::{CLOCK_HZ, SYS_TICK_HZ};
#[allow(unused_imports)]
use rtic::mutex_prelude::*;
use rtic_monotonics::systick::prelude::*;
use smoltcp::iface::SocketHandle;

use socket_storage::SocketStorage;

pub use network::Net;

type Instant = fugit::Instant<u32, 1, CLOCK_HZ>;
type Duration = fugit::Duration<u32, 1, CLOCK_HZ>;

#[cfg(feature = "ra6m3")]
ra_fsp_rs::event_link_select! {
    ra_fsp_rs::e_elc_event::ELC_EVENT_EDMAC0_EINT => pac::Interrupt::IEL0,
}

// fixme: use GPT and u64 instead of SYST
systick_monotonic!(Mono, CLOCK_HZ);
gpt_timer_monotonic!(GptMono, 120_000_000);

const POLL_NETWORK: fn() = network::request_network_poll;

fn init(mut ctx: app::init::Context) -> (app::Shared, app::Local) {
    logger_setup::init();

    info!("Init start");
    info!("Size of tasks: {}b", ctx.executors_size);

    ctx.core.SCB.set_sleepdeep();

    let io_port = IoPortInstance::new(ctx.device.PORT0, io_ports::BSP_PIN_CFG);
    let io_port = ctx.local.io_port.get_or_insert(io_port);

    io_port.open().expect("Failed to open ioports");

    GptMono::start(gpt::open_gpt().expect("Failed to open GPT")).expect("Failed to start GPT");

    Mono::start(ctx.core.SYST, SYS_TICK_HZ); // How does this relate to CLOCK_HZ?

    #[cfg(feature = "ra6m3")]
    let (net, device, sockets) = network::init_network(
        ctx.cs,
        &mut ctx.core.NVIC,
        ctx.device.EDMAC0,
        ctx.device.ETHERC0,
        ctx.local.sockets,
    );

    #[cfg(feature = "qemu")]
    let (mut net, device, sockets) = network::init_network(ctx.cs, ctx.local.sockets);
    let [mqtt_socket_handle, http_socket_handle] = sockets;

    app::blinky::spawn(ctx.device.PORT1, ctx.device.PORT4).unwrap();
    app::http_task::spawn(http_socket_handle).unwrap();
    app::mqtt_task::spawn(mqtt_socket_handle).unwrap();
    app::network_link_poll::spawn().unwrap();
    app::network_poller::spawn().unwrap();
    app::waiter::spawn().unwrap();

    info!("Init done");

    (app::Shared { net, device }, app::Local {})
}

async fn waiter(_: app::waiter::Context<'_>) -> ! {
    let mut next = Mono::now();

    info!("Waiter task: 1 at {}", Mono::now().ticks());

    next += 1000.millis();
    Mono::delay_until(next).await;

    info!("Waiter task: 2 at {}", Mono::now().ticks());

    next += 1000.millis();
    Mono::delay_until(next).await;

    info!("Waiter task: 3 at {}", Mono::now().ticks());

    core::future::pending().await
}

#[cfg(feature = "qemu")]
pub use qemu_app::app;

#[cfg(feature = "ra6m3")]
pub use ra6m3_app::app;

#[cfg(feature = "qemu")]
mod qemu_app {
    use super::*;

    #[rtic::app(
      device = lm3s6965,
      dispatchers = [GPIOA, GPIOB, GPIOC, /* GPIOD, GPIOE */],
      peripherals = true
    )]
    mod app {
        use super::*;

        #[shared]
        pub struct Shared {
            pub net: Net,
            pub device: net_device::Dev,
            pub next_net_poll: Option<smoltcp::time::Instant>,
        }

        #[local]
        pub struct Local {
            pub net_poll_schedule_tx: WakeSourceRef<'static>,
        }

        #[init(local = [sockets: SocketStorage = SocketStorage::new(), net_waker: DiatomicWaker = DiatomicWaker::new()])]
        fn init(ctx: init::Context) -> (Shared, Local) {
            super::init(ctx)
        }

        #[task(priority = 3)]
        async fn waiter(ctx: waiter::Context<'_>) -> ! {
            super::waiter(ctx).await
        }

        #[task(priority = 1, shared = [net], local = [storage: mqtt::Storage = mqtt::Storage::new()])]
        async fn mqtt_task(ctx: mqtt_task::Context, socket: SocketHandle) -> ! {
            mqtt::mqtt(ctx, socket).await
        }

        #[task(priority = 1, shared = [net], local = [storage: http::Storage = http::Storage::new()])]
        async fn http_task(ctx: http_task::Context, socket: SocketHandle) -> ! {
            http::http(ctx, socket).await
        }

        #[task(binds = ETHERNET, priority = 2, shared = [device])]
        fn ethernet_isr(mut ctx: ethernet_isr::Context) {
            let cause = ctx.shared.device.lock(net_device::isr_handler);

            if cause.map_or(false, |cause| cause.receive) {
                POLL_NETWORK();
            }
        }

        #[task(priority = 2, shared = [net, device, next_net_poll], local = [net_poll_schedule_tx])]
        async fn poll_network(ctx: poll_network::Context) {
            super::poll_network(ctx);
        }

        #[task(priority = 1, shared = [next_net_poll])]
        async fn network_poll_scheduler(
            ctx: network_poll_scheduler::Context,
            sink: WakeSinkRef<'static>,
        ) {
            super::network_poll_scheduler(ctx, sink).await
        }

        #[idle]
        fn idle(_: idle::Context) -> ! {
            loop {
                rtic::export::wfi();
            }
        }
    }
}

#[cfg(feature = "ra6m3")]
mod ra6m3_app {
    use super::*;

    #[rtic::app(
      device = pac,
      dispatchers = [IEL95, IEL94, IEL93, IEL92, IEL91, IEL90, IEL89, IEL88],
      peripherals = true
    )]
    mod app {
        use ra_fsp_rs::ether::InterruptCause;

        use super::*;

        #[shared]
        pub struct Shared {
            #[unsafe(link_section = ".noinit")]
            pub net: network::Net,
            #[unsafe(link_section = ".noinit")]
            pub device: net_device::Dev,
        }

        #[local]
        pub struct Local {}

        #[init(local = [
            sockets: SocketStorage = SocketStorage::new(),
            io_port: Option<IoPortInstance> = None,
        ])]
        fn init(ctx: init::Context) -> (Shared, Local) {
            super::init(ctx)
        }

        #[task(priority = 3)]
        async fn waiter(ctx: waiter::Context<'_>) -> ! {
            super::waiter(ctx).await
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
        #[task(binds = IEL0, priority = 2)]
        fn ethernet_isr(_ctx: ethernet_isr::Context) {
            net_device::ethernet_isr_handler();
        }

        #[task(priority = 3, shared = [device])]
        async fn populate_buffers(mut ctx: populate_buffers::Context, cause: InterruptCause) {
            ctx.shared.device.lock(|d| d.populate_buffers(cause));
        }

        #[task(priority = 2, shared = [net, device])]
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
            const PERIOD_MS: u32 = 100;

            let mut next = Mono::now();
            let mut i: usize = 0;

            port1.pcntr1().write(|w| w.pdr()._1().podr()._1());

            loop {
                let phase = i % 8;

                if phase == 0 || phase == 2 {
                    port4.pcntr1().write(|w| w.pdr()._1().podr()._1());
                } else {
                    port4.pcntr1().write(|w| w.pdr()._1().podr()._0());
                }

                i = i.wrapping_add(1);
                next += PERIOD_MS.millis();
                Mono::delay_until(next).await;
            }
        }

        #[idle]
        fn idle(_: idle::Context) -> ! {
            loop {
                rtic::export::wfi();
            }
        }
    }
}
