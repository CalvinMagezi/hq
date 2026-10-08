//! Look of an exported document: accent colour, font and footer text.

use hq_convert::brand::BrandKit;

pub const DEFAULT_ACCENT: &str = "#1f4e79";

/// Everything a writer needs to style a document. Values are validated on the
/// way in, so a writer can place them in its output without further escaping
/// worries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Theme {
    /// `#rgb`, `#rrggbb` or `#rrggbbaa`.
    pub accent: String,
    /// Preferred body font family. Falls back to the built-in fonts when absent.
    pub font: Option<String>,
    /// Short text printed in the footer, usually the brand name.
    pub footer: String,
}

impl Default for Theme {
    fn default() -> Self {
        Theme {
            accent: DEFAULT_ACCENT.to_owned(),
            font: None,
            footer: String::new(),
        }
    }
}

impl Theme {
    pub fn from_brand(brand: &BrandKit) -> Self {
        Theme {
            accent: valid_hex(&brand.primary_color).unwrap_or_else(|| DEFAULT_ACCENT.to_owned()),
            font: clean_name(&brand.font),
            footer: brand.brand.trim().chars().take(60).collect(),
        }
    }
}

fn valid_hex(value: &str) -> Option<String> {
    let value = value.trim();
    let digits = value.strip_prefix('#')?;
    let ok = matches!(digits.len(), 3 | 6 | 8) && digits.chars().all(|c| c.is_ascii_hexdigit());
    ok.then(|| format!("#{}", digits.to_ascii_lowercase()))
}

fn clean_name(value: &str) -> Option<String> {
    let name: String = value
        .chars()
        .filter(|c| c.is_alphanumeric() || matches!(c, ' ' | '-'))
        .collect();
    let name = name.trim().to_owned();
    (!name.is_empty()).then_some(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_colours_are_validated() {
        assert_eq!(valid_hex("#1F4E79").as_deref(), Some("#1f4e79"));
        assert_eq!(valid_hex("#abc").as_deref(), Some("#abc"));
        assert!(valid_hex("red").is_none());
        assert!(valid_hex("#12345").is_none());
        assert!(valid_hex("#12\") + x").is_none());
    }

    #[test]
    fn font_names_lose_markup_characters() {
        assert_eq!(clean_name("Poppins").as_deref(), Some("Poppins"));
        assert_eq!(clean_name("Evil\"); #x(").as_deref(), Some("Evil x"));
        assert!(clean_name("\"();").is_none());
    }
}
