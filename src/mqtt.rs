use core::task::{Poll, Waker};

use crate::{
    conf::{MQTT_BROKER_IP, MQTT_BROKER_PORT_TCP, MQTT_BROKER_PORT_TLS},
    poll_share::{self, TokenProvider},
    socket::{self},
    log::*,
};
use adapter::{Broker, EmbeddedNalAdapter, MqttAlocation, NetError, TlsArgs};
use minimq::{ConfigBuilder, Publication};
use rtic_monotonics::{
    Monotonic,
    fugit::{self, ExtU32},
};
use smoltcp::iface::SocketHandle;

#[cfg(feature = "tls")]
const TLS_TX_SIZE: usize = 16_640;
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

type Minimq = minimq::Minimq<'static, EmbeddedNalAdapter, Mono, Broker>;

pub type NetLock = impl rtic::Mutex<T = crate::Net> + 'static;
pub type MqttClient = minimq::mqtt_client::MqttClient<'static, EmbeddedNalAdapter, Mono, Broker>;
pub type OnMessageRtic = impl OnMessage;

pub trait OnMessage:
    FnMut(
    &mut MqttClient,
    &str,
    &[u8],
    &minimq::types::Properties<'_>,
) -> Result<(), minimq::Error<NetError>>
{
}

pub struct Mqtt<F: OnMessage> {
    minimq: Option<Minimq>,
    rest: MqttRest<F>,
}

struct MqttRest<F: OnMessage> {
    socket: SocketHandle,
    on_message: F,
    conf: Option<ConfigBuilder<'static, Broker>>,
    net: TokenProvider<NetLock>,
    waker: Waker,
    alloc: Option<&'static mut MqttAlocation>,
    #[cfg_attr(not(feature = "tls"), expect(dead_code))]
    tls: Option<TlsArgs<'static>>,
}

pub struct Storage {
    pub buffer: [u8; MQTT_BUFFER_SIZE],
    pub mqtt: Option<Mqtt<OnMessageRtic>>,
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

fn get_minimq<'a, F: OnMessage>(
    place: &'a mut Option<Minimq>,
    mqtt: &mut MqttRest<F>,
) -> &'a mut Minimq {
    place.get_or_insert_with(|| {
        let alloc = mqtt.alloc.take().unwrap();
        #[cfg(feature = "tls")]
        let tls = mqtt.tls.take();
        let adapter = EmbeddedNalAdapter::new(
            mqtt.net,
            mqtt.socket,
            alloc,
            mqtt.waker.clone(),
            #[cfg(feature = "tls")]
            tls,
        );
        Minimq::new(adapter, Mono, mqtt.conf.take().unwrap())
    })
}

impl<F: OnMessage> Mqtt<F> {
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
            rest: MqttRest {
                socket,
                conf: Some(conf),
                net,
                waker,
                on_message,
                alloc: Some(alloc),
                tls,
            },
        }
    }

    pub fn poll(&mut self) -> Poll<Result<!, minimq::Error<NetError>>> {
        let minimq = get_minimq(&mut self.minimq, &mut self.rest);
        match minimq.poll(|a, b, c, d| (self.rest.on_message)(a, b, c, d)) {
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
            let minimq = get_minimq(&mut self.minimq, &mut self.rest);
            match minimq.client().subscribe(topics, properties) {
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
            let minimq = get_minimq(&mut self.minimq, &mut self.rest);
            if minimq.client().subscriptions_pending() {
                Poll::Pending
            } else {
                Poll::Ready(Ok(()))
            }
        })
        .await?;

        Ok(())
    }
}

#[define_opaque(OnMessageRtic)]
fn on_message() -> OnMessageRtic {
    // note: annotations are for rust-analyzer, rustc does not require them
    |client: &mut MqttClient,
     topic: &str,
     bytes: &[u8],
     _props: &minimq::types::Properties<'_>|
     -> Result<(), minimq::Error<NetError>> {
        let msg = core::str::from_utf8(bytes).unwrap();
        info!("Received message on topic '{}': {}", topic, msg);

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
    let port = if cfg!(feature = "tls") {
        MQTT_BROKER_PORT_TLS
    } else {
        MQTT_BROKER_PORT_TCP
    };
    let conf = minimq::ConfigBuilder::new(
        Broker(core::net::SocketAddr::from(core::net::SocketAddrV4::new(
            MQTT_BROKER_IP,
            port,
        ))),
        &mut storage.buffer[..],
    );
    let keepalive_interval = 10.minutes();
    let waker = core::future::poll_fn(|cx| Poll::Ready(cx.waker().clone())).await;
    let conf = conf.keepalive_interval(keepalive_interval.to_secs() as u16);
    let callback = on_message();

    #[cfg(feature = "tls")]
    let tls = Some(TlsArgs {
        tls_rx: &mut storage.tls_rx,
        tls_tx: &mut storage.tls_tx,
        config: embedded_tls::TlsConfig::new()
            // .with_server_name("example.com") // It works without it for now, let it be like this
            .enable_rsa_signatures(),
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
            info!("Subscribed to topics");
            mqtt.join(keepalive_interval).await?
        } {
            Err(minimq::Error::Network(NetError::TcpConnectError(
                socket::ConnectError::ConnectionReset,
            ))) => {
                info!("Failed to connect to broker, retrying...");
            }
            Err(minimq::Error::Network(other)) => error!("Minimq Network error: {}", other),
            Err(minimq::Error::SessionReset) => warn!("Mqtt Session Reset"),
            Err(minimq::Error::NotReady) => unreachable!(),
            // `minimq::Error` is non-exhaustive + other variants are dead code in 0.10.0
            Err(_other) => error!("Unknown mqtt error"),
        }
    }
}

impl<F> OnMessage for F where
    F: FnMut(
        &mut MqttClient,
        &str,
        &[u8],
        &minimq::types::Properties<'_>,
    ) -> Result<(), minimq::Error<NetError>>
{
}
