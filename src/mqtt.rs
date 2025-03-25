/*
TOOD TLS:

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

#[cfg(feature = "tls")]
compile_error!("TODO: mqtt with tls");

use core::{
    net::SocketAddr,
    pin::{Pin, pin},
    task::{Context, Poll, Waker},
};

use crate::{
    conf::{MQTT_BROKER_IP, MQTT_BROKER_PORT},
    poll_share::TokenProvider,
    socket::{self, TcpSocket},
};
use embedded_nal::{TcpClientStack, TcpError, nb};
use minimq::{ConfigBuilder, Publication};
use rtic_monotonics::{
    Monotonic,
    fugit::{self, ExtU32},
};
use smoltcp::{iface::SocketHandle, socket::tcp::Socket};

use crate::fugit::Instant;
use crate::{Mono, conf::CLOCK_HZ, socket_storage::MQTT_BUFFER_SIZE};

const MQTT_CLIENT_PORT: u16 = 58737;
const RECONNECT_INTERVAL_MS: u32 = 2_000;

pub type NetLock = impl rtic::Mutex<T = crate::Net> + 'static;

pub struct Mqtt {
    pub socket: SocketHandle,
    minimq: Option<minimq::Minimq<'static, EmbeddedNalAdapter, Mono, Broker>>,
    conf: Option<ConfigBuilder<'static, Broker>>,
    net: TokenProvider<NetLock>,
    waker: Waker,
    alloc: Option<&'static mut MqttAlocation>,
}
pub struct MqttAlocation {
    connect_future_place: Option<ConnectFut>,
}

pub struct Broker(pub SocketAddr);

pub type MqttClient = minimq::mqtt_client::MqttClient<'static, EmbeddedNalAdapter, Mono, Broker>;

pub struct Storage {
    pub buffer: [u8; MQTT_BUFFER_SIZE],
    pub mqtt: Option<Mqtt>,
    pub alloc: MqttAlocation,
}

// type ConnectFut = impl Future<Output = (TcpSocket<NetLock>, Result<(), socket::ConnectError>)>;
type ConnectFut =
    stackfuture::StackFuture<'static, (TcpSocket<NetLock>, Result<(), socket::ConnectError>), 120>;

pub struct EmbeddedNalAdapter {
    last_connection_attempt: Option<Instant<u32, 1, 1000>>,
    port_shift: core::num::Wrapping<u8>,
    socket_handle: Option<SocketHandle>,
    net: TokenProvider<NetLock>,
    waker: Waker,
    pending_close: bool,
    connect_future: Pin<&'static mut Option<ConnectFut>>,
    socket: Option<TcpSocket<NetLock>>,
}

#[derive(PartialEq, Debug, defmt::Format)]
pub enum NetError {
    SocketUsed,
    PipeClosed,
    ConnectError(socket::ConnectError),
    SendError(socket::Error),
    RecvError(socket::Error),
}

impl embedded_time::Clock for Mono {
    type T = u32;

    const SCALING_FACTOR: embedded_time::rate::Fraction =
        embedded_time::rate::Fraction::new(1, CLOCK_HZ);

    fn try_now(&self) -> Result<embedded_time::Instant<Self>, embedded_time::clock::Error> {
        Ok(embedded_time::Instant::new(Mono::now().ticks()))
    }
}

impl minimq::Broker for Broker {
    fn get_address(&mut self) -> Option<SocketAddr> {
        Some(self.0)
    }

    fn set_port(&mut self, port: u16) {
        self.0.set_port(port);
    }
}

impl Storage {
    pub const fn new() -> Self {
        Self {
            mqtt: None,
            buffer: [0; MQTT_BUFFER_SIZE],
            alloc: MqttAlocation {
                connect_future_place: None,
            },
        }
    }
}

impl Mqtt {
    pub fn new(
        socket: SocketHandle,
        conf: ConfigBuilder<'static, Broker>,
        net: TokenProvider<NetLock>,
        waker: Waker,
        alloc: &'static mut MqttAlocation,
    ) -> Self {
        Self {
            minimq: None,
            socket,
            conf: Some(conf),
            net,
            waker,
            alloc: Some(alloc),
        }
    }

    fn minimq(&mut self) -> &mut minimq::Minimq<'static, EmbeddedNalAdapter, Mono, Broker> {
        self.minimq.get_or_insert_with(|| {
            let alloc = self.alloc.take().unwrap();
            let adapter = EmbeddedNalAdapter::new(self.net, self.socket, alloc, self.waker.clone());
            minimq::Minimq::new(adapter, Mono, self.conf.take().unwrap())
        })
    }

    pub fn poll(&mut self) -> Poll<Result<!, minimq::Error<NetError>>> {
        match self.minimq().poll(poll).map(|_| ()) {
            Ok(()) | Err(minimq::Error::NotReady) => Poll::Pending,
            Err(other) => Poll::Ready(Err(other)),
        }
    }

    pub async fn join(
        &mut self,
        keepalive_interval: fugit::Duration<u32, 1, 1000>,
    ) -> Result<!, minimq::Error<NetError>> {
        // fixme: it should work but minimq still doesn't doesnt send keepalive packets
        loop {
            let poller = core::future::poll_fn(|_| self.poll());
            match Mono::timeout_after(keepalive_interval / 3, poller).await {
                Ok(Err(err)) => return Err(err),
                Err(rtic_monotonics::TimeoutError) => continue,
            }
        }
    }

    pub async fn subscribe(
        &mut self,
        topics: &[minimq::types::TopicFilter<'_>],
        properties: &[minimq::Property<'_>],
    ) -> Result<(), minimq::Error<NetError>> {
        if let Poll::Ready(v) = self.poll() {
            return v.map(|_| ());
        }

        core::future::poll_fn(|_| {
            if let Poll::Ready(v) = self.poll() {
                return Poll::Ready(v.map(|_| ()));
            }
            match self.minimq().client().subscribe(topics, properties) {
                Ok(_) => Poll::Ready(Ok(())),
                Err(minimq::Error::NotReady) => {
                    // todo: do we need to poll here?
                    self.poll().map(|r| r.map(|_| ()))
                }
                Err(other) => Poll::Ready(Err(other)),
            }
        })
        .await?;

        core::future::poll_fn(|_| {
            if let Poll::Ready(v) = self.poll() {
                return Poll::Ready(v.map(|_| ()));
            }
            if self.minimq().client().subscriptions_pending() {
                Poll::Pending
            } else {
                Poll::Ready(Ok(()))
            }
        })
        .await?;

        Ok(())
    }
}

fn poll(
    client: &mut MqttClient,
    topic: &str,
    bytes: &[u8],
    _props: &minimq::types::Properties<'_>,
) {
    let msg = core::str::from_utf8(bytes).unwrap();
    defmt::info!("Received message on topic '{}': {}", topic, msg);

    let publication = Publication::new("/rtic_mqtt/hello_world_response", "Hello from RTIC");
    client.publish(publication).unwrap();
}

#[define_opaque(NetLock)]
pub async fn mqtt(ctx: crate::app::mqtt_task::Context<'static>, socket_handle: SocketHandle) -> ! {
    let storage = ctx.local.storage;
    let net = TokenProvider::new(ctx.local.token_place, ctx.shared.net);
    let conf = minimq::ConfigBuilder::new(
        Broker(core::net::SocketAddr::from(core::net::SocketAddrV4::new(
            MQTT_BROKER_IP,
            MQTT_BROKER_PORT,
        ))),
        &mut storage.buffer[..],
    );
    let keepalive_interval = 5.secs();
    let waker = core::future::poll_fn(|cx| Poll::Ready(cx.waker().clone())).await;
    let conf = conf.keepalive_interval(keepalive_interval.to_secs() as u16);
    let mqtt = Mqtt::new(socket_handle, conf, net, waker, &mut storage.alloc);
    let mqtt = storage.mqtt.get_or_insert(mqtt);

    // todo: sometimes qemu does not respond
    loop {
        match try {
            mqtt.subscribe(&["/rtic_mqtt/hello_world".into()], &[])
                .await?;
            defmt::info!("Subscribed to topics");
            mqtt.join(keepalive_interval).await?
        } {
            Err(minimq::Error::Network(NetError::ConnectError(
                socket::ConnectError::ConnectionReset,
            ))) => {
                defmt::info!("Failed to connect to broker, retrying...");
            }
            Err(minimq::Error::Network(other)) => defmt::error!("Minimq Network error: {}", other),
            // I want to see what kinds of errors it will generate
            Err(minimq::Error::SessionReset) => defmt::warn!("Mqtt Session Reset"),
            Err(other) => panic!("Minimq error: {:?}", other),
        }
    }
}

// todo: when tait would be better, store waiter in the allocation, where `ConnectFut` is stored.
mod waiter {
    use atomic_refcell::AtomicRefCell;
    use core::{
        future::Future,
        pin::Pin,
        task::{Context, Poll, Waker},
    };
    use rtic_monotonics::{
        Monotonic,
        fugit::{ExtU32, Instant},
    };
    use static_cell::StaticCell;

    use crate::Mono;

    pub type Waiter = impl Future<Output = ()> + 'static;

    static WAITER_PLACE: StaticCell<Waiter> = StaticCell::new();
    // Something like `AtomicPtr` would be enough, but I don't want to use unsafe code
    static WAITER: AtomicRefCell<Option<Pin<&'static mut Waiter>>> = AtomicRefCell::new(None);

    #[define_opaque(Waiter)]
    pub fn setup_waiter(mut at: Instant<u32, 1, 1000>, waker: &Waker) {
        let mut cx = Context::from_waker(waker);

        let mut pin_guard = WAITER.borrow_mut();

        loop {
            let fut = Mono::delay_until(at);

            match match pin_guard.as_mut() {
                Some(place) => {
                    place.set(fut);
                    place.as_mut().poll(&mut cx)
                }
                None => {
                    let mut place = Pin::static_mut(WAITER_PLACE.init(fut));
                    let poll = place.as_mut().poll(&mut cx);
                    pin_guard.replace(place);
                    poll
                }
            } {
                Poll::Pending => return,
                Poll::Ready(()) => {
                    at += 1000.millis();
                    continue;
                }
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
    ) -> Self {
        let socket = Some(TcpSocket::new(net, handle));
        let connect_future = Pin::static_mut(&mut alloc.connect_future_place);

        Self {
            last_connection_attempt: None,
            port_shift: core::num::Wrapping(0),
            socket_handle: Some(handle),
            pending_close: false,
            net,
            waker,
            connect_future,
            socket,
        }
    }

    fn setup_wakers(&mut self, handle: SocketHandle) {
        self.net.lock(|net| {
            let socket = net.sockets.get_mut::<Socket>(handle);
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
                    waiter::setup_waiter(poll_at, &self.waker);

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

    // note: this path is called x3-x5 times for some reason
    // #[define_opaque(ConnectFut)]
    fn connect(
        &mut self,
        _handle: &mut Self::TcpSocket,
        remote: core::net::SocketAddr,
    ) -> nb::Result<(), Self::Error> {
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

    // todo: this function is not called, thus minimq state machine does not work well
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
