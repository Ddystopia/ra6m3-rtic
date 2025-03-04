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

const MTU: usize = 1500;

pub struct Dev {
    rx: [u8; MTU],
    tx: [u8; MTU],
}

pub struct EthernetRxToken<'a>(&'a mut [u8; MTU]);
pub struct EthernetTxToken<'a>(&'a mut [u8; MTU]);

impl Dev {
    pub fn new(cs: CriticalSection<'_>) -> Self {
        init(cs);

        Self {
            rx: [0; MTU],
            tx: [0; MTU],
        }
    }
}

impl Drop for Dev {
    fn drop(&mut self) {
        disable()
    }
}

impl Device for Dev {
    type RxToken<'a> = EthernetRxToken<'a>;
    type TxToken<'a> = EthernetTxToken<'a>;

    // N.B.: Tokens are mutable borrowing `self`, so it is impossible to have
    //       multiple tokens.

    fn receive(&mut self, _: Instant) -> Option<(Self::RxToken<'_>, Self::TxToken<'_>)> {
        if is_packet_available() && is_send_space_available() {
            Some((EthernetRxToken(&mut self.rx), EthernetTxToken(&mut self.tx)))
        } else {
            None
        }
    }

    fn transmit(&mut self, _: Instant) -> Option<Self::TxToken<'_>> {
        if is_send_space_available() {
            Some(EthernetTxToken(&mut self.rx))
        } else {
            None
        }
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

impl RxToken for EthernetRxToken<'_> {
    fn consume<R, F>(self, f: F) -> R
    where
        F: FnOnce(&[u8]) -> R,
    {
        let len = packet_get(self.0);
        let len = usize::try_from(len).unwrap_or(MTU);

        f(&self.0[..len])
    }
}

impl TxToken for EthernetTxToken<'_> {
    fn consume<R, F>(self, length: usize, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        assert!(length <= MTU);

        let result = f(&mut self.0[..length]);

        assert!(is_send_space_available());

        send(&self.0[..length]);

        result
    }
}

fn init(_cs: CriticalSection<'_>) {
    unsafe {
        // I think it should not be 0... Docs say to use `SysCtlClockGet` or hardcode
        EthernetInitExpClk(ETH_BASE, 0);
        EthernetConfigSet(ETH_BASE, ETH_CFG_TX_DPLXEN | ETH_CFG_TX_PADEN);
        EthernetMACAddrSet(ETH_BASE, MAC.as_slice().as_ptr().cast_mut().cast());
        EthernetEnable(ETH_BASE);

        // stellaris_ether_netdev.c:69: dev->irq = ETH_INT_PHY;
        // - what this line means?

        EthernetIntEnable(ETH_BASE, ETH_INT_PHY);
    }
}

#[inline(always)]
pub fn isr_handler(device: &mut Dev) -> Option<InterruptCause> {
    unsafe { isr_handler_inner(device) }
}

#[inline(always)]
unsafe fn isr_handler_inner(_device: &mut Dev) -> Option<InterruptCause> {
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

fn disable() {
    unsafe { EthernetDisable(ETH_BASE) }
}

fn is_packet_available() -> bool {
    unsafe { EthernetPacketAvail(ETH_BASE) != 0 }
}

fn is_send_space_available() -> bool {
    unsafe { EthernetSpaceAvail(ETH_BASE) != 0 }
}

fn packet_get(buf: &mut [u8; MTU]) -> isize {
    let ptr = buf.as_mut_ptr();
    let len = buf.len() as i32;
    let len = unsafe { EthernetPacketGetNonBlocking(ETH_BASE, ptr, len) };

    if len == 0 {
        defmt::warn!("No packet available");
        return 0;
    }

    assert!(len < 0 || len as usize <= buf.len());

    cortex_m::asm::dmb();

    len as isize
}

fn send(packet: &[u8]) {
    let ptr = packet.as_ptr().cast_mut();
    let len = packet.len() as i32;

    cortex_m::asm::dmb();

    let ret = unsafe { EthernetPacketPutNonBlocking(ETH_BASE, ptr, len) };

    assert!(ret > 0);

    if ret == 0 {
        defmt::warn!("Packet sent while no space was available");
    }
}
