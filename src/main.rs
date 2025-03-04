#![no_main]
#![no_std]
#![feature(cell_update)]
#![feature(exclusive_wrapper)]

/**

https://github.com/nghiaducnt/LearningRIOT/blob/2019.01-my/cpu/cc2538/stellaris_ether/ethernet.c

todo:
- runnable on ra6m3
- http server
- tls v3
- real time measurements
*/
#[cfg(feature = "qemu")]
mod defmt_semihosting;
#[cfg(feature = "qemu")]
#[path = "net_semihosting.rs"]
mod net_device;

mod conf;
mod mqtt;
mod net;
#[cfg(feature = "ra6m3")]
mod net_device;
mod util;

use bare_metal::CriticalSection;
use conf::{
    CLOCK_HZ, IP_V4, IP_V4_GATEWAY, IP_V4_NETMASK, IP_V6, IP_V6_GATEWAY, IP_V6_NETMASK, MAC,
    MQTT_BROKER_IP, MQTT_BROKER_PORT, SYS_TICK_HZ,
};
use mqtt::Mqtt;
use rtic_monotonics::systick::prelude::*;
use smoltcp::{
    iface::{Config, Interface, SocketSet},
    socket::{tcp, udp},
    time::Instant,
    wire::{self, HardwareAddress, IpCidr},
};

use net::NetStorage;

defmt::timestamp!("{=usize}", {
    use core::sync::atomic::{AtomicUsize, Ordering};
    static COUNT: AtomicUsize = AtomicUsize::new(0);

    COUNT.fetch_add(1, Ordering::Relaxed)
});

// fixme: u32 overflow, as it is in milliseconds
systick_monotonic!(Mono, CLOCK_HZ);

pub struct Net {
    iface: Interface,
    sockets: SocketSet<'static>,
    mqtt: &'static mut Mqtt,
}

fn smol_now() -> Instant {
    let ticks = Mono::now().ticks() as i64 * 1_000_000 / CLOCK_HZ as i64;
    Instant::from_micros(ticks)
}

fn init_network(
    #[cfg(feature = "ra6m3")] etherc0: ra6m3::ETHERC0,
    cs: CriticalSection<'_>,
    storage: &'static mut NetStorage,
) -> (Net, net_device::Dev) {
    defmt::info!("Starting device at {}", Mono::now().ticks());

    let mut device = net_device::Dev::new(
        cs,
        #[cfg(feature = "ra6m3")]
        etherc0,
    );
    let address = smoltcp::wire::EthernetAddress(MAC);
    let conf = Config::new(HardwareAddress::Ethernet(address));

    let mut iface = Interface::new(conf, &mut device, smol_now());
    let mut sockets = SocketSet::new(&mut storage.sockets[..]);

    for s in storage.tcp_sockets.iter_mut() {
        let rx = tcp::SocketBuffer::new(&mut s.rx_payload[..]);
        let tx = tcp::SocketBuffer::new(&mut s.tx_payload[..]);
        sockets.add(tcp::Socket::new(rx, tx));
    }

    for s in storage.udp_sockets.iter_mut() {
        let rx = udp::PacketBuffer::new(&mut s.rx_metadata[..], &mut s.rx_payload[..]);
        let tx = udp::PacketBuffer::new(&mut s.tx_metadata[..], &mut s.tx_payload[..]);
        sockets.add(udp::Socket::new(rx, tx));
    }

    iface.update_ip_addrs(|ip_addrs| {
        ip_addrs.push(IpCidr::new(IP_V4, IP_V4_NETMASK)).unwrap();
        ip_addrs.push(IpCidr::new(IP_V6, IP_V6_NETMASK)).unwrap();
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

    let mqtt_storage = &mut storage.mqtt;
    let mqtt_socket_handle = sockets.iter().next().expect("Socket must be available").0;
    let conf = minimq::ConfigBuilder::new(
        mqtt::Broker(core::net::SocketAddr::from(core::net::SocketAddrV4::new(
            MQTT_BROKER_IP,
            MQTT_BROKER_PORT,
        ))),
        &mut mqtt_storage.buffer[..],
    );
    let conf = conf.keepalive_interval(30_000);
    let mqtt = mqtt_storage.mqtt.get_or_insert(Mqtt::new(
        &mut mqtt_storage.channel,
        mqtt_socket_handle,
        conf,
    ));

    (
        Net {
            iface,
            sockets,
            mqtt,
        },
        device,
    )
}

fn exit() -> ! {
    use cortex_m_semihosting::debug;

    defmt::info!("Exitter task");

    debug::exit(debug::EXIT_SUCCESS);

    cortex_m::asm::udf();
}

#[rtic::app(
  device = lm3s6965,
  // device = ra6m3,
  dispatchers = [GPIOA, GPIOB, GPIOC, GPIOD, GPIOE],
  peripherals = true
)]
mod app {
    use super::*;

