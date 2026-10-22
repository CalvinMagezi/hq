//! A QR code drawn with half-block characters, two rows of modules per line of text.

use qrcodegen::{QrCode, QrCodeEcc};

const QUIET_ZONE: i32 = 2;

pub fn render(text: &str) -> Option<String> {
    let code = QrCode::encode_text(text, QrCodeEcc::Low).ok()?;
    let size = code.size();
    let dark =
        |x: i32, y: i32| (0..size).contains(&x) && (0..size).contains(&y) && code.get_module(x, y);
    let mut out = String::new();
    let mut y = -QUIET_ZONE;
    while y < size + QUIET_ZONE {
        for x in -QUIET_ZONE..size + QUIET_ZONE {
            out.push(match (dark(x, y), dark(x, y + 1)) {
                (true, true) => '\u{2588}',
                (true, false) => '\u{2580}',
                (false, true) => '\u{2584}',
                (false, false) => ' ',
            });
        }
        out.push('\n');
        y += 2;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ticket_sized_text_makes_a_code_and_a_huge_one_does_not() {
        let art = render(&"hqpair://ticket/".repeat(30)).unwrap();
        assert!(
            art.lines().count() > 10
                && art
                    .lines()
                    .all(|l| l.chars().count() == art.lines().next().unwrap().chars().count())
        );
        assert!(render(&"x".repeat(10_000)).is_none());
    }
}
