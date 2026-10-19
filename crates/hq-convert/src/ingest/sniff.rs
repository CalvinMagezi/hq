//! What a file is, decided from its bytes first and its name second.
//!
//! A `.pdf` that holds a Word document, or a download saved without an
//! extension, still goes to the right reader. The extension only breaks ties
//! and names text formats that have no signature.

use std::io::{Cursor, Read};

use serde::Serialize;

use super::Capabilities;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Docx,
    Pptx,
    Xlsx,
    Odt,
    Ods,
    Odp,
    Doc,
    Xls,
    Ppt,
    Pdf,
    Rtf,
    Html,
    Xml,
    Csv,
    Tsv,
    Txt,
    Markdown,
    Image,
    Epub,
    /// A zip container that is none of the office formats.
    Zip,
    Unknown,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Docx => "docx",
            Kind::Pptx => "pptx",
            Kind::Xlsx => "xlsx",
            Kind::Odt => "odt",
            Kind::Ods => "ods",
            Kind::Odp => "odp",
            Kind::Doc => "doc",
            Kind::Xls => "xls",
            Kind::Ppt => "ppt",
            Kind::Pdf => "pdf",
            Kind::Rtf => "rtf",
            Kind::Html => "html",
            Kind::Xml => "xml",
            Kind::Csv => "csv",
            Kind::Tsv => "tsv",
            Kind::Txt => "txt",
            Kind::Markdown => "markdown",
            Kind::Image => "image",
            Kind::Epub => "epub",
            Kind::Zip => "zip",
            Kind::Unknown => "unknown",
        }
    }

    /// The kind a file name suggests, or `Unknown`.
    pub fn from_extension(ext: &str) -> Kind {
        match ext.to_ascii_lowercase().as_str() {
            "docx" | "docm" | "dotx" => Kind::Docx,
            "pptx" | "pptm" | "ppsx" => Kind::Pptx,
            "xlsx" | "xlsm" | "xlsb" => Kind::Xlsx,
            "odt" | "ott" => Kind::Odt,
            "ods" | "ots" => Kind::Ods,
            "odp" | "otp" => Kind::Odp,
            "doc" | "dot" => Kind::Doc,
            "xls" | "xlt" => Kind::Xls,
            "ppt" | "pps" => Kind::Ppt,
            "pdf" => Kind::Pdf,
            "rtf" => Kind::Rtf,
            "html" | "htm" | "xhtml" => Kind::Html,
            "xml" => Kind::Xml,
            "csv" => Kind::Csv,
            "tsv" | "tab" => Kind::Tsv,
            "txt" | "text" | "log" => Kind::Txt,
            "md" | "markdown" => Kind::Markdown,
            "png" | "jpg" | "jpeg" | "gif" | "bmp" | "tif" | "tiff" | "webp" | "heic" | "heif" => {
                Kind::Image
            }
            "epub" => Kind::Epub,
            "zip" => Kind::Zip,
            _ => Kind::Unknown,
        }
    }

    /// Whether this host can convert files of this kind, given what is installed.
    pub fn support(self, caps: &Capabilities) -> Support {
        match self {
            Kind::Docx
            | Kind::Pptx
            | Kind::Xlsx
            | Kind::Xls
            | Kind::Odt
            | Kind::Ods
            | Kind::Odp
            | Kind::Rtf
            | Kind::Html
            | Kind::Xml
            | Kind::Csv
            | Kind::Tsv
            | Kind::Txt
            | Kind::Pdf => Support::Ready,
            Kind::Image if caps.ocr => Support::Ready,
            Kind::Image => Support::NeedsOcr,
            Kind::Doc | Kind::Ppt if caps.legacy_office => Support::Ready,
            Kind::Doc | Kind::Ppt => Support::NeedsLibreOffice,
            Kind::Markdown | Kind::Epub | Kind::Zip | Kind::Unknown => Support::Unsupported,
        }
    }

    /// Formats a person can open in an Office suite but whose binary layout no
    /// native reader here understands.
    pub fn is_legacy_office(self) -> bool {
        matches!(self, Kind::Doc | Kind::Ppt)
    }
}

