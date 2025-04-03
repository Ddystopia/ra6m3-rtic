use core::task::{Poll, Waker};

use crate::{
    conf::{MQTT_BROKER_IP, MQTT_BROKER_PORT},
    poll_share::{self, TokenProvider},
    socket::{self},
};
use adapter::{TlsArgs, Broker, EmbeddedNalAdapter, MqttAlocation, NetError};
use minimq::{ConfigBuilder, Publication};
use rtic_monotonics::{
    Monotonic,
    fugit::{self, ExtU32},
};
use smoltcp::iface::SocketHandle;

#[cfg(feature = "tls")]
const TLS_TX_SIZE: usize = 13_640;
#[cfg(feature = "tls")]
const TLS_RX_SIZE: usize = 16_640;

mod adapter;
#[cfg(feature = "tls")]
mod tls_socket;

impl embedded_time::Clock for Mono {
    type T = u32;

    const SCALING_FACTOR: embedded_time::rate::Fraction =
        embedded_time::rate::Fraction::new(1, crate::conf::CLOCK_HZ);

    fn try_now(&self) -> Result<embedded_time::Instant<Self>, embedded_time::clock::Error> {
        Ok(embedded_time::Instant::new(Mono::now().ticks()))
    }
}

use crate::{Mono, socket_storage::MQTT_BUFFER_SIZE};

pub type NetLock = impl rtic::Mutex<T = crate::Net> + 'static;

pub struct Mqtt<
    F: FnMut(
        &mut MqttClient,
        &str,
        &[u8],
        &minimq::types::Properties<'_>,
    ) -> Result<(), minimq::Error<NetError>>,
> {
    socket: SocketHandle,
    on_message: F,
    minimq: Option<minimq::Minimq<'static, EmbeddedNalAdapter, Mono, Broker>>,
    conf: Option<ConfigBuilder<'static, Broker>>,
    net: TokenProvider<NetLock>,
    waker: Waker,
    alloc: Option<&'static mut MqttAlocation>,
    #[cfg_attr(not(feature = "tls"), expect(dead_code))]
    tls: Option<TlsArgs<'static>>,
}

pub type MqttClient = minimq::mqtt_client::MqttClient<'static, EmbeddedNalAdapter, Mono, Broker>;
pub type OnMessage = impl FnMut(
    &mut MqttClient,
    &str,
    &[u8],
    &minimq::types::Properties<'_>,
) -> Result<(), minimq::Error<NetError>>;

pub struct Storage {
    pub buffer: [u8; MQTT_BUFFER_SIZE],
    pub mqtt: Option<Mqtt<OnMessage>>,
    pub token_place: poll_share::TokenProviderPlace<NetLock>,
    pub alloc: MqttAlocation,
    #[cfg(feature = "tls")]
    pub tls_tx: [u8; TLS_TX_SIZE],
    #[cfg(feature = "tls")]
    pub tls_rx: [u8; TLS_RX_SIZE],
}

impl Storage {
    pub const fn new() -> Self {
        Self {
            mqtt: None,
            buffer: [0; MQTT_BUFFER_SIZE],
            alloc: MqttAlocation::new(),
            token_place: poll_share::TokenProviderPlace::new(),
            #[cfg(feature = "tls")]
            tls_tx: [0; TLS_TX_SIZE],
            #[cfg(feature = "tls")]
            tls_rx: [0; TLS_RX_SIZE],
        }
    }
}

