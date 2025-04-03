#![allow(dead_code)]

// Credit: https://github.com/embassy-rs/embassy/blob/main/embassy-net/src/tcp.rs

use core::{
    future::poll_fn,
    task::{Context, Poll},
};

use smoltcp::{
    iface::{Interface, SocketHandle},
    socket::tcp::{self, State},
    time::Duration,
    wire::{IpEndpoint, IpListenEndpoint},
};

use crate::poll_share::{NetMutex, TokenProvider};

/// Error returned by TcpSocket read/write functions.
#[derive(PartialEq, Eq, Clone, Copy, Debug, defmt::Format)]
pub enum Error {
    /// The connection was reset.
    ///
    /// This can happen on receiving a RST packet, or on timeout.
    ConnectionReset,
}

/// Error returned by [`TcpSocket::connect`].
#[derive(PartialEq, Eq, Clone, Copy, Debug, defmt::Format)]
pub enum ConnectError {
    /// The socket is already connected or listening.
    InvalidState,
    /// The remote host rejected the connection with a RST packet.
    ConnectionReset,
    /// Connect timed out.
    TimedOut,
    /// No route to host.
    NoRoute,
}

/// Error returned by [`TcpSocket::accept`].
#[derive(PartialEq, Eq, Clone, Copy, Debug, defmt::Format)]
pub enum AcceptError {
    /// The socket is already connected or listening.
    InvalidState,
    /// Invalid listen port
    InvalidPort,
    /// The remote host rejected the connection with a RST packet.
    ConnectionReset,
}

pub(crate) struct SocketInner<R: NetMutex> {
    net: TokenProvider<R>,
    handle: SocketHandle,
}

impl<R: NetMutex> Copy for SocketInner<R> {}
impl<R: NetMutex> Clone for SocketInner<R> {
    fn clone(&self) -> Self {
        *self
    }
}

pub struct TcpSocket<R: NetMutex> {
    io: SocketInner<R>,
}
pub struct TcpReader<'a, R: NetMutex> {
    io: SocketInner<R>,
    _marker: core::marker::PhantomData<&'a ()>,
}
pub struct TcpWriter<'a, R: NetMutex> {
    io: SocketInner<R>,
    _marker: core::marker::PhantomData<&'a ()>,
}

mod asserts {
    #![allow(dead_code)]

    use super::*;

    fn assert_unpin<T: Unpin>(_: &T) {}

    async fn asserts_read_unpin<M: NetMutex>(socket: &mut super::TcpSocket<M>) {
        let mut buf = [];
        let fut = socket.read(&mut buf[..]);
        assert_unpin(&fut);
    }
}

impl<M: NetMutex> SocketInner<M> {
    fn with<R>(&self, f: impl FnOnce(&tcp::Socket, &Interface) -> R) -> R {
        self.net.lock(|net| {
            let socket = net.sockets.get::<tcp::Socket>(self.handle);
            f(socket, &net.iface)
        })
    }

    fn with_mut<R>(&self, f: impl FnOnce(&mut tcp::Socket, &mut Interface) -> R) -> R {
        self.net.lock(|net| {
            let socket = net.sockets.get_mut::<tcp::Socket>(self.handle);
            let res = f(socket, &mut net.iface);
            crate::NET_WAKER.wake_by_ref();
            res
        })
    }

    fn poll_read_ready(&self, cx: &mut Context<'_>) -> Poll<()> {
        self.with_mut(|s, _| {
            if s.can_recv() {
                Poll::Ready(())
            } else {
                s.register_recv_waker(cx.waker());
                Poll::Pending
            }
        })
    }