/// Whether a file of some kind can be converted on this host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Support {
    Ready,
    NeedsOcr,
    NeedsLibreOffice,
    Unsupported,
}

impl Support {
    pub fn label(self) -> &'static str {
        match self {
            Support::Ready => "ready",
            Support::NeedsOcr => "needs tesseract-ocr",
            Support::NeedsLibreOffice => "needs LibreOffice",
            Support::Unsupported => "unsupported",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum By {
    Signature,
    Content,
    Extension,
    SignatureAndExtension,
}

impl By {
    pub fn label(self) -> &'static str {
        match self {
            By::Signature => "signature",
            By::Content => "content",
            By::Extension => "extension",
            By::SignatureAndExtension => "signature+extension",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sniffed {
    pub kind: Kind,
    pub by: By,
    /// What the file name says, when it says something different from the content.
    pub name_says: Option<Kind>,
    /// Password protected: the content cannot be read without the password.
    pub encrypted: bool,
}

const OLE2: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];

/// Directory entries of an OLE2 file name their streams in UTF-16.
fn utf16(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(|u| u.to_le_bytes()).collect()
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && haystack.len() >= needle.len()
        && haystack.windows(needle.len()).any(|w| w == needle)
}

pub fn sniff(bytes: &[u8], ext: &str) -> Sniffed {
    let by_name = Kind::from_extension(ext);
    let signed = |kind: Kind, encrypted: bool| {
        let agrees = by_name == kind;
        Sniffed {
            kind,
            by: if agrees {
                By::SignatureAndExtension
            } else {
                By::Signature
            },
            name_says: (!agrees && by_name != Kind::Unknown).then_some(by_name),
            encrypted,
        }
    };

    if bytes.starts_with(b"%PDF-") {
        return signed(Kind::Pdf, false);
    }
    if is_image(bytes) {
        return signed(Kind::Image, false);
    }
    if bytes.starts_with(b"PK\x03\x04") || bytes.starts_with(b"PK\x05\x06") {
        return signed(zip_kind(bytes, by_name), false);
    }
    if bytes.starts_with(&OLE2) {
        let head = &bytes[..bytes.len().min(8 * 1024 * 1024)];
        if contains(head, &utf16("EncryptedPackage")) {
            let kind = match by_name {
                Kind::Pptx | Kind::Xlsx | Kind::Docx => by_name,
                _ => Kind::Docx,
            };
            return signed(kind, true);
        }
        let kind = if contains(head, &utf16("WordDocument")) {
            Kind::Doc
        } else if contains(head, &utf16("PowerPoint Document")) {
            Kind::Ppt
        } else if contains(head, &utf16("Workbook")) || contains(head, &utf16("Book")) {
            Kind::Xls
        } else {
            match by_name {
                Kind::Doc | Kind::Xls | Kind::Ppt => by_name,
                _ => Kind::Doc,
            }
        };
        return signed(kind, false);
    }

    // Text formats have no signature; look at what the text is.
    if let Some(text) = head_text(bytes) {
        let lower: String = text
            .trim_start()
            .chars()
            .take(1500)
            .collect::<String>()
            .to_lowercase();
        if lower.starts_with("{\\rtf") {
            return content(Kind::Rtf, by_name);
        }
        if lower.starts_with("<!doctype html")
            || lower.starts_with("<html")
            || (lower.starts_with("<?xml") && has_tag(&lower, "html"))
            || (lower.starts_with('<')
                && !lower.starts_with("<?xml")
                && (has_tag(&lower, "body") || has_tag(&lower, "head")))
        {
            return content(Kind::Html, by_name);
        }
        if lower.starts_with("<?xml")
            || (lower.starts_with('<')
                && matches!(by_name, Kind::Unknown | Kind::Xml)
                && (by_name == Kind::Xml || lower.contains("</") || lower.contains("/>")))
        {
            return content(Kind::Xml, by_name);
        }
        return match by_name {
            Kind::Html | Kind::Xml | Kind::Csv | Kind::Tsv | Kind::Txt | Kind::Markdown => {
                Sniffed {
                    kind: by_name,
                    by: By::Extension,
                    name_says: None,
                    encrypted: false,
                }
            }
            // A named office file whose content is plain text is not that office file.
            _ => content(Kind::Txt, by_name),
        };
    }

    Sniffed {
        kind: by_name,
        by: By::Extension,
        name_says: None,
        encrypted: false,
    }
}

/// Whether `<name` appears as a whole tag name (`<head>` but not `<heading>`).
fn has_tag(text: &str, name: &str) -> bool {
    let open = format!("<{name}");
    text.match_indices(&open).any(|(i, _)| {
        matches!(
            text[i + open.len()..].chars().next(),
            Some('>' | '/' | ' ' | '\n' | '\t' | '\r')
        )
    })
}

fn content(kind: Kind, by_name: Kind) -> Sniffed {
    let agrees = by_name == kind;
    Sniffed {
        kind,
        by: if agrees {
            By::SignatureAndExtension
        } else {
            By::Content
        },
        name_says: (!agrees && by_name != Kind::Unknown).then_some(by_name),
        encrypted: false,
    }
}

fn is_image(b: &[u8]) -> bool {
    b.starts_with(&[0x89, b'P', b'N', b'G'])
        || b.starts_with(&[0xFF, 0xD8, 0xFF])
        || b.starts_with(b"GIF8")
        || b.starts_with(b"BM") && b.len() > 14 && b[6..10] == [0, 0, 0, 0]
        || b.starts_with(&[b'I', b'I', 42, 0])
        || b.starts_with(&[b'M', b'M', 0, 42])
        || (b.len() > 12 && &b[..4] == b"RIFF" && &b[8..12] == b"WEBP")
        || (b.len() > 12
            && &b[4..8] == b"ftyp"
            && matches!(&b[8..12], b"heic" | b"heix" | b"mif1" | b"msf1"))
}

/// What a zip container holds, from its entry names (and `mimetype` for ODF).
fn zip_kind(bytes: &[u8], by_name: Kind) -> Kind {
    let Ok(mut zip) = zip::ZipArchive::new(Cursor::new(bytes)) else {
        // Not a readable archive; the name decides, and the reader reports the damage.
        return if by_name == Kind::Unknown {
            Kind::Zip
        } else {
            by_name
        };
    };
    let names: Vec<String> = zip
        .file_names()
        .filter_map(Result::ok)
        .map(|n| n.into_owned())
        .collect();
    let has = |n: &str| names.iter().any(|x| x == n);
    if has("word/document.xml") {
        return Kind::Docx;
    }
    if has("ppt/presentation.xml") {
        return Kind::Pptx;
    }
    if has("xl/workbook.xml") || has("xl/workbook.bin") {
        return Kind::Xlsx;
    }
    if has("mimetype") {
        let mut mime = String::new();
        if let Ok(f) = zip.by_name("mimetype") {
            let _ = f.take(256).read_to_string(&mut mime);
        }
        let mime = mime.trim();
        return match mime {
            m if m.ends_with("opendocument.text") || m.ends_with("opendocument.text-template") => {
                Kind::Odt
            }
            m if m.ends_with("opendocument.spreadsheet")
                || m.ends_with("opendocument.spreadsheet-template") =>
            {
                Kind::Ods
            }
            m if m.ends_with("opendocument.presentation")
                || m.ends_with("opendocument.presentation-template") =>
            {
                Kind::Odp
            }
            "application/epub+zip" => Kind::Epub,
            _ => Kind::Zip,
        };
    }
    Kind::Zip
}

/// The start of the file as text, if it is text: UTF-8, or UTF-16 with or without a byte order mark.
fn head_text(bytes: &[u8]) -> Option<String> {
    let head = &bytes[..bytes.len().min(4096)];
    if let Some(enc) = super::text::utf16_guess(head) {
        return Some(enc.decode_without_bom_handling(head).0.into_owned());
    }
    if head.starts_with(&[0xFF, 0xFE]) {
        return Some(encoding_rs::UTF_16LE.decode(&head[2..]).0.into_owned());
    }
    if head.starts_with(&[0xFE, 0xFF]) {
        return Some(encoding_rs::UTF_16BE.decode(&head[2..]).0.into_owned());
    }
    if head.contains(&0) {
        return None;
    }
    let head = head.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(head);
    // A cut multi-byte character at the end of the window is not a reason to reject.
    let valid = match std::str::from_utf8(head) {
        Ok(s) => s.to_string(),
        Err(e) if e.error_len().is_none() => {
            String::from_utf8_lossy(&head[..e.valid_up_to()]).into_owned()
        }
        // Not UTF-8: a legacy single-byte encoding. Still text if it has no control bytes.
        Err(_) => {
            if head.iter().any(|&b| b < 9 || (b > 13 && b < 32)) {
                return None;
            }
            encoding_rs::WINDOWS_1252.decode(head).0.into_owned()
        }
    };
    Some(valid)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signatures_win_over_the_extension() {
        let s = sniff(b"%PDF-1.7 ...", "docx");
        assert_eq!((s.kind, s.name_says), (Kind::Pdf, Some(Kind::Docx)));
        let s = sniff(b"{\\rtf1\\ansi hi}", "");
        assert_eq!(s.kind, Kind::Rtf);
    }

    #[test]
    fn plain_text_in_an_office_named_file_is_text() {
        let s = sniff(b"just some notes", "docx");
        assert_eq!((s.kind, s.by), (Kind::Txt, By::Content));
    }

    #[test]
    fn html_and_xml_are_told_apart_by_content() {
        assert_eq!(
            sniff(b"<!DOCTYPE html><html></html>", "txt").kind,
            Kind::Html
        );
        assert_eq!(sniff(b"<?xml version=\"1.0\"?><a/>", "").kind, Kind::Xml);
        assert_eq!(
            sniff(b"<?xml version=\"1.0\"?><html><body/></html>", "xml").kind,
            Kind::Html
        );
    }

    #[test]
    fn xml_with_tags_that_start_like_html_stays_xml() {
        assert_eq!(
            sniff(b"<report><heading>Findings</heading></report>", "").kind,
            Kind::Xml
        );
        assert_eq!(
            sniff(
                b"<?xml version=\"1.0\"?><ONIXMessage><Header/></ONIXMessage>",
                "xml"
            )
            .kind,
            Kind::Xml
        );
    }

    #[test]
    fn utf16_with_a_bom_is_text() {
        let mut b = vec![0xFF, 0xFE];
        b.extend("<html></html>".encode_utf16().flat_map(|u| u.to_le_bytes()));
        assert_eq!(sniff(&b, "html").kind, Kind::Html);
    }

    #[test]
    fn ole2_names_the_legacy_format() {
        let mut doc = OLE2.to_vec();
        doc.extend(utf16("WordDocument"));
        assert_eq!(sniff(&doc, "").kind, Kind::Doc);
        let mut enc = OLE2.to_vec();
        enc.extend(utf16("EncryptedPackage"));
        let s = sniff(&enc, "xlsx");
        assert!(s.encrypted && s.kind == Kind::Xlsx);
    }

    #[test]
    fn unknown_binary_stays_unknown() {
        assert_eq!(sniff(&[0, 1, 2, 3, 4], "bin").kind, Kind::Unknown);
    }
}
