use crate::log::*;
use core::marker::PhantomPinned;
use core::mem::MaybeUninit;
use core::ops::{Deref, DerefMut};
use core::pin::Pin;
use core::ptr;

// todo: move map from init.c here
//       [VECTOR_NUMBER_EDMAC0_EINT]             = ELC_EVENT_EDMAC0_EINT,

use bare_metal::CriticalSection;
use ra_fsp_sys::Interrupt;
use smoltcp::phy::{ChecksumCapabilities, Device, DeviceCapabilities, Medium, RxToken, TxToken};
use smoltcp::time::Instant;

const MTU: usize = 1500;

pub const ETH_N_TX_DESC: usize = 4;
pub const ETH_N_RX_DESC: usize = 4;

struct EtherInstane(*const ra_fsp_ethernet::ether_instance_t);

#[derive(Debug, Clone, Copy)]
pub struct EthernetCallbackArgs {
    pub channel: u32,
    pub event: ra_fsp_ethernet::ether_event_t,
    pub status_ecsr: u32,
    pub status_eesr: u32,
}

#[repr(C, align(32))]
pub struct Buffer([u8; ETHER_PACKET_SIZE]);

pub struct Buffers<const N: usize>([Buffer; N], PhantomPinned);

pub struct Dev {
    rx: Pin<&'static mut Buffers<ETH_N_RX_DESC>>,
    tx: &'static mut Buffers<ETH_N_TX_DESC>,
    tx_available: [bool; ETH_N_TX_DESC],
}

pub struct InterruptCause {
    pub went_up: bool,
    pub went_down: bool,
    pub receive: bool,
    pub transmits: bool,
}

pub struct EthernetRxToken<'a>(&'a mut [u8]);
pub struct EthernetTxToken<'a>(&'a mut [u8; ETHER_PACKET_SIZE], &'a mut bool);

impl<const N: usize> Buffers<N> {
    pub const fn new() -> Self {
        Self([const { Buffer([0; ETHER_PACKET_SIZE]) }; N], PhantomPinned)
    }
}

impl Deref for Buffers<ETH_N_TX_DESC> {
    type Target = [Buffer; ETH_N_TX_DESC];
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for Buffers<ETH_N_TX_DESC> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Dev {
    pub fn new(
        cs: CriticalSection<'_>,
        nvic: &mut cortex_m::peripheral::NVIC,
        tx_buffers: &'static mut Buffers<ETH_N_TX_DESC>,
        rx_buffers: &'static mut Buffers<ETH_N_RX_DESC>,
        _: ra_fsp_sys::EDMAC0,
    ) -> Self {
        init(cs, nvic, crate::MAC);

        Self {
            rx: Pin::static_mut(rx_buffers),
            tx: tx_buffers,
            tx_available: [true; ETH_N_TX_DESC],
        }
    }

    pub fn poll_link(&mut self) {
        if ETH0.get_ctrl_open() != 0 {
            ETH0.link_process();
        }
    }

    pub fn is_up(&mut self) -> bool {
        ETH0.is_up()
    }

    fn send_packet_idx(&self) -> Option<usize> {
        self.tx_available.iter().position(|&x| x)
    }
}

impl Device for Dev {
    type RxToken<'a> = EthernetRxToken<'a>;
    type TxToken<'a> = EthernetTxToken<'a>;

    // N.B.: Tokens are mutable borrowing `self`, so it is impossible to have
    //       multiple tokens.

    fn receive(&mut self, _: Instant) -> Option<(Self::RxToken<'_>, Self::TxToken<'_>)> {
        if !ETH0.is_up() {
            log::warn!("Ethernet is not up");
            return None;
        }

        let tx = self.send_packet_idx()?;
        match ETH0.read() {
            Ok(buf) => Some((
                EthernetRxToken(buf),
                EthernetTxToken(&mut self.tx[tx].0, &mut self.tx_available[tx]),
            )),
            Err(ra_fsp_ethernet::e_fsp_err_FSP_ERR_ETHER_ERROR_NO_DATA) => None,
            Err(err) => {
                error!("Ethernet read error: {}", err);
                None
            }
        }
    }

