use core::{
    cell::Cell,
    future::poll_fn,
    net::SocketAddr,
    pin::{Pin, pin},
    task::{Context, Poll, Waker},
};

use embedded_nal::{TcpClientStack, TcpError, nb};
#[cfg(feature = "tls")]
use embedded_tls::{TlsContext, TlsError, UnsecureProvider};
use rtic_monotonics::Monotonic;
use smoltcp::{iface::SocketHandle, socket::tcp};

use crate::{
    Instant, Mono, TimeExt,
    log::*,
    poll_share::TokenProvider,
    socket::{self, TcpSocket},
    util::ExtendRefGuard,
};

use super::NetLock;

#[cfg(feature = "tls")]
use super::tls_socket::TlsSocket;

const MQTT_CLIENT_PORT: u16 = 58026;
const RECONNECT_INTERVAL_MS: crate::Ticks = 2_000;

type AdapterFut = impl Future<Output = !>;

#[cfg(feature = "tls")]
pub struct TlsArgs<'a> {
    pub tls_rx: &'a mut [u8],
    pub tls_tx: &'a mut [u8],
    pub config: embedded_tls::TlsConfig<'a>,
}
#[cfg(not(feature = "tls"))]
pub struct TlsArgs<'a>(core::marker::PhantomData<&'a ()>, !);

pub struct Broker(pub SocketAddr);

pub struct MqttAlocation {
    future_place: Option<AdapterFut>,
    tx_queue: Cell<Option<AdapterMessageIn>>,
    rx_queue: Cell<Option<AdapterMessageOut>>,
    waiting_for_message: Cell<bool>,
}

pub struct EmbeddedNalAdapter {
    socket_handle: SocketHandle,
    net: TokenProvider<NetLock>,
    waker: Waker,
    remote: Option<SocketAddr>,
    fut: Pin<&'static mut AdapterFut>,
    tx: &'static Cell<Option<AdapterMessageIn>>,
    rx: &'static Cell<Option<AdapterMessageOut>>,
    waiting_for_message: &'static Cell<bool>,
}

#[derive(Debug, strum::Display)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Error {
    TcpError(socket::Error),
    #[cfg(feature = "tls")]
    TlsError(TlsError),
}

#[derive(Debug, strum::Display)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum NetError {
    PipeClosed,
    TcpConnectError(socket::ConnectError),
    #[cfg(feature = "tls")]
    TlsConnectError(TlsError),
    SendError(Error),
    RecvError(Error),
}

#[derive(Debug, PartialEq)]
enum AdapterMessageIn {
    Connect(SocketAddr),
    Read(&'static mut [u8]),
    Write(ExtendRefGuard<[u8]>),
    Close,
}

#[derive(Debug, PartialEq)]
enum AdapterMessageOut {
    Connect(Poll<Result<(), NetError>>),
    Close(Poll<Result<(), NetError>>),
    Read(Option<Result<usize, Error>>, &'static mut [u8]),
    Write(Option<Result<usize, Error>>, ExtendRefGuard<[u8]>),
}

impl minimq::Broker for Broker {
    fn get_address(&mut self) -> Option<SocketAddr> {
        Some(self.0)
    }

