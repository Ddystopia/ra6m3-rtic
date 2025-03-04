use smoltcp::{iface::SocketStorage, socket::udp};

use crate::mqtt::MqttStorage;

const NUM_TCP_SOCKETS: usize = 3 /* http */ + 1 /* mqtt */;
const NUM_UDP_SOCKETS: usize = 0;
const NUM_SOCKETS: usize = NUM_UDP_SOCKETS + NUM_TCP_SOCKETS;

const UDP_RX_SOCKET_BUFFER_SIZE: usize = 512;
const UDP_TX_SOCKET_BUFFER_SIZE: usize = 512;
const UDP_SOCKET_METADATA_COUNT: usize = 10;

const TCP_RX_SOCKET_BUFFER_SIZE: usize = 512;
const TCP_TX_SOCKET_BUFFER_SIZE: usize = 512;

pub const MQTT_BUFFER_SIZE: usize = 3 * 1024;

pub struct NetStorage {
    pub sockets: [SocketStorage<'static>; NUM_SOCKETS],
    pub mqtt: MqttStorage,
    pub tcp_sockets: [TcpSocketStorage; NUM_TCP_SOCKETS],
    pub udp_sockets: [UdpSocketStorage; NUM_UDP_SOCKETS],
}

pub struct UdpSocketStorage {
    pub rx_payload: [u8; UDP_RX_SOCKET_BUFFER_SIZE],
    pub tx_payload: [u8; UDP_TX_SOCKET_BUFFER_SIZE],
    pub rx_metadata: [udp::PacketMetadata; UDP_SOCKET_METADATA_COUNT],
    pub tx_metadata: [udp::PacketMetadata; UDP_SOCKET_METADATA_COUNT],
}

pub struct TcpSocketStorage {
    pub rx_payload: [u8; TCP_RX_SOCKET_BUFFER_SIZE],
    pub tx_payload: [u8; TCP_TX_SOCKET_BUFFER_SIZE],
}

impl NetStorage {
    pub const fn new() -> Self {
        Self {
            mqtt: MqttStorage::new(),
            sockets: [SocketStorage::EMPTY; NUM_SOCKETS],
            tcp_sockets: [const { TcpSocketStorage::new() }; NUM_TCP_SOCKETS],
            udp_sockets: [const { UdpSocketStorage::new() }; NUM_UDP_SOCKETS],
        }
    }
}

impl UdpSocketStorage {
    const fn new() -> Self {
        Self {
            rx_payload: [0; UDP_RX_SOCKET_BUFFER_SIZE],
            tx_payload: [0; UDP_TX_SOCKET_BUFFER_SIZE],
            rx_metadata: [udp::PacketMetadata::EMPTY; UDP_SOCKET_METADATA_COUNT],
            tx_metadata: [udp::PacketMetadata::EMPTY; UDP_SOCKET_METADATA_COUNT],
        }
    }
}

impl TcpSocketStorage {
    const fn new() -> Self {
        Self {
            rx_payload: [0; TCP_RX_SOCKET_BUFFER_SIZE],
            tx_payload: [0; TCP_TX_SOCKET_BUFFER_SIZE],
        }
    }
}
