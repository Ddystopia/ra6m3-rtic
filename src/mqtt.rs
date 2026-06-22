use crate::{
    conf::{MQTT_BROKER_IP, MQTT_BROKER_PORT_TCP, MQTT_BROKER_PORT_TLS, MQTT_CLIENT_ID},
    log::*,
    poll_share::{self, TokenProvider},
    socket::{self, TcpSocket},
};
use minimq::{ConfigBuilder, Publication, QoS, Session, TopicFilter};
use rtic_monotonics::Monotonic;
use smoltcp::iface::SocketHandle;

use crate::{Mono, socket_storage::MQTT_BUFFER_SIZE};

#[cfg(feature = "tls")]
mod tls_socket;

/// Local TCP source port; shifted on every reconnect attempt to dodge
/// lingering TIME_WAIT state on the broker side.
const MQTT_CLIENT_PORT: u16 = 58026;
/// Backoff between connection attempts.
const RECONNECT_INTERVAL_MS: u64 = 2_000;
/// Keepalive interval advertised to the broker. minimq drives the PINGREQ
/// cadence internally via embassy-time, so no external timeout loop is needed.
const KEEPALIVE_SECS: u16 = 10 * 60;

/// Size of the rx half carved out of the shared mqtt buffer.
const MQTT_RX_SIZE: usize = MQTT_BUFFER_SIZE / 2;

#[cfg(feature = "tls")]
const TLS_TX_SIZE: usize = 16_640;
#[cfg(feature = "tls")]
const TLS_RX_SIZE: usize = 16_640;

pub type NetLock = impl rtic::Mutex<T = crate::Net> + 'static;

pub struct Storage {
    pub buffer: [u8; MQTT_BUFFER_SIZE],
    pub token_place: poll_share::TokenProviderPlace<NetLock>,
    #[cfg(feature = "tls")]
    pub tls_tx: [u8; TLS_TX_SIZE],
    #[cfg(feature = "tls")]
    pub tls_rx: [u8; TLS_RX_SIZE],
}

impl Storage {
    pub const fn new() -> Self {
        Self {
            buffer: [0; MQTT_BUFFER_SIZE],
            token_place: poll_share::TokenProviderPlace::new(),
            #[cfg(feature = "tls")]
            tls_tx: [0; TLS_TX_SIZE],
            #[cfg(feature = "tls")]
            tls_rx: [0; TLS_RX_SIZE],
        }
    }
}

const SUB_TOPIC: &str = "/rtic_mqtt/hello_world";
const RESPONSE_TOPIC: &str = "/rtic_mqtt/hello_world_response";

/// Build a freshly connected transport for the broker.
///
/// Applies the reconnect backoff, shifts the local port, and (under the `tls`
/// feature) performs the TLS handshake before handing back the IO object that
/// `Session::connect` will take ownership of.
#[cfg(not(feature = "tls"))]
async fn connect_transport(
    net: TokenProvider<NetLock>,
    handle: SocketHandle,
    port_shift: &mut core::num::Wrapping<u8>,
    last_attempt: &mut Option<crate::Instant>,
) -> Result<Transport<'static>, socket::ConnectError> {
    connect_transport_inner(net, handle, port_shift, last_attempt).await
}

#[cfg(feature = "tls")]
async fn connect_transport<'b>(
    net: TokenProvider<NetLock>,
    handle: SocketHandle,
    port_shift: &mut core::num::Wrapping<u8>,
    last_attempt: &mut Option<crate::Instant>,
    tls_rx: &'b mut [u8],
    tls_tx: &'b mut [u8],
) -> Result<Transport<'b>, socket::ConnectError> {
    connect_transport_inner(net, handle, port_shift, last_attempt, tls_rx, tls_tx).await
}

async fn connect_transport_inner<'b>(
    net: TokenProvider<NetLock>,
    handle: SocketHandle,
    port_shift: &mut core::num::Wrapping<u8>,
    last_attempt: &mut Option<crate::Instant>,
    #[cfg(feature = "tls")] tls_rx: &'b mut [u8],
    #[cfg(feature = "tls")] tls_tx: &'b mut [u8],
) -> Result<Transport<'b>, socket::ConnectError> {
    use crate::TimeExt;

    // Backoff between connection attempts.
    if let Some(last) = *last_attempt {
        let deadline = last + RECONNECT_INTERVAL_MS.millis();
        if Mono::now() < deadline {
            Mono::delay_until(deadline).await;
        }
    }
    *last_attempt = Some(Mono::now());

    let broker_port = if cfg!(feature = "tls") {
        MQTT_BROKER_PORT_TLS
    } else {
        MQTT_BROKER_PORT_TCP
    };
    let remote = core::net::SocketAddrV4::new(MQTT_BROKER_IP, broker_port);
    let local_port = MQTT_CLIENT_PORT.wrapping_add(port_shift.0 as u16);
    *port_shift += 1;

    let mut tcp = TcpSocket::new(net, handle);
    tcp.set_timeout(Some(smoltcp::time::Duration::from_secs(3600)));
    tcp.set_keep_alive(Some(smoltcp::time::Duration::from_secs(1200)));

    // If a previous session left the socket open, drop it before reconnecting.
    if tcp.is_open() {
        tcp.abort();
    }

    let remote = smoltcp::wire::IpEndpoint::from(core::net::SocketAddr::V4(remote));
    tcp.connect(remote, local_port).await?;

    #[cfg(not(feature = "tls"))]
    {
        Ok(tcp)
    }
    #[cfg(feature = "tls")]
    {
        use rand_chacha::rand_core::SeedableRng;

        let mut rng = rand_chacha::ChaCha12Rng::from_seed([183; 32]);
        let config = embedded_tls::TlsConfig::new().enable_rsa_signatures();
        let ctx =
            embedded_tls::TlsContext::new(&config, embedded_tls::UnsecureProvider::new(&mut rng));
        let mut tls = tls_socket::TlsSocket::new(tcp, tls_rx, tls_tx);
        match tls.open(ctx).await {
            Ok(()) => Ok(tls),
            Err(_err) => {
                error!("TLS handshake failed");
                Err(socket::ConnectError::ConnectionReset)
            }
        }
    }
}