    fn set_port(&mut self, port: u16) {
        self.0.set_port(port);
    }
}

async fn dequeue(rx: &Cell<Option<AdapterMessageIn>>, waiting: &Cell<bool>) -> AdapterMessageIn {
    poll_fn(|_| match rx.take() {
        Some(v) => Poll::Ready(v),
        None => {
            waiting.set(true);
            Poll::Pending
        }
    })
    .await
}

#[cfg(feature = "tls")]
async fn handle_tls_session(
    net: TokenProvider<NetLock>,
    handle: SocketHandle,
    tls: &mut TlsArgs<'static>,
    rx: &'static Cell<Option<AdapterMessageIn>>,
    tx: &'static Cell<Option<AdapterMessageOut>>,
    waiting_for_message: &Cell<bool>,
) {
    use rand_chacha::rand_core::SeedableRng;

    let mut rng = rand_chacha::ChaCha12Rng::from_seed([183; 32]);
    let tcp_socket = TcpSocket::new(net, handle);
    let ctx = TlsContext::new(&tls.config, UnsecureProvider::new(&mut rng));
    let mut tls_socket = TlsSocket::new(tcp_socket, tls.tls_rx, tls.tls_tx);

    let result = tls_socket
        .open(ctx)
        .await
        .map_err(NetError::TlsConnectError);

    let is_err = result.is_err();

    tx.set(Some(AdapterMessageOut::Connect(Poll::Ready(result))));

    poll_fn(|cx| Poll::Ready(cx.waker().wake_by_ref())).await;

    if is_err {
        return;
    }

    loop {
        match dequeue(rx, waiting_for_message).await {
            AdapterMessageIn::Connect(_) => unreachable!("Cannot connect twice"),
            AdapterMessageIn::Read(buffer) => {
                let result = match {
                    let mut fut = pin!(tls_socket.0.read(buffer));
                    poll_fn(|cx| Poll::Ready(fut.as_mut().poll(cx))).await
                } {
                    Poll::Ready(result) => Some(result.map_err(Error::TlsError)),
                    Poll::Pending => None,
                };
                tx.set(Some(AdapterMessageOut::Read(result, buffer)));
            }
            AdapterMessageIn::Write(buffer) => {
                let result = match {
                    let mut fut = pin!(tls_socket.0.write(&*buffer));
                    poll_fn(|cx| Poll::Ready(fut.as_mut().poll(cx))).await
                } {
                    Poll::Ready(result) => Some(result.map_err(Error::TlsError)),
                    Poll::Pending => None,
                };
                let mut flush_fut = pin!(tls_socket.0.flush());
                _ = poll_fn(|cx| Poll::Ready(flush_fut.as_mut().poll(cx))).await;
                tx.set(Some(AdapterMessageOut::Write(result, buffer)));
            }
            AdapterMessageIn::Close => {
                tx.set(Some(AdapterMessageOut::Close(Poll::Pending)));

                let res = match tls_socket.close().await {
                    Ok(()) => Ok(()),
                    Err(_err) => {
                        crate::log::error!("TLS close error");
                        let mut tcp_socket = TcpSocket::new(net, handle);
                        tcp_socket.abort();
                        Err(NetError::PipeClosed)
                    }
                };
                poll_fn(|cx| Poll::Ready(cx.waker().wake_by_ref())).await;
                tx.set(Some(AdapterMessageOut::Close(Poll::Ready(res))));
                return;
            }
        }
    }
}

async fn adapter_task(
    net: TokenProvider<NetLock>,
    handle: SocketHandle,
    #[cfg(feature = "tls")] mut tls: Option<TlsArgs<'static>>,
    rx: &'static Cell<Option<AdapterMessageIn>>,
    tx: &'static Cell<Option<AdapterMessageOut>>,
    waiting_for_message: &Cell<bool>,
) -> ! {
    let mut connected = false;
    let mut tcp_socket = TcpSocket::new(net, handle);
    // fixme: maybe we do not need keep alives here?
    tcp_socket.set_timeout(Some(smoltcp::time::Duration::from_secs(3600)));
    tcp_socket.set_keep_alive(Some(smoltcp::time::Duration::from_secs(3600 / 3)));
    let mut port_shift = core::num::Wrapping::<u8>(0);
    let mut last_connection_attempt: Option<Instant> = None;

    loop {
        match dequeue(rx, waiting_for_message).await {
            // AdapterMessageIn::Connect(_) if connected => unreachable!("Cannot connect twice"),
            AdapterMessageIn::Connect(socket_addr) => {
                tx.set(Some(AdapterMessageOut::Connect(Poll::Pending)));

                if let Some(last) = last_connection_attempt {
                    if let Some(diff) = Mono::now().checked_duration_since(last) {
                        if diff < RECONNECT_INTERVAL_MS.millis::<1, 1000>() {
                            Mono::delay_until(last + RECONNECT_INTERVAL_MS.millis()).await;
                        }
                    }
                }

                last_connection_attempt = Some(Mono::now());
                let port = MQTT_CLIENT_PORT + port_shift.0 as u16;

                port_shift += 1;

                if connected {
                    // may happen when broker sends rst to us. In that case no need to handshake close
                    tcp_socket.abort();
                }

                let result = tcp_socket
                    .connect(socket_addr, port)
                    .await
                    .map_err(NetError::TcpConnectError);

                if result.is_ok() {
                    poll_fn(|cx| Poll::Ready(cx.waker().wake_by_ref())).await;
                }

                #[cfg(feature = "tls")]
                if let Some(tls) = tls.as_mut() {
                    handle_tls_session(net, handle, tls, rx, tx, waiting_for_message).await;
                } else {
                    connected = result.is_ok();
                    tx.set(Some(AdapterMessageOut::Connect(Poll::Ready(result))));
                }
                #[cfg(not(feature = "tls"))]
                {
                    connected = result.is_ok();
                    tx.set(Some(AdapterMessageOut::Connect(Poll::Ready(result))));
                }
            }
            AdapterMessageIn::Read(buffer) => {
                let result = match {
                    let mut read_fut = pin!(tcp_socket.read(buffer));
                    poll_fn(|cx| Poll::Ready(read_fut.as_mut().poll(cx))).await
                } {
                    Poll::Ready(result) => Some(result.map_err(Error::TcpError)),
                    Poll::Pending => None,
                };
                tx.set(Some(AdapterMessageOut::Read(result, buffer)));
            }
            AdapterMessageIn::Write(buffer) => {
                let result = match {
                    let mut read_fut = pin!(tcp_socket.write(&*buffer));
                    poll_fn(|cx| Poll::Ready(read_fut.as_mut().poll(cx))).await
                } {
                    Poll::Ready(result) => Some(result.map_err(Error::TcpError)),
                    Poll::Pending => None,
                };
                tx.set(Some(AdapterMessageOut::Write(result, buffer)));
            }
            AdapterMessageIn::Close => {
                tx.set(Some(AdapterMessageOut::Close(Poll::Pending)));
                tcp_socket.disconnect().await;
                poll_fn(|cx| Poll::Ready(cx.waker().wake_by_ref())).await;
                tx.set(Some(AdapterMessageOut::Close(Poll::Ready(Ok(())))));
                connected = false;
            }
        }
    }
}

impl EmbeddedNalAdapter {
    #[define_opaque(AdapterFut)]
    pub fn new(
        net: TokenProvider<NetLock>,
        handle: SocketHandle,
        alloc: &'static mut MqttAlocation,
        waker: Waker,
        #[cfg(feature = "tls")] tls: Option<TlsArgs<'static>>,
    ) -> Self {
        let tx_tx @ tx_rx = &alloc.tx_queue;
        let rx_tx @ rx_rx = &alloc.rx_queue;
        let waiting_for_message = &alloc.waiting_for_message;

        let fut = adapter_task(
            net,
            handle,
            #[cfg(feature = "tls")]
            tls,
            tx_rx,
            rx_tx,
            waiting_for_message,
        );
        let fut = Pin::static_mut(alloc.future_place.get_or_insert(fut));

        Self {
            socket_handle: handle,
            net,
            waker,
            remote: None,
            tx: tx_tx,
            rx: rx_rx,
            fut,
            waiting_for_message,
        }
    }