    fn read<'s>(
        &'s mut self,
        buf: &'s mut [u8],
    ) -> impl Future<Output = Result<usize, Error>> + 's {
        poll_fn(|cx| {
            // CAUTION: smoltcp semantics around EOF are different to what you'd expect
            // from posix-like IO, so we have to tweak things here.
            self.with_mut(|s, _| match s.recv_slice(buf) {
                // Reading into empty buffer
                Ok(0) if buf.is_empty() => {
                    // embedded_io_async::Read's contract is to not block if buf is empty. While
                    // this function is not a direct implementor of the trait method, we still don't
                    // want our future to never resolve.
                    Poll::Ready(Ok(0))
                }
                // No data ready
                Ok(0) => {
                    s.register_recv_waker(cx.waker());
                    Poll::Pending
                }
                // Data ready!
                Ok(n) => Poll::Ready(Ok(n)),
                // EOF
                Err(tcp::RecvError::Finished) => Poll::Ready(Ok(0)),
                // Connection reset. TODO: this can also be timeouts etc, investigate.
                Err(tcp::RecvError::InvalidState) => Poll::Ready(Err(Error::ConnectionReset)),
            })
        })
    }

    fn poll_write_ready(&self, cx: &mut Context<'_>) -> Poll<()> {
        self.with_mut(|s, _| {
            if s.can_send() {
                Poll::Ready(())
            } else {
                s.register_send_waker(cx.waker());
                Poll::Pending
            }
        })
    }

    fn write<'s>(&'s mut self, buf: &'s [u8]) -> impl Future<Output = Result<usize, Error>> + 's {
        poll_fn(|cx| {
            self.with_mut(|s, _| match s.send_slice(buf) {
                // Not ready to send (no space in the tx buffer)
                Ok(0) => {
                    s.register_send_waker(cx.waker());
                    Poll::Pending
                }
                // Some data sent
                Ok(n) => Poll::Ready(Ok(n)),
                // Connection reset. TODO: this can also be timeouts etc, investigate.
                Err(tcp::SendError::InvalidState) => Poll::Ready(Err(Error::ConnectionReset)),
            })
        })
    }

    async fn write_with<F, R>(&mut self, f: F) -> Result<R, Error>
    where
        F: FnOnce(&mut [u8]) -> (usize, R),
    {
        let mut f = Some(f);
        poll_fn(move |cx| {
            self.with_mut(|s, _| {
                if !s.can_send() {
                    if s.may_send() {
                        // socket buffer is full wait until it has atleast one byte free
                        s.register_send_waker(cx.waker());
                        Poll::Pending
                    } else {
                        // if we can't transmit because the transmit half of the duplex connection is closed then return an error
                        Poll::Ready(Err(Error::ConnectionReset))
                    }
                } else {
                    if let Some(f) = f.take() {
                        Poll::Ready(match s.send(f) {
                            // Connection reset. TODO: this can also be timeouts etc, investigate.
                            Err(tcp::SendError::InvalidState) => Err(Error::ConnectionReset),
                            Ok(r) => Ok(r),
                        })
                    } else {
                        defmt::warn!("Poll after completion");
                        Poll::Pending
                    }
                }
            })
        })
        .await
    }

    async fn read_with<F, R>(&mut self, f: F) -> Result<R, Error>
    where
        F: FnOnce(&mut [u8]) -> (usize, R),
    {
        let mut f = Some(f);
        poll_fn(move |cx| {
            self.with_mut(|s, _| {
                if !s.can_recv() {
                    if s.may_recv() {
                        // socket buffer is empty wait until it has atleast one byte has arrived
                        s.register_recv_waker(cx.waker());
                        Poll::Pending
                    } else {
                        // if we can't receive because the receive half of the duplex connection is closed then return an error
                        Poll::Ready(Err(Error::ConnectionReset))
                    }
                } else {
                    if let Some(f) = f.take() {
                        Poll::Ready(match s.recv(f) {
                            // Connection reset. TODO: this can also be timeouts etc, investigate.
                            Err(tcp::RecvError::Finished) | Err(tcp::RecvError::InvalidState) => {
                                Err(Error::ConnectionReset)
                            }
                            Ok(r) => Ok(r),
                        })
                    } else {
                        defmt::warn!("Poll after completion");
                        Poll::Pending
                    }
                }
            })
        })
        .await
    }

    fn flush(&mut self) -> impl Future<Output = Result<(), Error>> + '_ {
        poll_fn(|cx| {
            self.with_mut(|s, _| {
                let data_pending = (s.send_queue() > 0) && s.state() != tcp::State::Closed;
                let fin_pending = matches!(
                    s.state(),
                    tcp::State::FinWait1 | tcp::State::Closing | tcp::State::LastAck
                );
                let rst_pending = s.state() == tcp::State::Closed && s.remote_endpoint().is_some();

                // If there are outstanding send operations, register for wake up and wait
                // smoltcp issues wake-ups when octets are dequeued from the send buffer
                if data_pending || fin_pending || rst_pending {
                    s.register_send_waker(cx.waker());
                    Poll::Pending
                // No outstanding sends, socket is flushed
                } else {
                    Poll::Ready(Ok(()))
                }
            })
        })
    }

    fn recv_capacity(&self) -> usize {
        self.with(|s, _| s.recv_capacity())
    }

    fn send_capacity(&self) -> usize {
        self.with(|s, _| s.send_capacity())
    }

    fn send_queue(&self) -> usize {
        self.with(|s, _| s.send_queue())
    }

    fn recv_queue(&self) -> usize {
        self.with(|s, _| s.recv_queue())
    }
}

