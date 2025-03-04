#![allow(dead_code)]
#![allow(unused_variables)]

use smoltcp::phy::{ChecksumCapabilities, Device, DeviceCapabilities, Medium, RxToken, TxToken};
use smoltcp::time::Instant;

const MTU: usize = 1500;

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum InterruptCause {
    TransmitComplete,
    Receive,
    TransmitError,
    ReceiveError,
}

pub struct Dev {
    etherc0: ra6m3::ETHERC0,
}

pub struct EthernetRxToken();
pub struct EthernetTxToken();

impl Dev {
    pub fn new(etherc0: ra6m3::ETHERC0) -> Self {
        Self { etherc0 }
    }
}

impl Drop for Dev {
    fn drop(&mut self) {}
}

impl Device for Dev {
    type RxToken<'a> = EthernetRxToken;
    type TxToken<'a> = EthernetTxToken;

    // N.B.: Tokens are mutable borrowing `self`, so it is impossible to have
    //       multiple tokens.

    fn receive(&mut self, _: Instant) -> Option<(Self::RxToken<'_>, Self::TxToken<'_>)> {
        None
    }

    fn transmit(&mut self, _: Instant) -> Option<Self::TxToken<'_>> {
        None
    }

    fn capabilities(&self) -> DeviceCapabilities {
        let mut caps = DeviceCapabilities::default();
        caps.medium = Medium::Ethernet;
        caps.max_transmission_unit = MTU;
        caps.max_burst_size = Some(MTU);
        caps.checksum = ChecksumCapabilities::default();
        caps
    }
}

impl RxToken for EthernetRxToken {
    fn consume<R, F>(self, f: F) -> R
    where
        F: FnOnce(&[u8]) -> R,
    {
        todo!()
    }
}

impl TxToken for EthernetTxToken {
    fn consume<R, F>(self, length: usize, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        todo!()
    }
}

pub fn isr_handler(_device: &mut Dev) -> Option<InterruptCause> {
    todo!()
}
