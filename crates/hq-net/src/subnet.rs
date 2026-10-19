//! IPv4 subnets for the private plane: parsing, containment, and picking one that nothing else uses.

use std::fmt;
use std::net::Ipv4Addr;

/// The subnet `hq net init` starts from (the plan's default).
pub const DEFAULT_SUBNET: &str = "10.77.0.0/24";
/// A subnet must hold the hub, at least one device and the two addresses no host can use.
pub const MIN_PREFIX: u8 = 16;
pub const MAX_PREFIX: u8 = 30;
/// Second octets `pick` tries, in order, starting from the default subnet's.
const PICK_SECOND_OCTETS: std::ops::RangeInclusive<u8> = 77..=99;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Subnet {
    network: u32,
    prefix: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubnetError(pub String);

impl fmt::Display for SubnetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SubnetError {}

fn err<T>(message: impl Into<String>) -> Result<T, SubnetError> {
    Err(SubnetError(message.into()))
}

const fn mask_of(prefix: u8) -> u32 {
    if prefix == 0 {
        0
    } else {
        u32::MAX << (32 - prefix as u32)
    }
}

/// Parses dotted decimal and nothing else: the text must be what `Ipv4Addr` prints back, so
/// leading zeros, spaces and signs never get through.
pub fn parse_canonical_ipv4(text: &str) -> Option<Ipv4Addr> {
    let addr: Ipv4Addr = text.parse().ok()?;
    (addr.to_string() == text).then_some(addr)
}

impl Subnet {
    pub fn new(network: Ipv4Addr, prefix: u8) -> Result<Self, SubnetError> {
        if !(MIN_PREFIX..=MAX_PREFIX).contains(&prefix) {
            return err(format!(
                "the prefix length must be between /{MIN_PREFIX} and /{MAX_PREFIX}"
            ));
        }
        let network = u32::from(network);
        if network & !mask_of(prefix) != 0 {
            return err(format!(
                "{}/{prefix} has host bits set; the network address would be {}/{prefix}",
                Ipv4Addr::from(network),
                Ipv4Addr::from(network & mask_of(prefix))
            ));
        }
        Ok(Self { network, prefix })
    }

    /// `a.b.c.d/n` in canonical form, with the host bits clear.
    pub fn parse(text: &str) -> Result<Self, SubnetError> {
        let Some((addr, prefix)) = text.split_once('/') else {
            return err("a subnet is written a.b.c.d/n, for example 10.77.0.0/24");
        };
        let Some(addr) = parse_canonical_ipv4(addr) else {
            return err("the network address is not plain dotted decimal");
        };
        let prefix_ok = !prefix.is_empty()
            && prefix.len() <= 2
            && prefix.bytes().all(|b| b.is_ascii_digit())
            && (prefix == "0" || !prefix.starts_with('0'));
        let Some(prefix) = prefix.parse::<u8>().ok().filter(|_| prefix_ok) else {
            return err("the prefix length is not a number");
        };
        Self::new(addr, prefix)
    }

    pub fn network(&self) -> Ipv4Addr {
        Ipv4Addr::from(self.network)
    }

    pub fn prefix(&self) -> u8 {
        self.prefix
    }

    pub fn broadcast(&self) -> Ipv4Addr {
        Ipv4Addr::from(self.network | !mask_of(self.prefix))
    }

    /// The hub's address: the first one after the network address.
    pub fn hub(&self) -> Ipv4Addr {
        Ipv4Addr::from(self.network + 1)
    }

    pub fn contains(&self, addr: Ipv4Addr) -> bool {
        u32::from(addr) & mask_of(self.prefix) == self.network
    }

    /// Inside the subnet and neither its network nor its broadcast address.
    pub fn is_usable_host(&self, addr: Ipv4Addr) -> bool {
        self.contains(addr) && addr != self.network() && addr != self.broadcast()
    }

    /// Every usable host address, lowest first.
    pub fn hosts(&self) -> impl Iterator<Item = Ipv4Addr> {
        let first = self.network + 1;
        let last = u32::from(self.broadcast()) - 1;
        (first..=last).map(Ipv4Addr::from)
    }

    pub fn overlaps(&self, other: &Subnet) -> bool {
        let shared = mask_of(self.prefix.min(other.prefix));
        self.network & shared == other.network & shared
    }

    /// RFC 1918 space: the only place the private plane may live.
    pub fn is_private(&self) -> bool {
        [PRIVATE_10, PRIVATE_172, PRIVATE_192]
            .iter()
            .any(|range| range.covers(self))
    }

    fn covers(&self, inner: &Subnet) -> bool {
        self.prefix <= inner.prefix && self.contains(inner.network())
    }
}

impl fmt::Display for Subnet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.network(), self.prefix)
    }
}

const PRIVATE_10: Subnet = Subnet {
    network: 0x0A00_0000,
    prefix: 8,
};
const PRIVATE_172: Subnet = Subnet {
    network: 0xAC10_0000,
    prefix: 12,
};
const PRIVATE_192: Subnet = Subnet {
    network: 0xC0A8_0000,
    prefix: 16,
};
/// Carrier-grade NAT space, where Tailscale hands out addresses. Kept clear so both can run
/// side by side during the cutover.
pub const CGNAT: Subnet = Subnet {
    network: 0x6440_0000,
    prefix: 10,
};

