use core::{future::poll_fn, marker::PhantomData, task::Poll};

use rtic_monotonics::{fugit::Duration, Monotonic};
use smoltcp::iface::SocketHandle;
use smoltcp::socket::tcp::State;

use crate::{net_channel::NetChannel, Mono};

pub struct Timer;
pub struct Socket(SocketHandle, &'static NetChannel);
pub struct Read<'a>(SocketHandle, &'static NetChannel, PhantomData<&'a ()>);
pub struct Write<'a>(SocketHandle, &'static NetChannel, PhantomData<&'a ()>);

#[derive(Debug, defmt::Format)]
pub enum Error {
    RecvError(smoltcp::socket::tcp::RecvError),
    SendError(smoltcp::socket::tcp::SendError),
}

impl From<smoltcp::socket::tcp::RecvError> for Error {
    fn from(e: smoltcp::socket::tcp::RecvError) -> Self {
        Self::RecvError(e)
    }
}

impl From<smoltcp::socket::tcp::SendError> for Error {
    fn from(e: smoltcp::socket::tcp::SendError) -> Self {
        Self::SendError(e)
    }
}

impl Socket {
    pub const fn new(handle: SocketHandle, channel: &'static NetChannel) -> Self {
        Self(handle, channel)
    }
}

impl picoserve::io::Error for Error {
    fn kind(&self) -> picoserve::io::ErrorKind {
        picoserve::io::ErrorKind::Other
    }
}

impl picoserve::io::ErrorType for Read<'_> {
    type Error = Error;
}

impl picoserve::io::ErrorType for Write<'_> {
    type Error = Error;
}

impl picoserve::io::Read for Read<'_> {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        if buf.is_empty() {
            return Ok(0);
        }

        let mut read = 0;

        poll_fn(|cx| {
            self.1.with(|_iface, sockets| {
                let socket = sockets.get_mut::<smoltcp::socket::tcp::Socket>(self.0);

                if !socket.can_recv() {
                    socket.register_recv_waker(cx.waker());
                    return Poll::Pending;
                }

                while socket.can_recv() {
                    socket.recv(|data| {
                        let len = data.len().min(buf[read..].len());
                        buf[read..][..len].copy_from_slice(&data[..len]);
                        read += len;
                        (len, ())
                    })?;
                }

                if read == 0 {
                    // todo: assert socket is closed
                }

                Poll::Ready(Ok(read))
            })
        })
        .await
    }
}

impl picoserve::io::Write for Write<'_> {
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        if buf.is_empty() {
            return Ok(0);
        }

        let mut written = 0;

        poll_fn(|cx| {
            self.1.with(|_iface, sockets| {
                let socket = sockets.get_mut::<smoltcp::socket::tcp::Socket>(self.0);

                if !socket.can_send() {
                    socket.register_send_waker(cx.waker());
                    return Poll::Pending;
                }

                while socket.can_send() && written < buf.len() {
                    socket.send(|data| {
                        let len = data.len().min(buf[written..].len());
                        data[..len].copy_from_slice(&buf[written..][..len]);
                        written += len;
                        (len, ())
                    })?
                }

                if written == 0 {
                    // assert socket is closed
                }

                crate::app::poll_network::spawn().ok();

                Poll::Ready(Ok(written))
            })
        })
        .await
    }
}

impl picoserve::io::Socket for Socket {
    type Error = Error;

    type ReadHalf<'a> = Read<'a>;

    type WriteHalf<'a> = Write<'a>;

    fn split(&mut self) -> (Self::ReadHalf<'_>, Self::WriteHalf<'_>) {
        (
            Read(self.0, self.1, PhantomData),
            Write(self.0, self.1, PhantomData),
        )
    }

    async fn shutdown<Timer: picoserve::Timer>(
        self,
        _timeouts: &picoserve::Timeouts<Timer::Duration>,
        _timer: &mut Timer,
    ) -> Result<(), picoserve::Error<Self::Error>> {
        let &Self(handle, channel) = &self;

        poll_fn(|cx| {
            channel.with(|_iface, sockets| {
                let socket = sockets.get_mut::<smoltcp::socket::tcp::Socket>(handle);

                loop {
                    match socket.state() {
                        State::TimeWait | State::Closed => break Poll::Ready(()),
                        State::FinWait1 | State::FinWait2 | State::Closing | State::LastAck => {
                            socket.register_send_waker(cx.waker());
                            break Poll::Pending;
                        }
                        State::CloseWait
                        | State::Established
                        | State::SynReceived
                        | State::SynSent
                        | State::Listen => {
                            socket.close();
                            socket.register_send_waker(cx.waker());
                            crate::app::poll_network::spawn().ok();
                            return Poll::Pending;
                        }
                    }
                }
            })
        })
        .await;

        Ok(())
    }
}

impl Drop for Socket {
    fn drop(&mut self) {
        let &Self(handle, channel) = &*self;

        channel.with(|_iface, sockets| {
            let socket = sockets.get_mut::<smoltcp::socket::tcp::Socket>(handle);
            socket.abort(); // maybe `close` and wait? (in `async fn shutdown`)
        });
    }
}

impl picoserve::Timer for Timer {
    type Duration = Duration<u32, 1, 1000>;

    type TimeoutError = ();

    async fn run_with_timeout<F: core::future::Future>(
        &mut self,
        duration: Self::Duration,
        future: F,
    ) -> Result<F::Output, Self::TimeoutError> {
        let future = async { Ok(future.await) };
        let delay = async { Err(Mono::delay(duration).await) };
        futures_lite::future::or(future, delay).await
    }
}
