// https://github.com/nghiaducnt/LearningRIOT/blob/2019.01-my/cpu/cc2538/stellaris_ether/ethernet.c

use core::cell::RefCell;

use bare_metal::CriticalSection;
use smoltcp::phy::{ChecksumCapabilities, Device, DeviceCapabilities, Medium, RxToken, TxToken};
use smoltcp::time::Instant;

use lm3s6965evb_ethernet_sys::*;

use crate::MAC;

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
#[allow(dead_code)]
pub enum InterruptCause {
    TransmitComplete,
    Receive,
    TransmitError,
    ReceiveError,
}

pub const MTU: usize = 1500;

pub struct SmoltcpDev<'a, M: rtic::Mutex> {
    token: RefCell<M>,
    tx: &'a mut [u8; MTU],
    rx: &'a mut [u8; MTU],
}

pub struct Dev {}

pub struct EthernetRxToken<'a, M: rtic::Mutex>(&'a mut [u8; MTU], &'a RefCell<M>);
pub struct EthernetTxToken<'a, M: rtic::Mutex>(&'a mut [u8; MTU], &'a RefCell<M>);

impl<'a, M: rtic::Mutex<T = Dev>> SmoltcpDev<'a, M> {
    pub fn new(token: M, tx: &'a mut [u8; MTU], rx: &'a mut [u8; MTU]) -> Self {
        Self {
            token: RefCell::new(token),
            tx,
            rx,
        }
    }
}

impl<M: rtic::Mutex<T = Dev>> Device for SmoltcpDev<'_, M> {
    type RxToken<'a>
        = EthernetRxToken<'a, M>
    where
        Self: 'a;
    type TxToken<'a>
        = EthernetTxToken<'a, M>
    where
        Self: 'a;

    // N.B.: Tokens are mutable borrowing `self`, so it is impossible to have
    //       multiple tokens.

    fn receive(&mut self, _: Instant) -> Option<(Self::RxToken<'_>, Self::TxToken<'_>)> {
        let (packet_available, send_space_available) = self
            .token
            .get_mut()
            .lock(|dev| (dev.is_packet_available(), dev.is_send_space_available()));

        if packet_available && send_space_available {
            Some((
                EthernetRxToken(self.rx, &self.token),
                EthernetTxToken(self.tx, &self.token),
            ))
        } else {
            None
        }
    }

    fn transmit(&mut self, _: Instant) -> Option<Self::TxToken<'_>> {
        let send_space_available = self
            .token
            .get_mut()
            .lock(|dev| dev.is_send_space_available());

        if send_space_available {
            Some(EthernetTxToken(self.rx, &self.token))
        } else {
            None
        }
    }

    fn capabilities(&self) -> DeviceCapabilities {
        let mut caps = DeviceCapabilities::default();
        caps.medium = Medium::Ethernet;
        caps.max_transmission_unit = MTU;
        // I don't know what `max_burst_size` is
        caps.max_burst_size = Some(MTU);
        caps.checksum = ChecksumCapabilities::default();
        caps
    }
}

impl<M: rtic::Mutex<T = Dev>> RxToken for EthernetRxToken<'_, M> {
    fn consume<R, F>(self, f: F) -> R
    where
        F: FnOnce(&[u8]) -> R,
    {
        match self.1.borrow_mut().lock(|d| d.packet_get(self.0)) {
            Ok(len) => f(&self.0[..len]),
            Err(code) => {
                defmt::warn!("Packet get error: {}", code);
                f(&[])
            }
        }
    }
}

impl<M: rtic::Mutex<T = Dev>> TxToken for EthernetTxToken<'_, M> {
    fn consume<R, F>(self, length: usize, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        let length = length.min(MTU);

        let result = f(&mut self.0[..length]);

        self.1.borrow_mut().lock(|d| d.send(&self.0[..length]));

        result
    }
}