impl<M: NetMutex> TcpReader<'_, M> {
    /// Wait until the socket becomes readable.
    ///
    /// A socket becomes readable when the receive half of the full-duplex connection is open
    /// (see [`may_recv()`](TcpSocket::may_recv)), and there is some pending data in the receive buffer.
    ///
    /// This is the equivalent of [read](#method.read), without buffering any data.
    pub fn wait_read_ready(&self) -> impl Future<Output = ()> + '_ {
        poll_fn(move |cx| self.io.poll_read_ready(cx))
    }

    /// Read data from the socket.
    ///
    /// Returns how many bytes were read, or an error. If no data is available, it waits
    /// until there is at least one byte available.
    ///
    /// # Note
    /// A return value of Ok(0) means that we have read all data and the remote
    /// side has closed our receive half of the socket. The remote can no longer
    /// send bytes.
    ///
    /// The send half of the socket is still open. If you want to reconnect using
    /// the socket you split this reader off the send half needs to be closed using
    /// [`abort()`](TcpSocket::abort).
    pub async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Error> {
        self.io.read(buf).await
    }

    /// Call `f` with the largest contiguous slice of octets in the receive buffer,
    /// and dequeue the amount of elements returned by `f`.
    ///
    /// If no data is available, it waits until there is at least one byte available.
    pub async fn read_with<F, R>(&mut self, f: F) -> Result<R, Error>
    where
        F: FnOnce(&mut [u8]) -> (usize, R),
    {
        self.io.read_with(f).await
    }

    /// Return the maximum number of bytes inside the transmit buffer.
    pub fn recv_capacity(&self) -> usize {
        self.io.recv_capacity()
    }

    /// Return the amount of octets queued in the receive buffer. This value can be larger than
    /// the slice read by the next `recv` or `peek` call because it includes all queued octets,
    /// and not only the octets that may be returned as a contiguous slice.
    pub fn recv_queue(&self) -> usize {
        self.io.recv_queue()
    }
}

impl<M: NetMutex> TcpWriter<'_, M> {
    /// Wait until the socket becomes writable.
    ///
    /// A socket becomes writable when the transmit half of the full-duplex connection is open
    /// (see [`may_send()`](TcpSocket::may_send)), and the transmit buffer is not full.
    ///
    /// This is the equivalent of [write](#method.write), without sending any data.
    pub fn wait_write_ready(&self) -> impl Future<Output = ()> + '_ {
        poll_fn(move |cx| self.io.poll_write_ready(cx))
    }

    /// Write data to the socket.
    ///
    /// Returns how many bytes were written, or an error. If the socket is not ready to
    /// accept data, it waits until it is.
    pub fn write<'s>(
        &'s mut self,
        buf: &'s [u8],
    ) -> impl Future<Output = Result<usize, Error>> + 's {
        self.io.write(buf)
    }

    /// Flushes the written data to the socket.
    ///
    /// This waits until all data has been sent, and ACKed by the remote host. For a connection
    /// closed with [`abort()`](TcpSocket::abort) it will wait for the TCP RST packet to be sent.
    pub fn flush(&mut self) -> impl Future<Output = Result<(), Error>> + '_ {
        self.io.flush()
    }

    /// Call `f` with the largest contiguous slice of octets in the transmit buffer,
    /// and enqueue the amount of elements returned by `f`.
    ///
    /// If the socket is not ready to accept data, it waits until it is.
    pub async fn write_with<F, R>(&mut self, f: F) -> Result<R, Error>
    where
        F: FnOnce(&mut [u8]) -> (usize, R),
    {
        self.io.write_with(f).await
    }

    /// Return the maximum number of bytes inside the transmit buffer.
    pub fn send_capacity(&self) -> usize {
        self.io.send_capacity()
    }

    /// Return the amount of octets queued in the transmit buffer.
    pub fn send_queue(&self) -> usize {
        self.io.send_queue()
    }
}

