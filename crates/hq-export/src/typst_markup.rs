//! Document model to Typst markup.
//!
//! Text goes through [`escape`], code and every other free-form string goes
//! through [`literal`] (a Typst string literal), so nothing a note says can
//! become Typst code. Images are read here, once, and handed to the engine as
//! in-memory files under generated names; the note never names a path the
//! engine resolves itself.

use std::fmt::Write as _;
use std::path::Path;

use crate::assets::AssetLoader;
use crate::util::{callout_color, capitalise, safe_url};
use crate::doc::{Align, Block, Document, Inline, ListItem, Table};
use crate::theme::Theme;

/// One page per sheet, or a single tall page (the natural shape for a PNG or
/// SVG of a whole note).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PageMode {
    #[default]
    Paged,
    Single,
}

/// Typst does not wrap code, so a long line would run off the page.
const CODE_WRAP_COLUMNS: usize = 92;

/// The Typst source plus the image files it refers to.
pub struct Markup {
    pub source: String,
    pub assets: Vec<(String, Vec<u8>)>,
}

pub fn build(doc: &Document, theme: &Theme, mode: PageMode, asset_root: Option<&Path>) -> Markup {
    let mut w = Writer {
        assets: AssetLoader::new(asset_root),
    };
    let body = w.blocks(&doc.blocks);
    let mut source = preamble(doc, theme, mode);
    let _ = write!(
        source,
        "#block(below: 1.3em)[#text(size: 22pt, weight: \"bold\", fill: accent, {})\n#v(3pt)\n#line(length: 100%, stroke: 1.2pt + accent)]\n\n",
        literal(&doc.title)
    );
    source.push_str(&body);
    Markup {
        source,
        assets: w
            .assets
            .into_assets()
            .into_iter()
            .map(|a| (a.name, a.bytes))
            .collect(),
    }
}

fn preamble(doc: &Document, theme: &Theme, mode: PageMode) -> String {
    let mut fonts: Vec<String> = Vec::new();
    if let Some(f) = &theme.font {
        fonts.push(literal(f));
    }
    // Fonts listed after the first match are only reached for glyphs the first
    // lacks; anything still missing falls through to whatever else is installed.
    for f in ["Noto Sans", "Libertinus Serif", "DejaVu Sans"] {
        fonts.push(literal(f));
    }
    let page = match mode {
        PageMode::Paged => format!(
            "#set page(paper: \"a4\", margin: (x: 20mm, top: 22mm, bottom: 24mm), footer: context {{\n  set text(size: 8pt, fill: luma(120))\n  [{footer} #h(1fr) #counter(page).display() / #counter(page).final().first()]\n}})\n",
            footer = format_args!("#{}", literal(&theme.footer))
        ),
        PageMode::Single => {
            "#set page(width: 210mm, height: auto, margin: (x: 20mm, y: 16mm))\n".to_owned()
        }
    };
    format!(
        r##"#let accent = rgb({accent})
#set document(title: {title})
{page}#set text(font: ({fonts},), size: 11pt, fill: luma(25))
#set par(leading: 0.68em, spacing: 1.05em)
#set list(indent: 0.3em, body-indent: 0.55em)
#set enum(indent: 0.3em, body-indent: 0.55em)
#set heading(numbering: none)
#show heading: set text(fill: accent)
#show heading.where(level: 1): set text(size: 1.45em)
#show heading.where(level: 2): set text(size: 1.25em)
#show heading.where(level: 3): set text(size: 1.1em)
#show heading: set block(above: 1.4em, below: 0.7em)
#show link: set text(fill: accent)
#show link: underline
#show raw: set text(size: 0.9em)
#show raw.where(block: false): box.with(fill: luma(242), inset: (x: 3pt), outset: (y: 3pt), radius: 2pt)
#show raw.where(block: true): block.with(fill: luma(245), inset: 9pt, radius: 3pt, width: 100%)
#show quote.where(block: true): it => block(width: 100%, inset: (left: 12pt, y: 5pt), stroke: (left: 2.5pt + accent), fill: luma(247), it.body)
#set table(stroke: 0.5pt + luma(190), inset: (x: 7pt, y: 5pt), fill: (x, y) => if y == 0 {{ luma(238) }})
#show table.cell.where(y: 0): strong
#let callout(color, label, body) = block(width: 100%, breakable: false, inset: 10pt, radius: 3pt, fill: color.lighten(88%), stroke: (left: 3pt + color))[
  #text(weight: "bold", fill: color)[#label]
  #if body != none [#v(-0.2em) #body]
]
#let fit-image(path, alt) = layout(size => {{
  let natural = image(path, alt: alt)
  if measure(natural).width > size.width {{ image(path, width: 100%, alt: alt) }} else {{ natural }}
}})
"##,
        accent = literal(&theme.accent),
        title = literal(&doc.title),
        fonts = fonts.join(", "),
    )
}

struct Writer {
    assets: AssetLoader,
}

impl Writer {
    fn blocks(&mut self, blocks: &[Block]) -> String {
        let mut out = String::new();
        for block in blocks {
            out.push_str(&self.block(block));
        }
        out
    }