    fn poll(&mut self) {
        let mut ctx = Context::from_waker(&self.waker);
        Poll::Pending = self.fut.as_mut().poll(&mut ctx);
    }

    fn setup_wakers(&mut self, handle: SocketHandle) {
        self.net.lock(|net| {
            let socket = net.sockets.get_mut::<tcp::Socket>(handle);
            socket.register_recv_waker(&self.waker);
            socket.register_send_waker(&self.waker);
        });
    }

    fn message(&mut self, msg: AdapterMessageIn) -> Result<AdapterMessageOut, AdapterMessageIn> {
        self.waiting_for_message.set(false);

        self.poll();

        match (self.tx.take(), self.rx.take()) {
            (None, None) if self.waiting_for_message.get() => {
                self.tx.set(Some(msg));
                self.poll();
                Ok(self.rx.take().unwrap())
            }

            (None, Some(ready @ AdapterMessageOut::Connect(Poll::Ready(_)))) => {
                if msg == AdapterMessageIn::Connect(self.remote.unwrap()) {
                    Ok(ready)
                } else {
                    unreachable!();
                }
            }
            (None, conn @ Some(AdapterMessageOut::Connect(Poll::Pending))) => {
                // we are still connecting
                self.rx.set(conn);
                Err(msg)
            }
            (None, close @ Some(AdapterMessageOut::Close(Poll::Pending))) => {
                // we are still closing
                self.rx.set(close);
                Err(msg)
            }
            (None, Some(AdapterMessageOut::Close(Poll::Ready(_)))) => {
                // ignore close status, loop.
                self.message(msg)
            }
            (None, None) => Err(msg),
            c => unreachable!("{:?}", c),
        }
    }
}

impl TcpClientStack for EmbeddedNalAdapter {
    type TcpSocket = SocketHandle;

    type Error = NetError;

    fn socket(&mut self) -> Result<Self::TcpSocket, Self::Error> {
        Ok(self.socket_handle)
    }

    fn connect(
        &mut self,
        _handle: &mut Self::TcpSocket,
        remote: core::net::SocketAddr,
    ) -> nb::Result<(), Self::Error> {
        trace!("Connect to MQTT TCP socket");
        let old_remote = *self.remote.get_or_insert(remote);

        assert_eq!(
            old_remote, remote,
            "We assume that we connect to the same address in order to not deal with connection cancelling"
        );

        match self.message(AdapterMessageIn::Connect(remote)) {
            Ok(AdapterMessageOut::Connect(Poll::Ready(res))) => res.map_err(nb::Error::Other),
            _ => Err(nb::Error::WouldBlock),
        }
    }

