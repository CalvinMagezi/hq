//! What the applier hands to `wg`: the interface's own settings carried forward, then the peers.
//!
//! `wg syncconf` and `wg setconf` treat the file as the whole truth about the interface: a file
//! with no `PrivateKey` or no `ListenPort` sets them to nothing (wireguard-tools, `config_read_init`
//! marks both as set before it reads a line). So the applier cannot pass peers alone. It reads
//! what `wg showconf` says the running interface has, keeps the three settings that command
//! prints, and puts the validated peers after them.

use crate::grammar::{self, Peer};
use crate::key;
use std::collections::BTreeMap;

const INTERFACE_HEADER: &str = "[Interface]";
const PEER_HEADER: &str = "[Peer]";
const MAX_LISTEN_PORT: u32 = u16::MAX as u32;
const MAX_FWMARK_DIGITS: usize = 8;

/// `syncconf` changes only what differs, so connected peers keep their sessions. For an empty peer
/// list the applier uses `setconf`, which states the intent outright (replace every peer with
/// none) instead of relying on how `sync_conf` returns early for a file with no peers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Subcommand {
    Syncconf,
    Setconf,
}

impl Subcommand {
    pub fn arg(self) -> &'static str {
        match self {
            Subcommand::Syncconf => "syncconf",
            Subcommand::Setconf => "setconf",
        }
    }
}

fn is_decimal(text: &str, max: u32) -> bool {
    !text.is_empty()
        && text.len() <= 5
        && text.bytes().all(|b| b.is_ascii_digit())
        && (text == "0" || !text.starts_with('0'))
        && text.parse::<u32>().is_ok_and(|n| n <= max)
}

fn is_fwmark(text: &str) -> bool {
    text == "off"
        || text.strip_prefix("0x").is_some_and(|hex| {
            (1..=MAX_FWMARK_DIGITS).contains(&hex.len())
                && hex.bytes().all(|b| b.is_ascii_hexdigit())
        })
}

/// The `[Interface]` part of `wg showconf` output, rebuilt from only the three settings it may
/// hold, in a fixed order. Anything else in that part, or a repeat, is an error: the output of a
/// tool is checked as strictly as a file.
pub fn interface_section(showconf: &str) -> Result<String, &'static str> {
    let mut lines = showconf.lines();
    if lines.next() != Some(INTERFACE_HEADER) {
        return Err("wg showconf did not start with [Interface]");
    }
    let (mut private, mut port, mut fwmark) = (None, None, None);
    for line in lines.map(str::trim) {
        if line == PEER_HEADER {
            break;
        }
        if line.is_empty() {
            continue;
        }
        let Some((name, value)) = line.split_once('=') else {
            return Err("wg showconf printed a line this applier does not know");
        };
        let value = value.trim();
        let (slot, valid) = match name.trim() {
            "PrivateKey" => (&mut private, key::decode(value).is_some()),
            "ListenPort" => (&mut port, is_decimal(value, MAX_LISTEN_PORT)),
            "FwMark" => (&mut fwmark, is_fwmark(value)),
            _ => return Err("wg showconf printed a setting this applier does not know"),
        };
        if !valid || slot.replace(value.to_string()).is_some() {
            return Err(
                "wg showconf printed a setting twice or with a value this applier does not accept",
            );
        }
    }
    let mut out = String::from(INTERFACE_HEADER);
    out.push('\n');
    for (name, value) in [
        ("PrivateKey", private),
        ("ListenPort", port),
        ("FwMark", fwmark),
    ] {
        if let Some(value) = value {
            out.push_str(&format!("{name} = {value}\n"));
        }
    }
    Ok(out)
}

const FINGERPRINT_CHARS: usize = 8;
const MAX_ALLOWED_IPS_LOG: usize = 64;

/// The peers the running interface has, from the same `wg showconf` output: public key to its
/// `AllowedIPs` text. This feeds the journal only and never a decision, so a line it cannot read
/// is skipped.
pub fn current_peers(showconf: &str) -> BTreeMap<String, String> {
    let mut found = BTreeMap::new();
    let mut key: Option<String> = None;
    let mut allowed = String::new();
    let mut finish = |key: &mut Option<String>, allowed: &mut String| {
        if let Some(k) = key.take() {
            found.insert(k, std::mem::take(allowed));
        }
        allowed.clear();
    };
    let mut in_peer = false;
    for line in showconf.lines().map(str::trim) {
        if line == PEER_HEADER {
            finish(&mut key, &mut allowed);
            in_peer = true;
        } else if line.starts_with('[') {
            finish(&mut key, &mut allowed);
            in_peer = false;
        } else if let (true, Some((name, value))) = (in_peer, line.split_once('=')) {
            let value = value.trim();
            match name.trim() {
                "PublicKey" if key::decode(value).is_some() => key = Some(value.to_string()),
                "AllowedIPs" => {
                    allowed = value
                        .chars()
                        .filter(|c| c.is_ascii_graphic())
                        .take(MAX_ALLOWED_IPS_LOG)
                        .collect();
                }
                _ => {}
            }
        }
    }
    finish(&mut key, &mut allowed);
    found
}

fn fingerprint(public_key: &str) -> String {
    public_key.chars().take(FINGERPRINT_CHARS).collect()
}

