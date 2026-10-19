//! Plain text, CSV and TSV, and the text decoding shared by the readers that
//! take bytes in an unknown encoding.

use encoding_rs::Encoding;

use super::ir::{Block, Inline, Parsed, Table};
use super::report::{LossKind, Losses};
use crate::types::ConvertError;

/// Text from bytes of unknown encoding, and whether the encoding was a guess.
///
/// A byte order mark wins, then UTF-8, then `declared` (a label from a
/// `<meta>` tag or similar), then Windows-1252 as the usual legacy default.
pub fn decode(bytes: &[u8], declared: Option<&str>) -> (String, bool) {
    if let Some((enc, len)) = Encoding::for_bom(bytes) {
        return (
            enc.decode_without_bom_handling(&bytes[len..])
                .0
                .into_owned(),
            false,
        );
    }
    if let Some(enc) = utf16_guess(bytes) {
        return (enc.decode_without_bom_handling(bytes).0.into_owned(), false);
    }
    if let Ok(s) = std::str::from_utf8(bytes) {
        return (s.to_string(), false);
    }
    if let Some(enc) = declared.and_then(|l| Encoding::for_label(l.trim().as_bytes())) {
        return (enc.decode_without_bom_handling(bytes).0.into_owned(), false);
    }
    (
        encoding_rs::WINDOWS_1252
            .decode_without_bom_handling(bytes)
            .0
            .into_owned(),
        true,
    )
}

/// UTF-16 without a byte order mark: ASCII text shows as a zero byte in every
/// second position.
pub fn utf16_guess(bytes: &[u8]) -> Option<&'static Encoding> {
    let n = bytes.len().min(512) & !1;
    if n < 16 {
        return None;
    }
    let (mut even, mut odd) = (0usize, 0usize);
    for (i, b) in bytes[..n].iter().enumerate() {
        if *b == 0 {
            if i % 2 == 0 {
                even += 1;
            } else {
                odd += 1;
            }
        }
    }
    let pairs = n / 2;
    let enc = if odd * 10 >= pairs * 3 && even * 10 < pairs {
        encoding_rs::UTF_16LE
    } else if even * 10 >= pairs * 3 && odd * 10 < pairs {
        encoding_rs::UTF_16BE
    } else {
        return None;
    };
    // Binary data with many zero bytes is not text: the decoded sample must read as text.
    let (sample, _) = enc.decode_without_bom_handling(&bytes[..n]);
    let readable = sample
        .chars()
        .filter(|c| !c.is_control() || c.is_whitespace())
        .filter(|c| *c != '\u{fffd}')
        .count();
    (readable * 10 >= sample.chars().count() * 9).then_some(enc)
}

/// Plain text and Markdown pass through; line endings are normalised.
pub fn read_plain(bytes: &[u8]) -> Result<Parsed, ConvertError> {
    let (text, guessed) = decode(bytes, None);
    let mut losses = Losses::default();
    if guessed {
        losses.note(
            LossKind::EncodingGuessed,
            "the text is not UTF-8; read as Windows-1252",
        );
    }
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    Ok(Parsed {
        blocks: vec![Block::Raw(text.trim().to_string())],
        title: None,
        losses,
    })
}

/// Delimited text as a table. The delimiter is the one that splits the first
/// line most evenly, for files from spreadsheets that use `;` or tabs.
pub fn read_delimited(
    bytes: &[u8],
    force_tab: bool,
    max_rows: usize,
) -> Result<Parsed, ConvertError> {
    let (text, guessed) = decode(bytes, None);
    let mut losses = Losses::default();
    if guessed {
        losses.note(
            LossKind::EncodingGuessed,
            "the text is not UTF-8; read as Windows-1252",
        );
    }
    let delim = if force_tab {
        '\t'
    } else {
        pick_delimiter(&text)
    };
    let mut rows = parse_rows(&text, delim);
    while rows
        .last()
        .is_some_and(|r| r.iter().all(|c| c.trim().is_empty()))
    {
        rows.pop();
    }
    if rows.is_empty() {
        return Err(ConvertError::InboundFailed("no rows found".into()));
    }
    let width = rows.iter().map(Vec::len).max().unwrap_or(0);
    if rows.len() > max_rows + 1 {
        losses.note(
            LossKind::Truncated,
            format!(
                "showing the first {max_rows} of {} data rows",
                rows.len() - 1
            ),
        );
        rows.truncate(max_rows + 1);
    }
    let table = Table {
        rows: rows
            .into_iter()
            .map(|mut r| {
                r.resize(width, String::new());
                r.into_iter()
                    .map(|c| vec![Inline::text(c.trim().to_string())])
                    .collect()
            })
            .collect(),
    };
    Ok(Parsed {
        blocks: vec![Block::Table(table)],
        title: None,
        losses,
    })
}

