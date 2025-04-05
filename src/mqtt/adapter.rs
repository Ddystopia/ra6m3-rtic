/*

TLS status: Library reports that handshake is done. Wireshark can't detect that there is
any TLS handshake.
*/

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

use super::NetLock;

#[cfg(feature = "tls")]
use super::tls_socket::TlsSocket;

const MQTT_CLIENT_PORT: u16 = 58026;
const RECONNECT_INTERVAL_MS: u32 = 2_000;

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
}

pub struct EmbeddedNalAdapter {
    socket_handle: SocketHandle,
    net: TokenProvider<NetLock>,
    waker: Waker,
    fut: Pin<&'static mut AdapterFut>,
    tx: &'static Cell<Option<AdapterMessageIn>>,
    rx: &'static Cell<Option<AdapterMessageOut>>,
}

#[derive(Debug, defmt::Format)]
pub enum Error {
    TcpError(socket::Error),
    #[cfg(feature = "tls")]
    TlsError(TlsError),
}

#[derive(Debug, defmt::Format)]
pub enum NetError {
    SocketUsed,
    PipeClosed,
    TcpConnectError(socket::ConnectError),
    #[cfg(feature = "tls")]
    TlsConnectError(TlsError),
    SendError(Error),
    RecvError(Error),
}

#[derive(Debug)]
enum AdapterMessageIn {
    Connect(SocketAddr),
    Read(&'static mut [u8]),
    Write(&'static [u8]),
    Close,
    Socket,
    Ping,
}

#[derive(Debug, PartialEq)]
enum AdapterMessageOut {
    Connect(Poll<Result<(), NetError>>),
    Closing,
    Socket(Option<()>),
    Read(Option<Result<usize, Error>>, &'static mut [u8]),
    Write(Option<Result<usize, Error>>, &'static [u8]),
    Pong,
}

impl minimq::Broker for Broker {
    fn get_address(&mut self) -> Option<SocketAddr> {
        Some(self.0)
    }

    fn set_port(&mut self, port: u16) {
        self.0.set_port(port);
    }
}

async fn dequeue(rx: &Cell<Option<AdapterMessageIn>>) -> AdapterMessageIn {
    poll_fn(|_| match rx.take() {
        Some(v) => Poll::Ready(v),
        None => Poll::Pending,
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
) {
    use rand_chacha::rand_core::SeedableRng;

    let mut rng = rand_chacha::ChaCha12Rng::from_seed([183; 32]);
    let tcp_socket = TcpSocket::new(net, handle);
    let ctx = TlsContext::new(&tls.config, UnsecureProvider::new(&mut rng));
    let mut tls_socket = TlsSocket::new(tcp_socket, tls.tls_rx, tls.tls_tx);

    defmt::info!("Starting TLS handshake");

    let result = tls_socket
        .open(ctx)
        .await
        .map_err(NetError::TlsConnectError);

    defmt::info!("TLS handshake done: {}", result.is_ok());

    let is_err = result.is_err();

    tx.set(Some(AdapterMessageOut::Connect(Poll::Ready(result))));

    if is_err {
        return;
    }

    loop {
        match dequeue(rx).await {
            AdapterMessageIn::Connect(_) => {
                tx.set(Some(AdapterMessageOut::Connect(Poll::Ready(Ok(())))))
            }
            // AdapterMessageIn::Connect(_) => unreachable!("Cannot connect twice"),
            AdapterMessageIn::Read(buffer) => {
                let result = match {
                    let mut read_fut = pin!(tls_socket.0.read(buffer));
                    poll_fn(|cx| Poll::Ready(read_fut.as_mut().poll(cx))).await
                } {
                    Poll::Ready(result) => Some(result.map_err(Error::TlsError)),
                    Poll::Pending => None,
                };
                defmt::info!("TLS Read request: fulfilled: {}", result.is_some());
                tx.set(Some(AdapterMessageOut::Read(result, buffer)));
            }
            AdapterMessageIn::Write(buffer) => {
                defmt::info!("TLS Write request: buffer: {:?}", buffer);
                let result = match {
                    let mut read_fut = pin!(tls_socket.0.write(buffer));
                    poll_fn(|cx| Poll::Ready(read_fut.as_mut().poll(cx))).await
                } {
                    Poll::Ready(result) => Some(result.map_err(Error::TlsError)),
                    Poll::Pending => None,
                };
                defmt::info!("TLS Write request: fulfilled: {}", result.is_some());
                tx.set(Some(AdapterMessageOut::Write(result, buffer)));
            }
            AdapterMessageIn::Close => {
                tx.set(Some(AdapterMessageOut::Closing));
                if let Err(err) = tls_socket.close().await {
                    defmt::error!("TLS close error: {}", err);
                    let mut tcp_socket = TcpSocket::new(net, handle);
                    tcp_socket.abort();
                }
                return;
            }
            AdapterMessageIn::Socket => _ = tx.set(Some(AdapterMessageOut::Socket(None))),
            AdapterMessageIn::Ping => _ = tx.set(Some(AdapterMessageOut::Pong)),
        }
    }
}

async fn adapter_task(
    net: TokenProvider<NetLock>,
    handle: SocketHandle,
    #[cfg(feature = "tls")] mut tls: Option<TlsArgs<'static>>,
    rx: &'static Cell<Option<AdapterMessageIn>>,
    tx: &'static Cell<Option<AdapterMessageOut>>,
) -> ! {
    let mut connected = false;
    let mut tcp_socket = TcpSocket::new(net, handle);
    let mut port_shift = core::num::Wrapping::<u8>(0);
    let mut last_connection_attempt: Option<Instant<u32, 1, 1000>> = None;

    loop {
        match dequeue(rx).await {
            AdapterMessageIn::Connect(_) if connected => unreachable!("Cannot connect twice"),
            AdapterMessageIn::Connect(socket_addr) => {
                tx.set(Some(AdapterMessageOut::Connect(Poll::Pending)));

                defmt::info!("TCP connecting");

                if let Some(last) = last_connection_attempt {
                    if let Some(diff) = Mono::now().checked_duration_since(last) {
                        if diff < RECONNECT_INTERVAL_MS.millis::<1, 1000>() {
                            defmt::info!("TCP connecting -- waiting");
                            Mono::delay_until(last + RECONNECT_INTERVAL_MS.millis()).await;
                            defmt::info!("TCP connecting -- really connecting");
                        }
                    }
                }

                last_connection_attempt = Some(Mono::now());
                let port = MQTT_CLIENT_PORT + port_shift.0 as u16;

                let result = tcp_socket
                    .connect(socket_addr, port)
                    .await
                    .map_err(NetError::TcpConnectError);

                defmt::info!("TCP connected: {}", result.is_ok());

                #[cfg(feature = "tls")]
                if let Some(tls) = tls.as_mut() {
                    defmt::info!("TCP connected -> Passing to TLS");
                    handle_tls_session(net, handle, tls, rx, tx).await;
                } else {
                    connected = result.is_ok();
                    tx.set(Some(AdapterMessageOut::Connect(Poll::Ready(result))));
                }
                #[cfg(not(feature = "tls"))]
                {
                    connected = result.is_ok();
                    tx.set(Some(AdapterMessageOut::Connect(Poll::Ready(result))));
                }

                port_shift += 1;
            }
            AdapterMessageIn::Read(buffer) => {
                defmt::info!("TCP READ request");
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
                defmt::info!("TCP Write request");
                let result = match {
                    let mut read_fut = pin!(tcp_socket.write(buffer));
                    poll_fn(|cx| Poll::Ready(read_fut.as_mut().poll(cx))).await
                } {
                    Poll::Ready(result) => Some(result.map_err(Error::TcpError)),
                    Poll::Pending => None,
                };
                tx.set(Some(AdapterMessageOut::Write(result, buffer)));
            }
            AdapterMessageIn::Close => {
                tx.set(Some(AdapterMessageOut::Closing));
                tcp_socket.disconnect().await;
                connected = false;
            }
            AdapterMessageIn::Socket => {
                tx.set(Some(AdapterMessageOut::Socket((!connected).then_some(()))))
            }
            AdapterMessageIn::Ping => tx.set(Some(AdapterMessageOut::Pong)),
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

        let fut = adapter_task(
            net,
            handle,
            #[cfg(feature = "tls")]
            tls,
            tx_rx,
            rx_tx,
        );
        let fut = Pin::static_mut(alloc.future_place.get_or_insert(fut));

        Self {
            socket_handle: handle,
            net,
            waker,
            tx: tx_tx,
            rx: rx_rx,
            fut,
        }
    }

    fn setup_wakers(&mut self, handle: SocketHandle) {
        self.net.lock(|net| {
            let socket = net.sockets.get_mut::<tcp::Socket>(handle);
            socket.register_recv_waker(&self.waker);
            socket.register_send_waker(&self.waker);
        });
    }

    fn message(&mut self, msg: AdapterMessageIn) -> Poll<AdapterMessageOut> {
        match (self.tx.take(), self.rx.take()) {
            (ping @ Some(AdapterMessageIn::Ping), None) => {
                self.tx.set(ping);
                return Poll::Pending;
            }
            (close @ Some(AdapterMessageIn::Close), None) => {
                self.tx.set(close);
                return Poll::Pending;
            }
            (Some(AdapterMessageIn::Close), Some(out)) => {
                assert_eq!(out, AdapterMessageOut::Closing)
            }
            (None, Some(AdapterMessageOut::Pong) | None) => (),

            _ => unreachable!(),
        }

        let mut ctx = Context::from_waker(&self.waker);

        // assert that actor is not waiting.
        {
            self.tx.set(Some(AdapterMessageIn::Ping));
            _ = self.fut.as_mut().poll(&mut ctx);

            match self.rx.take() {
                Some(AdapterMessageOut::Pong) => (),
                Some(_other) => unreachable!(),
                None => return Poll::Pending,
            }
        }

        self.tx.set(Some(msg));
        _ = self.fut.as_mut().poll(&mut ctx);
        Poll::Ready(self.rx.take().unwrap())
    }
}

impl TcpClientStack for EmbeddedNalAdapter {
    type TcpSocket = SocketHandle;

    type Error = NetError;

    fn socket(&mut self) -> Result<Self::TcpSocket, Self::Error> {
        match self.message(AdapterMessageIn::Socket) {
            Poll::Ready(AdapterMessageOut::Socket(Some(()))) => Ok(self.socket_handle),
            _ => Err(NetError::SocketUsed),
        }
    }

    // note: this path is called x3-x5 times for some reason. Who wakes mqtt task so much?
    fn connect(
        &mut self,
        _handle: &mut Self::TcpSocket,
        remote: core::net::SocketAddr,
    ) -> nb::Result<(), Self::Error> {
        defmt::debug!("Connecting to {:?}", remote);
        match self.message(AdapterMessageIn::Connect(remote)) {
            Poll::Ready(AdapterMessageOut::Connect(Poll::Ready(res))) => Ok(res?),
            _ => Err(nb::Error::WouldBlock),
        }
    }

    fn send(
        &mut self,
        _handle: &mut Self::TcpSocket,
        buffer: &[u8],
    ) -> nb::Result<usize, Self::Error> {
        if buffer.len() == 0 {
            return Ok(0);
        }

        let len = buffer.len();
        // SAFETY: This buffer will not be stored inside the future
        let buffer = unsafe { &*core::ptr::from_ref(buffer) };
        assert!(buffer.len() == len);

        let ptr = buffer.as_ptr();

        let Poll::Ready(message) = self.message(AdapterMessageIn::Write(buffer)) else {
            return Err(nb::Error::WouldBlock);
        };

        match message {
            AdapterMessageOut::Write(Some(Ok(len)), b) => {
                assert_eq!(b.as_ptr(), ptr);
                Ok(len)
            }
            AdapterMessageOut::Write(Some(Err(err)), b) => {
                assert_eq!(b.as_ptr(), ptr);
                Err(nb::Error::Other(NetError::SendError(err)))
            }
            AdapterMessageOut::Write(None, b) => {
                assert_eq!(b.as_ptr(), ptr);
                Err(nb::Error::WouldBlock)
            }
            _ => unreachable!(),
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

        // SAFETY: This buffer will not be stored inside the future
        let buffer = unsafe { &mut *core::ptr::from_mut(buffer) };
        let ptr = buffer.as_mut_ptr();

        let Poll::Ready(message) = self.message(AdapterMessageIn::Read(buffer)) else {
            return Err(nb::Error::WouldBlock);
        };

        match message {
            AdapterMessageOut::Read(Some(Ok(0)), b) => {
                assert_eq!(b.as_mut_ptr(), ptr);
                Err(nb::Error::Other(NetError::PipeClosed))
            }
            AdapterMessageOut::Read(Some(Ok(len)), b) => {
                assert_eq!(b.as_mut_ptr(), ptr);
                self.setup_wakers(handle);
                Ok(len)
            }
            AdapterMessageOut::Read(Some(Err(e)), b) => {
                assert_eq!(b.as_mut_ptr(), ptr);
                Err(nb::Error::Other(NetError::RecvError(e)))
            }
            AdapterMessageOut::Read(None, b) => {
                assert_eq!(b.as_mut_ptr(), ptr);
                Err(nb::Error::WouldBlock)
            }
            _ => unreachable!(),
        }
    }

    fn close(&mut self, _handle: Self::TcpSocket) -> Result<(), Self::Error> {
        defmt::info!("Closing socket");
        _ = self.message(AdapterMessageIn::Close);
        Ok(())
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
            Self::SendError(Error::TlsError(_)) | Self::RecvError(Error::TlsError(_)) => {
                embedded_nal::TcpErrorKind::PipeClosed
            }
            _ => embedded_nal::TcpErrorKind::Other,
        }
    }
}

impl MqttAlocation {
    pub const fn new() -> Self {
        Self {
            future_place: None,
            tx_queue: Cell::new(None),
            rx_queue: Cell::new(None),
        }
    }
}

impl PartialEq for NetError {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::SocketUsed, Self::SocketUsed) => true,
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
