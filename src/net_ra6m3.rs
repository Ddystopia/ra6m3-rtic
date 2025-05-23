use crate::{POLL_NETWORK, log::*};

use core::{cell::RefCell, pin::Pin};

use crate::pac;
use crate::pac::Interrupt;
use bare_metal::CriticalSection;
use ra_fsp_rs::{
    ether::{
        self, Buffer, Buffers, Descriptor, EtherConfig, EtherInstance, InterruptCause,
        ether_callback_args_t,
    },
    ether_phy::{self, e_ether_phy_lsi_type, e_ether_phy_mii_type},
};
use smoltcp::{
    phy::{ChecksumCapabilities, Device, DeviceCapabilities, Medium, RxToken, TxToken},
    time::Instant,
};
use static_cell::{ConstStaticCell, StaticCell};

const MTU: usize = 1500;

pub const ETH_N_TX_DESC: usize = 4;
pub const ETH_N_RX_DESC: usize = 4;

static ETH0: ConstStaticCell<EtherInstance<MTU>> =
    ConstStaticCell::new(EtherInstance::<MTU>::new());

static PHY0: ether_phy::ether_phy_instance_t = ra_fsp_rs::const_c_dyn!(ether_phy, &PHY0_CFG);

// todo: table 31.1 shows that hw supports multi-buffer frame transmission and reception.
//       that way, we don't need to have a large buffer for smaller stuff, right? Not sure
//       now it interacts with the limit (8) of buffers.

static PHY0_CFG: ether_phy::EtherPhyConfig = ether_phy::EtherPhyConfig {
    channel: 0,
    phy_lsi_address: 0,
    phy_reset_wait_time: 0x00020000,
    mii_bit_access_wait_time: 8,
    phy_lsi_type: e_ether_phy_lsi_type::ETHER_PHY_LSI_TYPE_DEFAULT,
    flow_control: false,
    mii_type: e_ether_phy_mii_type::ETHER_PHY_MII_TYPE_RMII,
};

static RX_DESCRIPTORS: ConstStaticCell<[Descriptor<MTU>; ETH_N_RX_DESC]> =
    ConstStaticCell::new([const { Descriptor::new() }; ETH_N_RX_DESC]);
static TX_DESCRIPTORS: ConstStaticCell<[Descriptor<MTU>; ETH_N_TX_DESC]> =
    ConstStaticCell::new([const { Descriptor::new() }; ETH_N_TX_DESC]);
static BUFFERS_PLACE: StaticCell<Buffers<MTU, ETH_N_TX_DESC, ETH_N_TX_DESC>> = StaticCell::new();
static TX_BUFFERS: ConstStaticCell<[Buffer<MTU>; ETH_N_TX_DESC]> =
    ConstStaticCell::new([const { Buffer::new() }; ETH_N_TX_DESC]);
static RX_BUFFERS: ConstStaticCell<[Buffer<MTU>; ETH_N_RX_DESC]> =
    ConstStaticCell::new([const { Buffer::new() }; ETH_N_RX_DESC]);

static CONF: ConstStaticCell<EtherConfig<MTU>> = ConstStaticCell::new(
    EtherConfig::new(&PHY0)
        .channel(0)
        .zerocopy()
        .multicast()
        .promiscuous()
        .flow_control()
        .broadcast_filter(0)
        .irq(Interrupt::IEL0)
        .callback(user_ethernet_callback),
);

pub struct Dev {
    eth: RefCell<Pin<&'static mut EtherInstance<MTU>>>,
}

pub struct EthernetRxToken<'a>(
    Option<Pin<&'static mut Buffer<MTU>>>,
    usize,
    &'a RefCell<Pin<&'static mut EtherInstance<MTU>>>,
);

pub struct EthernetTxToken<'a>(
    Option<Pin<&'static mut Buffer<MTU>>>,
    &'a RefCell<Pin<&'static mut EtherInstance<MTU>>>,
);

impl Dev {
    pub fn new(_cs: CriticalSection<'_>, _: pac::EDMAC0) -> Self {
        let conf = CONF.take();

        conf.p_mac_address = {
            static MAC: StaticCell<[u8; 6]> = StaticCell::new();

            let mut mac = crate::MAC;
            mac.reverse();
            MAC.init(mac)
        };
        conf.tx_descriptors = TX_DESCRIPTORS.take();
        conf.rx_descriptors = RX_DESCRIPTORS.take();

        conf.set_buffers(BUFFERS_PLACE.init(Buffers::new(
            TX_BUFFERS.take().each_mut(),
            RX_BUFFERS.take().each_mut(),
        )));

        conf.unchange_irq_priority();

        let before = cortex_m::peripheral::NVIC::get_priority(Interrupt::IEL0);

        let mut eth = Pin::static_mut(ETH0.take());

        eth.as_mut().open(conf).expect("Failed to open ethernet");

        let after = cortex_m::peripheral::NVIC::get_priority(Interrupt::IEL0);

        assert_eq!(before, after);

        info!("Ethernet open() -> Ok(())");

        Self {
            eth: RefCell::new(eth),
        }
    }

    fn eth(&mut self) -> Pin<&mut EtherInstance<MTU>> {
        self.eth.get_mut().as_mut()
    }

    pub fn poll_link(&mut self) {
        if self.eth().get_open() != 0 {
            _ = self.eth().link_process();
        }
    }

