use crate::log::*;
use core::mem::MaybeUninit;
use core::ptr;

// todo: map
//       [VECTOR_NUMBER_EDMAC0_EINT]             = ELC_EVENT_EDMAC0_EINT,

use bare_metal::CriticalSection;
use smoltcp::phy::{ChecksumCapabilities, Device, DeviceCapabilities, Medium, RxToken, TxToken};
use smoltcp::time::Instant;

const MTU: usize = 1500;

pub struct Dev {
    rx: [u8; MTU],
    tx: [u8; MTU],
    tx_free: bool,
}

pub struct InterruptCause {
    pub receive: bool,
    pub transmit_complete: bool,
}

pub struct EthernetRxToken<'a>(&'a mut [u8; MTU]);
pub struct EthernetTxToken<'a>(&'a mut [u8; MTU]);

impl Dev {
    pub fn new(cs: CriticalSection<'_>, _: ra_fsp_sys::ETHERC0) -> Self {
        init(cs, crate::MAC);

        Self {
            rx: [0; MTU],
            tx: [0; MTU],
            tx_free: true,
        }
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

extern "C" fn callback(_: *mut ra_fsp_ethernet::st_ether_callback_args) {}

unsafe extern "C" {
    fn IEL0();
}

const IP_PACKET_SIZE: u32 = MTU as u32 - 18;

const ISR: u32 = 0;

const ETH_N_TX_DESC: usize = 4;
const ETH_N_RX_DESC: usize = 4;

#[repr(C, align(16))]
struct EtherInstanceDescriptor<const N: usize> {
    descriptors: MaybeUninit<[ra_fsp_ethernet::ether_instance_descriptor_t; N]>,
}

// #if (BSP_FEATURE_TZ_HAS_TRUSTZONE == 1) && (BSP_TZ_SECURE_BUILD != 1) && (BSP_TZ_NONSECURE_BUILD != 1)
//    place in ".ns_buffer.eth" section

#[cfg_attr(any(), link_section = ".ns_buffer.eth")]
static mut MAC0_TX_DESCRIPTORS: EtherInstanceDescriptor<ETH_N_TX_DESC> = EtherInstanceDescriptor {
    descriptors: MaybeUninit::uninit(),
};
#[cfg_attr(any(), link_section = ".ns_buffer.eth")]
static mut MAC0_RX_DESCRIPTORS: EtherInstanceDescriptor<ETH_N_RX_DESC> = EtherInstanceDescriptor {
    descriptors: MaybeUninit::uninit(),
};

static CONF_EXTEND: ra_fsp_ethernet::ether_extended_cfg_t = ra_fsp_ethernet::ether_extended_cfg_t {
    p_rx_descriptors: (&raw mut MAC0_RX_DESCRIPTORS).cast(),
    p_tx_descriptors: (&raw mut MAC0_TX_DESCRIPTORS).cast(),
};

static mut CONF: ra_fsp_ethernet::ether_cfg_t = ra_fsp_ethernet::ether_cfg_t {
    channel: 0,
    zerocopy: ra_fsp_ethernet::e_ether_zerocopy_ETHER_ZEROCOPY_ENABLE,
    multicast: ra_fsp_ethernet::e_ether_multicast_ETHER_MULTICAST_ENABLE,
    promiscuous: ra_fsp_ethernet::e_ether_promiscuous_ETHER_PROMISCUOUS_ENABLE,
    flow_control: ra_fsp_ethernet::e_ether_flow_control_ETHER_FLOW_CONTROL_ENABLE,
    padding: ra_fsp_ethernet::e_ether_padding_ETHER_PADDING_2BYTE,
    padding_offset: 14,
    broadcast_filter: 0,
    p_mac_address: ptr::null_mut(), // inserted on init
    num_tx_descriptors: ETH_N_TX_DESC as u8,
    num_rx_descriptors: ETH_N_RX_DESC as u8,
    ether_buffer_size: IP_PACKET_SIZE,
    irq: 0,                // see EVENT_EDMAC0_EINT position in g_interrupt_event_link_select
    interrupt_priority: 0, // todo: before init, read the priority of that interrupt, and restore it after, asserting that it is 0 in the meantime
    p_callback: None,
    p_ether_phy_instance: &raw const PHY0,
    p_context: ptr::null_mut(),
    p_extend: (&raw const CONF_EXTEND).cast(),
    pp_ether_buffers: ptr::null_mut(),
};

static mut CTRL: MaybeUninit<ra_fsp_ethernet::ether_instance_ctrl_t> = MaybeUninit::zeroed();
static mut PHY0_CTRL: MaybeUninit<ra_fsp_ethernet::ether_phy_instance_ctrl_t> =
    MaybeUninit::zeroed();

static PHY0_CFG: ra_fsp_ethernet::ether_phy_cfg_t = ra_fsp_ethernet::ether_phy_cfg_t {
    channel: 0,
    phy_lsi_address: 1,
    phy_reset_wait_time: 0x00020000,
    mii_bit_access_wait_time: 8,
    phy_lsi_type: ra_fsp_ethernet::e_ether_phy_lsi_type_ETHER_PHY_LSI_TYPE_DP83620,
    flow_control: ra_fsp_ethernet::e_ether_phy_flow_control_ETHER_PHY_FLOW_CONTROL_DISABLE,
    mii_type: ra_fsp_ethernet::e_ether_phy_mii_type_ETHER_PHY_MII_TYPE_RMII,
    p_context: ptr::null_mut(),
    p_extend: ptr::null_mut(),
};

static ETHER: ra_fsp_ethernet::ether_instance_t = ra_fsp_ethernet::ether_instance_t {
    p_ctrl: (&raw mut CTRL).cast(),
    p_cfg: &raw const CONF,
    p_api: &raw const ra_fsp_ethernet::g_ether_on_ether,
};

static PHY0: ra_fsp_ethernet::ether_phy_instance_t = ra_fsp_ethernet::ether_phy_instance_t {
    p_ctrl: (&raw mut PHY0_CTRL).cast(),
    p_cfg: &raw const PHY0_CFG,
    p_api: &raw const ra_fsp_ethernet::g_ether_phy_on_ether_phy,
};

fn init(_cs: CriticalSection<'_>, mac: [u8; 6]) {
    static mut MAC: [u8; 6] = [0; 6];

    unsafe {
        ptr::write(&raw mut MAC, mac);
        MAC.reverse(); // NIC has reversed bytes order
        ptr::write(&raw mut CONF.p_mac_address, (&raw mut MAC).cast());

        let res: u32 = ((*ETHER.p_api).open.unwrap())(ETHER.p_ctrl, ETHER.p_cfg);

        assert_eq!(res, 0);

        loop {
            /* When the Ethernet link status read from the PHY-LSI Basic Status register is link-up,
             * Initializes the module and make auto negotiation. */

            let res: u32 = ((*ETHER.p_api).linkProcess.unwrap())(ETHER.p_ctrl);

            if res == 0 {
                break;
            }
        }
    }
}

#[inline(always)]
pub fn isr_handler(device: &mut Dev) -> Option<InterruptCause> {
    unsafe { isr_handler_inner(device) }
}

#[inline(always)]
unsafe fn isr_handler_inner(device: &mut Dev) -> Option<InterruptCause> {
    let args = unsafe {
        // For some reason, fsp is not including isr handlers in headers
        unsafe extern "C" {
            fn ether_eint_isr();
        }

        let mut args: MaybeUninit<ra_fsp_ethernet::st_ether_callback_args> = MaybeUninit::uninit();
        let res: u32 = ((*ETHER.p_api).callbackSet.unwrap())(
            ETHER.p_ctrl,
            Some(callback),
            ptr::null(),
            args.as_mut_ptr().cast(),
        );

        if res != 0 {
            return None;
        }

        ether_eint_isr();

        let res: u32 = ((*ETHER.p_api).callbackSet.unwrap())(
            ETHER.p_ctrl,
            Some(callback),
            ptr::null(),
            ptr::null_mut(),
        );
        assert_eq!(res, 0);

        args.assume_init()
    };

    // Those constants are not exported but used in r_ether.c

    const ETHER_ETHERC_INTERRUPT_FACTOR_LCHNG: u32 = 1 << 2;
    const ETHER_ETHERC_INTERRUPT_FACTOR_MPD: u32 = 1 << 1;
    const ETHER_EDMAC_INTERRUPT_FACTOR_ALL: u32 = 0x47FF0F9F;
    const ETHER_EDMAC_INTERRUPT_FACTOR_RFCOF: u32 = 1 << 24;
    const ETHER_EDMAC_INTERRUPT_FACTOR_ECI: u32 = 1 << 22;
    /* Transmit Complete. */
    const ETHER_EDMAC_INTERRUPT_FACTOR_TC: u32 = 1 << 21;
    /* Frame Receive. */
    const ETHER_EDMAC_INTERRUPT_FACTOR_FR: u32 = 1 << 18;
    const ETHER_EDMAC_INTERRUPT_FACTOR_RDE: u32 = 1 << 17;
    const ETHER_EDMAC_INTERRUPT_FACTOR_RFOF: u32 = 1 << 16;

    let mut cause = InterruptCause {
        receive: false,
        transmit_complete: false,
    };

    match args.event {
        ra_fsp_ethernet::ether_event_t_ETHER_EVENT_INTERRUPT => {
            let receive_mask = ETHER_EDMAC_INTERRUPT_FACTOR_FR;
            let trasmit_mask = ETHER_EDMAC_INTERRUPT_FACTOR_TC;

            /* Packet received. */
            if receive_mask == (args.status_eesr & receive_mask) {
                cause.receive = true;
            }

            if trasmit_mask == (args.status_eesr & trasmit_mask) {
                cause.transmit_complete = true;
                device.tx_free = true;

                unsafe {
                    let mut p_buffer_current: *mut u8 = ptr::null_mut();
                    let mut p_buffer_last: *mut u8 = ptr::null_mut();

                    if (*ETHER.p_api).txStatusGet.unwrap()(
                        ETHER.p_ctrl,
                        ptr::from_mut(&mut p_buffer_last).cast(),
                    ) != 0
                    {
                        log::error!("Failed to get the last buffer transmitted.");
                    }

                    assert!(!p_buffer_last.is_null());
                }
            }
        }
        ra_fsp_ethernet::ether_event_t_ETHER_EVENT_LINK_ON => todo!(),
        ra_fsp_ethernet::ether_event_t_ETHER_EVENT_LINK_OFF => todo!(),
        ra_fsp_ethernet::ether_event_t_ETHER_EVENT_WAKEON_LAN => {}
        _ => return None,
    };

    Some(cause)
}

fn disable() {
    unsafe {
        (*ETHER.p_api).close.unwrap()(ETHER.p_ctrl);
    }
}

fn is_packet_available() -> bool {
    // unsafe { EthernetPacketAvail(ETH_BASE) != 0 }
}

fn is_send_space_available(dev: &Dev) -> bool {
    dev.tx_free
}

fn packet_get(buf: &mut [u8; MTU]) -> isize {
    // let ptr = buf.as_mut_ptr();
    // let len = buf.len() as i32;
    // let len = unsafe { EthernetPacketGetNonBlocking(ETH_BASE, ptr, len) };
    //
    // if len == 0 {
    //     warn!("No packet available");
    //     return 0;
    // }
    //
    // assert!(len < 0 || len as usize <= buf.len());
    //
    // cortex_m::asm::dmb();
    //
    // len as isize
}

fn send(packet: &[u8]) {
    let ptr = packet.as_ptr().cast_mut();
    let len = packet.len() as i32;

    cortex_m::asm::dmb();

    let ret = unsafe { (*ETHER.p_api).write.unwrap()(ETHER.p_ctrl, ptr, len) };

    assert!(ret == 0);
}
