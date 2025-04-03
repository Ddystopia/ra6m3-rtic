const CAP: usize = 8;

use core::{
    future::poll_fn,
    net::SocketAddr,
    pin::{Pin, pin},
    task::{Context, Poll, Waker},
};

use embedded_nal::{TcpClientStack, TcpError, nb};
#[cfg(feature = "tls")]
use embedded_tls::{TlsContext, TlsError, UnsecureProvider};
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

use super::NetLock;

#[cfg(feature = "tls")]
use super::tls_socket::{Rng, TlsSocket};

const MQTT_CLIENT_PORT: u16 = 58737;
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
    tx_queue: heapless::spsc::Queue<AdapterMessageIn, CAP>,
    rx_queue: heapless::spsc::Queue<AdapterMessageOut, CAP>,
}

pub struct EmbeddedNalAdapter {
    socket_handle: SocketHandle,
    net: TokenProvider<NetLock>,
    waker: Waker,
    fut: Pin<&'static mut AdapterFut>,
    tx: Producer<'static, AdapterMessageIn, CAP>,
    rx: Consumer<'static, AdapterMessageOut, CAP>,
    waiting_for_pong: bool,
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
    Close(SocketHandle),
    Socket,
    Ping,
}

#[derive(Debug)]
enum AdapterMessageOut {
    Connected,
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

async fn dequeue(rx: &mut Consumer<'static, AdapterMessageIn, CAP>) -> AdapterMessageIn {
    poll_fn(|_| match rx.dequeue() {
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
    rx: &mut Consumer<'static, AdapterMessageIn, CAP>,
    tx: &mut Producer<'static, AdapterMessageOut, CAP>,
) {
    let mut rng = Rng;
    let tcp_socket = TcpSocket::new(net, handle);
    let ctx = TlsContext::new(&tls.config, UnsecureProvider::new(&mut rng));
    let mut tls_socket = TlsSocket::new(tcp_socket, tls.tls_rx, tls.tls_tx);

    tls_socket.open(ctx).await.unwrap();

    tx.enqueue(AdapterMessageOut::Connected).unwrap();

    loop {
        match dequeue(rx).await {
            AdapterMessageIn::Connect(_) => {}
            AdapterMessageIn::Read(buffer) => {
                let result = match {
                    let mut read_fut = pin!(tls_socket.0.read(buffer));
                    poll_fn(|cx| Poll::Ready(read_fut.as_mut().poll(cx))).await
                } {
                    Poll::Ready(result) => Some(result.map_err(Error::TlsError)),
                    Poll::Pending => None,
                };
                tx.enqueue(AdapterMessageOut::Read(result, buffer)).unwrap();
            }
            AdapterMessageIn::Write(buffer) => {
                let result = match {
                    let mut read_fut = pin!(tls_socket.0.write(buffer));
                    poll_fn(|cx| Poll::Ready(read_fut.as_mut().poll(cx))).await
                } {
                    Poll::Ready(result) => Some(result.map_err(Error::TlsError)),
                    Poll::Pending => None,
                };
                tx.enqueue(AdapterMessageOut::Write(result, buffer))
                    .unwrap();
            }
            AdapterMessageIn::Close(_) => {
                tx.enqueue(AdapterMessageOut::Closing).unwrap();
                tls_socket.close().await.unwrap();
                return;
            }
            AdapterMessageIn::Socket => _ = tx.enqueue(AdapterMessageOut::Socket(None)).unwrap(),
            AdapterMessageIn::Ping => _ = tx.enqueue(AdapterMessageOut::Pong).unwrap(),
        }
    }
}

async fn adapter_task(
    net: TokenProvider<NetLock>,
    handle: SocketHandle,
    #[cfg(feature = "tls")] mut tls: Option<TlsArgs<'static>>,
    mut rx: Consumer<'static, AdapterMessageIn, CAP>,
    mut tx: Producer<'static, AdapterMessageOut, CAP>,
) -> ! {
    let mut connected = false;
    let mut tcp_socket = TcpSocket::new(net, handle);
    let mut port_shift = core::num::Wrapping::<u8>(0);
    let mut last_connection_attempt: Option<Instant<u32, 1, 1000>> = None;

    loop {
        match dequeue(&mut rx).await {
            AdapterMessageIn::Connect(socket_addr) => {
                if let Some(last) = last_connection_attempt {
                    if let Some(diff) = Mono::now().checked_duration_since(last) {
                        if diff < RECONNECT_INTERVAL_MS.millis::<1, 1000>() {
                            Mono::delay_until(last + RECONNECT_INTERVAL_MS.millis()).await;
                        }
                    }
                }

                last_connection_attempt = Some(Mono::now());
                let port = MQTT_CLIENT_PORT + port_shift.0 as u16;
                tcp_socket.connect(socket_addr, port).await.expect("todo");

                #[cfg(feature = "tls")]
                if let Some(tls) = tls.as_mut() {
                    handle_tls_session(net, handle, tls, &mut rx, &mut tx).await;
                } else {
                    connected = true;
                    tx.enqueue(AdapterMessageOut::Connected).unwrap();
                }
                #[cfg(not(feature = "tls"))]
                {
                    connected = true;
                    tx.enqueue(AdapterMessageOut::Connected).unwrap();
                }

                port_shift += 1;
            }
            AdapterMessageIn::Read(buffer) => {
                let result = match {
                    let mut read_fut = pin!(tcp_socket.read(buffer));
                    poll_fn(|cx| Poll::Ready(read_fut.as_mut().poll(cx))).await
                } {
                    Poll::Ready(result) => Some(result.map_err(Error::TcpError)),
                    Poll::Pending => None,
                };
                tx.enqueue(AdapterMessageOut::Read(result, buffer)).unwrap();
            }
            AdapterMessageIn::Write(buffer) => {
                let result = match {
                    let mut read_fut = pin!(tcp_socket.write(buffer));
                    poll_fn(|cx| Poll::Ready(read_fut.as_mut().poll(cx))).await
                } {
                    Poll::Ready(result) => Some(result.map_err(Error::TcpError)),
                    Poll::Pending => None,
                };
                tx.enqueue(AdapterMessageOut::Write(result, buffer))
                    .unwrap();
            }
            AdapterMessageIn::Close(_socket_handle) => {
                tx.enqueue(AdapterMessageOut::Closing).unwrap();
                tcp_socket.disconnect().await;
                connected = false;
            }
            AdapterMessageIn::Socket => {
                _ = tx
                    .enqueue(AdapterMessageOut::Socket((!connected).then_some(())))
                    .unwrap();
            }
            AdapterMessageIn::Ping => _ = tx.enqueue(AdapterMessageOut::Pong).unwrap(),
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
        let (tx_tx, tx_rx) = alloc.tx_queue.split();
        let (rx_tx, rx_rx) = alloc.rx_queue.split();

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
            waiting_for_pong: false,
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

    fn message(&mut self, msg: AdapterMessageIn) -> Option<AdapterMessageOut> {
        while let Some(msg) = self.rx.dequeue() {
            match msg {
                AdapterMessageOut::Pong => self.waiting_for_pong = false,
                other => return Some(other),
            }
        }

        if self.waiting_for_pong {
            return None;
        }

        let mut ctx = Context::from_waker(&self.waker);

        self.tx.enqueue(AdapterMessageIn::Ping).unwrap();
        _ = self.fut.as_mut().poll(&mut ctx);

        match self.rx.dequeue() {
            Some(AdapterMessageOut::Pong) => (),
            Some(_other) => unreachable!(),
            None => {
                self.waiting_for_pong = true;
                return None;
            }
        }

        self.tx.enqueue(msg).unwrap();

        _ = self.fut.as_mut().poll(&mut ctx);

        self.rx.dequeue()
    }
}

impl TcpClientStack for EmbeddedNalAdapter {
    type TcpSocket = SocketHandle;

    type Error = NetError;

    fn socket(&mut self) -> Result<Self::TcpSocket, Self::Error> {
        match self.message(AdapterMessageIn::Socket) {
            Some(AdapterMessageOut::Socket(Some(()))) => Ok(self.socket_handle),
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
            Some(AdapterMessageOut::Connected) => Ok(()),
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

        match self.message(AdapterMessageIn::Write(buffer)) {
            Some(AdapterMessageOut::Write(Some(Ok(len)), b)) => {
                assert_eq!(b.as_ptr(), ptr);
                Ok(len)
            }
            Some(AdapterMessageOut::Write(Some(Err(err)), b)) => {
                assert_eq!(b.as_ptr(), ptr);
                Err(nb::Error::Other(NetError::SendError(err)))
            }
            Some(AdapterMessageOut::Write(None, b)) => {
                assert_eq!(b.as_ptr(), ptr);
                Err(nb::Error::WouldBlock)
            }
            None => Err(nb::Error::WouldBlock),
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

        match self.message(AdapterMessageIn::Read(buffer)) {
            Some(AdapterMessageOut::Read(Some(Ok(0)), b)) => {
                assert_eq!(b.as_mut_ptr(), ptr);
                Err(nb::Error::Other(NetError::PipeClosed))
            }
            Some(AdapterMessageOut::Read(Some(Ok(len)), b)) => {
                assert_eq!(b.as_mut_ptr(), ptr);
                self.setup_wakers(handle);
                Ok(len)
            }
            Some(AdapterMessageOut::Read(Some(Err(e)), b)) => {
                assert_eq!(b.as_mut_ptr(), ptr);
                Err(nb::Error::Other(NetError::RecvError(e)))
            }
            Some(AdapterMessageOut::Read(None, b)) => {
                assert_eq!(b.as_mut_ptr(), ptr);
                Err(nb::Error::WouldBlock)
            }
            None => Err(nb::Error::WouldBlock),
            _ => unreachable!(),
        }
    }

    fn close(&mut self, handle: Self::TcpSocket) -> Result<(), Self::Error> {
        defmt::info!("Closing socket");
        match self.message(AdapterMessageIn::Close(handle)) {
            Some(AdapterMessageOut::Closing) => Ok(()),
            _ => unreachable!(),
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
            Self::SendError(Error::TlsError(TlsError::ConnectionClosed))
            | Self::RecvError(Error::TlsError(TlsError::ConnectionClosed))
            | Self::TlsConnectError(_) => embedded_nal::TcpErrorKind::PipeClosed,
            _ => embedded_nal::TcpErrorKind::Other,
        }
    }
}

impl MqttAlocation {
    pub const fn new() -> Self {
        Self {
            future_place: None,
            tx_queue: heapless::spsc::Queue::new(),
            rx_queue: heapless::spsc::Queue::new(),
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