#[inline(always)]
pub fn isr_handler(mut device: impl rtic::Mutex<T = Dev>) -> Option<InterruptCause> {
    let irq_status = device.lock(|_| {
        unsafe {
            let irq_status = EthernetIntStatus(ETH_BASE, 0);
            EthernetIntClear(ETH_BASE, irq_status);

            if irq_status & ETH_INT_PHY != 0 {
                let mr1 = EthernetPHYRead(ETH_BASE, PHY_MR1 as u8);
                if mr1 & PHY_MR1_LINK != 0 {
                    defmt::info!("Link up");
                    // priv_stellaris_eth_dev.link_up = true;
                    // netdev->event_callback(netdev, NETDEV_EVENT_LINK_UP);
                } else {
                    defmt::info!("Link down");
                    // priv_stellaris_eth_dev.link_up = false;
                    // netdev->event_callback(netdev, NETDEV_EVENT_LINK_DOWN);
                }
            }
            irq_status
        }
    });

    if irq_status & ETH_INT_RXOF != 0 || irq_status & ETH_INT_RXER != 0 {
        defmt::warn!("RXOF or RX");
        // netdev->event_callback(netdev, NETDEV_EVENT_RX_TIMEOUT);
    }

    if irq_status & ETH_INT_RX != 0 {
        // netdev->event_callback(netdev, NETDEV_EVENT_RX_COMPLETE);
        // defmt::info!("RX Complete"); // New packet arrived
    }

    if irq_status & ETH_INT_TX != 0 {
        // netdev->event_callback(netdev, NETDEV_EVENT_TX_COMPLETE);
        // defmt::info!("TX Complete");
    }

    if irq_status & ETH_INT_TXER != 0 {
        // netdev->event_callback(netdev, NETDEV_EVENT_TX_TIMEOUT);
        defmt::warn!("TX Error");
    }

    // netdev->event_callback(netdev, NETDEV_EVENT_ISR);

    // inside `cortexm_isr_end` they always have some kind of barrier
    cortex_m::asm::dsb();
    cortex_m::asm::dmb();

    Some(InterruptCause::Receive)
}

impl Dev {
    pub fn new(_cs: CriticalSection<'_>, #[cfg(feature = "ra6m3")] eth: ra6m3::ETHERC0) -> Self {
        unsafe {
            // I think it should not be 0... Docs say to use `SysCtlClockGet` or hardcode
            EthernetInitExpClk(ETH_BASE, 0);
            EthernetConfigSet(ETH_BASE, ETH_CFG_TX_DPLXEN | ETH_CFG_TX_PADEN);
            EthernetMACAddrSet(ETH_BASE, MAC.as_slice().as_ptr().cast_mut().cast());
            EthernetEnable(ETH_BASE);

            // stellaris_ether_netdev.c:69: dev->irq = ETH_INT_PHY;
            // - what this line means?

            EthernetIntEnable(ETH_BASE, ETH_INT_PHY);
        };

        Self {}
    }
    fn is_packet_available(&mut self) -> bool {
        unsafe { EthernetPacketAvail(ETH_BASE) != 0 }
    }

    fn is_send_space_available(&mut self) -> bool {
        unsafe { EthernetSpaceAvail(ETH_BASE) != 0 }
    }

    fn packet_get(&mut self, buf: &mut [u8; MTU]) -> Result<usize, i32> {
        let ptr = buf.as_mut_ptr();
        let len = buf.len() as i32;
        let len = unsafe { EthernetPacketGetNonBlocking(ETH_BASE, ptr, len) };

        cortex_m::asm::dmb();

        match usize::try_from(len) {
            Ok(len) => Ok(len),
            Err(_) => Err(len),
        }
    }

    fn send(&mut self, packet: &[u8]) {
        let ptr = packet.as_ptr().cast_mut();
        let len = packet.len() as i32;

        cortex_m::asm::dmb();

        let ret = unsafe { EthernetPacketPutNonBlocking(ETH_BASE, ptr, len) };

        if ret == 0 {
            defmt::warn!("Packet sent while no space was available");
        }
    }
}

impl Drop for Dev {
    fn drop(&mut self) {
        unsafe { EthernetDisable(ETH_BASE) }
    }
}

impl rtic::Mutex for Dev {
    type T = Self;

    fn lock<R>(&mut self, f: impl FnOnce(&mut Self::T) -> R) -> R {
        f(self)
    }
}