    fn block(&mut self, block: &Block) -> String {
        match block {
            Block::Heading { level, content } => format!(
                "#heading(level: {})[{}]\n\n",
                (*level).clamp(1, 6),
                self.inlines(content)
            ),
            Block::Paragraph(content) => format!("{}\n\n", self.inlines(content)),
            Block::Quote(body) => format!("#quote(block: true)[{}]\n\n", self.blocks(body)),
            Block::Callout { kind, title, body } => {
                let label = match title {
                    Some(t) => t.clone(),
                    None => capitalise(kind),
                };
                let body = if body.is_empty() {
                    "none".to_owned()
                } else {
                    format!("[{}]", self.blocks(body))
                };
                format!(
                    "#callout(rgb({}), {}, {})\n\n",
                    literal(callout_color(kind)),
                    literal(&label),
                    body
                )
            }
            Block::List {
                ordered,
                start,
                items,
            } => self.list(*ordered, *start, items),
            Block::Code { lang, text } => {
                let lang = lang
                    .as_deref()
                    .filter(|l| l.chars().all(|c| c.is_ascii_alphanumeric() || "+-#_.".contains(c)))
                    .map(|l| format!(", lang: {}", literal(l)))
                    .unwrap_or_default();
                format!(
                    "#raw({}, block: true{lang})\n\n",
                    literal(&wrap_code(text, CODE_WRAP_COLUMNS))
                )
            }
            Block::Table(t) => self.table(t),
            Block::Rule => "#line(length: 100%, stroke: 0.5pt + luma(190))\n\n".to_owned(),
        }
    }

    fn list(&mut self, ordered: bool, start: u64, items: &[ListItem]) -> String {
        let tight = items.iter().all(|i| {
            i.blocks
                .iter()
                .filter(|b| !matches!(b, Block::List { .. }))
                .count()
                <= 1
        });
        let rendered: Vec<String> = items
            .iter()
            .map(|item| {
                let mut body = String::new();
                match item.checked {
                    Some(true) => body.push_str("#raw(\"[x]\") "),
                    Some(false) => body.push_str("#raw(\"[ ]\") "),
                    None => {}
                }
                // Lists put a paragraph break between sibling blocks, which would
                // push the first line of a nested list far from its parent.
                let mut first = true;
                for block in &item.blocks {
                    if !first && !matches!(block, Block::List { .. }) {
                        body.push_str("\n\n");
                    }
                    body.push_str(self.block(block).trim_end());
                    body.push('\n');
                    first = false;
                }
                format!("[{body}]")
            })
            .collect();
        let tight = format!("tight: {tight}");
        if ordered {
            format!("#enum(start: {start}, {tight}, {})\n\n", rendered.join(", "))
        } else {
            format!("#list({tight}, {})\n\n", rendered.join(", "))
        }
    }

    fn table(&mut self, t: &Table) -> String {
        let n = t.columns();
        if n == 0 {
            return String::new();
        }
        let aligns: Vec<&str> = (0..n)
            .map(|i| match t.aligns.get(i) {
                Some(Align::Right) => "right",
                Some(Align::Center) => "center",
                _ => "left",
            })
            .collect();
        let mut cells = |row: &[Vec<Inline>]| -> String {
            (0..n)
                .map(|i| match row.get(i) {
                    Some(c) => format!("[{}]", self.inlines(c)),
                    None => "[]".to_owned(),
                })
                .collect::<Vec<_>>()
                .join(", ")
        };
        let header = cells(&t.header);
        let rows: Vec<String> = t.rows.iter().map(|r| cells(r)).collect();
        let mut out = format!(
            "#table(columns: {n}, align: ({},), table.header({header})",
            aligns.join(", ")
        );
        for r in rows {
            out.push_str(", ");
            out.push_str(&r);
        }
        out.push_str(")\n\n");
        out
    }

    fn inlines(&mut self, inlines: &[Inline]) -> String {
        let mut out = String::new();
        for inline in inlines {
            match inline {
                Inline::Text(t) => out.push_str(&escape(t)),
                Inline::Emph(c) => {
                    let _ = write!(out, "#emph[{}]", self.inlines(c));
                }
                Inline::Strong(c) => {
                    let _ = write!(out, "#strong[{}]", self.inlines(c));
                }
                Inline::Strike(c) => {
                    let _ = write!(out, "#strike[{}]", self.inlines(c));
                }
                Inline::Code(c) => {
                    let _ = write!(out, "#raw({})", literal(c));
                }
                Inline::Link { url, content } => {
                    if safe_url(url) {
                        let _ = write!(out, "#link({})[{}]", literal(url), self.inlines(content));
                    } else {
                        out.push_str(&self.inlines(content));
                    }
                }
                Inline::Image { alt, src } => match self.assets.load(src).map(|a| a.name.clone()) {
                    Some(name) => {
                        let _ = write!(out, "#fit-image({}, {})", literal(&name), literal(alt));
                    }
                    None => {
                        let label = if alt.is_empty() { "image" } else { alt };
                        let _ = write!(out, "#emph[\\[{} unavailable\\]]", escape(label));
                    }
                },
                Inline::SoftBreak => out.push(' '),
                Inline::HardBreak => out.push_str("#linebreak()"),
            }
        }
        out
    }
}

