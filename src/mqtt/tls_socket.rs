#![allow(dead_code)]

use embedded_tls::{Aes128GcmSha256, TlsContext, TlsError};
use rand_chacha::ChaCha12Rng;
use rtic::Mutex;

use crate::Net;

use super::socket::{SocketInner, TcpSocket};

type TlsConnection<'a, M> = embedded_tls::TlsConnection<'a, SocketInner<M>, Aes128GcmSha256>;
type TlsReader<'a, M> = embedded_tls::TlsReader<'a, SocketInner<M>, Aes128GcmSha256>;
type TlsWriter<'a, M> = embedded_tls::TlsWriter<'a, SocketInner<M>, Aes128GcmSha256>;

pub type Rng = ChaCha12Rng;

pub(crate) struct TlsSocket<'a, M: Mutex<T = Net> + 'static>(pub TlsConnection<'a, M>);

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