/// What this run changes on the interface, one line each: a peer added, removed, or given a
/// different address. A changed preshared key is not listed (the interface prints it in clear and
/// it is not tracked here).
pub fn changes(current: &BTreeMap<String, String>, peers: &[Peer]) -> Vec<String> {
    let mut lines = Vec::new();
    for peer in peers {
        let wanted = format!("{}/32", peer.address);
        match current.get(&peer.public_key) {
            None => lines.push(format!(
                "added {} {}",
                peer.address,
                fingerprint(&peer.public_key)
            )),
            Some(old) if *old != wanted => lines.push(format!(
                "changed {}: {old} -> {wanted}",
                fingerprint(&peer.public_key)
            )),
            Some(_) => {}
        }
    }
    for (public_key, allowed) in current {
        if !peers.iter().any(|p| p.public_key == *public_key) {
            lines.push(format!("removed {allowed} {}", fingerprint(public_key)));
        }
    }
    lines
}

/// The whole file for `wg`, and the subcommand to hand it to.
pub fn build(peers: &[Peer], showconf: &str) -> Result<(Subcommand, String), &'static str> {
    let interface = interface_section(showconf)?;
    let config = format!("{interface}\n{}", grammar::render(peers));
    let subcommand = if peers.is_empty() {
        Subcommand::Setconf
    } else {
        Subcommand::Syncconf
    };
    Ok((subcommand, config))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn k(seed: u8) -> String {
        key::encode(&[seed; 32])
    }

    fn showconf(extra: &str) -> String {
        format!(
            "[Interface]\nListenPort = 51820\nPrivateKey = {}\n{extra}\n[Peer]\nPublicKey = {}\nAllowedIPs = 10.77.0.9/32\n",
            k(5),
            k(6)
        )
    }

    #[test]
    fn the_interface_settings_are_carried_forward_in_a_fixed_order_and_old_peers_are_dropped() {
        let out = interface_section(&showconf("FwMark = 0x1a")).unwrap();
        assert_eq!(
            out,
            format!(
                "[Interface]\nPrivateKey = {}\nListenPort = 51820\nFwMark = 0x1a\n",
                k(5)
            )
        );
        assert!(!out.contains("10.77.0.9"));
    }

    #[test]
    fn an_interface_without_a_key_stays_without_one() {
        let out = interface_section("[Interface]\nListenPort = 1\n").unwrap();
        assert_eq!(out, "[Interface]\nListenPort = 1\n");
    }

    #[test]
    fn unexpected_showconf_output_is_an_error() {
        let key = k(5);
        for bad in [
            String::new(),
            "[Peer]\n".into(),
            "[Interface]\nPostUp = x\n".into(),
            "[Interface]\nListenPort = 70000\n".into(),
            "[Interface]\nListenPort = 05\n".into(),
            "[Interface]\nListenPort = 1\nListenPort = 2\n".into(),
            "[Interface]\nPrivateKey = short\n".into(),
            "[Interface]\nFwMark = 0xzz\n".into(),
            "[Interface]\nnoise\n".into(),
            format!("[Interface]\nPrivateKey = {key}\nPrivateKey = {key}\n"),
        ] {
            assert!(interface_section(&bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn peers_follow_the_interface_and_an_empty_list_uses_setconf() {
        let peer = Peer {
            public_key: k(1),
            address: Ipv4Addr::new(10, 77, 0, 2),
            preshared_key: None,
        };
        let (sub, text) = build(&[peer], &showconf("")).unwrap();
        assert_eq!(sub, Subcommand::Syncconf);
        assert!(text.starts_with("[Interface]\nPrivateKey = "));
        assert!(text.contains("\n[Peer]\nPublicKey = "));
        assert!(!text.contains("10.77.0.9"));
        let (sub, text) = build(&[], &showconf("")).unwrap();
        assert_eq!(sub, Subcommand::Setconf);
        assert!(!text.contains("[Peer]"));
        assert_eq!(sub.arg(), "setconf");
    }

    #[test]
    fn the_peers_on_the_interface_are_read_for_the_journal_only() {
        let text = format!(
            "[Interface]\nListenPort = 1\n\n[Peer]\nPublicKey = {}\nAllowedIPs = 10.77.0.2/32\nEndpoint = 1.2.3.4:5\n\n[Peer]\nPublicKey = nonsense\nAllowedIPs = 10.77.0.3/32\n\n[Peer]\nPublicKey = {}\nAllowedIPs = 10.77.0.4/32, 10.77.0.5/32\n",
            k(1),
            k(2)
        );
        let found = current_peers(&text);
        assert_eq!(found.len(), 2);
        assert_eq!(found[&k(1)], "10.77.0.2/32");
        assert_eq!(found[&k(2)], "10.77.0.4/32,10.77.0.5/32");
        assert!(current_peers("garbage \u{0} [Peer").is_empty());
    }

    #[test]
    fn changes_list_additions_removals_and_moves_and_nothing_for_an_unchanged_peer() {
        let peer = |seed: u8, last: u8| Peer {
            public_key: k(seed),
            address: Ipv4Addr::new(10, 77, 0, last),
            preshared_key: None,
        };
        let mut current = BTreeMap::new();
        current.insert(k(1), "10.77.0.2/32".to_string());
        current.insert(k(2), "10.77.0.3/32".to_string());
        current.insert(k(3), "10.77.0.4/32".to_string());
        let lines = changes(&current, &[peer(1, 2), peer(2, 9), peer(4, 5)]);
        assert_eq!(lines.len(), 3, "{lines:?}");
        assert!(
            lines
                .iter()
                .any(|l| l.starts_with("changed ") && l.contains("10.77.0.3/32 -> 10.77.0.9/32"))
        );
        assert!(lines.iter().any(|l| l.starts_with("added 10.77.0.5 ")));
        assert!(lines.iter().any(|l| l.starts_with("removed 10.77.0.4/32 ")));
        assert!(changes(&current, &[peer(1, 2), peer(2, 3), peer(3, 4)]).is_empty());
        assert!(
            lines.iter().all(|l| !l.contains(&k(1))),
            "only short fingerprints are listed"
        );
    }
}
