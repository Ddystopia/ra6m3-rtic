use core::{convert::Infallible, net::SocketAddr, task::Poll};

use channel::{FedMqttToken, MqttNetChannel};
use embedded_nal::{TcpClientStack, TcpError};
use minimq::{ConfigBuilder, Publication};
use rtic::Mutex;
use rtic_monotonics::{fugit::ExtU32, Monotonic};
use smoltcp::{
    iface::{Interface, SocketHandle, SocketSet},
    socket::tcp::{RecvError, SendError, Socket},
};

use crate::fugit::Instant;
use crate::{conf::CLOCK_HZ, net::MQTT_BUFFER_SIZE, Mono, Net};

mod channel;

const MQTT_CLIENT_PORT: u16 = 58766;
const RECONNECT_INTERVAL_MS: u32 = 1_000;

pub struct Mqtt {
    pub socket: SocketHandle,
    minimq: Option<minimq::Minimq<'static, EmbeddedNalAdapter, Mono, Broker>>,
    channel: &'static MqttNetChannel,
    conf: Option<ConfigBuilder<'static, Broker>>,
}

pub struct Broker(pub SocketAddr);

pub type MqttClient = minimq::mqtt_client::MqttClient<'static, EmbeddedNalAdapter, Mono, Broker>;

pub struct MqttStorage {
    pub channel: MqttNetChannel,
    pub buffer: [u8; MQTT_BUFFER_SIZE],
    pub mqtt: Option<Mqtt>,
}

pub struct EmbeddedNalAdapter {
    last_connection: Option<Instant<u32, 1, 1000>>,
    port_shift: core::num::Wrapping<u8>,
    socket: Option<SocketHandle>,
    net: &'static MqttNetChannel,
}

#[derive(PartialEq, Debug)]
pub enum Error {
    SocketUsed,
    PipeClosed,
    SendError(SendError),
    RecvError(RecvError),
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

impl MqttStorage {
    pub const fn new() -> Self {
        Self {
            channel: MqttNetChannel::new(),
            mqtt: None,
            buffer: [0; MQTT_BUFFER_SIZE],
        }
    }
}

impl Mqtt {
    pub fn new(
        channel: &'static mut MqttNetChannel,
        socket: SocketHandle,
        conf: ConfigBuilder<'static, Broker>,
    ) -> Self {
        Self {
            minimq: None,
            socket,
            channel,
            conf: Some(conf),
        }
    }

    fn minimq(
        &mut self,
        _: FedMqttToken,
    ) -> &mut minimq::Minimq<'static, EmbeddedNalAdapter, Mono, Broker> {
        self.minimq.get_or_insert_with(|| {
            let adapter = EmbeddedNalAdapter::new(self.channel, self.socket);
            minimq::Minimq::new(adapter, Mono, self.conf.take().unwrap())
        })
    }

    pub fn poll(&mut self, iface: &mut Interface, sockets: &mut SocketSet<'static>) {
        self.channel.feed(iface, sockets, |t| {
            self.minimq(t).poll(poll).unwrap(); // I want to see what kinds of errors it will generate
        });
    }

