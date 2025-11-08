use core::{
    future::poll_fn,
    sync::atomic::{AtomicBool, Ordering},
    task::Poll,
};

use ra_fsp_rs::pac;
use rtic::export::CriticalSection;
use rtic::mutex_prelude::*;
use rtic_monotonics::Monotonic;
use smoltcp::{
    iface::{Config, Interface, SocketHandle, SocketSet},
    socket::tcp,
    wire::{self, HardwareAddress, IpCidr},
};

use crate::{
    Duration, Instant, Mono,
    app::network_poller,
    conf::{IP_V4, IP_V4_GATEWAY, IP_V4_NETMASK, IP_V6, IP_V6_GATEWAY, IP_V6_NETMASK, MAC},
    info, net_device,
    socket_storage::{SocketStorage, TcpSocketStorage},
    trace,
};

pub struct Net {
    pub iface: Interface,
    pub sockets: SocketSet<'static>,
}

static NETWORK_POLL_REQUEST: AtomicBool = AtomicBool::new(false);

pub fn request_network_poll() {
    NETWORK_POLL_REQUEST.store(true, Ordering::Release);
    crate::app::network_poller::waker().wake();
}

fn wait_until_network_poll_requested() -> impl Future<Output = ()> {
    poll_fn(|_cx| {
        if NETWORK_POLL_REQUEST.swap(false, Ordering::Acquire) {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    })
}

fn smol_now() -> smoltcp::time::Instant {
    let duration = Mono::now().duration_since_epoch();
    let micros = duration.to_micros() as i64;
    smoltcp::time::Instant::from_micros(micros)
}

pub fn init_network(
    cs: CriticalSection<'_>,
    // fixme: maybe require &mut NVIC for eth open? It is using set_priority inside
    nvic: &mut cortex_m::peripheral::NVIC,
    edmac: pac::EDMAC0,
    ether0: pac::ETHERC0,
    storage: &'static mut SocketStorage,
) -> (Net, net_device::Dev, [SocketHandle; 2]) {
    let mut device = net_device::create_dev(cs, nvic, edmac, ether0);
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
        info!("IPV4: {IP_V4}/{IP_V4_NETMASK}");
        ip_addrs.push(IpCidr::new(IP_V6, IP_V6_NETMASK)).unwrap();
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

async fn wait_until(next_poll_at: &mut Option<Instant>) {
    match *next_poll_at {
        Some(at) => {
            Mono::delay_until(at).await;
            *next_poll_at = None;
        }
        None => core::future::pending().await,
    }
}

#[inline(always)]
pub async fn network_poller_task(mut ctx: network_poller::Context<'static>) -> ! {
    let mut next_poll_at = None;
    loop {
        let poll_at = wait_until(&mut next_poll_at);
        let requested = wait_until_network_poll_requested();

        futures_lite::future::or(requested, poll_at).await;

        let prev_delay = next_poll_at;
        next_poll_at = poll_network(&mut ctx);

        if let Some(at) = next_poll_at
            && prev_delay != prev_delay
        {
            crate::info!("New Delay: {} (now is {})", at, Mono::now());
        }
    }
}

#[inline(always)]
fn poll_network(ctx: &mut network_poller::Context<'_>) -> Option<Instant> {
    let net = &mut ctx.shared.net;
    let dev = &mut ctx.shared.device;

    // This is important because smoltcp will give `poll_at` at `now` if device is down,
    // causing an infinite loop of polling.
    if !dev.lock(|dev| dev.is_up()) {
        return None;
    }

    (&mut *net, &mut *dev).lock(|net, dev| net.iface.poll(smol_now(), dev, &mut net.sockets));

    let poll_at = (&mut *net).lock(|net| net.iface.poll_at(smol_now(), &mut net.sockets));

    poll_at.map(|at| {
        let micros = at.total_micros() & (crate::Ticks::MAX as i64);
        let duration = Duration::micros_at_least(micros as _);
        Instant::from_ticks(duration.ticks())
    })
}
