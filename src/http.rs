use picoserve::{Config, response::File, routing::get_service};
use rtic_monotonics::fugit::ExtU32;
use smoltcp::iface::SocketHandle;
use timer::Timer;

use crate::{poll_share::TokenProvider, socket::TcpSocket, socket_storage::HTTP_BUFFER_SIZE};

// todo: use https://docs.rs/smoltcp/latest/smoltcp/socket/tcp/struct.Socket.html#method.set_timeout and others

mod timer;

pub type AppState = ();
pub type NetLock = impl rtic::Mutex<T = crate::Net> + 'static;
pub type AppRouter = impl picoserve::routing::PathRouter<AppState>;

pub struct Storage {
    pub app: Option<picoserve::Router<AppRouter, AppState>>,
    pub buf: [u8; HTTP_BUFFER_SIZE],
}

impl Storage {
    pub const fn new() -> Self {
        Self {
            app: None,
            buf: [0; HTTP_BUFFER_SIZE],
        }
    }
}

#[define_opaque(NetLock)]
pub async fn http(ctx: crate::app::http_task::Context<'static>, socket_handle: SocketHandle) -> ! {
    let net = TokenProvider::new(ctx.local.token_place, ctx.shared.net);
    let storage = ctx.local.storage;
    let app = storage.app.get_or_insert(make_app());

    loop {
        let mut socket = TcpSocket::new(net, socket_handle);

        socket.accept(80).await.unwrap();

        handle_connection(&app, socket, &mut storage.buf).await;
    }
}

async fn handle_connection(
    app: &picoserve::Router<AppRouter, AppState>,
    socket: TcpSocket<NetLock>,
    buf: &mut [u8],
) {
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
