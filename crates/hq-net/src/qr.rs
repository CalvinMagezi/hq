//! A QR code drawn with terminal characters, two rows of modules per line.

use qrcodegen::{QrCode, QrCodeEcc};

/// Scanners want a quiet zone of four modules.
const QUIET_ZONE: i32 = 4;
/// Black on white whatever the terminal's own colours are, so a dark theme cannot invert the code.
const INK_ON_PAPER: &str = "\x1b[30;47m";
const RESET: &str = "\x1b[0m";

/// The QR code for `text`, or `None` when it does not fit in the largest code.
pub fn terminal(text: &str) -> Option<String> {
    let code = QrCode::encode_text(text, QrCodeEcc::Low).ok()?;
    let size = code.size();
    let dark =
        |x: i32, y: i32| (0..size).contains(&x) && (0..size).contains(&y) && code.get_module(x, y);
    let mut out = String::new();
    let mut y = -QUIET_ZONE;
    while y < size + QUIET_ZONE {
        out.push_str(INK_ON_PAPER);
        for x in -QUIET_ZONE..size + QUIET_ZONE {
            out.push(match (dark(x, y), dark(x, y + 1)) {
                (true, true) => '█',
                (true, false) => '▀',
                (false, true) => '▄',
                (false, false) => ' ',
            });
        }
        out.push_str(RESET);
        out.push('\n');
        y += 2;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_short_text_makes_a_square_code_with_a_quiet_zone() {
        let out = terminal("hello").unwrap();
        let size = QrCode::encode_text("hello", QrCodeEcc::Low).unwrap().size();
        let lines: Vec<_> = out.lines().collect();
        let rows = (size + 2 * QUIET_ZONE + 1) / 2;
        assert_eq!(lines.len() as i32, rows);
        let width = |line: &str| {
            line.chars()
                .filter(|c| matches!(c, '█' | '▀' | '▄' | ' '))
                .count() as i32
        };
        assert!(lines.iter().all(|l| width(l) == size + 2 * QUIET_ZONE));
        assert!(
            lines
                .iter()
                .all(|l| l.starts_with(INK_ON_PAPER) && l.ends_with(RESET))
        );
    }

    #[test]
    fn a_text_too_long_for_any_code_gives_none() {
        assert!(terminal(&"x".repeat(4000)).is_none());
    }

    #[test]
    fn a_device_config_fits() {
        let config = format!(
            "[Interface]\nAddress = 10.77.0.2/32\n\n[Peer]\nPublicKey = {}\nEndpoint = vpn.example.com:51820\nAllowedIPs = 10.77.0.1/32\nPersistentKeepalive = 25\n",
            "A".repeat(43) + "="
        );
        assert!(terminal(&config).is_some());
    }
}
