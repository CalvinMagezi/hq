//! `hq net status`: join `wg show <interface> latest-handshakes` with the admitted devices.
//!
//! That command prints only public keys and times. `wg show dump` and `wg showconf` print the
//! private key, so nothing here ever asks for them.

use crate::key;
use crate::store::PeerRecord;
use std::collections::HashMap;

const MINUTE: u64 = 60;
const HOUR: u64 = 60 * MINUTE;
const DAY: u64 = 24 * HOUR;

/// `<public key>\t<unix seconds>` per line. Lines that do not fit are skipped, so a changed or
/// noisy output can only make a device show as "never", not break the table.
pub fn parse_handshakes(output: &str) -> HashMap<String, u64> {
    output
        .lines()
        .filter_map(|line| {
            let (public_key, seconds) = line.split_once('\t')?;
            let seconds = seconds.trim().parse::<u64>().ok()?;
            key::decode(public_key).map(|_| (public_key.to_string(), seconds))
        })
        .collect()
}

/// "never" for 0 (WireGuard's own value for no handshake yet), else how long ago.
pub fn age(now_unix: u64, handshake_unix: u64) -> String {
    if handshake_unix == 0 {
        return "never".into();
    }
    let secs = now_unix.saturating_sub(handshake_unix);
    match secs {
        s if s < MINUTE => format!("{s}s ago"),
        s if s < HOUR => format!("{}m ago", s / MINUTE),
        s if s < DAY => format!("{}h ago", s / HOUR),
        s => format!("{}d ago", s / DAY),
    }
}

/// A table of devices. `handshakes` is `None` when `wg` could not be read, so the column says so
/// instead of claiming "never".
pub fn table(
    peers: &[PeerRecord],
    handshakes: Option<&HashMap<String, u64>>,
    now_unix: u64,
) -> String {
    let mut out = format!(
        "{:<16} {:<7} {:<12} {}\n",
        "DEVICE", "KIND", "ADDRESS", "LAST HANDSHAKE"
    );
    for peer in peers {
        let seen = match handshakes {
            None => "unknown".to_string(),
            Some(map) => age(now_unix, map.get(&peer.public_key).copied().unwrap_or(0)),
        };
        out.push_str(&format!(
            "{:<16} {:<7} {:<12} {seen}\n",
            peer.label,
            peer.kind.name(),
            peer.address
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Kind;

    fn k(seed: u8) -> String {
        key::encode(&[seed; 32])
    }

    #[test]
    fn handshake_output_is_read_and_noise_is_skipped() {
        let out = format!(
            "{}\t1700000000\n{}\t0\nnoise\n{}\tabc\nshort\t5\n",
            k(1),
            k(2),
            k(3)
        );
        let map = parse_handshakes(&out);
        assert_eq!(map.len(), 2);
        assert_eq!(map[&k(1)], 1_700_000_000);
        assert_eq!(map[&k(2)], 0);
    }

    #[test]
    fn ages_read_naturally() {
        assert_eq!(age(100, 0), "never");
        assert_eq!(age(1000, 990), "10s ago");
        assert_eq!(age(1000, 1000 - 5 * 60), "5m ago");
        assert_eq!(age(100_000, 100_000 - 3 * 3600), "3h ago");
        assert_eq!(age(1_000_000, 1_000_000 - 2 * 86_400), "2d ago");
        assert_eq!(age(5, 10), "0s ago");
    }

    #[test]
    fn the_table_marks_unreadable_handshakes_as_unknown() {
        let peers = vec![PeerRecord {
            label: "mac".into(),
            kind: Kind::Laptop,
            public_key: k(1),
            address: "10.77.0.2".into(),
            preshared_key: None,
            keepalive: None,
        }];
        assert!(table(&peers, None, 100).contains("unknown"));
        let map = parse_handshakes(&format!("{}\t90\n", k(1)));
        assert!(table(&peers, Some(&map), 100).contains("10s ago"));
        assert!(table(&peers, Some(&HashMap::new()), 100).contains("never"));
    }
}