    fn transmit(&mut self, _: Instant) -> Option<Self::TxToken<'_>> {
        if !ETH0.is_up() {
            return None;
        }

        if let Some(rx) = self.send_packet_idx() {
            Some(EthernetTxToken(
                &mut self.tx[rx].0,
                &mut self.tx_available[rx],
            ))
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
        f(self.0)
    }
}

impl Drop for EthernetRxToken<'_> {
    fn drop(&mut self) {
        assert!(ETH0.rx_buffer_update(self.0.as_mut_ptr()) == 0);
    }
}

impl TxToken for EthernetTxToken<'_> {
    fn consume<R, F>(self, length: usize, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        let result = f(&mut self.0[..length]);

        if length < 60 {
            self.0[length..60].fill(0);
        }
        let packet: &[u8] = &self.0[..length.max(60)];
        let ret = ETH0.write(packet.as_ptr(), packet.len() as u32);

        if ret == ra_fsp_ethernet::e_fsp_err_FSP_ERR_ETHER_ERROR_LINK {
            return result;
        }

        assert!(ret == 0);

        *self.1 = false; // that tx buffer has been passed to the driver

        result
    }
}

#[repr(C, align(16))]
struct EtherInstanceDescriptor<const N: usize> {
    descriptors: MaybeUninit<[ra_fsp_ethernet::ether_instance_descriptor_t; N]>,
}

static mut PHY0_CTRL: MaybeUninit<ra_fsp_ethernet::ether_phy_instance_ctrl_t> =
    MaybeUninit::zeroed();

// todo: table 31.1 shows that hw supports multi-buffer frame transmission and reception.
//       that way, we don't need to have a large buffer for smaller stuff, right? Not sure
//       now it interacts with the limit (8) of buffers.

static PHY0_CFG: ra_fsp_ethernet::ether_phy_cfg_t = ra_fsp_ethernet::ether_phy_cfg_t {
    channel: 0,
    phy_lsi_address: 0,
    phy_reset_wait_time: 0x00020000,
    mii_bit_access_wait_time: 8,
    // idk why it doesn work with KSZ8091RNB, manual says it has it
    // phy_lsi_type: ra_fsp_ethernet::e_ether_phy_lsi_type_ETHER_PHY_LSI_TYPE_KSZ8091RNB,
    phy_lsi_type: ra_fsp_ethernet::e_ether_phy_lsi_type_ETHER_PHY_LSI_TYPE_DEFAULT,
    flow_control: ra_fsp_ethernet::e_ether_phy_flow_control_ETHER_PHY_FLOW_CONTROL_DISABLE,
    mii_type: ra_fsp_ethernet::e_ether_phy_mii_type_ETHER_PHY_MII_TYPE_RMII,
    p_context: ptr::null_mut(),
    p_extend: ptr::null_mut(),
};

static PHY0: ra_fsp_ethernet::ether_phy_instance_t = ra_fsp_ethernet::ether_phy_instance_t {
    p_ctrl: (&raw mut PHY0_CTRL).cast(),
    p_cfg: &raw const PHY0_CFG,
    p_api: &raw const ra_fsp_ethernet::g_ether_phy_on_ether_phy,
};

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

const ETHER_PACKET_SIZE: usize = 1568;

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
    // padding: ra_fsp_ethernet::e_ether_padding_ETHER_PADDING_2BYTE,
    // padding_offset: 14,
    padding: ra_fsp_ethernet::e_ether_padding_ETHER_PADDING_DISABLE,
    padding_offset: 0,
    broadcast_filter: 0,
    p_mac_address: ptr::null_mut(), // configured in init
    num_tx_descriptors: ETH_N_TX_DESC as u8,
    num_rx_descriptors: ETH_N_RX_DESC as u8,
    ether_buffer_size: ETHER_PACKET_SIZE as u32,
    irq: 0, // note: we don't want to use it, but api requires it. So we will restore the priority
    interrupt_priority: 0,
    p_ether_phy_instance: &raw const PHY0,
    p_callback: Some(c_user_ethernet_callback), // configured in init
    p_context: ptr::null_mut(),                 // configured in init
    p_extend: (&raw const CONF_EXTEND).cast(),
    pp_ether_buffers: ptr::null_mut(),
};

