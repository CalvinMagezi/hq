//! Frontmatter: split it off, read flat keys, and change one key without touching the others.
//!
//! The suite's keys are flat on purpose (`suite_kind`, `suite_saved_bytes`, ...): the vault's own
//! frontmatter tools take only strings, numbers, booleans and lists. Because a save must change
//! no byte it was not asked to change, this module never re-serialises the YAML. Reading goes
//! through `serde_yaml`; writing edits one line.

use std::fmt;

/// A frontmatter block is rejected above this size rather than parsed.
pub const MAX_FRONTMATTER_BYTES: usize = 64 * 1024;

/// A document cut into its frontmatter and the rest. Both keep their own line endings, so
/// `raw + body` is always the original text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Split<'a> {
    /// From the opening `---` through the newline after the closing `---`. Empty if none.
    pub raw: &'a str,
    pub body: &'a str,
}

/// Cuts `src` after a leading `---` block. A document whose first line is `---` but has no
/// closing line has no frontmatter, and neither does one whose block is over the size cap.
pub fn split(src: &str) -> Split<'_> {
    let none = Split { raw: "", body: src };
    let Some(first_end) = line_end(src, 0) else {
        return none;
    };
    if src[..first_end].trim_end() != "---" {
        return none;
    }
    let mut pos = first_end;
    while pos < src.len() {
        let end = line_end(src, pos).unwrap_or(src.len());
        let line = src[pos..end].trim_end();
        if line == "---" || line == "..." {
            if end > MAX_FRONTMATTER_BYTES {
                return none;
            }
            return Split {
                raw: &src[..end],
                body: &src[end..],
            };
        }
        pos = end;
    }
    none
}

/// End offset of the line starting at `start`, including its newline.
fn line_end(s: &str, start: usize) -> Option<usize> {
    if start >= s.len() {
        return None;
    }
    Some(s[start..].find('\n').map_or(s.len(), |i| start + i + 1))
}

/// A value the suite writes into frontmatter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scalar {
    Str(String),
    Int(i64),
    Bool(bool),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrontmatterError {
    /// The key is not letters, digits and `_` starting with a letter.
    BadKey(String),
    /// The value holds a line break; the suite's keys are single-line scalars.
    MultiLine,
    /// The existing value spans several lines or is a list, so a one-line edit would corrupt it.
    ExistingNotScalar(String),
    NotYaml(String),
}

impl fmt::Display for FrontmatterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FrontmatterError::BadKey(k) => write!(f, "`{k}` is not a valid frontmatter key"),
            FrontmatterError::MultiLine => write!(f, "frontmatter values must fit on one line"),
            FrontmatterError::ExistingNotScalar(k) => {
                write!(f, "`{k}` already holds a list or a multi-line value")
            }
            FrontmatterError::NotYaml(e) => write!(f, "the frontmatter is not valid YAML: {e}"),
        }
    }
}

impl std::error::Error for FrontmatterError {}

