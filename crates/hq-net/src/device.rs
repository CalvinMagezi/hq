//! The config a device uses to reach the hub, and the QR code of the same text.
//!
//! It never holds a private key. The device made its own, in the WireGuard app or with
//! `wg genkey`, and only the public half ever reached HQ. The device's only peer is the hub and its
//! only allowed address is the hub's `/32`, so nothing else is routed into the tunnel and a
//! full-tunnel setup cannot be produced from here.

use crate::qr;
use crate::store::{Kind, Net, PeerRecord, Result, fail};

/// Keeps the path from a laptop to the hub open behind home routers and firewalls. A phone does
/// not set it: the phone starts every exchange, and a timer would only cost battery and data.
const LAPTOP_KEEPALIVE_SECS: u32 = 25;
const HOST_PREFIX: &str = "/32";

pub fn config_text(net: &Net, record: &PeerRecord) -> Result<String> {
    let endpoint = net.endpoint()?;
    if endpoint.contains(char::is_whitespace) {
        return fail(
            "the stored server address contains whitespace; set it again with `hq net init --endpoint`",
        );
    }
    let mut out = String::new();
    out.push_str("[Interface]\n");
    out.push_str(
        "# PrivateKey is not here: this device made its own and it never leaves the device.\n",
    );
    out.push_str(&format!("Address = {}{HOST_PREFIX}\n\n", record.address));
    out.push_str("[Peer]\n");
    out.push_str(&format!("PublicKey = {}\n", net.hub_public_key));
    if let Some(psk) = &record.preshared_key {
        out.push_str(&format!("PresharedKey = {psk}\n"));
    }
    out.push_str(&format!("Endpoint = {endpoint}\n"));
    out.push_str(&format!("AllowedIPs = {}{HOST_PREFIX}\n", net.hub));
    if record.kind == Kind::Laptop {
        out.push_str(&format!("PersistentKeepalive = {LAPTOP_KEEPALIVE_SECS}\n"));
    }
    Ok(out)
}

/// The config and its QR code, with how to use them on this kind of device.
pub fn describe(net: &Net, record: &PeerRecord, with_qr: bool) -> Result<String> {
    let config = config_text(net, record)?;
    let mut out = String::new();
    out.push_str(&format!(
        "Config for {} ({}), address {}:\n\n{config}\n",
        record.label,
        record.kind.name(),
        record.address
    ));
    if with_qr {
        match qr::terminal(&config) {
            Some(code) => out.push_str(&format!(
                "The same text as a QR code (widen the terminal if it wraps):\n\n{code}\n"
            )),
            None => out.push_str("The config is too long for a QR code; copy the text above.\n"),
        }
    }
    out.push_str(&match record.kind {
        Kind::Laptop => "On the laptop: in the WireGuard app choose \"Add Empty Tunnel\". The app writes a PrivateKey line and shows the public key (that is what you gave --pubkey). Paste the lines above below it.\n".to_string(),
        Kind::Phone => format!(
            "On the phone: in the WireGuard app create the tunnel from scratch, let the app generate the key, then enter the address, the hub's public key, the endpoint and the allowed address shown above.{} Leave \"Block connections without VPN\" off.\n",
            if with_qr {
                " The Android app refuses to import a config without a PrivateKey line (checked in its source; the iOS app was not checked), so expect a scan of the QR code to be refused; use it to read the values from another screen."
            } else {
                ""
            }
        ),
    });
    if record.preshared_key.is_some() {
        out.push_str("This config holds a preshared key. Treat it like a password: share it only with that device.\n");
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::tests::{device_key, sample_net};

    fn record(kind: Kind, psk: Option<&str>) -> PeerRecord {
        PeerRecord {
            label: "mac".into(),
            kind,
            public_key: device_key(),
            address: "10.77.0.2".into(),
            preshared_key: psk.map(String::from),
        }
    }

    #[test]
    fn a_device_has_one_peer_the_hub_with_only_its_own_slash_32() {
        let net = sample_net();
        let text = config_text(&net, &record(Kind::Laptop, None)).unwrap();
        assert_eq!(text.matches("[Peer]").count(), 1);
        assert_eq!(text.matches("[Interface]").count(), 1);
        assert!(text.contains("AllowedIPs = 10.77.0.1/32\n"));
        assert!(text.contains("Address = 10.77.0.2/32\n"));
        assert!(text.contains(&format!("PublicKey = {}\n", net.hub_public_key)));
        assert!(text.contains("Endpoint = vpn.example.com:51820\n"));
        assert_eq!(text.matches("AllowedIPs").count(), 1);
        assert!(!text.contains("0.0.0.0"), "never a full tunnel");
        assert!(!text.contains("/0"));
    }

    #[test]
    fn no_private_key_value_is_ever_in_the_text() {
        let net = sample_net();
        let text = config_text(&net, &record(Kind::Phone, None)).unwrap();
        let key_lines: Vec<_> = text
            .lines()
            .filter(|l| l.trim_start().starts_with("PrivateKey"))
            .collect();
        assert!(key_lines.is_empty(), "{key_lines:?}");
        assert!(text.contains("PrivateKey is not here"));
    }

    #[test]
    fn only_a_laptop_keeps_the_path_open() {
        let net = sample_net();
        assert!(
            config_text(&net, &record(Kind::Laptop, None))
                .unwrap()
                .contains("PersistentKeepalive = 25\n")
        );
        assert!(
            !config_text(&net, &record(Kind::Phone, None))
                .unwrap()
                .contains("PersistentKeepalive")
        );
    }

    #[test]
    fn a_preshared_key_appears_only_when_the_device_has_one() {
        let net = sample_net();
        let psk = crate::key::new_preshared_key();
        let with = config_text(&net, &record(Kind::Laptop, Some(&psk))).unwrap();
        assert!(with.contains(&format!("PresharedKey = {psk}\n")));
        assert!(
            !config_text(&net, &record(Kind::Laptop, None))
                .unwrap()
                .contains("PresharedKey")
        );
    }

    #[test]
    fn without_a_server_address_there_is_no_config() {
        let mut net = sample_net();
        net.endpoint = None;
        let err = config_text(&net, &record(Kind::Laptop, None)).unwrap_err();
        assert!(err.to_string().contains("hq net init --endpoint"));
    }

    #[test]
    fn the_description_carries_the_config_a_qr_code_and_the_right_instructions() {
        let net = sample_net();
        let laptop = describe(&net, &record(Kind::Laptop, None), true).unwrap();
        assert!(laptop.contains("Add Empty Tunnel"));
        assert!(laptop.contains("QR code"));
        let phone = describe(&net, &record(Kind::Phone, None), false).unwrap();
        assert!(!phone.contains("QR code"));
        let phone_qr = describe(&net, &record(Kind::Phone, None), true).unwrap();
        assert!(phone_qr.contains("expect a scan of the QR code to be refused"));
    }
}
