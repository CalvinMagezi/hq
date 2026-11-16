//! The info string of a typed block: ``sheet name="Stock" types="text,int"``.
//!
//! Grammar (see `docs/suite/HQMD.md`):
//!
//! ```text
//! info  = type *( WS attr )
//! type  = lower *( lower / digit / "-" )
//! attr  = key "=" value
//! key   = lower *( lower / digit / "_" / "-" )
//! value = bare / quoted
//! bare  = 1*( any character except WS, quote, backtick )
//! quoted = DQUOTE *( char / "\\" / "\"" ) DQUOTE
//! ```
//!
//! Parsing is strict about shape and lenient about nothing else: a bare word, a duplicate key or
//! an unterminated quote is an error, so a block with a damaged header is reported and left
//! alone, never "fixed" by guessing.

use std::fmt;

/// Longest info string accepted, in bytes. A real header is under 200.
pub const MAX_INFO_LEN: usize = 1024;
/// Most attributes one header may carry.
pub const MAX_ATTRS: usize = 24;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attr {
    pub key: String,
    pub value: String,
    /// Whether the value was written in quotes. Kept so that changing one attribute of a header
    /// does not rewrite the others' spelling.
    pub quoted: bool,
}

impl Attr {
    /// An attribute to be written. Values with anything beyond letters, digits, `_`, `.` and
    /// `-` are quoted, which keeps headers easy to read in a plain editor.
    pub fn new(key: impl Into<String>, value: impl Into<String>) -> Attr {
        let value = value.into();
        let simple = !value.is_empty()
            && value
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '-');
        Attr {
            key: key.into(),
            quoted: !simple,
            value,
        }
    }

    fn needs_quotes(&self) -> bool {
        self.quoted
            || self.value.is_empty()
            || self.value.chars().any(|c| c.is_whitespace() || c == '"')
    }
}

/// A parsed info string: the block type and its attributes in the order written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Info {
    pub kind: String,
    pub attrs: Vec<Attr>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InfoError {
    Empty,
    TooLong,
    TooManyAttrs,
    BadType(String),
    BadKey(String),
    /// A word with no `=`, such as ``sheet stock``.
    BareWord(String),
    UnterminatedQuote,
    /// A character other than `\\` or `\"` after a backslash inside quotes.
    BadEscape(char),
    /// Text right after a closing quote, such as `a="b"c`.
    JunkAfterQuote,
    DuplicateKey(String),
}

impl fmt::Display for InfoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InfoError::Empty => write!(f, "the block has no type"),
            InfoError::TooLong => write!(f, "the header is longer than {MAX_INFO_LEN} bytes"),
            InfoError::TooManyAttrs => write!(f, "the header has more than {MAX_ATTRS} attributes"),
            InfoError::BadType(t) => write!(f, "`{t}` is not a valid block type"),
            InfoError::BadKey(k) => write!(f, "`{k}` is not a valid attribute name"),
            InfoError::BareWord(w) => write!(f, "`{w}` needs a value, written as {w}=value"),
            InfoError::UnterminatedQuote => write!(f, "a quoted value is never closed"),
            InfoError::BadEscape(c) => write!(f, "`\\{c}` is not an escape; use \\\\ or \\\""),
            InfoError::JunkAfterQuote => write!(f, "text follows a closing quote"),
            InfoError::DuplicateKey(k) => write!(f, "the attribute `{k}` is given twice"),
        }
    }
}

impl std::error::Error for InfoError {}

