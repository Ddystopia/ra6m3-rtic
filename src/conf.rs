use core::net::Ipv4Addr;

use konst::{iter, result::unwrap_or_else, string};
use smoltcp::wire::{self, IpAddress};

macro_rules! parse_ipv4 {
    ($s:expr, $constructor:expr) => {{
        let [a, b, c, d] = iter::collect_const!(u8 =>
            string::split($s, "."),
            map(|s| parse_u8(s, 10, "Invalid IP_V4 address")),
        );
        $constructor(a, b, c, d)
    }};
}
macro_rules! parse_ipv6 {
    ($s:expr, $constructor:expr) => {{
        let [a, b, c, d, e, f, g, h] = iter::collect_const!(u16 =>
            string::split($s, ":"),
            map(|s| parse_u16(s, 16, "Invalid IP_V6 address")),
        );
        $constructor(a, b, c, d, e, f, g, h)
    }};
}

// Must device systick frequency evenly, which is 120MHz.
// If too much, systick's isr would overthrow the application
pub const CLOCK_HZ: u32 = 12_000; // 83.33(3)us
pub const MAC: [u8; 6] = iter::collect_const!(u8 =>
    string::split(env!("MAC"), ":"),
        map(|s| parse_u8(s, 16, "Invalid MAC address")),
);
pub const IP_V4: IpAddress = parse_ipv4!(split_ip(env!("IP_V4")).0, IpAddress::v4);
pub const IP_V4_NETMASK: u8 = split_ip(env!("IP_V4")).1;
pub const IP_V4_GATEWAY: wire::Ipv4Address =
    parse_ipv4!(env!("IP_V4_GATEWAY"), wire::Ipv4Address::new);

pub const IP_V6: IpAddress = parse_ipv6!(split_ip(env!("IP_V6")).0, IpAddress::v6);
pub const IP_V6_NETMASK: u8 = split_ip(env!("IP_V6")).1;
pub const IP_V6_GATEWAY: wire::Ipv6Address =
    parse_ipv6!(env!("IP_V6_GATEWAY"), wire::Ipv6Address::new);

pub const MQTT_CLIENT_ID: &str = env!("MQTT_CLIENT_ID");
pub const MQTT_BROKER_IP: Ipv4Addr = parse_ipv4!(env!("MQTT_BROKER_IP"), Ipv4Addr::new);
pub const MQTT_BROKER_PORT_TLS: u16 =
    parse_u16(env!("MQTT_BROKER_PORT_TLS"), 10, "Invalid MQTT port");
pub const MQTT_BROKER_PORT_TCP: u16 =
    parse_u16(env!("MQTT_BROKER_PORT_TCP"), 10, "Invalid MQTT port");

const fn split_ip(ip: &str) -> (&str, u8) {
    let (ip, mask) = konst::option::unwrap!(string::split_once(ip, "/"));
    (ip, parse_u8(mask, 10, "Invalid IP mask"))
}
const fn parse_u8(s: &str, radix: u32, msg: &str) -> u8 {
    unwrap_or_else!(u8::from_str_radix(s, radix), |_| panic!("{}", msg))
}
const fn parse_u16(s: &str, radix: u32, msg: &str) -> u16 {
    unwrap_or_else!(u16::from_str_radix(s, radix), |_| panic!("{}", msg))
}