/// The transport handed to `minimq::Session`. Plain TCP by default; a TLS
/// stream (still implementing `embedded_io_async::Read + Write`) under `tls`.
#[cfg(not(feature = "tls"))]
type Transport<'a> = TcpSocket<NetLock>;
#[cfg(feature = "tls")]
type Transport<'a> = tls_socket::TlsSocket<'a, NetLock>;

#[define_opaque(NetLock)]
pub async fn mqtt(ctx: crate::app::mqtt_task::Context<'static>, socket_handle: SocketHandle) -> ! {
    let storage = ctx.local.storage;
    let net = TokenProvider::new(&mut storage.token_place, ctx.shared.net);

    let mut port_shift = core::num::Wrapping::<u8>(0);
    let mut last_attempt: Option<crate::Instant> = None;

    // One `Session` for the whole lifetime — including under TLS. With this minimq
    // fork the transport `IO` no longer lives in the `Session`; it lives in the
    // `Connection` handle returned by `connect`. So each reconnect can hand in a
    // fresh transport (a new TLS stream with its own record-buffer lifetime) while
    // the single `Session` keeps subscriptions and in-flight QoS replay state.
    let config = ConfigBuilder::from_buffer(&mut storage.buffer[..], MQTT_RX_SIZE)
        .expect("mqtt buffer too small")
        .client_id(MQTT_CLIENT_ID)
        .expect("invalid client id")
        .keepalive_interval(KEEPALIVE_SECS);
    let mut session = Session::new(config);

    loop {
        let mut conn = None;

        let Err(err): Result<!, minimq::Error<_>> = try {
            let io = match connect_transport(
                net,
                socket_handle,
                &mut port_shift,
                &mut last_attempt,
                #[cfg(feature = "tls")]
                &mut storage.tls_rx,
                #[cfg(feature = "tls")]
                &mut storage.tls_tx,
            )
            .await
            {
                Ok(io) => io,
                Err(err) => {
                    info!(
                        "Failed to connect transport to broker: {}, retrying...",
                        err
                    );
                    continue;
                }
            };

            // Hand the connected transport to the session; the returned handle owns
            // the transport and borrows the session. Dropping it at the end of this
            // iteration releases the transport (and its TLS buffer borrows) for the
            // next reconnect. State carried across reconnects is reset inside
            // `connect`, so it does not matter how the previous handle ended.
            let conn = {
                conn = Some(session.connect(io).await?);
                conn.as_mut().expect("Must be some")
            };
            info!("Connected to MQTT broker");

            let sub = conn.subscribe(&[TopicFilter::new(SUB_TOPIC)], &[]).await?;
            while conn.is_pending(&sub) {
                conn.poll().await?;
            }
            info!("Subscribed to topics");

            loop {
                let msg = conn.recv().await?;
                let topic = msg.topic();
                let payload = msg.payload();

                let Ok(text) = core::str::from_utf8(payload) else {
                    warn!("Received non-utf8 message on topic '{}'", topic);
                    Err::<!, _>(minimq::Error::Disconnected)?;
                };
                info!("Received message on topic '{}': {}", topic, text);

                let mut string = heapless::String::<512>::new();
                _ = string.push_str("Echoing back: ");
                let take = text.len().min(string.capacity() - string.len());
                _ = string.push_str(&text[..take]);

                let publication =
                    Publication::new(RESPONSE_TOPIC, string.as_str()).qos(QoS::AtLeastOnce);
                match conn.publish(publication).await {
                    Ok(_) => {}
                    Err(minimq::PubError::Session(err)) => Err(err)?,
                    Err(minimq::PubError::Payload(_)) => {
                        warn!("Mqtt buffer too small for response");
                    }
                }
            }
        };

        if let Some(conn) = conn.take() {
            conn.into_inner().disconnect().await;
        }

        match err {
            minimq::Error::Disconnected => info!("MQTT session disconnected, reconnecting..."),
            minimq::Error::Transport(err) => {
                error!("MQTT transport error: {err}, reconnecting...")
            }
            err => error!("MQTT error: {err}"),
        }
    }
}