/// Letters, digits and `_`, starting with a letter. The vault's own keys are camel case
/// (`noteType`, `sourceFormat`), so capitals are allowed; the suite's own keys are snake case.
fn valid_key(k: &str) -> bool {
    let mut chars = k.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// The text between the fences of a raw block.
fn inner(raw: &str) -> &str {
    let after_open = raw.find('\n').map_or(raw.len(), |i| i + 1);
    let body = &raw[after_open..];
    match body.rfind("---").or_else(|| body.rfind("...")) {
        Some(i) if body[i..].trim_end().len() == 3 => &body[..i],
        _ => body,
    }
}

/// Parses the frontmatter into a YAML mapping. `Ok(None)` when the block is empty.
pub fn parse_mapping(raw: &str) -> Result<Option<serde_yaml::Mapping>, FrontmatterError> {
    if raw.is_empty() {
        return Ok(None);
    }
    let text = inner(raw);
    if text.trim().is_empty() {
        return Ok(None);
    }
    match serde_yaml::from_str::<serde_yaml::Value>(text) {
        Ok(serde_yaml::Value::Mapping(m)) => Ok(Some(m)),
        Ok(serde_yaml::Value::Null) => Ok(None),
        Ok(_) => Err(FrontmatterError::NotYaml("not a mapping".into())),
        Err(e) => Err(FrontmatterError::NotYaml(e.to_string())),
    }
}

/// Reads a top-level scalar as text. Numbers and booleans come back in their written form.
pub fn get(raw: &str, key: &str) -> Option<String> {
    let map = parse_mapping(raw).ok()??;
    match map.get(serde_yaml::Value::String(key.to_string()))? {
        serde_yaml::Value::String(s) => Some(s.clone()),
        serde_yaml::Value::Number(n) => Some(n.to_string()),
        serde_yaml::Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

fn render_scalar(v: &Scalar) -> Result<String, FrontmatterError> {
    match v {
        Scalar::Int(n) => Ok(n.to_string()),
        Scalar::Bool(b) => Ok(b.to_string()),
        Scalar::Str(s) => {
            if s.contains(['\n', '\r']) {
                return Err(FrontmatterError::MultiLine);
            }
            Ok(if is_plain_word(s) {
                s.clone()
            } else {
                quote(s)
            })
        }
    }
}

/// A word every YAML reader keeps as a string: lowercase words with `_` or `-`, and not one of
/// the spellings YAML 1.1 reads as a boolean or null.
fn is_plain_word(s: &str) -> bool {
    let mut chars = s.chars();
    let ok = matches!(chars.next(), Some(c) if c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-');
    ok && !matches!(
        s,
        "true" | "false" | "yes" | "no" | "on" | "off" | "null" | "y" | "n"
    )
}

fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Returns `raw` with `key` set. An existing one-line `key: value` is replaced in place; a new
/// key goes just before the closing fence. Every other line is returned byte for byte. With no
/// frontmatter yet, a new block is made.
pub fn set(raw: &str, key: &str, value: &Scalar, eol: &str) -> Result<String, FrontmatterError> {
    if !valid_key(key) {
        return Err(FrontmatterError::BadKey(key.to_string()));
    }
    let rendered = render_scalar(value)?;
    let new_line = format!("{key}: {rendered}{eol}");
    if raw.is_empty() {
        return Ok(format!("---{eol}{new_line}---{eol}"));
    }
    if let Some(m) = parse_mapping(raw)?
        && m.get(serde_yaml::Value::String(key.to_string())).is_some()
    {
        return replace_line(raw, key, &new_line);
    }
    // Insert before the closing fence line.
    let mut pos = line_end(raw, 0).unwrap_or(raw.len());
    let mut close_at = None;
    while pos < raw.len() {
        let end = line_end(raw, pos).unwrap_or(raw.len());
        let line = raw[pos..end].trim_end();
        if line == "---" || line == "..." {
            close_at = Some(pos);
        }
        pos = end;
    }
    let close = close_at.unwrap_or(raw.len());
    let mut out = String::with_capacity(raw.len() + new_line.len());
    out.push_str(&raw[..close]);
    // The last body line may lack a newline only if the block was malformed; keep it valid.
    if !out.ends_with('\n') {
        out.push_str(eol);
    }
    out.push_str(&new_line);
    out.push_str(&raw[close..]);
    Ok(out)
}

fn replace_line(raw: &str, key: &str, new_line: &str) -> Result<String, FrontmatterError> {
    let prefix = format!("{key}:");
    let mut out = String::with_capacity(raw.len());
    let mut pos = 0;
    let mut replaced = false;
    let mut first = true;
    while pos < raw.len() {
        let end = line_end(raw, pos).unwrap_or(raw.len());
        let line = &raw[pos..end];
        if !first && !replaced && line.starts_with(&prefix) {
            // A scalar sits on one line and the next line is not an indented continuation.
            let next = raw[end..].lines().next().unwrap_or("");
            let next_indented =
                next.starts_with([' ', '\t']) || next.starts_with("- ") || next == "-";
            let value = line[prefix.len()..].trim();
            if value.is_empty() || value.starts_with(['|', '>', '[', '{']) || next_indented {
                return Err(FrontmatterError::ExistingNotScalar(key.to_string()));
            }
            out.push_str(new_line);
            replaced = true;
        } else {
            out.push_str(line);
        }
        first = false;
        pos = end;
    }
    if replaced {
        Ok(out)
    } else {
        Err(FrontmatterError::ExistingNotScalar(key.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOC: &str =
        "---\ntitle: Stock\ntags:\n- shop\n- uganda\nsuite_kind: sheet\n---\n# Body\n";

    #[test]
    fn splits_and_rejoins_exactly() {
        let s = split(DOC);
        assert_eq!(s.body, "# Body\n");
        assert_eq!(format!("{}{}", s.raw, s.body), DOC);
        assert_eq!(split("# no front\n").raw, "");
        assert_eq!(split("---\nnever closed\n").raw, "");
        assert_eq!(split("").raw, "");
    }

    #[test]
    fn crlf_blocks_split_too() {
        let src = "---\r\ntitle: A\r\n---\r\nbody\r\n";
        let s = split(src);
        assert_eq!(s.body, "body\r\n");
        assert_eq!(get(s.raw, "title").as_deref(), Some("A"));
    }

    #[test]
    fn reads_scalars() {
        let raw = split(DOC).raw;
        assert_eq!(get(raw, "suite_kind").as_deref(), Some("sheet"));
        assert_eq!(get(raw, "title").as_deref(), Some("Stock"));
        assert_eq!(get(raw, "tags"), None, "a list is not a scalar");
        assert_eq!(get(raw, "missing"), None);
    }

    #[test]
    fn set_replaces_one_line_and_keeps_the_rest() {
        let raw = split(DOC).raw;
        let out = set(raw, "suite_kind", &Scalar::Str("doc".into()), "\n").unwrap();
        assert_eq!(
            out,
            "---\ntitle: Stock\ntags:\n- shop\n- uganda\nsuite_kind: doc\n---\n"
        );
    }

    #[test]
    fn set_appends_before_the_closing_fence() {
        let raw = split(DOC).raw;
        let out = set(raw, "suite_saved_bytes", &Scalar::Int(1234), "\n").unwrap();
        assert!(out.ends_with("suite_kind: sheet\nsuite_saved_bytes: 1234\n---\n"));
        assert!(out.starts_with("---\ntitle: Stock\n"));
        // Setting the same value again changes nothing.
        let again = set(&out, "suite_saved_bytes", &Scalar::Int(1234), "\n").unwrap();
        assert_eq!(again, out);
    }

    #[test]
    fn set_creates_a_block_when_there_is_none() {
        let out = set("", "suite_kind", &Scalar::Str("doc".into()), "\n").unwrap();
        assert_eq!(out, "---\nsuite_kind: doc\n---\n");
        assert_eq!(split(&out).raw, out);
    }

    #[test]
    fn set_refuses_to_flatten_a_list() {
        let raw = split(DOC).raw;
        assert_eq!(
            set(raw, "tags", &Scalar::Str("x".into()), "\n"),
            Err(FrontmatterError::ExistingNotScalar("tags".into()))
        );
    }

    #[test]
    fn strings_are_quoted_when_yaml_could_misread_them() {
        let raw = "---\na: 1\n---\n";
        for (v, expect) in [
            ("safe_word", "safe_word"),
            ("true", "\"true\""),
            ("2026-11-16", "\"2026-11-16\""),
            ("12", "\"12\""),
            ("has space", "\"has space\""),
            ("say \"hi\"", "\"say \\\"hi\\\"\""),
        ] {
            let out = set(raw, "k", &Scalar::Str(v.into()), "\n").unwrap();
            assert!(out.contains(&format!("k: {expect}\n")), "{v}: {out}");
            assert_eq!(get(&out, "k").as_deref(), Some(v));
        }
    }

    #[test]
    fn rejects_bad_keys_and_multiline_values() {
        let raw = "---\na: 1\n---\n";
        assert!(matches!(
            set(raw, "Bad Key", &Scalar::Int(1), "\n"),
            Err(FrontmatterError::BadKey(_))
        ));
        assert_eq!(
            set(raw, "k", &Scalar::Str("a\nb".into()), "\n"),
            Err(FrontmatterError::MultiLine)
        );
    }

    #[test]
    fn crlf_set_uses_the_documents_line_ending() {
        let raw = "---\r\ntitle: A\r\n---\r\n";
        let out = set(raw, "suite_kind", &Scalar::Str("doc".into()), "\r\n").unwrap();
        assert_eq!(out, "---\r\ntitle: A\r\nsuite_kind: doc\r\n---\r\n");
    }
}