    pub fn is_up(&mut self) -> bool {
        self.eth().is_up()
    }

    fn send_buffer(&mut self) -> Option<Pin<&'static mut Buffer<MTU>>> {
        self.eth.borrow_mut().as_mut().take_tx_buf()
    }
}

impl Device for Dev {
    type RxToken<'a> = EthernetRxToken<'a>;
    type TxToken<'a> = EthernetTxToken<'a>;

    // N.B.: Tokens are mutable borrowing `self`, so it is impossible to have
    //       multiple tokens.

    fn receive(&mut self, _: Instant) -> Option<(Self::RxToken<'_>, Self::TxToken<'_>)> {
        let tx = self.send_buffer()?;
        match self.eth().read_zerocopy() {
            Ok((buf, len)) => Some((
                EthernetRxToken(Some(buf), len, &self.eth),
                EthernetTxToken(Some(tx), &self.eth),
            )),
            Err(ether::FSP_ERR_ETHER_ERROR_NO_DATA) => {
                trace!("No data");
                self.eth().as_mut().tx_buffer_update(tx);
                None
            }
            Err(err) => {
                error!("Ethernet read error: {}", err);
                self.eth().as_mut().tx_buffer_update(tx);
                None
            }
        }
    }

    fn transmit(&mut self, _: Instant) -> Option<Self::TxToken<'_>> {
        Some(EthernetTxToken(Some(self.send_buffer()?), &self.eth))
    }

    fn capabilities(&self) -> DeviceCapabilities {
        let mut caps = DeviceCapabilities::default();
        caps.medium = Medium::Ethernet;
        caps.max_transmission_unit = MTU;
        caps.max_burst_size = Some(ETH_N_TX_DESC.min(ETH_N_RX_DESC));
        caps.checksum = ChecksumCapabilities::default();
        caps
    }
}

impl RxToken for EthernetRxToken<'_> {
    fn consume<R, F>(self, f: F) -> R
    where
        F: FnOnce(&[u8]) -> R,
    {
        f(&self.0.as_ref().unwrap()[..self.1])
    }
}

impl Drop for EthernetRxToken<'_> {
    fn drop(&mut self) {
        let buf = self.0.take().unwrap();

        self.2
            .borrow_mut()
            .as_mut()
            .rx_buffer_update(buf)
            .expect("Failed to update buffer")
    }
}

impl TxToken for EthernetTxToken<'_> {
    fn consume<R, F>(mut self, length: usize, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        let mut pin_buf = self.0.take().unwrap();
        let buf = pin_buf.as_mut().as_mut_bytes();
        let result = f(&mut buf[..length]);

        if length < 60 {
            buf[length..60].fill(0);
        }

        match self
            .1
            .borrow_mut()
            .as_mut()
            .write_zerocopy(pin_buf, length.max(60))
        {
            Ok(()) | Err(ether::FSP_ERR_ETHER_ERROR_LINK) => result,
            Err(err) => {
                error!("Failed to write to the network: {err}");
                result
            }
        }
    }
}

impl Drop for EthernetTxToken<'_> {
    fn drop(&mut self) {
        if let Some(buf) = self.0.take() {
            trace!("Dropping Unused TxToken");
            let back = self.1.borrow_mut().as_mut().tx_buffer_update(buf);

            assert!(back.is_none(), "Going to leak the transmit buffer");
        }
    }
}

pub use ether::ether_eint_isr as ethernet_isr_handler;

extern "C" fn user_ethernet_callback(args: &mut ether_callback_args_t) {
    let cause = ether::interrupt_cause(args);

    let receive = cause.receive;
    let transmits = cause.transmits;
    let went_up = cause.went_up;

    crate::app::populate_rx_buffers::spawn(cause).unwrap();

    if receive || transmits || went_up {
        POLL_NETWORK();
    }
}

pub fn populate_rx_buffers(dev: &mut Dev, cause: InterruptCause) {
    if cause.went_up {
        dev.eth().as_mut().update_rx_buffers(cause);
    }
}

#[allow(dead_code)]
pub fn eth0_mac_generate() -> [u8; 6] {
    use sha2::{Digest, Sha256};

    let cpuid = cpuid_get();

    let mut hasher = Sha256::new();
    hasher.update(unsafe { cpuid.align_to() }.1);
    let cpuid_hash = hasher.finalize();

    let mut eth0_mac: [u8; 6] = cpuid_hash[..6].try_into().unwrap();

    // force locally administered unicast address
    eth0_mac[0] &= 0xFC;
    eth0_mac[0] |= 0x02;

    eth0_mac
}

pub fn cpuid_get() -> [u32; 4] {
    // see RA6M3 group reference manual 55.3.4

    // The FMIFRT is a read-only register that stores a base address
    // of the Unique ID register, Part Numbering register and MCU Version register.
    const FMIFRT: *const u32 = 0x407FB19C as *const u32;

    let base = unsafe { FMIFRT.read_volatile() };

    let uidr: *const u32 = (base + 0x14) as *const u32;
    // let pnr: *const u32 = (base + 0x24) as *const u32;
    // let mcuver: *const u32 = (base + 0x44) as *const u32;

    let mut cpuid = [0u32; 4];
    for i in 0..4 {
        cpuid[i] = unsafe { *uidr.offset(i as isize) };
    }

    cpuid
}
