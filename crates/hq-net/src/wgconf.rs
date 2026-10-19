//! What the applier hands to `wg`: the interface's own settings carried forward, then the peers.
//!
//! `wg syncconf` and `wg setconf` treat the file as the whole truth about the interface: a file
//! with no `PrivateKey` or no `ListenPort` sets them to nothing (wireguard-tools, `config_read_init`
//! marks both as set before it reads a line). So the applier cannot pass peers alone. It reads
//! what `wg showconf` says the running interface has, keeps the three settings that command
//! prints, and puts the validated peers after them.

use crate::grammar::{self, Peer};
use crate::key;

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
}