    fn send(
        &mut self,
        &mut handle: &mut Self::TcpSocket,
        buffer: &[u8],
    ) -> nb::Result<usize, Self::Error> {
        if buffer.len() == 0 {
            return Ok(0);
        }

        crate::util::extend_ref(buffer, |guard| {
            let message = match self.message(AdapterMessageIn::Write(guard)) {
                Ok(message) => message,
                Err(AdapterMessageIn::Write(guard)) => return (guard, Err(nb::Error::WouldBlock)),
                Err(_) => unreachable!(),
            };

            self.setup_wakers(handle);

            match message {
                AdapterMessageOut::Write(Some(Ok(len)), b) => (b, Ok(len)),
                AdapterMessageOut::Write(Some(Err(err)), b) => {
                    (b, Err(nb::Error::Other(NetError::SendError(err))))
                }
                AdapterMessageOut::Write(None, b) => (b, Err(nb::Error::WouldBlock)),
                _ => unreachable!(),
            }
        })
    }

    fn receive(
        &mut self,
        &mut handle: &mut Self::TcpSocket,
        buffer: &mut [u8],
    ) -> nb::Result<usize, Self::Error> {
        if buffer.len() == 0 {
            return Ok(0);
        }

        extend_mut::extend_mut(buffer, |buffer| {
            let message = match self.message(AdapterMessageIn::Read(buffer)) {
                Ok(message) => message,
                Err(AdapterMessageIn::Read(buffer)) => return (buffer, Err(nb::Error::WouldBlock)),
                Err(_) => unreachable!(),
            };

            self.setup_wakers(handle);

            match message {
                AdapterMessageOut::Read(Some(Ok(0)), b) => {
                    (b, Err(nb::Error::Other(NetError::PipeClosed)))
                }
                AdapterMessageOut::Read(Some(Ok(len)), b) => (b, Ok(len)),
                AdapterMessageOut::Read(Some(Err(e)), b) => {
                    (b, Err(nb::Error::Other(NetError::RecvError(e))))
                }
                AdapterMessageOut::Read(None, b) => (b, Err(nb::Error::WouldBlock)),
                _ => unreachable!(),
            }
        })
    }

    fn close(&mut self, _handle: Self::TcpSocket) -> Result<(), Self::Error> {
        trace!("Close MQTT TCP socket");
        match self.message(AdapterMessageIn::Close) {
            Ok(AdapterMessageOut::Close(Poll::Ready(res))) => res,
            Err(_) | Ok(AdapterMessageOut::Close(Poll::Pending)) => {
                // we are still closing
                return Err(NetError::PipeClosed);
            }
            Ok(_) => unreachable!(),
        }
    }
}

impl TcpError for NetError {
    fn kind(&self) -> embedded_nal::TcpErrorKind {
        match *self {
            Self::PipeClosed
            | Self::SendError(Error::TcpError(socket::Error::ConnectionReset))
            | Self::RecvError(Error::TcpError(socket::Error::ConnectionReset))
            | Self::TcpConnectError(_) => embedded_nal::TcpErrorKind::PipeClosed,
            #[cfg(feature = "tls")]
            Self::TlsConnectError(_) => embedded_nal::TcpErrorKind::PipeClosed,
            #[cfg(feature = "tls")]
            Self::SendError(Error::TlsError(_)) | Self::RecvError(Error::TlsError(_)) => {
                embedded_nal::TcpErrorKind::PipeClosed
            }
        }
    }
}

impl MqttAlocation {
    pub const fn new() -> Self {
        Self {
            future_place: None,
            tx_queue: Cell::new(None),
            rx_queue: Cell::new(None),
            waiting_for_message: Cell::new(false),
        }
    }
}

impl PartialEq for NetError {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::PipeClosed, Self::PipeClosed) => true,
            (Self::TcpConnectError(e1), Self::TcpConnectError(e2)) => e1 == e2,
            #[cfg(feature = "tls")]
            (Self::TlsConnectError(_), Self::TlsConnectError(_)) => true,
            (Self::SendError(Error::TcpError(e1)), Self::SendError(Error::TcpError(e2))) => {
                e1 == e2
            }
            (Self::RecvError(Error::TcpError(e1)), Self::RecvError(Error::TcpError(e2))) => {
                e1 == e2
            }
            _ => false,
        }
    }
}

impl PartialEq for Error {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::TcpError(e1), Self::TcpError(e2)) => e1 == e2,
            #[cfg(feature = "tls")]
            _ => false,
        }
    }
}
