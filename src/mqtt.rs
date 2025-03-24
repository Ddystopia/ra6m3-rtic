use core::{
    net::SocketAddr,
    pin::{Pin, pin},
    task::{Context, Poll, Waker},
};

use crate::{
    conf::{MQTT_BROKER_IP, MQTT_BROKER_PORT},
    poll_share::TokenProvider,
    socket::{ConnectError, TcpSocket},
};
use embedded_nal::{TcpClientStack, TcpError, nb};
use minimq::{ConfigBuilder, Publication};
use rtic_monotonics::{Monotonic, fugit::ExtU32};
use smoltcp::{
    iface::{Interface, SocketHandle},
    socket::tcp::Socket,
};

use crate::fugit::Instant;
use crate::{Mono, conf::CLOCK_HZ, socket_storage::MQTT_BUFFER_SIZE};

const MQTT_CLIENT_PORT: u16 = 58737;
const RECONNECT_INTERVAL_MS: u32 = 1_000;

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

// type ConnectFut = impl Future<Output = (TcpSocket<NetLock>, Result<(), ConnectError>)>;
type ConnectFut =
    stackfuture::StackFuture<'static, (TcpSocket<NetLock>, Result<(), ConnectError>), 120>;

pub struct EmbeddedNalAdapter {
    last_connection: Option<Instant<u32, 1, 1000>>,
    port_shift: core::num::Wrapping<u8>,
    socket_handle: Option<SocketHandle>,
    net: TokenProvider<NetLock>,
    waker: Waker,
    connect_future: Pin<&'static mut Option<ConnectFut>>,
    socket: Option<TcpSocket<NetLock>>,
}

#[derive(PartialEq, Debug, defmt::Format)]
pub enum Error {
    SocketUsed,
    PipeClosed,
    ConnectError(crate::socket::ConnectError),
    SendError(crate::socket::Error),
    RecvError(crate::socket::Error),
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

    pub fn poll(&mut self) -> Poll<Result<!, minimq::Error<Error>>> {
        match self.minimq().poll(poll).map(|_| ()) {
            Ok(()) | Err(minimq::Error::NotReady) => Poll::Pending,
            Err(other) => Poll::Ready(Err(other)),
        }
    }

    pub async fn join(&mut self) -> Result<!, minimq::Error<Error>> {
        core::future::poll_fn(|_| self.poll()).await
    }