impl<M: NetMutex> TcpSocket<M> {
    pub const fn new(net: TokenProvider<M>, handle: SocketHandle) -> Self {
        Self {
            io: SocketInner { net, handle },
        }
    }

    pub fn destruct(self) -> (TokenProvider<M>, SocketHandle) {
        (self.io.net, self.io.handle)
    }

    /// Return the maximum number of bytes inside the recv buffer.
    pub fn recv_capacity(&self) -> usize {
        self.io.recv_capacity()
    }

    /// Return the maximum number of bytes inside the transmit buffer.
    pub fn send_capacity(&self) -> usize {
        self.io.send_capacity()
    }

    /// Return the amount of octets queued in the transmit buffer.
    pub fn send_queue(&self) -> usize {
        self.io.send_queue()
    }

    /// Return the amount of octets queued in the receive buffer. This value can be larger than
    /// the slice read by the next `recv` or `peek` call because it includes all queued octets,
    /// and not only the octets that may be returned as a contiguous slice.
    pub fn recv_queue(&self) -> usize {
        self.io.recv_queue()
    }

    /// Call `f` with the largest contiguous slice of octets in the transmit buffer,
    /// and enqueue the amount of elements returned by `f`.
    ///
    /// If the socket is not ready to accept data, it waits until it is.
    pub async fn write_with<F, R>(&mut self, f: F) -> Result<R, Error>
    where
        F: FnOnce(&mut [u8]) -> (usize, R),
    {
        self.io.write_with(f).await
    }

    /// Call `f` with the largest contiguous slice of octets in the receive buffer,
    /// and dequeue the amount of elements returned by `f`.
    ///
    /// If no data is available, it waits until there is at least one byte available.
    pub async fn read_with<F, R>(&mut self, f: F) -> Result<R, Error>
    where
        F: FnOnce(&mut [u8]) -> (usize, R),
    {
        self.io.read_with(f).await
    }

    /// Split the socket into reader and a writer halves.
    pub fn split(&mut self) -> (TcpReader<'_, M>, TcpWriter<'_, M>) {
        (
            TcpReader {
                io: self.io,
                _marker: core::marker::PhantomData,
            },
            TcpWriter {
                io: self.io,
                _marker: core::marker::PhantomData,
            },
        )
    }

    /// Connect to a remote host.
    pub async fn connect<T>(
        &mut self,
        remote_endpoint: T,
        local_port: u16,
    ) -> Result<(), ConnectError>
    where
        T: Into<IpEndpoint>,
    {
        match {
            self.io
                .with_mut(|s, i| s.connect(i.context(), remote_endpoint, local_port))
        } {
            Ok(()) => {}
            Err(tcp::ConnectError::InvalidState) => return Err(ConnectError::InvalidState),
            Err(tcp::ConnectError::Unaddressable) => return Err(ConnectError::NoRoute),
        }

        poll_fn(|cx| {
            self.io.with_mut(|s, _| match s.state() {
                tcp::State::Closed | tcp::State::TimeWait => {
                    Poll::Ready(Err(ConnectError::ConnectionReset))
                }
                tcp::State::Listen => unreachable!(),
                tcp::State::SynSent | tcp::State::SynReceived => {
                    s.register_send_waker(cx.waker());
                    Poll::Pending
                }
                _ => Poll::Ready(Ok(())),
            })
        })
        .await
    }

    /// Accept a connection from a remote host.
    ///
    /// This function puts the socket in listening mode, and waits until a connection is received.
    pub async fn accept<T>(&mut self, local_endpoint: T) -> Result<(), AcceptError>
    where
        T: Into<IpListenEndpoint>,
    {
        match self.io.with_mut(|s, _| s.listen(local_endpoint)) {
            Ok(()) => {}
            Err(tcp::ListenError::InvalidState) => return Err(AcceptError::InvalidState),
            Err(tcp::ListenError::Unaddressable) => return Err(AcceptError::InvalidPort),
        }

        poll_fn(|cx| {
            self.io.with_mut(|s, _| match s.state() {
                tcp::State::Listen | tcp::State::SynSent | tcp::State::SynReceived => {
                    s.register_send_waker(cx.waker());
                    Poll::Pending
                }
                _ => Poll::Ready(Ok(())),
            })
        })
        .await
    }

