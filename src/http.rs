use core::{future::poll_fn, task::Poll};

use make_app::AppRouter;
use picoserve::Config;
use smoltcp::{iface::SocketHandle, socket::tcp};
use socket::{Socket, Timer};

use crate::{poll_share::TokenProvider, socket_storage::HTTP_BUFFER_SIZE};

mod socket;

pub type AppState = ();
pub use crate::app::http_type_inference::NetLock;

pub struct Storage {
    pub buf: [u8; HTTP_BUFFER_SIZE],
}

impl Storage {
    pub const fn new() -> Self {
        let buf = [0; HTTP_BUFFER_SIZE];
        Self { buf }
    }
}

pub async fn http(
    net: TokenProvider<NetLock>,
    socket_handle: SocketHandle,
    storage: &'static mut Storage,
) -> ! {
    let app = make_app::make_app();

    loop {
        net.lock(|net| {
            let socket = net.sockets.get_mut::<tcp::Socket>(socket_handle);
            socket.listen(80).unwrap();
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

        handle_connection(&app, socket, &mut storage.buf).await
    }
}

async fn handle_connection(
    app: &picoserve::Router<AppRouter, AppState>,
    socket: Socket<NetLock>,
    buf: &mut [u8],
) {
    let config = Config::new(picoserve::Timeouts {
        start_read_request: None,
        read_request: None,
        write: None,
    });

    match picoserve::serve_with_state(app, Timer, &config, buf, socket, &()).await {
        Ok(count) => defmt::trace!("Handled {} requests", count),
        Err(picoserve::Error::Read(e)) => defmt::error!("Failed to serve with Read Error: {}", e),
        Err(picoserve::Error::Write(e)) => defmt::error!("Failed to serve with Write Error: {}", e),
        Err(picoserve::Error::ReadTimeout) => defmt::error!("Failed to serve with Read Timeout"),
        Err(picoserve::Error::WriteTimeout) => defmt::error!("Failed to serve with Write Timeout"),
    }
}

mod make_app {
    use picoserve::{response::File, routing::get_service};

    use super::AppState;

    pub type AppRouter = impl picoserve::routing::PathRouter<AppState>;

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

    pub fn make_app() -> picoserve::Router<AppRouter, AppState> {
        picoserve::Router::new().route("/", get_service(File::html(HELLO_WORLD)))
    }
}