fn pick_delimiter(text: &str) -> char {
    let first: String = text
        .trim_start_matches('\u{feff}')
        .lines()
        .take(5)
        .collect::<Vec<_>>()
        .join("\n");
    [',', ';', '\t', '|']
        .into_iter()
        .max_by_key(|d| first.matches(*d).count())
        .filter(|d| first.contains(*d))
        .unwrap_or(',')
}

/// RFC 4180 rows: quoted fields may hold the delimiter, quotes and newlines.
fn parse_rows(text: &str, delim: char) -> Vec<Vec<String>> {
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.trim_start_matches('\u{feff}').chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    field.push('"');
                    chars.next();
                } else {
                    quoted = false;
                }
            } else {
                field.push(c);
            }
            continue;
        }
        match c {
            '"' if field.is_empty() => quoted = true,
            c if c == delim => row.push(std::mem::take(&mut field)),
            '\r' => {}
            '\n' => {
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
            }
            c => field.push(c),
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn md(p: Parsed) -> String {
        super::super::ir::render(p.blocks)
    }

    #[test]
    fn csv_with_quotes_and_embedded_delimiters() {
        let csv = "Site,Note\nMukono,\"rain, then sun\"\nWakiso,\"said \"\"ok\"\"\"\n";
        assert_eq!(
            md(read_delimited(csv.as_bytes(), false, 100).unwrap()),
            "| Site | Note |\n| --- | --- |\n| Mukono | rain, then sun |\n| Wakiso | said \"ok\" |\n"
        );
    }

    #[test]
    fn semicolon_delimited_files_are_recognised() {
        let csv = "Site;Farmers\nMukono;42\n";
        assert_eq!(
            md(read_delimited(csv.as_bytes(), false, 100).unwrap()),
            "| Site | Farmers |\n| --- | --- |\n| Mukono | 42 |\n"
        );
    }

    #[test]
    fn rows_beyond_the_cap_are_counted() {
        let csv = "a\n1\n2\n3\n";
        let p = read_delimited(csv.as_bytes(), false, 2).unwrap();
        assert_eq!(
            p.losses.into_warnings()[0].detail,
            "showing the first 2 of 3 data rows"
        );
    }

    #[test]
    fn legacy_encodings_are_read_and_flagged() {
        let (s, guessed) = decode(b"caf\xe9", None);
        assert_eq!((s.as_str(), guessed), ("caf\u{e9}", true));
        let (s, guessed) = decode(b"caf\xe9", Some("iso-8859-1"));
        assert_eq!((s.as_str(), guessed), ("caf\u{e9}", false));
    }

    #[test]
    fn utf16_without_a_bom_is_recognised() {
        let le: Vec<u8> = "<html><body>hello</body></html>"
            .encode_utf16()
            .flat_map(|u| u.to_le_bytes())
            .collect();
        assert_eq!(decode(&le, None).0, "<html><body>hello</body></html>");
        let be: Vec<u8> = "<html><body>hello</body></html>"
            .encode_utf16()
            .flat_map(|u| u.to_be_bytes())
            .collect();
        assert_eq!(decode(&be, None).0, "<html><body>hello</body></html>");
    }

    #[test]
    fn short_binary_data_is_not_mistaken_for_utf16() {
        assert!(utf16_guess(&[0, 1, 2, 3, 0xff, 0xfe, 0, 9]).is_none());
        let noise: Vec<u8> = (0..64u8).map(|i| if i % 2 == 0 { 0 } else { i }).collect();
        assert!(utf16_guess(&noise).is_none());
    }

    #[test]
    fn text_keeps_its_content_and_drops_carriage_returns() {
        assert_eq!(
            md(read_plain(b"one\r\n\r\ntwo\r\n").unwrap()),
            "one\n\ntwo\n"
        );
    }
}
