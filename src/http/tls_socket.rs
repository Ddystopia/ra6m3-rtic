#![allow(dead_code)]

use embedded_tls::{Aes128GcmSha256, TlsContext, TlsError};
use rand_core::{CryptoRng, RngCore};
use rtic::Mutex;

use crate::Net;

use super::socket::{Socket, SocketInner, shutdown};

type TlsConnection<'a, R> = embedded_tls::TlsConnection<'a, SocketInner<R>, Aes128GcmSha256>;
type TlsReader<'a, R> = embedded_tls::TlsReader<'a, SocketInner<R>, Aes128GcmSha256>;
type TlsWriter<'a, R> = embedded_tls::TlsWriter<'a, SocketInner<R>, Aes128GcmSha256>;

pub struct Rng;

pub struct TlsSocket<'a, R: Mutex<T = Net> + 'static>(TlsConnection<'a, R>);

impl CryptoRng for Rng {}
impl RngCore for Rng {
    fn next_u32(&mut self) -> u32 {
        todo!()
    }

    fn next_u64(&mut self) -> u64 {
        todo!()
    }

    fn fill_bytes(&mut self, _dest: &mut [u8]) {
        todo!()
    }

    fn try_fill_bytes(&mut self, _dest: &mut [u8]) -> Result<(), rand_core::Error> {
        todo!()
    }
}

impl<R: Mutex<T = Net>> picoserve::io::Socket for TlsSocket<'_, R> {
    type Error = TlsError;

    type ReadHalf<'a>
        = TlsReader<'a, R>
    where
        Self: 'a;

    type WriteHalf<'a>
        = TlsWriter<'a, R>
    where
        Self: 'a;

    fn split(&mut self) -> (Self::ReadHalf<'_>, Self::WriteHalf<'_>) {
        self.0.split()
    }

    async fn shutdown<Timer: picoserve::Timer>(
        self,
        timeouts: &picoserve::Timeouts<Timer::Duration>,
        timer: &mut Timer,
    ) -> Result<(), picoserve::Error<Self::Error>> {
        match self.0.close().await {
            Ok(socket) => Ok(shutdown(socket, timeouts, timer).await),
            Err((socket, tls_err)) => {
                shutdown(socket, timeouts, timer).await;
                Err(picoserve::Error::Write(tls_err))
            }
        }
    }
}

impl<'a, R: Mutex<T = Net>> TlsSocket<'a, R> {
    pub fn new(
        delagate: Socket<R>,
        record_read_buf: &'a mut [u8],
        record_write_buf: &'a mut [u8],
    ) -> Self {
        Self(TlsConnection::new(
            delagate.0,
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
}

