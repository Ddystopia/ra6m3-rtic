use core::mem::MaybeUninit;

use picoserve::{Config, response::File, routing::get_service};
use smoltcp::iface::SocketHandle;
use timer::Timer;

use crate::{
    TimeExt,
    log::*,
    poll_share::{self, TokenProvider},
    socket::TcpSocket,
    socket_storage::HTTP_BUFFER_SIZE,
};

// todo: use https://docs.rs/smoltcp/latest/smoltcp/socket/tcp/struct.Socket.html#method.set_timeout and others

mod timer;

pub type AppState = ();
pub type NetLock = impl rtic::Mutex<T = crate::Net> + 'static;
pub type AppRouter = impl picoserve::routing::PathRouter<AppState>;

pub struct Storage {
    pub app: MaybeUninit<picoserve::Router<AppRouter, AppState>>,
    pub buf: [u8; HTTP_BUFFER_SIZE],
    pub token_place: poll_share::TokenProviderPlace<NetLock>,
}

impl Storage {
    pub const fn new() -> Self {
        Self {
            app: MaybeUninit::uninit(),
            buf: [0; HTTP_BUFFER_SIZE],
            token_place: poll_share::TokenProviderPlace::new(),
        }
    }
}

#[define_opaque(NetLock)]
pub async fn http(ctx: crate::app::http_task::Context<'static>, socket_handle: SocketHandle) -> ! {
    let storage = ctx.local.storage;
    let net = TokenProvider::new(&mut storage.token_place, ctx.shared.net);
    let app = storage.app.write(make_app());

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
        // https://github.com/sammhicks/picoserve/issues/83
        persistent_start_read_request: Some(100.millis()),
    });

    match picoserve::serve_with_state(app, Timer, &config, buf, socket, &()).await {
        Ok(count) => trace!("Handled {} requests", count),
        Err(picoserve::Error::Read(e)) => error!("Failed to serve with Read Error: {}", e),
        Err(picoserve::Error::Write(e)) => error!("Failed to serve with Write Error: {}", e),
        Err(picoserve::Error::ReadTimeout) => error!("Failed to serve with Read Timeout"),
        Err(picoserve::Error::WriteTimeout) => error!("Failed to serve with Write Timeout"),
    }
}

#[define_opaque(AppRouter)]
pub fn make_app() -> picoserve::Router<AppRouter, AppState> {
    const HELLO_WORLD: &str = r#"<!DOCTYPE html>
<html>
<head>
    <title>RA6M3 Picoserve</title>
    <meta charset="utf-8">
</head>
<body>
    <h1>Welcome to RA6M3 Picoserve</h1>
    <p>This is a simple static page page.</p>
</body>
</html>
"#;

    picoserve::Router::new().route("/", get_service(File::html(HELLO_WORLD)))
}
