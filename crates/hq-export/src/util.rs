//! Small helpers shared by several writers.

use std::io::{Cursor, Write};

use crate::error::ExportError;

/// Links the exporter will keep. `javascript:`, `file:` and friends are
/// dropped so a shared document cannot carry an active or local link.
pub(crate) fn safe_url(url: &str) -> bool {
    let url = url.trim();
    match url.split_once(':') {
        // Relative links and fragments carry no scheme.
        None => true,
        Some((scheme, _)) => matches!(
            scheme.to_ascii_lowercase().as_str(),
            "http" | "https" | "mailto" | "tel"
        ),
    }
}

pub(crate) fn callout_color(kind: &str) -> &'static str {
    match kind {
        "tip" | "hint" | "success" | "check" | "done" => "#2e7d32",
        "warning" | "caution" | "attention" | "important" => "#e65100",
        "danger" | "error" | "bug" | "failure" | "fail" | "missing" => "#c62828",
        "quote" | "cite" | "example" | "abstract" | "summary" | "tldr" => "#6a1b9a",
        _ => "#1565c0",
    }
}

pub(crate) fn capitalise(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(f) => f.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// A file-name-safe lowercase slug, `fallback` when nothing usable remains.
pub(crate) fn slug(text: &str, fallback: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for c in text.chars() {
        if c.is_alphanumeric() {
            out.extend(c.to_lowercase());
            dash = false;
        } else if !dash && !out.is_empty() {
            out.push('-');
            dash = true;
        }
    }
    let out = out.trim_matches('-');
    let out: String = out.chars().take(40).collect();
    if out.is_empty() {
        fallback.to_owned()
    } else {
        out
    }
}

/// A cell value that is plainly a number: `-12`, `3.5`, `0`. Leading zeros
/// (`007`) and very long digit runs (ids, phone numbers) stay text so no
/// information is lost to floating point.
pub(crate) fn parse_number(cell: &str) -> Option<f64> {
    let s = cell.trim();
    let digits = s.strip_prefix('-').unwrap_or(s);
    let (int, frac) = match digits.split_once('.') {
        Some((i, f)) => (i, Some(f)),
        None => (digits, None),
    };
    if int.is_empty() || !int.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    if let Some(f) = frac
        && (f.is_empty() || !f.bytes().all(|b| b.is_ascii_digit()))
    {
        return None;
    }
    if int.len() > 1 && int.starts_with('0') {
        return None;
    }
    if int.len() + frac.map_or(0, str::len) > 15 {
        return None;
    }
    s.parse().ok()
}

/// Defuse spreadsheet formula injection in text that will be opened in Excel
/// or Sheets: a cell starting `=`, `+`, `@` or a tab, or `-` followed by
/// something that is not a number, gets a leading apostrophe.
pub(crate) fn defuse_formula(cell: &str) -> String {
    let risky = match cell.chars().next() {
        Some('=' | '+' | '@' | '\t' | '\r') => true,
        Some('-') => parse_number(cell).is_none(),
        _ => false,
    };
    if risky {
        format!("'{cell}")
    } else {
        cell.to_owned()
    }
}

/// Pack `(name, bytes)` pairs into a zip archive.
pub(crate) fn zip_files(files: Vec<(String, Vec<u8>)>) -> Result<Vec<u8>, ExportError> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for (name, bytes) in files {
        writer
            .start_file(name, options)
            .map_err(|e| ExportError::Render(e.to_string()))?;
        writer.write_all(&bytes)?;
    }
    let cursor = writer
        .finish()
        .map_err(|e| ExportError::Render(e.to_string()))?;
    Ok(cursor.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsafe_link_schemes_are_dropped() {
        assert!(safe_url("https://example.com"));
        assert!(safe_url("mailto:a@example.com"));
        assert!(safe_url("#section"));
        assert!(safe_url("notes/other.md"));
        assert!(!safe_url("javascript:alert(1)"));
        assert!(!safe_url("file:///etc/passwd"));
        assert!(!safe_url("JaVaScRiPt:alert(1)"));
    }

    #[test]
    fn numbers_are_recognised_conservatively() {
        assert_eq!(parse_number("42"), Some(42.0));
        assert_eq!(parse_number(" -3.5 "), Some(-3.5));
        assert_eq!(parse_number("0"), Some(0.0));
        assert_eq!(parse_number("0.5"), Some(0.5));
        assert_eq!(parse_number("007"), None, "leading zeros are identifiers");
        assert_eq!(parse_number("256700123456789012"), None, "long ids stay text");
        assert_eq!(parse_number("1,200"), None);
        assert_eq!(parse_number("12%"), None);
        assert_eq!(parse_number("1."), None);
        assert_eq!(parse_number("-"), None);
        assert_eq!(parse_number(""), None);
        assert_eq!(parse_number("NaN"), None);
        assert_eq!(parse_number("1e5"), None);
    }

    #[test]
    fn formula_prefixes_are_defused_but_negative_numbers_are_not() {
        assert_eq!(defuse_formula("=SUM(A1:A9)"), "'=SUM(A1:A9)");
        assert_eq!(defuse_formula("+1+1"), "'+1+1");
        assert_eq!(defuse_formula("@cmd"), "'@cmd");
        assert_eq!(defuse_formula("-cmd|calc"), "'-cmd|calc");
        assert_eq!(defuse_formula("-5"), "-5");
        assert_eq!(defuse_formula("plain"), "plain");
        assert_eq!(defuse_formula(""), "");
    }

    #[test]
    fn slugs_are_file_safe() {
        assert_eq!(slug("Field Visits: Q3!", "x"), "field-visits-q3");
        assert_eq!(slug("../../etc", "x"), "etc");
        assert_eq!(slug("!!!", "fallback"), "fallback");
    }

    #[test]
    fn zip_round_trips() {
        let bytes = zip_files(vec![("a.txt".into(), b"hi".to_vec())]).unwrap();
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        assert_eq!(archive.len(), 1);
        assert_eq!(archive.by_index(0).unwrap().name(), "a.txt");
    }
}