static mut CTRL: MaybeUninit<ra_fsp_ethernet::ether_instance_ctrl_t> = MaybeUninit::zeroed();

static ETHER: ra_fsp_ethernet::ether_instance_t = ra_fsp_ethernet::ether_instance_t {
    p_ctrl: (&raw mut CTRL).cast(),
    p_cfg: &raw const CONF,
    p_api: &raw const ra_fsp_ethernet::g_ether_on_ether,
};

const ETH0: EtherInstane = EtherInstane(&raw const ETHER);

impl EtherInstane {
    fn open(&self) -> u32 {
        unsafe { (*(*self.0).p_api).open.unwrap()((*self.0).p_ctrl, (*self.0).p_cfg) }
    }
    fn is_up(&self) -> bool {
        let status = unsafe {
            (*(*self.0)
                .p_ctrl
                .cast::<ra_fsp_ethernet::ether_instance_ctrl_t>())
            .link_establish_status
        };
        status == ra_fsp_ethernet::e_ether_link_establish_status_ETHER_LINK_ESTABLISH_STATUS_UP
    }
    fn link_process(&self) -> u32 {
        unsafe { (*(*self.0).p_api).linkProcess.unwrap()((*self.0).p_ctrl) }
    }
    fn get_ctrl_open(&self) -> u32 {
        unsafe {
            (*(*self.0)
                .p_ctrl
                .cast::<ra_fsp_ethernet::ether_instance_ctrl_t>())
            .open
        }
    }
    #[allow(dead_code)]
    fn close(&self) -> u32 {
        unsafe { (*(*self.0).p_api).close.unwrap()((*self.0).p_ctrl) }
    }
    #[allow(dead_code)]
    fn buffer_release(&self) -> u32 {
        unsafe { (*(*self.0).p_api).bufferRelease.unwrap()((*self.0).p_ctrl) }
    }
    #[allow(dead_code)]
    fn callback_set(
        &self,
        callback: Option<unsafe extern "C" fn(*mut ra_fsp_ethernet::st_ether_callback_args)>,
        context: *const (),
        memory: *mut ra_fsp_ethernet::st_ether_callback_args,
    ) -> u32 {
        unsafe {
            (*(*self.0).p_api).callbackSet.unwrap()(
                (*self.0).p_ctrl,
                callback,
                context.cast(),
                memory,
            )
        }
    }
    fn tx_status_get(&self, buffer: *mut *mut u8) -> u32 {
        unsafe { (*(*self.0).p_api).txStatusGet.unwrap()((*self.0).p_ctrl, buffer.cast()) }
    }
    fn rx_buffer_update(&self, buffer: *mut u8) -> u32 {
        unsafe { (*(*self.0).p_api).rxBufferUpdate.unwrap()((*self.0).p_ctrl, buffer.cast()) }
    }
    fn write(&self, buffer: *const u8, len: u32) -> u32 {
        unsafe {
            (*(*self.0).p_api).write.unwrap()((*self.0).p_ctrl, buffer.cast_mut().cast(), len)
        }
    }
    fn read(&self) -> Result<&mut [u8], u32> {
        unsafe {
            let mut ptr: *mut u8 = ptr::null_mut();
            let mut len: u32 = 0;

            let ret = (*(*self.0).p_api).read.unwrap()(
                (*self.0).p_ctrl,
                ptr::from_mut(&mut ptr).cast(),
                ptr::from_mut(&mut len).cast(),
            );
            if ret == 0 {
                assert!(!ptr.is_null(), "Pointer to buffer is null");
                Ok(core::slice::from_raw_parts_mut(ptr, len as usize))
            } else {
                Err(ret)
            }
        }
    }
}