    /// Wait until the socket becomes readable.
    ///
    /// A socket becomes readable when the receive half of the full-duplex connection is open
    /// (see [may_recv](#method.may_recv)), and there is some pending data in the receive buffer.
    ///
    /// This is the equivalent of [read](#method.read), without buffering any data.
    pub fn wait_read_ready(&self) -> impl Future<Output = ()> + '_ {
        poll_fn(move |cx| self.io.poll_read_ready(cx))
    }

    /// Read data from the socket.
    ///
    /// Returns how many bytes were read, or an error. If no data is available, it waits
    /// until there is at least one byte available.
    ///
    /// A return value of Ok(0) means that the socket was closed and is longer
    /// able to receive any data.
    pub fn read<'s>(
        &'s mut self,
        buf: &'s mut [u8],
    ) -> impl Future<Output = Result<usize, Error>> + 's {
        self.io.read(buf)
    }

    /// Wait until the socket becomes writable.
    ///
    /// A socket becomes writable when the transmit half of the full-duplex connection is open
    /// (see [may_send](#method.may_send)), and the transmit buffer is not full.
    ///
    /// This is the equivalent of [write](#method.write), without sending any data.
    pub fn wait_write_ready(&self) -> impl Future<Output = ()> + '_ {
        poll_fn(move |cx| self.io.poll_write_ready(cx))
    }

    /// Write data to the socket.
    ///
    /// Returns how many bytes were written, or an error. If the socket is not ready to
    /// accept data, it waits until it is.
    pub fn write<'s>(
        &'s mut self,
        buf: &'s [u8],
    ) -> impl Future<Output = Result<usize, Error>> + 's {
        self.io.write(buf)
    }

    /// Flushes the written data to the socket.
    ///
    /// This waits until all data has been sent, and ACKed by the remote host. For a connection
    /// closed with [`abort()`](TcpSocket::abort) it will wait for the TCP RST packet to be sent.
    pub fn flush(&mut self) -> impl Future<Output = Result<(), Error>> + '_ {
        self.io.flush()
    }

    /// Set the timeout for the socket.
    ///
    /// If the timeout is set, the socket will be closed if no data is received for the
    /// specified duration.
    ///
    /// # Note:
    /// Set a keep alive interval ([`set_keep_alive`] to prevent timeouts when
    /// the remote could still respond.
    pub fn set_timeout(&mut self, duration: Option<Duration>) {
        self.io.with_mut(|s, _| s.set_timeout(duration))
    }

    /// Set the keep-alive interval for the socket.
    ///
    /// If the keep-alive interval is set, the socket will send keep-alive packets after
    /// the specified duration of inactivity.
    ///
    /// If not set, the socket will not send keep-alive packets.
    ///
    /// By setting a [`timeout`](Self::timeout) larger then the keep alive you
    /// can detect a remote endpoint that no longer answers.
    pub fn set_keep_alive(&mut self, interval: Option<Duration>) {
        self.io.with_mut(|s, _| s.set_keep_alive(interval))
    }

    /// Set the hop limit field in the IP header of sent packets.
    pub fn set_hop_limit(&mut self, hop_limit: Option<u8>) {
        self.io.with_mut(|s, _| s.set_hop_limit(hop_limit))
    }

    /// Get the local endpoint of the socket.
    ///
    /// Returns `None` if the socket is not bound (listening) or not connected.
    pub fn local_endpoint(&self) -> Option<IpEndpoint> {
        self.io.with(|s, _| s.local_endpoint())
    }

    /// Get the remote endpoint of the socket.
    ///
    /// Returns `None` if the socket is not connected.
    pub fn remote_endpoint(&self) -> Option<IpEndpoint> {
        self.io.with(|s, _| s.remote_endpoint())
    }

    /// Get the state of the socket.
    pub fn state(&self) -> State {
        self.io.with(|s, _| s.state())
    }

    /// Close the write half of the socket.
    ///
    /// This closes only the write half of the socket. The read half side remains open, the
    /// socket can still receive data.
    ///
    /// Data that has been written to the socket and not yet sent (or not yet ACKed) will still
    /// still sent. The last segment of the pending to send data is sent with the FIN flag set.
    pub fn close(&mut self) {
        self.io.with_mut(|s, _| s.close())
    }

    pub fn is_open(&self) -> bool {
        self.io.with(|s, _| s.is_open())
    }

    pub async fn disconnect(&mut self) {
        self.io.with_mut(|s, _| s.close());
        poll_fn(|cx| {
            if self.io.with(|s, _| s.is_open()) {
                self.io.with_mut(|s, _| s.register_recv_waker(cx.waker()));
                Poll::Pending
            } else {
                Poll::Ready(())
            }
        })
        .await
    }

    /// Forcibly close the socket.
    ///
    /// This instantly closes both the read and write halves of the socket. Any pending data
    /// that has not been sent will be lost.
    ///
    /// Note that the TCP RST packet is not sent immediately - if the `TcpSocket` is dropped too soon
    /// the remote host may not know the connection has been closed.
    /// `abort()` callers should wait for a [`flush()`](TcpSocket::flush) call to complete before
    /// dropping or reusing the socket.
    pub fn abort(&mut self) {
        self.io.with_mut(|s, _| s.abort())
    }

    /// Return whether the transmit half of the full-duplex connection is open.
    ///
    /// This function returns true if it's possible to send data and have it arrive
    /// to the remote endpoint. However, it does not make any guarantees about the state
    /// of the transmit buffer, and even if it returns true, [write](#method.write) may
    /// not be able to enqueue any octets.
    ///
    /// In terms of the TCP state machine, the socket must be in the `ESTABLISHED` or
    /// `CLOSE-WAIT` state.
    pub fn may_send(&self) -> bool {
        self.io.with(|s, _| s.may_send())
    }

    /// Check whether the transmit half of the full-duplex connection is open
    /// (see [may_send](#method.may_send)), and the transmit buffer is not full.
    pub fn can_send(&self) -> bool {
        self.io.with(|s, _| s.can_send())
    }

    /// return whether the receive half of the full-duplex connection is open.
    /// This function returns true if it’s possible to receive data from the remote endpoint.
    /// It will return true while there is data in the receive buffer, and if there isn’t,
    /// as long as the remote endpoint has not closed the connection.
    pub fn may_recv(&self) -> bool {
        self.io.with(|s, _| s.may_recv())
    }

    /// Get whether the socket is ready to receive data, i.e. whether there is some pending data in the receive buffer.
    pub fn can_recv(&self) -> bool {
        self.io.with(|s, _| s.can_recv())
    }
}