/// The first /24 from `10.77.0.0/24` upwards that overlaps nothing in `routed` and not the
/// carrier-grade NAT range.
pub fn pick(routed: &[Subnet]) -> Option<Subnet> {
    for second in PICK_SECOND_OCTETS {
        for third in 0..=u8::MAX {
            let candidate = Subnet::new(Ipv4Addr::new(10, second, third, 0), 24).ok()?;
            if !candidate.overlaps(&CGNAT) && !routed.iter().any(|r| candidate.overlaps(r)) {
                return Some(candidate);
            }
        }
    }
    None
}

/// The destinations in `/proc/net/route`. The columns are the kernel's own host-order words, so a
/// native-endian byte view gives the address back in network order on any CPU. The default route
/// and lines that do not parse are skipped.
pub fn routed_subnets(proc_net_route: &str) -> Vec<Subnet> {
    proc_net_route
        .lines()
        .skip(1)
        .filter_map(|line| {
            let mut cols = line.split_whitespace();
            let _iface = cols.next()?;
            let destination = u32::from_str_radix(cols.next()?, 16).ok()?;
            let mask = u32::from_str_radix(cols.nth(5)?, 16).ok()?;
            let mask = u32::from_be_bytes(mask.to_ne_bytes());
            let prefix = mask.leading_ones() as u8;
            if prefix == 0 || mask != mask_of(prefix) {
                return None;
            }
            let network = u32::from_be_bytes(destination.to_ne_bytes()) & mask;
            // Any route length counts for the overlap test, so this skips the constructor's range.
            Some(Subnet { network, prefix })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn net(text: &str) -> Subnet {
        Subnet::parse(text).unwrap()
    }

    #[test]
    fn parses_only_canonical_networks() {
        assert_eq!(net("10.77.0.0/24").to_string(), "10.77.0.0/24");
        for bad in [
            "10.77.0.0",
            "10.77.0.1/24",
            "10.077.0.0/24",
            "10.77.0.0/024",
            "10.77.0.0/+24",
            " 10.77.0.0/24",
            "10.77.0.0/24 ",
            "10.77.0.0/",
            "10.77.0.0/8",
            "10.77.0.0/31",
            "10.77.0.0/256",
            "::1/64",
            "",
        ] {
            assert!(Subnet::parse(bad).is_err(), "{bad:?} should be refused");
        }
    }

    #[test]
    fn usable_hosts_exclude_network_and_broadcast() {
        let s = net("10.77.0.0/24");
        assert_eq!(s.hub(), Ipv4Addr::new(10, 77, 0, 1));
        assert!(!s.is_usable_host(Ipv4Addr::new(10, 77, 0, 0)));
        assert!(!s.is_usable_host(Ipv4Addr::new(10, 77, 0, 255)));
        assert!(s.is_usable_host(Ipv4Addr::new(10, 77, 0, 254)));
        assert!(!s.is_usable_host(Ipv4Addr::new(10, 77, 1, 5)));
        assert_eq!(s.hosts().count(), 254);
    }

    #[test]
    fn private_ranges_are_the_only_allowed_homes() {
        assert!(net("10.77.0.0/24").is_private());
        assert!(net("172.16.0.0/24").is_private());
        assert!(net("172.31.255.0/24").is_private());
        assert!(net("192.168.50.0/24").is_private());
        assert!(!net("172.32.0.0/24").is_private());
        assert!(!net("100.64.0.0/24").is_private());
        assert!(!net("8.8.8.0/24").is_private());
    }

    #[test]
    fn pick_skips_routed_subnets_and_the_tailscale_range() {
        assert_eq!(pick(&[]).unwrap().to_string(), "10.77.0.0/24");
        let busy = [net("10.77.0.0/24"), net("10.77.1.0/24")];
        assert_eq!(pick(&busy).unwrap().to_string(), "10.77.2.0/24");
        let wide = [net("10.77.0.0/16")];
        assert_eq!(pick(&wide).unwrap().to_string(), "10.78.0.0/24");
        assert!(CGNAT.overlaps(&net("100.100.0.0/24")));
    }

    #[test]
    fn proc_net_route_is_read_in_network_order() {
        let text = "Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\t\tMTU\tWindow\tIRTT\n\
                    eth0\t00000000\t0102A8C0\t0003\t0\t0\t100\t00000000\t0\t0\t0\n\
                    eth0\t0002A8C0\t00000000\t0001\t0\t0\t100\t00FFFFFF\t0\t0\t0\n";
        let text = if cfg!(target_endian = "big") {
            text.replace("0002A8C0", "C0A80200")
                .replace("00FFFFFF", "FFFFFF00")
        } else {
            text.to_string()
        };
        let routed = routed_subnets(&text);
        assert_eq!(routed, vec![net("192.168.2.0/24")]);
    }

    #[test]
    fn garbage_route_tables_yield_nothing() {
        assert!(routed_subnets("").is_empty());
        assert!(routed_subnets("x\nnot a route\n\u{0}\n").is_empty());
    }
}
