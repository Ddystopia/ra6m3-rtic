use core::{
    convert::Infallible,
    future::{poll_fn, Future},
    pin::pin,
    task::Poll,
};

use make_app::AppRouter;
use picoserve::Config;
use rtic::Mutex;
use smoltcp::{iface::SocketHandle, socket::tcp};
use socket::{Socket, Timer};

use crate::{net_channel::NetChannel, Net};

mod socket;

pub type AppState = ();

pub struct Http {
    socket: SocketHandle,
    channel: &'static NetChannel,
}

pub struct HttpStorage {
    pub channel: NetChannel,
    // pub buffer: [u8; HTTP_BUFFER_SIZE],
    pub http: Option<Http>,
}

impl HttpStorage {
    pub const fn new() -> Self {
        Self {
            channel: NetChannel::new(),
            http: None,
        }
    }
}

impl Http {
    pub fn new(socket: SocketHandle, channel: &'static NetChannel) -> Self {
        Self { socket, channel }
    }
}

pub async fn http(mut ctx: crate::app::http::Context<'_>) -> Infallible {
    let app = make_app::make_app();

    loop {
        poll_fn(|cx| {
            ctx.shared.net.lock(|Net { sockets, http, .. }| {
                let socket = sockets.get_mut::<tcp::Socket>(http.socket);
                socket.listen(80).unwrap();
                socket.register_recv_waker(cx.waker());
                crate::app::poll_network::spawn().ok();
                Poll::Ready(())
            })
        })
        .await;

        let socket = poll_fn(|cx| {
            ctx.shared.net.lock(|Net { sockets, http, .. }| {
                let socket = sockets.get_mut::<tcp::Socket>(http.socket);

                if socket.can_recv() {
                    Poll::Ready(Socket::new(http.socket, http.channel))
                } else {
                    socket.register_recv_waker(cx.waker());
                    Poll::Pending
                }
            })
        })
        .await;

        let mut fut = pin!(handle_connection(&app, socket));

        poll_fn(|cx| {
            ctx.shared.net.lock(
                |Net {
                     iface,
                     sockets,
                     http,
                     ..
                 }| {
                    let poll = http.channel.feed(iface, sockets, |_| fut.as_mut().poll(cx));
                    crate::app::poll_network::spawn().ok();
                    poll
                },
            )
        })
        .await
    }
}

async fn handle_connection(app: &picoserve::Router<AppRouter, AppState>, socket: Socket) {
    let config = Config::new(picoserve::Timeouts {
        start_read_request: None,
        read_request: None,
        write: None,
    });

    let mut buf = [0; 1024];

    match picoserve::serve_with_state(app, Timer, &config, &mut buf, socket, &()).await {
        Ok(count) => defmt::debug!("Handled {} requests", count),
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
