use crate::POLL_NETWORK;

use crate::pac::Interrupt;
use cortex_m::peripheral::NVIC;
use ra_fsp_rs::{
    DriverPlace,
    ether::{self, Buffer, Buffers, Descriptor, Ether, EtherConfig},
    ether_phy::{self, e_ether_phy_lsi_type, e_ether_phy_mii_type},
    pac,
    state_markers::{Closed, Opened},
};
use rtic::export::CriticalSection;
use smoltcp::phy::{ChecksumCapabilities, DeviceCapabilities, Medium};
use static_cell::{ConstStaticCell, StaticCell};

pub type Dev = ra_fsp_rs::smoltcp::ether::Dev;

const MTU: usize = 1504;

pub const ETH_N_TX_DESC: usize = 4;
pub const ETH_N_RX_DESC: usize = 4;

static ETH0: DriverPlace<Ether<Opened>> = DriverPlace::new();
static PHY0: DriverPlace<ether_phy::EtherPhy<Closed>> = DriverPlace::new();

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

// todo: move that stuff to hsram. They are already there, but it should be guaranteed.

static RX_DESCRIPTORS: ConstStaticCell<[Descriptor; ETH_N_RX_DESC]> =
    ConstStaticCell::new([const { Descriptor::new() }; _]);
static TX_DESCRIPTORS: ConstStaticCell<[Descriptor; ETH_N_TX_DESC]> =
    ConstStaticCell::new([const { Descriptor::new() }; _]);
static BUFFERS_PLACE: StaticCell<Buffers<ETH_N_TX_DESC, ETH_N_TX_DESC>> = StaticCell::new();
static TX_BUFFERS: ConstStaticCell<[Buffer<[u8; MTU]>; ETH_N_TX_DESC]> =
    ConstStaticCell::new([const { Buffer::new() }; _]);
static RX_BUFFERS: ConstStaticCell<[Buffer<[u8; MTU]>; ETH_N_RX_DESC]> =
    ConstStaticCell::new([const { Buffer::new() }; _]);

struct NetCallback;

impl ra_fsp_rs::Callback<ether::InterruptCause, Ether<'static, Opened>> for NetCallback {
    #[unsafe(link_section = ".code_in_ram")]
    fn call_with_block(
        _context: &Self,
        block: core::pin::Pin<&mut Ether<'static, Opened>>,
        cause: ether::InterruptCause,
    ) {
        let receive = cause.receive;
        let transmits = cause.transmits;
        let went_up = cause.went_up;

        // Re-arms the RX ring only if FSP actually reset it on a link-up; the
        // decision is derived from the driver's ring state, so this is safe to
        // call unconditionally and `cause` is no longer trusted for it.
        block.update_rx_buffers();

        if receive || transmits || went_up {
            POLL_NETWORK();
        }
    }
}

pub fn create_dev(
    _cs: CriticalSection<'_>,
    _nvic: &mut NVIC,
    edmac: pac::EDMAC0,
    etherc: pac::ETHERC0,
) -> Dev {
    let initializer = ether_phy::EtherPhy::new_closed(edmac, PHY0_CFG);
    let Ok(phy) = PHY0.write_pin_init(initializer);

    let mut conf = EtherConfig::new(phy)
        .channel(0)
        .zerocopy()
        .multicast()
        .promiscuous()
        .flow_control()
        .broadcast_filter(0)
        .irq(Interrupt::IEL0);

    conf.p_mac_address = {
        static MAC: StaticCell<[u8; 6]> = StaticCell::new();

        let mut mac = crate::conf::MAC;
        mac.reverse();
        MAC.init(mac)
    };
    conf.tx_descriptors = TX_DESCRIPTORS.take();
    conf.rx_descriptors = RX_DESCRIPTORS.take();

    conf.set_buffers(BUFFERS_PLACE.init(Buffers::new(
        TX_BUFFERS.take().each_mut(),
        RX_BUFFERS.take().each_mut(),
    )));

    let before = cortex_m::peripheral::NVIC::get_priority(Interrupt::IEL0);

    let initializer = Ether::<Opened>::new_open(etherc, conf);

    let mut eth = ETH0
        .write_pin_init(initializer)
        .expect("Failed to open ethernet");

    eth.as_mut()
        .callback_set(&NetCallback)
        .expect("Failed to set ethernet callback");

    log::info!("Ethernet open() -> Ok(())");

    let after = cortex_m::peripheral::NVIC::get_priority(Interrupt::IEL0);

    assert_eq!(before, after);

    let mut caps = DeviceCapabilities::default();
    caps.medium = Medium::Ethernet;
    caps.max_transmission_unit = MTU;
    caps.max_burst_size = Some(ETH_N_TX_DESC.min(ETH_N_RX_DESC));
    caps.checksum = ChecksumCapabilities::default();

    ra_fsp_rs::smoltcp::ether::Dev::new(eth, caps)
}

/*
pub fn eth0_mac_generate() -> [u8; 6] {
    use sha2::{Digest, Sha256};

    let cpuid = cpuid_get();

    let mut hasher = Sha256::new();
    hasher.update(&cpuid[0].to_le_bytes());
    hasher.update(&cpuid[1].to_le_bytes());
    hasher.update(&cpuid[2].to_le_bytes());
    hasher.update(&cpuid[3].to_le_bytes());
    let cpuid_hash = hasher.finalize();

    let mut eth0_mac: [u8; 6] = cpuid_hash[..6].try_into().unwrap();

    // force locally administered unicast address
    eth0_mac[0] &= 0xFC;
    eth0_mac[0] |= 0x02;

    eth0_mac
}
*/