impl<M: NetMutex> From<SocketInner<M>> for TcpSocket<M> {
    fn from(io: SocketInner<M>) -> Self {
        Self { io }
    }
}
impl<M: NetMutex> From<TcpSocket<M>> for SocketInner<M> {
    fn from(this: TcpSocket<M>) -> Self {
        this.io
    }
}

mod embedded_io_impls {
    use super::*;

    impl embedded_io_async::Error for ConnectError {
        fn kind(&self) -> embedded_io_async::ErrorKind {
            match self {
                ConnectError::ConnectionReset => embedded_io_async::ErrorKind::ConnectionReset,
                ConnectError::TimedOut => embedded_io_async::ErrorKind::TimedOut,
                ConnectError::NoRoute => embedded_io_async::ErrorKind::NotConnected,
                ConnectError::InvalidState => embedded_io_async::ErrorKind::Other,
            }
        }
    }

    impl embedded_io_async::Error for Error {
        fn kind(&self) -> embedded_io_async::ErrorKind {
            match self {
                Error::ConnectionReset => embedded_io_async::ErrorKind::ConnectionReset,
            }
        }
    }

    impl<M: NetMutex> embedded_io_async::ErrorType for SocketInner<M> {
        type Error = Error;
    }

    impl<M: NetMutex> embedded_io_async::ErrorType for TcpSocket<M> {
        type Error = Error;
    }

