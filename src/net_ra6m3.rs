use crate::POLL_NETWORK;

use core::pin::Pin;

use crate::pac::Interrupt;
use bare_metal::CriticalSection;
use cortex_m::peripheral::NVIC;
use ra_fsp_rs::{
    ether::{self, Buffer, Buffers, Descriptor, EtherConfig, EtherInstance, ether_callback_args_t},
    ether_phy::{self, e_ether_phy_lsi_type, e_ether_phy_mii_type},
};
use smoltcp::phy::{ChecksumCapabilities, DeviceCapabilities, Medium};
use static_cell::{ConstStaticCell, StaticCell};

pub type Dev = ra_fsp_rs::smoltcp::ether::Dev<MTU>;

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

// `core::array::from_fn` is not const yet

static RX_DESCRIPTORS: ConstStaticCell<[Descriptor<MTU>; ETH_N_RX_DESC]> =
    ConstStaticCell::new([const { Descriptor::new() }; _]);
static TX_DESCRIPTORS: ConstStaticCell<[Descriptor<MTU>; ETH_N_TX_DESC]> =
    ConstStaticCell::new([const { Descriptor::new() }; _]);
static BUFFERS_PLACE: StaticCell<Buffers<MTU, ETH_N_TX_DESC, ETH_N_TX_DESC>> = StaticCell::new();
static TX_BUFFERS: ConstStaticCell<[Buffer<MTU>; ETH_N_TX_DESC]> =
    ConstStaticCell::new([const { Buffer::new() }; _]);
static RX_BUFFERS: ConstStaticCell<[Buffer<MTU>; ETH_N_RX_DESC]> =
    ConstStaticCell::new([const { Buffer::new() }; _]);

static CONF: ConstStaticCell<EtherConfig<MTU>> = ConstStaticCell::new(
    EtherConfig::new(&PHY0)
        .channel(0)
        .zerocopy()
        .multicast()
        .promiscuous()
        .flow_control()
        .broadcast_filter(0)
        .irq(Interrupt::IEL0)
        .callback({
            extern "C" fn user_ethernet_callback(args: &mut ether_callback_args_t) {
                let cause = ether::interrupt_cause(args);

                let receive = cause.receive;
                let transmits = cause.transmits;
                let went_up = cause.went_up;

                if went_up || transmits {
                    // We need to access `Dev`.
                    crate::app::populate_rx_buffers::spawn(cause).expect(
                        "
`populate_rx_buffers` should have enough priority to preempt\
the current task, so we should always be able to spawn it.
",
                    )
                }

                if receive || transmits || went_up {
                    POLL_NETWORK();
                }
            }

            user_ethernet_callback
        }),
);

pub use ether::ether_eint_isr as ethernet_isr_handler;

pub fn create_dev(_cs: CriticalSection<'_>, nvic: &mut NVIC) -> Dev {
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

    eth.as_mut()
        .open(conf, nvic)
        .expect("Failed to open ethernet");

    let after = cortex_m::peripheral::NVIC::get_priority(Interrupt::IEL0);

    assert_eq!(before, after);

    let mut caps = DeviceCapabilities::default();
    caps.medium = Medium::Ethernet;
    caps.max_transmission_unit = MTU;
    caps.max_burst_size = Some(ETH_N_TX_DESC.min(ETH_N_RX_DESC));
    caps.checksum = ChecksumCapabilities::default();

    ra_fsp_rs::smoltcp::ether::Dev::new(eth, caps)
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
