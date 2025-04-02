use futures_concurrency::prelude::*;
use futures_lite::prelude::*;
use picoserve::{Config, response::File, routing::get_service};
use rtic_monotonics::fugit::ExtU32;
use smoltcp::iface::SocketHandle;
use timer::Timer;

use crate::{
    poll_share::{self, TokenProvider},
    socket_storage::HTTP_BUFFER_SIZE,
};

// todo: use https://docs.rs/smoltcp/latest/smoltcp/socket/tcp/struct.Socket.html#method.set_timeout and others

// note: for some reason, only one worker is heavily used, while the other very rarely
//       is it a bug or is it expected? I only tested that hello world, try with real
//       front-end and the browser


// todo: maybe instruct handler wether to keep alive or not. Maybe we can track the number of available
//       sockets and if there are more than 1, we can keep the connection alive

mod timer;

pub const WORKERS: usize = 2;

pub type AppState = ();
pub type NetLock = impl rtic::Mutex<T = crate::Net> + 'static;
pub type AppRouter = impl picoserve::routing::PathRouter<AppState>;

type TcpSocket = crate::socket::TcpSocket<NetLock>;

pub struct Storage {
    pub app: Option<picoserve::Router<AppRouter, AppState>>,
    pub buf: [[u8; HTTP_BUFFER_SIZE]; WORKERS],
    pub token_place: poll_share::TokenProviderPlace<NetLock>,
}

impl Storage {
    pub const fn new() -> Self {
        Self {
            app: None,
            buf: [[0; HTTP_BUFFER_SIZE]; WORKERS],
            token_place: poll_share::TokenProviderPlace::new(),
        }
    }
}

#[define_opaque(NetLock)]
pub async fn http(
    ctx: crate::app::http_task::Context<'static>,
    socket_handles: [SocketHandle; WORKERS],
) -> ! {
    let storage = ctx.local.storage;
    let net = TokenProvider::new(&mut storage.token_place, ctx.shared.net);
    let app = storage.app.get_or_insert(make_app());

    let accepted = crate::mpmc::MpMcChannel::new();
    let handled = crate::mpmc::MpMcChannel::new();

    for socket_handle in socket_handles {
        let socket = TcpSocket::new(net, socket_handle);
        handled
            .try_send(socket)
            .expect("Channel capacity and sockets count must match");
    }

    let mut handlers: [_; WORKERS] = [const { None }; WORKERS];
    for (i, buf) in storage.buf.iter_mut().enumerate() {
        handlers[i] = Some(hander(i, &app, &accepted, &handled, buf));
    }
    let handlers = handlers.map(|x| x.expect("We just initialized it")).join();
    let acceptor = async {
        loop {
            let mut socket = handled.recv().await;
            assert!(!socket.is_open(), "Handled socket must be closed");

            socket.accept(80).await.expect("Port 80 is a valid port");
            accepted.send(socket).await;
        }
    };

    handlers.or(acceptor).await[0]
}

async fn hander(
    i: usize,
    app: &picoserve::Router<AppRouter, AppState>,
    accepted: &crate::mpmc::MpMcChannel<TcpSocket, WORKERS>,
    handled: &crate::mpmc::MpMcChannel<TcpSocket, WORKERS>,
    buffer: &mut [u8],
) -> ! {
    loop {
        let socket = accepted.recv().await;
        defmt::info!("Worker {}: accepted socket", i);
        handle_connection(&app, socket.clone(), buffer).await;
        handled.send(socket).await;
    }
}

async fn handle_connection(
    app: &picoserve::Router<AppRouter, AppState>,
    socket: TcpSocket,
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