fn init(_cs: CriticalSection<'_>, nvic: &mut cortex_m::peripheral::NVIC, mut mac: [u8; 6]) {
    static mut MAC: [u8; 6] = [0; 6];

    unsafe {
        mac.reverse();
        ptr::write(&raw mut MAC, mac);
        ptr::write(&raw mut CONF.p_mac_address, (&raw mut MAC).cast());

        let isr_num0_priority = cortex_m::peripheral::NVIC::get_priority(Interrupt::IEL0);
        let res: u32 = ETH0.open();

        let overwritten_priority = cortex_m::peripheral::NVIC::get_priority(Interrupt::IEL0);
        assert_eq!(overwritten_priority, 0);

        nvic.set_priority(Interrupt::IEL0, isr_num0_priority);
        assert_eq!(
            cortex_m::peripheral::NVIC::get_priority(Interrupt::IEL0),
            isr_num0_priority
        );

        assert_eq!(res, 0);
    }
}

pub fn ethernet_callback(device: &mut Dev, args: EthernetCallbackArgs) -> Option<InterruptCause> {
    /* Transmit Complete. (all pending transmissions) */
    const ETHER_EDMAC_INTERRUPT_FACTOR_TC: u32 = 1 << 21;
    /* Frame Receive. */
    const ETHER_EDMAC_INTERRUPT_FACTOR_FR: u32 = 1 << 18;

    let mut cause = InterruptCause {
        receive: false,
        transmits: false,
        went_up: false,
        went_down: false,
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
                cause.transmits = true;

                device.tx_available.iter_mut().for_each(|x| *x = true);

                #[cfg(debug_assertions)]
                {
                    let mut p_buffer_last: *mut u8 = ptr::null_mut();

                    if ETH0.tx_status_get(&mut p_buffer_last) != 0 {
                        error!("Failed to get the last buffer transmitted.");
                    }

                    assert!(!p_buffer_last.is_null());

                    if let Some(_idx) = device.tx.iter().position(|x| {
                        x.0.as_ptr() <= p_buffer_last
                            && x.0.as_ptr().wrapping_add(MTU) > p_buffer_last
                    }) {
                        // ok
                    } else {
                        panic!(
                            "Failed to find the last buffer transmitted, driver gave {:p}, while we have {:p}, {:p}, {:p}, {:p}",
                            p_buffer_last,
                            device.tx[0].0.as_ptr(),
                            device.tx[1].0.as_ptr(),
                            device.tx[2].0.as_ptr(),
                            device.tx[3].0.as_ptr()
                        );
                    }
                }
            }
        }
        ra_fsp_ethernet::ether_event_t_ETHER_EVENT_LINK_ON => {
            cause.went_up = true;
            for i in 0..ETH_N_RX_DESC {
                // SAFETY: when link on, driver does not have pointers to the buffers
                let rx = unsafe { device.rx.as_mut().get_unchecked_mut() };
                ETH0.rx_buffer_update(rx[i].0.as_mut_ptr());
            }
        }
        ra_fsp_ethernet::ether_event_t_ETHER_EVENT_LINK_OFF => {
            cause.went_down = true;
            /*
             * When the link is re-established, the Ethernet driver will reset all of the buffer descriptors.
             */
            device.tx_available.iter_mut().for_each(|x| *x = true);
        }
        _ => return None,
    };

    Some(cause)
}

pub fn ethernet_isr_handler() {
    unsafe extern "C" {
        unsafe fn ether_eint_isr();
    }
    unsafe { ether_eint_isr() }
}

unsafe extern "C" fn c_user_ethernet_callback(args: *mut ra_fsp_ethernet::st_ether_callback_args) {
    let args = unsafe { *args };

    crate::app::network_poll_callback::spawn(EthernetCallbackArgs {
        channel: args.channel,
        event: args.event,
        status_ecsr: args.status_ecsr,
        status_eesr: args.status_eesr,
    })
    .expect("`network_poll_callback` should have high enough priority to not let us spawn it until it competes")
}