    pub fn with_client<R>(
        &mut self,
        iface: &mut Interface,
        sockets: &mut SocketSet<'static>,
        f: impl FnOnce(&mut MqttClient) -> R,
    ) -> R {
        self.channel.feed(iface, sockets, |t| {
            f(self.minimq(t).client()) //
        })
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

pub async fn mqtt(mut ctx: crate::app::mqtt::Context<'_>) -> Infallible {
    use crate::app::mqtt::Context;
    type MError = minimq::Error<crate::mqtt::Error>;

    #[rustfmt::skip]
    async fn with_client<R, F>(ctx: &mut Context<'_>, f: F) -> Result<R, MError>
    where
        F: FnMut(&mut MqttClient) -> Result<R, MError> + Copy,
    {
        core::future::poll_fn(|_| {
            ctx.shared.net.lock(|Net { sockets, iface, mqtt }| {
                mqtt.poll(iface, sockets); // ingress
                match mqtt.with_client(iface, sockets, f) {
                    Err(minimq::Error::NotReady) => {
                        mqtt.poll(iface, sockets); // egress
                        Poll::Pending
                    },
                    other => Poll::Ready(other),
                }
            })
        })
        .await
    }

    with_client(&mut ctx, |client| {
        client.subscribe(&["/rtic_mqtt/hello_world".into()], &[])
    })
    .await
    .unwrap();

    with_client(&mut ctx, |client| {
        if client.subscriptions_pending() {
            return Err(minimq::Error::NotReady);
        }
        Ok(())
    })
    .await
    .unwrap();

    defmt::info!("Subscribed to topics");

    core::future::poll_fn(|_| {
        ctx.shared.net.lock(|net| {
            net.mqtt.poll(&mut net.iface, &mut net.sockets);
        });
        Poll::<Infallible>::Pending
    })
    .await
}

mod waiter {
    use atomic_refcell::AtomicRefCell;
    use core::{
        future::Future,
        pin::Pin,
        sync::Exclusive,
        task::{Context, Poll},
    };
    use rtic_monotonics::{
        fugit::{ExtU32, Instant},
        Monotonic,
    };
    use stackfuture::StackFuture;
    use static_cell::StaticCell;

    use crate::Mono;

    pub type Waiter = Exclusive<StackFuture<'static, (), 120>>;

    static WAITER_PLACE: StaticCell<Waiter> = StaticCell::new();
    // Something like `AtomicPtr` would be enough, but I don't want to use unsafe code
    static WAITER: AtomicRefCell<Option<Pin<&'static mut Waiter>>> = AtomicRefCell::new(None);

    pub fn setup_waiter(mut at: Instant<u32, 1, 1000>) {
        let waker = crate::app::mqtt::waker();
        let mut cx = Context::from_waker(&waker);

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
    pub const fn new(net: &'static MqttNetChannel, handle: SocketHandle) -> Self {
        Self {
            last_connection: None,
            port_shift: core::num::Wrapping(0),
            socket: Some(handle),
            net,
        }
    }
    fn with<R>(
        &mut self,
        handle: SocketHandle,
        f: impl FnOnce(&mut Interface, &mut Socket<'_>) -> R,
    ) -> R {
        let (iface, sockets) = self.net.take();
        let socket = sockets.get_mut::<Socket>(handle);
        let ret = f(&mut *iface, &mut *socket);

        let waker = crate::app::mqtt::waker();
        socket.register_recv_waker(&waker);
        socket.register_send_waker(&waker);

        self.net.put(iface, sockets);

        ret
    }
}

// fixme: when broker RST, client does not reconnect for some reason

impl TcpClientStack for EmbeddedNalAdapter {
    type TcpSocket = SocketHandle;

    type Error = Error;

    fn socket(&mut self) -> Result<Self::TcpSocket, Self::Error> {
        let socket = self.socket.take().ok_or(Error::SocketUsed)?;
        self.with(socket, |_, socket| socket.abort());
        Ok(socket)
    }

    // note: this path is called x3-x5 times for some reason
    fn connect(
        &mut self,
        handle: &mut Self::TcpSocket,
        remote: core::net::SocketAddr,
    ) -> embedded_nal::nb::Result<(), Self::Error> {
        let now = Mono::now();
        defmt::info!("Connecting poll: got {}", now.ticks());

        let mut should_try_connect = true;
        if let Some(last) = self.last_connection {
            if let Some(diff) = now.checked_duration_since(last) {
                if diff < RECONNECT_INTERVAL_MS.millis::<1, 1000>() {
                    let to = last + RECONNECT_INTERVAL_MS.millis::<1, 1000>();
                    defmt::info!("Connecting poll: request {}", to.ticks());
                    should_try_connect = false;

                    // This is not async function and is not pinned, so let the
                    // global state be our `Pin<&mut Self>`.
                    waiter::setup_waiter(last + RECONNECT_INTERVAL_MS.millis::<1, 1000>());
                }
            }
        }

        let mut last_connection = self.last_connection;

        let port = MQTT_CLIENT_PORT + self.port_shift.0 as u16;
        self.port_shift += 1;
        let res = self.with(*handle, |iface, socket| {
            // note: this is in SynSent state for several seconds sometimes
            if !socket.is_open() && should_try_connect {
                last_connection = Some(now);
                defmt::info!("Connecting");
                socket.connect(iface.context(), remote, port).expect(
                    "Inspection of error conditions, they will only happen during development",
                );
            }
            // We need to deal with close_wait stuff
            // todo: why is it getting in close_wait state at the first place?
            if socket.state() == smoltcp::socket::tcp::State::CloseWait {
                socket.close()
            }
            if socket.state() == smoltcp::socket::tcp::State::Established {
                Ok(())
            } else {
                defmt::info!("TCP socket connect state: {}", socket.state());
                Err(embedded_nal::nb::Error::<Self::Error>::WouldBlock)
            }
        });

        self.last_connection = last_connection;

        res
    }

    fn send(
        &mut self,
        handle: &mut Self::TcpSocket,
        buffer: &[u8],
    ) -> embedded_nal::nb::Result<usize, Self::Error> {
        if buffer.len() == 0 {
            return Ok(0);
        }

        self.with(*handle, |_, socket| {
            if socket.state() == smoltcp::socket::tcp::State::Closed {
                return Err(embedded_nal::nb::Error::Other(Error::PipeClosed));
            }

            if !socket.can_send() {
                defmt::info!("Send Would Block");
                return Err(embedded_nal::nb::Error::WouldBlock);
            }

            let len = socket
                .send_slice(buffer)
                .map_err(Error::SendError)
                .map_err(embedded_nal::nb::Error::Other)?;

            // Notify network stack that it is time to be polled
            crate::app::poll_network::spawn().ok();

            Ok(len)
        })
    }

    fn receive(
        &mut self,
        handle: &mut Self::TcpSocket,
        buffer: &mut [u8],
    ) -> embedded_nal::nb::Result<usize, Self::Error> {
        self.with(*handle, |_, socket| {
            if socket.state() == smoltcp::socket::tcp::State::Closed {
                return Err(embedded_nal::nb::Error::Other(Error::PipeClosed));
            }

            if !socket.can_recv() {
                defmt::trace!("Recv Would Block");
                return Err(embedded_nal::nb::Error::WouldBlock);
            }

            let len = socket
                .recv_slice(buffer)
                .map_err(Error::RecvError)
                .map_err(embedded_nal::nb::Error::Other)?;

            Ok(len)
        })
    }

    fn close(&mut self, handle: Self::TcpSocket) -> Result<(), Self::Error> {
        self.with(handle, |_, socket| socket.close());
        self.socket = Some(handle);
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
