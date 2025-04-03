#[cfg(feature = "tls")]
/*
TOOD TLS:

- setup rumqttd with tls
- connect to it via mqttx
- wrap stream in tls

#[cfg(feature = "tls")]
use embedded_tls::{TlsConfig, TlsContext, UnsecureProvider};
#[cfg(feature = "tls")]
use tls_socket::{Rng, TlsSocket};

// const TLS_TX_SIZE: usize = 16_640;
// const TLS_RX_SIZE: usize = 16_640;
const TLS_TX_SIZE: usize = 13_640;
const TLS_RX_SIZE: usize = 16_640;

    pub tls_tx: [u8; TLS_TX_SIZE],
    pub tls_rx: [u8; TLS_RX_SIZE],

#[cfg(feature = "tls")]
let socket = {
    let tls_config = TlsConfig::new().with_server_name("example.com");
    let mut socket = TlsSocket::new(socket, &mut storage.tls_rx, &mut storage.tls_tx);
    let mut rng = Rng;
    let tls_ctx = TlsContext::new(&tls_config, UnsecureProvider::new(&mut rng));
    socket.open(tls_ctx).await.unwrap();

    socket
};

*/

const CAP: usize = 8;

use core::{
    future::poll_fn,
    net::SocketAddr,
    pin::{Pin, pin},
    task::{Context, Poll, Waker},
};

use embedded_nal::{TcpClientStack, TcpError, nb};
use embedded_tls::{TlsContext, UnsecureProvider};
use heapless::spsc::{Consumer, Producer};
use rtic_monotonics::{
    Monotonic,
    fugit::{ExtU32, Instant},
};
use smoltcp::{iface::SocketHandle, socket::tcp};

use crate::{
    Mono,
    poll_share::TokenProvider,
    socket::{self, TcpSocket},
};

use super::{
    NetLock,
    tls_socket::{Rng, TlsSocket},
};

const MQTT_CLIENT_PORT: u16 = 58737;
const RECONNECT_INTERVAL_MS: u32 = 2_000;

// type ConnectFut = impl Future<Output = (TcpSocket<NetLock>, Result<(), socket::ConnectError>)>;
type ConnectFut =
    stackfuture::StackFuture<'static, (TcpSocket<NetLock>, Result<(), socket::ConnectError>), 120>;

pub type WaiterFut = impl Future<Output = ()> + 'static;

pub struct TlsArgs<'a> {
    pub tls_rx: &'a mut [u8],
    pub tls_tx: &'a mut [u8],
    pub config: embedded_tls::TlsConfig<'a>,
}

pub struct Broker(pub SocketAddr);

pub struct MqttAlocation {
    connect_future_place: Option<ConnectFut>,
    waiter_future_place: Option<WaiterFut>,
}

pub struct EmbeddedNalAdapter {
    last_connection_attempt: Option<Instant<u32, 1, 1000>>,
    port_shift: core::num::Wrapping<u8>,
    socket_handle: Option<SocketHandle>,
    net: TokenProvider<NetLock>,
    waker: Waker,
    pending_close: bool,
    connect_future: Pin<&'static mut Option<ConnectFut>>,
    waiter_future: Pin<&'static mut Option<WaiterFut>>,
    socket: Option<TcpSocket<NetLock>>,
    tls: Option<TlsArgs<'static>>,
}

#[derive(PartialEq, Debug, defmt::Format)]
pub enum NetError {
    SocketUsed,
    PipeClosed,
    ConnectError(socket::ConnectError),
    SendError(socket::Error),
    RecvError(socket::Error),
}

