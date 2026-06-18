#![allow(dead_code)]

use embedded_tls::{Aes128GcmSha256, TlsContext, TlsError};
use rand_chacha::ChaCha12Rng;
use rtic::Mutex;

use crate::Net;

use crate::socket::{SocketInner, TcpSocket};

type TlsConnection<'a, M> = embedded_tls::TlsConnection<'a, SocketInner<M>, Aes128GcmSha256>;

pub type Rng = ChaCha12Rng;

pub(crate) struct TlsSocket<'a, M: Mutex<T = Net> + 'static>(pub TlsConnection<'a, M>);

/// Error wrapper mapping `embedded-tls`'s `TlsError` into an `embedded_io_async`
/// error so `TlsSocket` is a valid `minimq::Session` transport.
#[derive(Debug)]
pub struct TlsIoError(pub TlsError);

impl core::fmt::Display for TlsIoError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{:?}", self.0)
    }
}

impl core::error::Error for TlsIoError {}

impl embedded_io_async::Error for TlsIoError {
    fn kind(&self) -> embedded_io_async::ErrorKind {
        embedded_io_async::ErrorKind::Other
    }
}

impl<'a, R: Mutex<T = Net>> TlsSocket<'a, R> {
    pub fn new(
        delagate: TcpSocket<R>,
        record_read_buf: &'a mut [u8],
        record_write_buf: &'a mut [u8],
    ) -> Self {
        Self(TlsConnection::new(
            delagate.into(),
            record_read_buf,
            record_write_buf,
        ))
    }

    pub async fn open<'v, Provider>(
        &mut self,
        ctx: TlsContext<'v, Provider>,
    ) -> Result<(), TlsError>
    where
        Provider: embedded_tls::CryptoProvider<CipherSuite = Aes128GcmSha256>,
    {
        self.0.open::<Provider>(ctx).await
    }

    pub async fn close(self) -> Result<(), TlsError> {
        match self.0.close().await {
            Ok(socket) => Ok(TcpSocket::from(socket).disconnect().await),
            Err((socket, tls_err)) => {
                TcpSocket::from(socket).disconnect().await;
                Err(tls_err)
            }
        }
    }
}

impl<M: Mutex<T = Net>> embedded_io_async::ErrorType for TlsSocket<'_, M> {
    type Error = TlsIoError;
}

impl<M: Mutex<T = Net>> embedded_io_async::Read for TlsSocket<'_, M> {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        self.0.read(buf).await.map_err(TlsIoError)
    }
}

impl<M: Mutex<T = Net>> embedded_io_async::Write for TlsSocket<'_, M> {
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        self.0.write(buf).await.map_err(TlsIoError)
    }

    async fn flush(&mut self) -> Result<(), Self::Error> {
        self.0.flush().await.map_err(TlsIoError)
    }
}
