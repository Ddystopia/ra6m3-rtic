use core::{future::poll_fn, task::Poll};

#[cfg(feature = "tls")]
use embedded_tls::{TlsConfig, TlsContext, UnsecureProvider};
#[cfg(feature = "tls")]
use tls_socket::{Rng, TlsSocket};

use picoserve::{Config, response::File, routing::get_service};
use rtic_monotonics::fugit::ExtU32;
use smoltcp::{iface::SocketHandle, socket::tcp};
use socket::{Socket, Timer};

use crate::{poll_share::TokenProvider, socket_storage::HTTP_BUFFER_SIZE};

// todo: use https://docs.rs/smoltcp/latest/smoltcp/socket/tcp/struct.Socket.html#method.set_timeout and others

mod socket;
#[cfg(feature = "tls")]
mod tls_socket;

// const TLS_TX_SIZE: usize = 16_640;
// const TLS_RX_SIZE: usize = 16_640;
const TLS_TX_SIZE: usize = 13_640;
const TLS_RX_SIZE: usize = 16_640;

pub type AppState = ();
pub type NetLock = impl rtic::Mutex<T = crate::Net> + 'static;
pub type AppRouter = impl picoserve::routing::PathRouter<AppState>;

pub struct Storage {
    pub app: Option<picoserve::Router<AppRouter, AppState>>,
    pub buf: [u8; HTTP_BUFFER_SIZE],
    pub tls_tx: [u8; TLS_TX_SIZE],
    pub tls_rx: [u8; TLS_RX_SIZE],
}

impl Storage {
    pub const fn new() -> Self {
        Self {
            app: None,
            buf: [0; HTTP_BUFFER_SIZE],
            tls_tx: [0; TLS_TX_SIZE],
            tls_rx: [0; TLS_RX_SIZE],
        }
    }
}

#[define_opaque(NetLock)]
pub async fn http(ctx: crate::app::http_task::Context<'static>, socket_handle: SocketHandle) -> ! {
    let net = TokenProvider::new(ctx.local.token_place, ctx.shared.net);
    let storage = ctx.local.storage;
    let app = storage.app.get_or_insert(make_app());

    loop {
        net.lock(|net| {
            let socket = net.sockets.get_mut::<tcp::Socket>(socket_handle);
            let port = if cfg!(feature = "tls") { 443 } else { 80 };
            socket.listen(port).unwrap();
            crate::app::poll_network::spawn().ok();
        });

        let socket = poll_fn(|cx| {
            net.lock(|locked| {
                let socket = locked.sockets.get_mut::<tcp::Socket>(socket_handle);

                if socket.can_recv() {
                    Poll::Ready(Socket::new(socket_handle, net))
                } else {
                    socket.register_recv_waker(cx.waker());
                    Poll::Pending
                }
            })
        })
        .await;

        #[cfg(feature = "tls")]
        let socket = {
            let tls_config = TlsConfig::new().with_server_name("example.com");
            let mut socket = TlsSocket::new(socket, &mut storage.tls_rx, &mut storage.tls_tx);
            let mut rng = Rng;
            let tls_ctx = TlsContext::new(&tls_config, UnsecureProvider::new(&mut rng));
            socket.open(tls_ctx).await.unwrap();

            socket
        };

        handle_connection(&app, socket, &mut storage.buf).await;
    }
}

async fn handle_connection<S, E>(
    app: &picoserve::Router<AppRouter, AppState>,
    socket: S,
    buf: &mut [u8],
) where
    S: picoserve::io::Socket<Error = E>,
    E: defmt::Format + picoserve::io::Error,
{
    let config = Config::new(picoserve::Timeouts {
        start_read_request: Some(5000.millis()),
        read_request: Some(1000.millis()),
        write: Some(5000.millis()),
    });

    match picoserve::serve_with_state(app, Timer, &config, buf, socket, &()).await {
        Ok(count) => defmt::trace!("Handled {} requests", count),
        Err(picoserve::Error::Read(e)) => defmt::error!("Failed to serve with Read Error: {}", e),
        Err(picoserve::Error::Write(e)) => defmt::error!("Failed to serve with Write Error: {}", e),
        Err(picoserve::Error::ReadTimeout) => defmt::error!("Failed to serve with Read Timeout"),
        Err(picoserve::Error::WriteTimeout) => defmt::error!("Failed to serve with Write Timeout"),
    }
}

#[define_opaque(AppRouter)]
pub fn make_app() -> picoserve::Router<AppRouter, AppState> {
    const HELLO_WORLD: &str = r#"<!DOCTYPE html>
<html>
<head>
    <title>Qemu Picoserve</title>
    <meta charset="utf-8">
</head>
<body>
    <h1>Welcome to Qemu Picoserve</h1>
    <p>This is a placeholder page.</p>
</body>
</html>
"#;
    picoserve::Router::new().route("/", get_service(File::html(HELLO_WORLD)))
}