fn is_type(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn is_key(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

/// The first word of an info string, or `None` if it is empty. Cheap: used to decide whether a
/// fenced block is one of ours before the rest of the header is parsed.
pub fn first_word(info: &str) -> Option<&str> {
    info.split_whitespace().next()
}

impl Info {
    pub fn parse(info: &str) -> Result<Info, InfoError> {
        let info = info.trim();
        if info.is_empty() {
            return Err(InfoError::Empty);
        }
        if info.len() > MAX_INFO_LEN {
            return Err(InfoError::TooLong);
        }
        let mut rest = info;
        let kind_end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        let kind = &rest[..kind_end];
        if !is_type(kind) {
            return Err(InfoError::BadType(kind.to_string()));
        }
        rest = rest[kind_end..].trim_start();
        let mut attrs: Vec<Attr> = Vec::new();
        while !rest.is_empty() {
            if attrs.len() >= MAX_ATTRS {
                return Err(InfoError::TooManyAttrs);
            }
            let word_end = rest
                .find(|c: char| c.is_whitespace() || c == '=')
                .unwrap_or(rest.len());
            let word = &rest[..word_end];
            if !rest[word_end..].starts_with('=') {
                return Err(InfoError::BareWord(word.to_string()));
            }
            if !is_key(word) {
                return Err(InfoError::BadKey(word.to_string()));
            }
            if attrs.iter().any(|a| a.key == word) {
                return Err(InfoError::DuplicateKey(word.to_string()));
            }
            rest = &rest[word_end + 1..];
            let quoted = rest.starts_with('"');
            let (value, after) = if let Some(inner) = rest.strip_prefix('"') {
                read_quoted(inner)?
            } else {
                let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
                let v = &rest[..end];
                if v.contains('"') {
                    return Err(InfoError::JunkAfterQuote);
                }
                (v.to_string(), &rest[end..])
            };
            attrs.push(Attr {
                key: word.to_string(),
                value,
                quoted,
            });
            rest = after.trim_start();
        }
        Ok(Info {
            kind: kind.to_string(),
            attrs,
        })
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|a| a.key == key)
            .map(|a| a.value.as_str())
    }

    /// The block's `id=`, if it has one.
    pub fn id(&self) -> Option<&str> {
        self.get("id")
    }

    /// The canonical header text. Values are quoted only when they need to be, so printing a
    /// parsed header gives back what a person would have written.
    pub fn render(&self) -> String {
        let mut out = self.kind.clone();
        for a in &self.attrs {
            out.push(' ');
            out.push_str(&a.key);
            out.push('=');
            push_value(&mut out, a);
        }
        out
    }

    /// Whether the header holds a backtick, which a backtick fence cannot carry.
    pub fn has_backtick(&self) -> bool {
        self.attrs.iter().any(|a| a.value.contains('`'))
    }
}

fn read_quoted(s: &str) -> Result<(String, &str), InfoError> {
    let mut out = String::new();
    let mut chars = s.char_indices();
    while let Some((i, c)) = chars.next() {
        match c {
            '"' => {
                let after = &s[i + 1..];
                if after.chars().next().is_some_and(|n| !n.is_whitespace()) {
                    return Err(InfoError::JunkAfterQuote);
                }
                return Ok((out, after));
            }
            '\\' => match chars.next() {
                Some((_, e @ ('\\' | '"'))) => out.push(e),
                Some((_, other)) => return Err(InfoError::BadEscape(other)),
                None => return Err(InfoError::UnterminatedQuote),
            },
            c => out.push(c),
        }
    }
    Err(InfoError::UnterminatedQuote)
}

fn push_value(out: &mut String, a: &Attr) {
    if !a.needs_quotes() {
        out.push_str(&a.value);
        return;
    }
    out.push('"');
    for c in a.value.chars() {
        if c == '"' || c == '\\' {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_bare_and_quoted_values() {
        let i = Info::parse(r#"sheet name="Stock take" types="text,int,ugx" id=stock-1"#).unwrap();
        assert_eq!(i.kind, "sheet");
        assert_eq!(i.get("name"), Some("Stock take"));
        assert_eq!(i.get("types"), Some("text,int,ugx"));
        assert_eq!(i.id(), Some("stock-1"));
    }

    #[test]
    fn render_keeps_each_values_spelling() {
        let canonical = r#"sheet name="Stock" types="text,int" id=a rows=40"#;
        let i = Info::parse(canonical).unwrap();
        assert_eq!(i.render(), canonical);
        assert_eq!(Info::parse(&i.render()).unwrap(), i);
    }

    #[test]
    fn escapes_survive() {
        let i = Info::parse(r#"tile label="say \"hi\" \\ now" from=A1:A2 fn=sum"#).unwrap();
        assert_eq!(i.get("label"), Some(r#"say "hi" \ now"#));
        assert_eq!(Info::parse(&i.render()).unwrap(), i);
    }

    #[test]
    fn a_header_with_only_a_type_is_fine() {
        let i = Info::parse("chart").unwrap();
        assert!(i.attrs.is_empty());
        assert_eq!(i.render(), "chart");
    }

    #[test]
    fn rejects_damaged_headers() {
        assert_eq!(Info::parse(""), Err(InfoError::Empty));
        assert!(matches!(Info::parse("Sheet"), Err(InfoError::BadType(_))));
        assert!(matches!(
            Info::parse("sheet stock"),
            Err(InfoError::BareWord(w)) if w == "stock"
        ));
        assert!(matches!(
            Info::parse("sheet Name=x"),
            Err(InfoError::BadKey(_))
        ));
        assert_eq!(
            Info::parse(r#"sheet name="open"#),
            Err(InfoError::UnterminatedQuote)
        );
        assert_eq!(
            Info::parse(r#"sheet name="a"b"#),
            Err(InfoError::JunkAfterQuote)
        );
        assert!(matches!(
            Info::parse(r#"sheet name="a\n""#),
            Err(InfoError::BadEscape('n'))
        ));
        assert!(matches!(
            Info::parse("sheet a=1 a=2"),
            Err(InfoError::DuplicateKey(_))
        ));
    }

    #[test]
    fn caps_hostile_headers() {
        let long = format!("sheet a={}", "x".repeat(MAX_INFO_LEN));
        assert_eq!(Info::parse(&long), Err(InfoError::TooLong));
        let many: String = (0..=MAX_ATTRS)
            .map(|i| format!(" k{i}=v"))
            .collect::<Vec<_>>()
            .join("");
        assert_eq!(
            Info::parse(&format!("sheet{many}")),
            Err(InfoError::TooManyAttrs)
        );
    }

    #[test]
    fn empty_value_is_quoted_on_render() {
        let i = Info::parse(r#"tile label="" from=A1:A2 fn=sum"#).unwrap();
        assert_eq!(i.get("label"), Some(""));
        assert!(i.render().contains(r#"label="""#));
    }
}