    pub async fn subscribe(
        &mut self,
        topics: &[minimq::types::TopicFilter<'_>],
        properties: &[minimq::Property<'_>],
    ) -> Result<(), minimq::Error<Error>> {
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
    let waker = core::future::poll_fn(|cx| Poll::Ready(cx.waker().clone())).await;
    // todo: keepalive doesn't work
    let conf = conf.keepalive_interval(30_000);
    let mqtt = Mqtt::new(socket_handle, conf, net, waker, &mut storage.alloc);
    let mqtt = storage.mqtt.get_or_insert(mqtt);

    loop {
        mqtt.subscribe(&["/rtic_mqtt/hello_world".into()], &[])
            .await
            .unwrap();

        defmt::info!("Subscribed to topics");

        // For some reason, no error if I stop the broker
        match mqtt.join().await {
            Err(minimq::Error::Network(Error::ConnectError(ConnectError::ConnectionReset))) => {
                defmt::info!("Failed to connect to broker, retrying...");
            }
            Err(minimq::Error::Network(other)) => defmt::error!("Minimq Network error: {}", other),
            // I want to see what kinds of errors it will generate
            Err(other) => panic!("Minimq error: {:?}", other),
        }
    }
}

mod waiter {
    use atomic_refcell::AtomicRefCell;
    use core::{
        future::Future,
        pin::Pin,
        sync::Exclusive,
        task::{Context, Poll, Waker},
    };
    use rtic_monotonics::{
        Monotonic,
        fugit::{ExtU32, Instant},
    };
    use stackfuture::StackFuture;
    use static_cell::StaticCell;

    use crate::Mono;

    pub type Waiter = Exclusive<StackFuture<'static, (), 120>>;

    static WAITER_PLACE: StaticCell<Waiter> = StaticCell::new();
    // Something like `AtomicPtr` would be enough, but I don't want to use unsafe code
    static WAITER: AtomicRefCell<Option<Pin<&'static mut Waiter>>> = AtomicRefCell::new(None);

    pub fn setup_waiter(mut at: Instant<u32, 1, 1000>, waker: &Waker) {
        let mut cx = Context::from_waker(waker);

        let mut pin_guard = WAITER.borrow_mut();

        loop {
            let fut = Exclusive::new(StackFuture::from(Mono::delay_until(at)));

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
            last_connection: None,
            port_shift: core::num::Wrapping(0),
            socket_handle: Some(handle),
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

    fn with<R>(
        &mut self,
        handle: SocketHandle,
        f: impl FnOnce(&mut Interface, &mut Socket<'_>) -> R,
    ) -> R {
        self.net.lock(|net| {
            let socket = net.sockets.get_mut::<Socket>(handle);
            let ret = f(&mut net.iface, &mut *socket);

            socket.register_recv_waker(&self.waker);
            socket.register_send_waker(&self.waker);

            ret
        })
    }

    fn should_try_connect(&mut self) -> bool {
        let now = Mono::now();

        if let Some(last) = self.last_connection {
            if let Some(diff) = now.checked_duration_since(last) {
                if diff < RECONNECT_INTERVAL_MS.millis::<1, 1000>() {
                    self.last_connection = Some(now);
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

// fixme: when broker RST, client does not reconnect for some reason

impl TcpClientStack for EmbeddedNalAdapter {
    type TcpSocket = SocketHandle;

    type Error = Error;

    fn socket(&mut self) -> Result<Self::TcpSocket, Self::Error> {
        let socket = self.socket_handle.take().ok_or(Error::SocketUsed)?;
        self.with(socket, |_, socket| socket.abort());
        Ok(socket)
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
                Poll::Ready((socket, Ok(()))) => {
                    self.socket = Some(socket);
                    self.last_connection = Some(Mono::now());
                    self.connect_future.set(None);
                    Ok(())
                }
                Poll::Ready((socket, Err(e))) => {
                    self.socket = Some(socket);
                    self.connect_future.set(None);
                    Err(nb::Error::Other(Error::ConnectError(e)))
                }
                Poll::Pending => Err(nb::Error::WouldBlock),
            };
        }

        let port = MQTT_CLIENT_PORT + self.port_shift.0 as u16;
        self.port_shift += 1;

        if self.should_try_connect() {
            if let Some(socket) = self.socket.take() {
                let fut = async move {
                    let mut socket = socket;
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

        if let Some(socket) = self.socket.as_mut() {
            let fut = pin!(socket.write(buffer));
            let mut ctx = Context::from_waker(&self.waker);
            match fut.poll(&mut ctx) {
                Poll::Ready(Ok(len)) => Ok(len),
                Poll::Ready(Err(e)) => Err(nb::Error::Other(Error::SendError(e))),
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

        if let Some(socket) = self.socket.as_mut() {
            match {
                let fut = pin!(socket.read(buffer));
                fut.poll(&mut Context::from_waker(&self.waker))
            } {
                Poll::Ready(Ok(len)) => {
                    self.setup_wakers(handle);
                    Ok(len)
                }
                Poll::Ready(Err(e)) => Err(nb::Error::Other(Error::RecvError(e))),
                Poll::Pending => Err(nb::Error::WouldBlock),
            }
        } else {
            Err(nb::Error::WouldBlock)
        }
    }

    fn close(&mut self, handle: Self::TcpSocket) -> Result<(), Self::Error> {
        self.with(handle, |_, socket| socket.close());
        self.socket_handle = Some(handle);
        Ok(())
    }
}

impl TcpError for Error {
    fn kind(&self) -> embedded_nal::TcpErrorKind {
        if *self == Error::PipeClosed {
            return embedded_nal::TcpErrorKind::PipeClosed;
        }

        embedded_nal::TcpErrorKind::Other
    }
}
