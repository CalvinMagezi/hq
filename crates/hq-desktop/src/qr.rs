//! The pairing QR code as a matrix of modules the window draws itself, so no image or markup
//! built from the link ever reaches the page.

use qrcodegen::{QrCode, QrCodeEcc};
use serde::Serialize;

/// Dark modules are `1`, light ones `0`, one string per row. The window adds the quiet zone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct QrMatrix {
    pub size: usize,
    pub rows: Vec<String>,
}

/// `None` when the text is too long for any QR code.
pub fn matrix(text: &str) -> Option<QrMatrix> {
    let code = QrCode::encode_text(text, QrCodeEcc::Low).ok()?;
    let size = code.size();
    let rows = (0..size)
        .map(|y| {
            (0..size)
                .map(|x| if code.get_module(x, y) { '1' } else { '0' })
                .collect()
        })
        .collect();
    Some(QrMatrix {
        size: size as usize,
        rows,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_link_makes_a_square_matrix_and_a_huge_text_does_not() {
        let m = matrix("hqpair://ticket/abc").unwrap();
        assert_eq!(m.rows.len(), m.size);
        assert!(
            m.rows
                .iter()
                .all(|r| r.len() == m.size && r.bytes().all(|b| b == b'0' || b == b'1'))
        );
        assert!(matrix(&"x".repeat(10_000)).is_none());
    }
}