impl<F> Mqtt<F>
where
    F: FnMut(
        &mut MqttClient,
        &str,
        &[u8],
        &minimq::types::Properties<'_>,
    ) -> Result<(), minimq::Error<NetError>>,
{
    pub fn new(
        socket: SocketHandle,
        conf: ConfigBuilder<'static, Broker>,
        net: TokenProvider<NetLock>,
        waker: Waker,
        alloc: &'static mut MqttAlocation,
        on_message: F,
        tls: Option<TlsArgs<'static>>,
    ) -> Self {
        Self {
            minimq: None,
            socket,
            conf: Some(conf),
            net,
            waker,
            on_message,
            alloc: Some(alloc),
            tls,
        }
    }

    fn minimq(&mut self) -> &mut minimq::Minimq<'static, EmbeddedNalAdapter, Mono, Broker> {
        self.minimq.get_or_insert_with(|| {
            let alloc = self.alloc.take().unwrap();
            let adapter = EmbeddedNalAdapter::new(
                self.net,
                self.socket,
                alloc,
                self.waker.clone(),
                #[cfg(feature = "tls")]
                None,
            );
            minimq::Minimq::new(adapter, Mono, self.conf.take().unwrap())
        })
    }

    pub fn poll(&mut self) -> Poll<Result<!, minimq::Error<NetError>>> {
        let minimq = self.minimq.get_or_insert_with(|| {
            let alloc = self.alloc.take().unwrap();
            #[cfg(feature = "tls")]
            let tls = self.tls.take();
            let adapter = EmbeddedNalAdapter::new(
                self.net,
                self.socket,
                alloc,
                self.waker.clone(),
                #[cfg(feature = "tls")]
                tls,
            );
            minimq::Minimq::new(adapter, Mono, self.conf.take().unwrap())
        });

        match minimq.poll(|a, b, c, d| (self.on_message)(a, b, c, d)) {
            Ok(Some(Ok(()))) | Ok(None) | Err(minimq::Error::NotReady) => Poll::Pending,
            Ok(Some(Err(err))) | Err(err) => Poll::Ready(Err(err)),
        }
    }

    pub async fn join(
        &mut self,
        keepalive_interval: fugit::Duration<u32, 1, 1000>,
    ) -> Result<!, minimq::Error<NetError>> {
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
                Err(minimq::Error::NotReady) => self.poll().map(|r| r.map(|_| ())),
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

#[define_opaque(OnMessage)]
fn on_message() -> OnMessage {
    // note: annotations are for rust-analyzer, rustc does not require them
    |client: &mut MqttClient,
     topic: &str,
     bytes: &[u8],
     _props: &minimq::types::Properties<'_>|
     -> Result<(), minimq::Error<NetError>> {
        let msg = core::str::from_utf8(bytes).unwrap();
        defmt::info!("Received message on topic '{}': {}", topic, msg);

        let publication = Publication::new("/rtic_mqtt/hello_world_response", "Hello from RTIC");
        match client.publish(publication) {
            Ok(()) => Ok(()),
            Err(minimq::PubError::Error(mqtt_error)) => Err(mqtt_error),
            Err(minimq::PubError::Serialization(_)) => todo!("Mqtt buffer is too small"),
        }
    }
}

#[define_opaque(NetLock)]
pub async fn mqtt(ctx: crate::app::mqtt_task::Context<'static>, socket_handle: SocketHandle) -> ! {
    let storage = ctx.local.storage;
    let net = TokenProvider::new(&mut storage.token_place, ctx.shared.net);
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
    let callback = on_message();

    #[cfg(feature = "tls")]
    let tls = Some(TlsArgs {
        tls_rx: &mut storage.tls_rx,
        tls_tx: &mut storage.tls_tx,
        config: embedded_tls::TlsConfig::new().with_server_name("example.com"),
    });
    #[cfg(not(feature = "tls"))]
    let tls = None;

    let mqtt = Mqtt::new(
        socket_handle,
        conf,
        net,
        waker,
        &mut storage.alloc,
        callback,
        tls,
    );
    let mqtt = storage.mqtt.get_or_insert(mqtt);

    loop {
        match try {
            mqtt.subscribe(&["/rtic_mqtt/hello_world".into()], &[])
                .await?;
            defmt::info!("Subscribed to topics");
            mqtt.join(keepalive_interval).await?
        } {
            Err(minimq::Error::Network(NetError::TcpConnectError(
                socket::ConnectError::ConnectionReset,
            ))) => {
                defmt::info!("Failed to connect to broker, retrying...");
            }
            Err(minimq::Error::Network(other)) => defmt::error!("Minimq Network error: {}", other),
            Err(minimq::Error::SessionReset) => defmt::warn!("Mqtt Session Reset"),
            Err(minimq::Error::NotReady) => unreachable!(),
            // `minimq::Error` is non-exhaustive + other variants are dead code in 0.10.0
            Err(_other) => defmt::error!("Unknown mqtt error"),
        }
    }
}