    use rtic_sync::{
        channel::{Receiver, Sender},
        make_channel,
    };

    pub fn trigger_network_poll() {
        #[cfg(feature = "qemu")]
        rtic::pend(lm3s6965::Interrupt::ETHERNET);
        #[cfg(feature = "ra6m3")]
        todo!();
    }

    #[shared]
    struct Shared {
        net: Net,
        device: net_device::Dev,
    }

    #[local]
    struct Local {
        net_timeout_sender: Sender<'static, smoltcp::time::Instant, 1>,
    }

    #[init(local = [
        net_storage: NetStorage = NetStorage::new(),
    ])]
    fn init(mut ctx: init::Context) -> (Shared, Local) {
        ctx.core.SCB.set_sleepdeep();

        defmt::info!("Init start");
        defmt::info!("Size of tasks: {}b", ctx.executors_size);

        Mono::start(ctx.core.SYST, SYS_TICK_HZ); // How does this relate to CLOCK_HZ?

        #[cfg(feature = "ra6m3")]
        let (net, device) = init_network(ctx.device.ETHERC0, ctx.local.net_storage);
        #[cfg(feature = "qemu")]
        let (net, device) = init_network(ctx.cs, ctx.local.net_storage);

        let (net_timeout_sender, receiver) = make_channel!(smoltcp::time::Instant, 1);

        network_poll_waiter::spawn(receiver).ok();

        defmt::info!("Network initialized");

        waiter::spawn().ok();
        mqtt::spawn().ok();

        defmt::info!("Init done");

        (Shared { net, device }, Local { net_timeout_sender })
    }

    #[task(priority = 3)]
    async fn waiter(_: waiter::Context) {
        let mut next = Mono::now();

        defmt::info!("Waiter task: 1 {}", Mono::now().ticks());

        next += 1000.millis();
        Mono::delay_until(next).await;

        defmt::info!("Waiter task: 2 {}", Mono::now().ticks());

        next += 1000.millis();
        Mono::delay_until(next).await;

        defmt::info!("Waiter task: 3 {}", Mono::now().ticks());

        core::future::pending::<()>().await;

        exit();
    }

    #[task(priority = 1, shared = [net])]
    async fn mqtt(ctx: mqtt::Context) {
        match crate::mqtt::mqtt(ctx).await {};
    }

    #[task(binds = ETHERNET, priority = 1, shared = [device])]
    fn ethernet_isr(mut ctx: ethernet_isr::Context) {
        let cause = ctx.shared.device.lock(|device| {
            net_device::isr_handler(device) //
        });
        if cause == Some(net_device::InterruptCause::Receive) {
            poll_network::spawn().ok();
        }
    }

    #[task(priority = 1)]
    async fn network_poll_waiter(
        _ctx: network_poll_waiter::Context,
        mut receiver: Receiver<'static, smoltcp::time::Instant, 1>,
    ) {
        let mut delay = None;

        enum Event {
            Timeout(()),
            NewTimeout(smoltcp::time::Instant),
        }

        loop {
            let race_delay = async move {
                match delay {
                    Some(delay) => Event::Timeout(delay.await),
                    None => core::future::pending().await,
                }
            };
            let receiver = async { Event::NewTimeout(receiver.recv().await.unwrap()) };
            match futures_lite::future::or(race_delay, receiver).await {
                Event::Timeout(()) => {
                    poll_network::spawn().ok();
                    delay = None
                }
                Event::NewTimeout(at) => {
                    let next = fugit::Instant::<u32, 1, 1000>::from_ticks(at.millis() as u32);
                    delay = Some(Mono::delay_until(next));
                }
            }
        }
    }

    #[task(priority = 1, shared = [net, device], local = [net_timeout_sender])]
    async fn poll_network(mut ctx: poll_network::Context) {
        let net = &mut ctx.shared.net;
        let dev = &mut ctx.shared.device;

        loop {
            match (&mut *net, &mut *dev).lock(|mut net, device| {
                let Net { sockets, iface, .. } = &mut net;

                iface.poll(smol_now(), device, sockets)
            }) {
                smoltcp::iface::PollResult::None => break,
                smoltcp::iface::PollResult::SocketStateChanged => continue,
            };
        }

        let poll_at = net.lock(|mut net| {
            let Net { sockets, iface, .. } = &mut net;

            iface.poll_at(smol_now(), sockets)
        });

        if let Some(poll_at) = poll_at {
            ctx.local.net_timeout_sender.try_send(poll_at).ok();
        }
    }

    #[idle]
    fn idle(_: idle::Context) -> ! {
        defmt::info!("Idle start");
        loop {
            rtic::export::wfi();
        }
    }
}
