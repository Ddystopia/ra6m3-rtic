#![no_main]
#![no_std]
#![feature(never_type)]
#![feature(try_blocks)]
#![feature(type_alias_impl_trait)]

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

#[cfg(all(feature = "ra6m3", feature = "log"))]
#[path = "log_ra6m3.rs"]
mod logger_setup;
#[cfg(feature = "ra6m3")]
#[path = "net_ra6m3.rs"]
mod net_device;
// mod rand;

#[cfg(feature = "ra6m3")]
mod io_ports;

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
mod poll_share;
mod socket;
mod socket_storage;
mod util;

#[cfg(feature = "ra6m3")]
use ra_fsp_rs::ioport::{IoPort, IoPortInstance};

use bare_metal::CriticalSection;
use conf::{
    CLOCK_HZ, IP_V4, IP_V4_GATEWAY, IP_V4_NETMASK, IP_V6, IP_V6_GATEWAY, IP_V6_NETMASK, MAC,
    SYS_TICK_HZ,
};
use diatomic_waker::{DiatomicWaker, WakeSinkRef, WakeSourceRef};
use rtic::mutex_prelude::*;
use rtic_monotonics::systick::prelude::*;
use smoltcp::{
    iface::{Config, Interface, SocketHandle, SocketSet},
    socket::tcp,
    wire::{self, HardwareAddress, IpCidr},
};

use socket_storage::{SocketStorage, TcpSocketStorage};

type Instant = fugit::Instant<u32, 1, CLOCK_HZ>;
type Duration = fugit::Duration<u32, 1, CLOCK_HZ>;

#[cfg(feature = "ra6m3")]
ra_fsp_rs::event_link_select! {
    ra_fsp_rs::e_elc_event::ELC_EVENT_EDMAC0_EINT => pac::Interrupt::IEL0,
}

// fixme: use GPT and u64 instead of SYST
systick_monotonic!(Mono, CLOCK_HZ);

#[allow(dead_code)] // maybe we will need this waker idk
const NET_WAKER: core::task::Waker = util::waker(POLL_NETWORK);
const POLL_NETWORK: fn() = || _ = app::poll_network::spawn().ok();

pub struct Net {
    iface: Interface,
    sockets: SocketSet<'static>,
}

fn smol_now() -> smoltcp::time::Instant {
    let ticks = Mono::now().ticks() as i64 * 1_000_000 / CLOCK_HZ as i64;
    smoltcp::time::Instant::from_micros(ticks)
}

fn init(mut ctx: app::init::Context) -> (app::Shared, app::Local) {
    ctx.core.SCB.set_sleepdeep();

    #[cfg(feature = "ra6m3")]
    core::pin::Pin::static_mut(ctx.local.io_port)
        .as_mut()
        .open(&io_ports::BSP_PIN_CFG)
        .expect("Failed to open ioports");

    logger_setup::init();

    info!("Init start");
    info!("Size of tasks: {}b", ctx.executors_size);

    Mono::start(ctx.core.SYST, SYS_TICK_HZ); // How does this relate to CLOCK_HZ?

    #[cfg(feature = "ra6m3")]
    let (mut net, device, sockets) = init_network(
        ctx.device.EDMAC0,
        &mut ctx.core.NVIC,
        ctx.cs,
        ctx.local.sockets,
    );
    #[cfg(feature = "qemu")]
    let (mut net, device, sockets) = init_network(ctx.cs, ctx.local.sockets);
    let [mqtt_socket_handle, http_socket_handle] = sockets;

    let sink = ctx.local.net_waker.sink_ref();
    let net_poll_schedule_tx = sink.source_ref();

    let next_net_poll = net.iface.poll_at(smol_now(), &mut net.sockets);

    app::network_poll_scheduler::spawn(sink).ok();

    app::waiter::spawn().ok();
    app::mqtt_task::spawn(mqtt_socket_handle).ok();
    app::http_task::spawn(http_socket_handle).ok();
    #[cfg(feature = "ra6m3")]
    app::network_link_poll::spawn().ok();
    #[cfg(feature = "ra6m3")]
    app::blinky::spawn(ctx.device.PORT1, ctx.device.PORT4).ok();

    info!("Init done");

    (
        app::Shared {
            net,
            device,
            next_net_poll,
        },
        app::Local {
            net_poll_schedule_tx,
        },
    )
}