enum AdapterMessageIn {
    Connect(SocketHandle, SocketAddr),
    Close(SocketHandle),
    Socket,
    WithConnection(fn(Option<&mut TcpSocket<NetLock>>, Option<&mut TlsSocket<'_, NetLock>>)),
}

enum AdapterMessageOut {
    Connected,
    Socket(Option<()>),
}

impl minimq::Broker for Broker {
    fn get_address(&mut self) -> Option<SocketAddr> {
        Some(self.0)
    }

    fn set_port(&mut self, port: u16) {
        self.0.set_port(port);
    }
}

async fn handle_tls_session(
    net: TokenProvider<NetLock>,
    handle: SocketHandle,
    tls: &mut TlsArgs<'static>,
    rx: &mut Consumer<'static, AdapterMessageIn, CAP>,
    tx: &mut Producer<'static, AdapterMessageOut, CAP>,
) {
    let mut rng = Rng;
    let tcp_socket = TcpSocket::new(net, handle);
    let ctx = TlsContext::new(&tls.config, UnsecureProvider::new(&mut rng));
    let mut tls_socket = TlsSocket::new(tcp_socket, tls.tls_rx, tls.tls_tx);

    tls_socket.open(ctx).await.unwrap();

    tx.enqueue(AdapterMessageOut::Connected);

    loop {
        match dequeue(rx).await {
            AdapterMessageIn::Connect(_, _) => {}
            AdapterMessageIn::Close(_) => {
                tls_socket.close().await.unwrap();
                return;
            }
            AdapterMessageIn::WithConnection(f) => f(None, Some(&mut tls_socket)),
            AdapterMessageIn::Socket => _ = tx.enqueue(AdapterMessageOut::Socket(None)),
        }
    }
}

async fn dequeue(rx: &mut Consumer<'static, AdapterMessageIn, CAP>) -> AdapterMessageIn {
    poll_fn(|_| match rx.dequeue() {
        Some(v) => Poll::Ready(v),
        None => Poll::Pending,
    })
    .await
}

async fn adapter_task(
    net: TokenProvider<NetLock>,
    handle: SocketHandle,
    mut tls: Option<TlsArgs<'static>>,
    mut rx: Consumer<'static, AdapterMessageIn, CAP>,
    mut tx: Producer<'static, AdapterMessageOut, CAP>,
) -> ! {
    let mut connected = false;
    let mut tcp_socket = TcpSocket::new(net, handle);
    let mut port_shift = core::num::Wrapping::<u8>(0);
    let mut last_connection_attempt: Option<Instant<u32, 1, 1000>> = None;

    loop {
        match dequeue(&mut rx).await {
            AdapterMessageIn::Connect(_, socket_addr) => {
                if let Some(last) = last_connection_attempt {
                    if let Some(diff) = Mono::now().checked_duration_since(last) {
                        if diff < RECONNECT_INTERVAL_MS.millis::<1, 1000>() {
                            Mono::delay_until(last + RECONNECT_INTERVAL_MS.millis()).await;
                        }
                    }
                }

                last_connection_attempt = Some(Mono::now());
                let port = MQTT_CLIENT_PORT + port_shift.0 as u16;
                tcp_socket.connect(socket_addr, port).await;

                if let Some(tls) = tls.as_mut() {
                    handle_tls_session(net, handle, tls, &mut rx, &mut tx).await;
                } else {
                    connected = true;
                    tx.enqueue(AdapterMessageOut::Connected);
                }

                port_shift += 1;
            }
            AdapterMessageIn::WithConnection(f) if connected => f(Some(&mut tcp_socket), None),
            AdapterMessageIn::WithConnection(f) => f(None, None),
            AdapterMessageIn::Close(_socket_handle) => {
                tcp_socket.disconnect().await;
                connected = false;
            }
            AdapterMessageIn::Socket => {
                _ = tx.enqueue(AdapterMessageOut::Socket((!connected).then_some(())));
            }
        }
    }
}

impl EmbeddedNalAdapter {
    pub const fn new(
        net: TokenProvider<NetLock>,
        handle: SocketHandle,
        alloc: &'static mut MqttAlocation,
        waker: Waker,
        tls: Option<TlsArgs<'static>>,
    ) -> Self {
        let socket = Some(TcpSocket::new(net, handle));
        let connect_future = Pin::static_mut(&mut alloc.connect_future_place);
        let waiter_future = Pin::static_mut(&mut alloc.waiter_future_place);

        Self {
            last_connection_attempt: None,
            port_shift: core::num::Wrapping(0),
            socket_handle: Some(handle),
            pending_close: false,
            net,
            waker,
            connect_future,
            waiter_future,
            socket,
            tls,
        }
    }

    fn setup_wakers(&mut self, handle: SocketHandle) {
        self.net.lock(|net| {
            let socket = net.sockets.get_mut::<tcp::Socket>(handle);
            socket.register_recv_waker(&self.waker);
            socket.register_send_waker(&self.waker);
        });
    }

    fn should_try_connect(&mut self) -> bool {
        let now = Mono::now();

        if let Some(last) = self.last_connection_attempt {
            if let Some(diff) = now.checked_duration_since(last) {
                if diff < RECONNECT_INTERVAL_MS.millis::<1, 1000>() {
                    self.last_connection_attempt = Some(now);
                    let poll_at = last + RECONNECT_INTERVAL_MS.millis::<1, 1000>();

                    // This is not async function and is not pinned, so let the
                    // global state be our `Pin<&mut Self>`.
                    setup_waiter(poll_at, &self.waker, &mut self.waiter_future);

                    return false;
                }
            }
        }

        true
    }
}

impl TcpClientStack for EmbeddedNalAdapter {
    type TcpSocket = SocketHandle;

    type Error = NetError;

    fn socket(&mut self) -> Result<Self::TcpSocket, Self::Error> {
        Ok(self.socket_handle.take().ok_or(NetError::SocketUsed)?)
    }

    // note: this path is called x3-x5 times for some reason. Who wakes mqtt task so much?
    // #[define_opaque(ConnectFut)]
    fn connect(
        &mut self,
        _handle: &mut Self::TcpSocket,
        remote: core::net::SocketAddr,
    ) -> nb::Result<(), Self::Error> {
        defmt::debug!("Connecting to {:?}", remote);
        if let Some(mut fut) = self.connect_future.as_mut().as_pin_mut() {
            let mut cx = Context::from_waker(&self.waker);
            return match fut.as_mut().poll(&mut cx) {
                Poll::Ready((socket, v)) => {
                    self.socket = Some(socket);
                    self.last_connection_attempt = Some(Mono::now());
                    self.connect_future.set(None);
                    self.pending_close = false;
                    v.map_err(NetError::ConnectError).map_err(nb::Error::Other)
                }
                Poll::Pending => Err(nb::Error::WouldBlock),
            };
        }

        let port = MQTT_CLIENT_PORT + self.port_shift.0 as u16;
        self.port_shift += 1;

        if self.should_try_connect() {
            if let Some(socket) = self.socket.take() {
                let pending_close = self.pending_close;
                let fut = async move {
                    let mut socket = socket;

                    if pending_close {
                        socket.disconnect().await;
                    }

                    // https://docs.rs/embedded-tls/latest/embedded_tls/struct.TlsConnection.html#method.open
                    let res = socket.connect(remote, port).await;
                    (socket, res)
                };

                // self.connect_future.set(Some(fut));
                self.connect_future
                    .set(Some(stackfuture::StackFuture::from(fut)));
            }
        }

        Err(nb::Error::WouldBlock)
    }

    fn send(
        &mut self,
        _handle: &mut Self::TcpSocket,
        buffer: &[u8],
    ) -> nb::Result<usize, Self::Error> {
        if buffer.len() == 0 {
            return Ok(0);
        }

        // cannot send during connect
        if let Some(socket) = self.socket.as_mut() {
            // https://docs.rs/embedded-tls/latest/embedded_tls/struct.TlsConnection.html#method.write
            // https://docs.rs/embedded-tls/latest/embedded_tls/struct.TlsConnection.html#method.flush
            let fut = pin!(socket.write(buffer));
            let mut ctx = Context::from_waker(&self.waker);
            match fut.poll(&mut ctx) {
                Poll::Ready(Ok(len)) => Ok(len),
                Poll::Ready(Err(e)) => Err(nb::Error::Other(NetError::SendError(e))),
                Poll::Pending => Err(nb::Error::WouldBlock),
            }
        } else {
            Err(nb::Error::WouldBlock)
        }
    }

    fn receive(
        &mut self,
        &mut handle: &mut Self::TcpSocket,
        buffer: &mut [u8],
    ) -> nb::Result<usize, Self::Error> {
        if buffer.len() == 0 {
            return Ok(0);
        }

        // cannot receive during connect
        if let Some(socket) = self.socket.as_mut() {
            match {
                // https://docs.rs/embedded-tls/latest/embedded_tls/struct.TlsConnection.html#method.read
                let fut = pin!(socket.read(buffer));
                fut.poll(&mut Context::from_waker(&self.waker))
            } {
                Poll::Ready(Ok(0)) => Err(nb::Error::Other(NetError::PipeClosed)),
                Poll::Ready(Ok(len)) => {
                    self.setup_wakers(handle);
                    Ok(len)
                }
                Poll::Ready(Err(e)) => Err(nb::Error::Other(NetError::RecvError(e))),
                Poll::Pending => Err(nb::Error::WouldBlock),
            }
        } else {
            Err(nb::Error::WouldBlock)
        }
    }

    fn close(&mut self, handle: Self::TcpSocket) -> Result<(), Self::Error> {
        defmt::info!("Closing socket");
        self.pending_close = true;
        self.socket_handle = Some(handle);
        Ok(())
    }
}

impl TcpError for NetError {
    fn kind(&self) -> embedded_nal::TcpErrorKind {
        match *self {
            Self::SocketUsed => embedded_nal::TcpErrorKind::Other,
            Self::PipeClosed
            | Self::SendError(socket::Error::ConnectionReset)
            | Self::RecvError(socket::Error::ConnectionReset)
            | Self::ConnectError(_) => embedded_nal::TcpErrorKind::PipeClosed,
        }
    }
}

impl MqttAlocation {
    pub const fn new() -> Self {
        Self {
            connect_future_place: None,
            waiter_future_place: None,
        }
    }
}

#[define_opaque(WaiterFut)]
pub fn setup_waiter(
    mut at: Instant<u32, 1, 1000>,
    waker: &Waker,
    place: &mut Pin<&'static mut Option<WaiterFut>>,
) {
    let mut cx = Context::from_waker(waker);

    loop {
        let fut = Mono::delay_until(at);

        match match place.as_mut().as_pin_mut() {
            Some(mut place) => {
                place.set(fut);
                place.as_mut().poll(&mut cx)
            }
            None => {
                place.set(Some(fut));
                place.as_mut().as_pin_mut().unwrap().poll(&mut cx)
            }
        } {
            Poll::Pending => return,
            Poll::Ready(()) => {
                place.set(None);
                at += 1000.millis();
                continue;
            }
        }
    }
}