/// Escape free text for Typst markup mode.
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 8);
    let mut prev: Option<char> = None;
    for c in text.chars() {
        let special = "\\#$*_`<>@[]~=-+/".contains(c)
            // "1." at the start of a line would otherwise become a list.
            || (c == '.' && prev.is_some_and(|p| p.is_ascii_digit()));
        if special {
            out.push('\\');
        }
        out.push(c);
        prev = Some(c);
    }
    out
}

/// A Typst string literal, quotes included.
pub fn literal(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => {
                let _ = write!(out, "\\u{{{:x}}}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Hard-wrap lines longer than `width` characters. Breaks on a space when one
/// is near, otherwise mid-token.
fn wrap_code(text: &str, width: usize) -> String {
    let mut out = String::with_capacity(text.len());
    for (i, line) in text.lines().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        let chars: Vec<char> = line.chars().collect();
        if chars.len() <= width {
            out.push_str(line);
            continue;
        }
        let indent: String = chars.iter().take_while(|c| **c == ' ').collect();
        let hang = format!("{indent}  ");
        // Continuation lines carry the hanging indent, so they hold fewer
        // characters; the floor keeps a deeply indented line from degenerating
        // into one character per row.
        let continuation = width.saturating_sub(hang.chars().count()).max(20);
        let mut start = 0;
        let mut budget = width;
        while chars.len() - start > budget {
            let window = &chars[start..start + budget];
            let cut = window
                .iter()
                .rposition(|c| *c == ' ')
                .filter(|p| *p > budget / 2)
                .map(|p| p + 1)
                .unwrap_or(budget);
            out.extend(&chars[start..start + cut]);
            out.push('\n');
            out.push_str(&hang);
            start += cut;
            budget = continuation;
        }
        out.extend(&chars[start..]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_neutralises_markup_characters() {
        assert_eq!(escape("a # b $ c"), "a \\# b \\$ c");
        assert_eq!(escape("*_`"), "\\*\\_\\`");
        assert_eq!(escape("2024. Next"), "2024\\. Next");
        assert_eq!(escape("plain words"), "plain words");
    }

    #[test]
    fn literal_escapes_quotes_backslashes_and_control_characters() {
        assert_eq!(literal("a\"b\\c"), "\"a\\\"b\\\\c\"");
        assert_eq!(literal("l1\nl2"), "\"l1\\nl2\"");
        assert_eq!(literal("\u{7}"), "\"\\u{7}\"");
    }

    #[test]
    fn long_code_lines_are_wrapped_with_a_hanging_indent() {
        let line = format!("    {}", "word ".repeat(40));
        let wrapped = wrap_code(&line, 60);
        assert!(wrapped.lines().count() > 1);
        assert!(wrapped.lines().all(|l| l.chars().count() <= 60));
        assert!(wrapped.lines().nth(1).unwrap().starts_with("      "));
    }

    #[test]
    fn short_code_is_untouched() {
        assert_eq!(wrap_code("a\n  b", 60), "a\n  b");
    }

    #[test]
    fn images_outside_the_asset_root_are_never_read() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let secret = outside.path().join("secret.png");
        std::fs::write(&secret, b"not really a png").unwrap();
        let doc = Document::from_markdown(
            "t",
            &format!("![x]({})\n", secret.display()),
        );
        let m = build(&doc, &Theme::default(), PageMode::Paged, Some(root.path()));
        assert!(m.assets.is_empty());
        assert!(m.source.contains("unavailable"));
    }

    #[test]
    fn images_without_an_asset_root_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let img = dir.path().join("a.png");
        std::fs::write(&img, b"x").unwrap();
        let doc = Document::from_markdown("t", &format!("![x]({})\n", img.display()));
        let m = build(&doc, &Theme::default(), PageMode::Paged, None);
        assert!(m.assets.is_empty());
    }

    #[test]
    fn images_inside_the_root_are_embedded_once() {
        let root = tempfile::tempdir().unwrap();
        let img = root.path().join("a.png");
        std::fs::write(&img, b"x").unwrap();
        let md = format!("![x]({0})\n\n![y]({0})\n", img.display());
        let doc = Document::from_markdown("t", &md);
        let m = build(&doc, &Theme::default(), PageMode::Paged, Some(root.path()));
        assert_eq!(m.assets.len(), 1);
        assert_eq!(m.source.matches("asset0.png").count(), 2);
    }

    #[test]
    fn remote_and_data_images_are_not_fetched() {
        let root = tempfile::tempdir().unwrap();
        let doc = Document::from_markdown(
            "t",
            "![a](https://example.com/a.png) ![b](data:image/png;base64,AAAA)\n",
        );
        let m = build(&doc, &Theme::default(), PageMode::Paged, Some(root.path()));
        assert!(m.assets.is_empty());
    }
}