fn init_network(
    #[cfg(feature = "ra6m3")] etherc0: pac::EDMAC0,
    // fixme: maybe require &mut NVIC for eth open? It is using set_priority inside
    #[cfg(feature = "ra6m3")] _nvic: &mut cortex_m::peripheral::NVIC,
    cs: CriticalSection<'_>,
    storage: &'static mut SocketStorage,
) -> (Net, net_device::Dev, [SocketHandle; 2]) {
    #[cfg(feature = "ra6m3")]
    let mut device = net_device::Dev::new(cs, etherc0);
    #[cfg(not(feature = "ra6m3"))]
    let mut device = net_device::Dev::new(cs);
    let address = smoltcp::wire::EthernetAddress(MAC);
    let conf = Config::new(HardwareAddress::Ethernet(address));

    let mut iface = Interface::new(conf, &mut device, smol_now());
    let mut sockets = SocketSet::new(&mut storage.sockets[..]);

    let mut add_tcp_socket = |s: &'static mut TcpSocketStorage| -> SocketHandle {
        let rx = tcp::SocketBuffer::new(&mut s.rx_payload[..]);
        let tx = tcp::SocketBuffer::new(&mut s.tx_payload[..]);
        sockets.add(tcp::Socket::new(rx, tx))
    };

    let [mqtt, http, ..] = &mut storage.tcp_sockets;
    let mqtt = add_tcp_socket(mqtt);
    let http = add_tcp_socket(http);

    iface.update_ip_addrs(|ip_addrs| {
        trace!("Adding IP addresses");
        ip_addrs.push(IpCidr::new(IP_V4, IP_V4_NETMASK)).unwrap();
        #[cfg(feature = "ra6m3")]
        info!("IPV4: {IP_V4}/{IP_V4_NETMASK}");
        ip_addrs.push(IpCidr::new(IP_V6, IP_V6_NETMASK)).unwrap();
        #[cfg(feature = "ra6m3")]
        info!("IPV6: {IP_V6}");
        let loopback = wire::IpAddress::v6(0xfe80, 0, 0, 0, 0, 0, 0, 1);
        ip_addrs.push(IpCidr::new(loopback, 64)).unwrap();
    });

    iface
        .routes_mut()
        .add_default_ipv4_route(IP_V4_GATEWAY)
        .unwrap();
    iface
        .routes_mut()
        .add_default_ipv6_route(IP_V6_GATEWAY)
        .unwrap();

    (Net { iface, sockets }, device, [mqtt, http])
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

fn poll_network(mut ctx: app::poll_network::Context<'_>) {
    let net = &mut ctx.shared.net;
    let dev = &mut ctx.shared.device;
    let next_net_poll = &mut ctx.shared.next_net_poll;

    if !dev.lock(|dev| dev.is_up()) {
        return;
    }

    (&mut *net, &mut *dev).lock(|net, dev| net.iface.poll(smol_now(), dev, &mut net.sockets));

    (&mut *net, next_net_poll).lock(|net, next_net_poll| {
        *next_net_poll = net.iface.poll_at(smol_now(), &mut net.sockets)
    });
}

/// This task is responsible for delayed polling of the network stack.
/// It is used solely by `poll_network` to shcedule itself for `poll_at`.
async fn network_poll_scheduler(
    mut ctx: app::network_poll_scheduler::Context<'_>,
    mut sink: WakeSinkRef<'static>,
) {
    let mut delay = None;

    enum Event {
        Timeout(()),
        NewTimeout(smoltcp::time::Instant),
    }

    loop {
        let receiver = sink.wait_until(|| {
            ctx.shared
                .next_net_poll
                .lock(Option::take)
                .map(Event::NewTimeout)
        });
        let race_delay = async move {
            match delay {
                Some(delay) => Event::Timeout(delay.await),
                None => core::future::pending().await,
            }
        };
        match futures_lite::future::or(receiver, race_delay).await {
            Event::Timeout(()) => {
                POLL_NETWORK();
                delay = None
            }
            Event::NewTimeout(at) => {
                crate::info!(
                    "New Delay: {} micros (now is {} ticks)",
                    at.total_micros(),
                    Mono::now().ticks()
                );
                let total_micros = at.total_micros();
                let total_millis_round_up = ((total_micros + 999) / 1000) as u32;
                let next = Instant::from_ticks(total_millis_round_up);
                delay = Some(Mono::delay_until(next));
            }
        }
    }
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
            pub net: Net,
            #[unsafe(link_section = ".noinit")]
            pub device: net_device::Dev,
            #[unsafe(link_section = ".noinit")]
            pub next_net_poll: Option<smoltcp::time::Instant>,
        }

        #[local]
        pub struct Local {
            #[unsafe(link_section = ".noinit")]
            pub net_poll_schedule_tx: WakeSourceRef<'static>,
        }

        #[init(local = [
            sockets: SocketStorage = SocketStorage::new(),
            net_waker: DiatomicWaker = DiatomicWaker::new()
            io_port: IoPortInstance = IoPortInstance::new(),
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
        async fn populate_rx_buffers(mut ctx: populate_rx_buffers::Context, cause: InterruptCause) {
            ctx.shared
                .device
                .lock(|d| crate::net_device::populate_rx_buffers(d, cause));
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
