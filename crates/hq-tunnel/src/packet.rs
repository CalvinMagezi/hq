//! A strict look at an IPv4 header before anything else sees the packet. The stack takes TCP over
//! IPv4 to its own address and nothing else, so everything else is dropped here.

use std::net::Ipv4Addr;

pub const PROTO_TCP: u8 = 6;
const MIN_HEADER: usize = 20;
const VERSION_4: u8 = 4;
const FRAGMENT_BITS: u16 = 0x3fff;
/// Source and destination port, the first four bytes of a TCP header.
const TCP_PORTS_BYTES: usize = 4;
const TCP_FLAGS_OFFSET: usize = 13;
const TCP_SYN: u8 = 0x02;
const TCP_ACK: u8 = 0x10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ipv4Meta {
    pub src: Ipv4Addr,
    pub dst: Ipv4Addr,
    pub protocol: u8,
    /// For a TCP packet with room for the ports, where it came from and where it is going.
    pub tcp_src_port: Option<u16>,
    pub tcp_dst_port: Option<u16>,
    /// A SYN that is not also an ACK: the first packet of a connection.
    pub tcp_syn: bool,
}

/// The header of a complete, unfragmented IPv4 packet, or `None` for anything else.
pub fn inspect(packet: &[u8]) -> Option<Ipv4Meta> {
    if packet.len() < MIN_HEADER || packet[0] >> 4 != VERSION_4 {
        return None;
    }
    let header_len = usize::from(packet[0] & 0x0f) * 4;
    let total = usize::from(u16::from_be_bytes([packet[2], packet[3]]));
    let frag = u16::from_be_bytes([packet[6], packet[7]]);
    if header_len < MIN_HEADER
        || total < header_len
        || total > packet.len()
        || frag & FRAGMENT_BITS != 0
    {
        return None;
    }
    let protocol = packet[9];
    let has_ports = protocol == PROTO_TCP && total >= header_len + TCP_PORTS_BYTES;
    let port_at =
        |at: usize| u16::from_be_bytes([packet[header_len + at], packet[header_len + at + 1]]);
    let tcp_src_port = has_ports.then(|| port_at(0));
    let tcp_dst_port = has_ports.then(|| port_at(2));
    let tcp_syn = protocol == PROTO_TCP
        && total > header_len + TCP_FLAGS_OFFSET
        && packet[header_len + TCP_FLAGS_OFFSET] & (TCP_SYN | TCP_ACK) == TCP_SYN;
    Some(Ipv4Meta {
        src: Ipv4Addr::new(packet[12], packet[13], packet[14], packet[15]),
        dst: Ipv4Addr::new(packet[16], packet[17], packet[18], packet[19]),
        protocol,
        tcp_src_port,
        tcp_dst_port,
        tcp_syn,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(src: [u8; 4], dst: [u8; 4], protocol: u8) -> Vec<u8> {
        let mut p = vec![0u8; 40];
        p[0] = 0x45;
        p[2..4].copy_from_slice(&40u16.to_be_bytes());
        p[9] = protocol;
        p[12..16].copy_from_slice(&src);
        p[16..20].copy_from_slice(&dst);
        p
    }

    #[test]
    fn a_plain_packet_is_read() {
        let meta = inspect(&header([10, 77, 0, 1], [10, 77, 0, 4], PROTO_TCP)).unwrap();
        assert_eq!(meta.src, Ipv4Addr::new(10, 77, 0, 1));
        assert_eq!(meta.dst, Ipv4Addr::new(10, 77, 0, 4));
        assert_eq!(meta.protocol, PROTO_TCP);
        assert_eq!(meta.tcp_dst_port, Some(0), "an empty header has port zero");
        let mut with_port = header([10, 77, 0, 1], [10, 77, 0, 4], PROTO_TCP);
        with_port[22..24].copy_from_slice(&8443u16.to_be_bytes());
        assert_eq!(inspect(&with_port).unwrap().tcp_dst_port, Some(8443));
        // Not TCP, or no room for the ports: no port.
        assert_eq!(
            inspect(&header([10, 77, 0, 1], [10, 77, 0, 4], 17))
                .unwrap()
                .tcp_dst_port,
            None
        );
        let mut short = header([10, 77, 0, 1], [10, 77, 0, 4], PROTO_TCP);
        short[2..4].copy_from_slice(&22u16.to_be_bytes());
        assert_eq!(inspect(&short).unwrap().tcp_dst_port, None);
    }

    #[test]
    fn anything_odd_is_refused() {
        let good = header([10, 77, 0, 1], [10, 77, 0, 4], PROTO_TCP);
        let mut cases: Vec<Vec<u8>> = vec![vec![], good[..19].to_vec()];
        for edit in [(0usize, 0x65u8), (0, 0x44), (2, 0xff), (6, 0x20), (7, 0x01)] {
            let mut p = good.clone();
            p[edit.0] = edit.1;
            cases.push(p);
        }
        let mut short_total = good.clone();
        short_total[2..4].copy_from_slice(&10u16.to_be_bytes());
        cases.push(short_total);
        for case in cases {
            assert!(inspect(&case).is_none(), "{case:?}");
        }
        // Trailing bytes after the stated length are fine; a stated length past the end is not.
        let mut padded = good.clone();
        padded.extend_from_slice(&[0; 8]);
        assert!(inspect(&padded).is_some());
    }

    #[test]
    fn random_bytes_never_panic() {
        use rand::{Rng, SeedableRng};
        let mut rng = rand::rngs::StdRng::seed_from_u64(0x9ac4);
        for _ in 0..20_000 {
            let len = rng.gen_range(0..80);
            let bytes: Vec<u8> = (0..len).map(|_| rng.r#gen()).collect();
            let _ = inspect(&bytes);
        }
    }
}