    impl<M: NetMutex> embedded_io_async::Read for SocketInner<M> {
        async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
            self.read(buf).await
        }
    }

    impl<M: NetMutex> embedded_io_async::Write for SocketInner<M> {
        async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
            self.write(buf).await
        }
    }

    impl<M: NetMutex> embedded_io_async::Read for TcpSocket<M> {
        async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
            self.io.read(buf).await
        }
    }

    impl<M: NetMutex> embedded_io_async::ReadReady for TcpSocket<M> {
        fn read_ready(&mut self) -> Result<bool, Self::Error> {
            Ok(self.io.with(|s, _| s.can_recv() || !s.may_recv()))
        }
    }

    impl<M: NetMutex> embedded_io_async::Write for TcpSocket<M> {
        async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
            self.io.write(buf).await
        }

        async fn flush(&mut self) -> Result<(), Self::Error> {
            self.io.flush().await
        }
    }

    impl<M: NetMutex> embedded_io_async::WriteReady for TcpSocket<M> {
        fn write_ready(&mut self) -> Result<bool, Self::Error> {
            Ok(self.io.with(|s, _| s.can_send()))
        }
    }

    impl<M: NetMutex> embedded_io_async::ErrorType for TcpReader<'_, M> {
        type Error = Error;
    }

    impl<M: NetMutex> embedded_io_async::Read for TcpReader<'_, M> {
        async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
            self.io.read(buf).await
        }
    }

    impl<M: NetMutex> embedded_io_async::ReadReady for TcpReader<'_, M> {
        fn read_ready(&mut self) -> Result<bool, Self::Error> {
            Ok(self.io.with(|s, _| s.can_recv() || !s.may_recv()))
        }
    }

    impl<M: NetMutex> embedded_io_async::ErrorType for TcpWriter<'_, M> {
        type Error = Error;
    }

    impl<M: NetMutex> embedded_io_async::Write for TcpWriter<'_, M> {
        async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
            self.io.write(buf).await
        }

        async fn flush(&mut self) -> Result<(), Self::Error> {
            self.io.flush().await
        }
    }

    impl<M: NetMutex> embedded_io_async::WriteReady for TcpWriter<'_, M> {
        fn write_ready(&mut self) -> Result<bool, Self::Error> {
            Ok(self.io.with(|s, _| s.can_send()))
        }
    }
}

mod picoserve_impl {
    use super::*;

    use picoserve::{io::ReadExt, time::Timer};

    async fn run_with_maybe_timeout<T: Timer, F: core::future::Future>(
        this: &mut T,
        duration: Option<T::Duration>,
        future: F,
    ) -> Result<F::Output, T::TimeoutError> {
        if let Some(duration) = duration {
            this.run_with_timeout(duration, future).await
        } else {
            Ok(future.await)
        }
    }

    impl<M: NetMutex> picoserve::io::Socket for TcpSocket<M> {
        type Error = Error;

        type ReadHalf<'a> = TcpReader<'a, M>;

        type WriteHalf<'a> = TcpWriter<'a, M>;

        fn split(&mut self) -> (Self::ReadHalf<'_>, Self::WriteHalf<'_>) {
            self.split()
        }

        async fn shutdown<Timer: picoserve::Timer>(
            mut self,
            timeouts: &picoserve::Timeouts<Timer::Duration>,
            timer: &mut Timer,
        ) -> Result<(), picoserve::Error<Self::Error>> {
            self.close();

            let (mut rx, mut tx) = self.split();

            // Flush the write half until the read half has been closed by the client
            futures_util::future::select(
                core::pin::pin!(async {
                    run_with_maybe_timeout(
                        timer,
                        timeouts.read_request.clone(),
                        rx.discard_all_data(),
                    )
                    .await
                    .map_err(|_err| picoserve::Error::ReadTimeout)?
                    .map_err(picoserve::Error::Read)
                }),
                core::pin::pin!(async {
                    tx.flush().await.map_err(picoserve::Error::Write)?;
                    core::future::pending().await
                }),
            )
            .await
            .factor_first()
            .0?;

            // Flush the write half until the socket is closed.

            run_with_maybe_timeout(timer, timeouts.write.clone(), self.flush())
                .await
                .map_err(|_err| picoserve::Error::WriteTimeout)?
                .map_err(picoserve::Error::Write)
        }
    }
}
