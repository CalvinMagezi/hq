//! A single self-contained HTML file: styles inline, images embedded.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;

use crate::assets::AssetLoader;
use crate::doc::{Align, Block, Document, Inline, ListItem, Table};
use crate::theme::Theme;
use crate::util::{callout_color, capitalise, safe_url};

pub fn to_html(doc: &Document, theme: &Theme, asset_root: Option<&std::path::Path>) -> String {
    let mut w = Writer {
        assets: AssetLoader::new(asset_root),
    };
    let body = w.blocks(&doc.blocks);
    let font = theme
        .font
        .as_deref()
        .map(|f| format!("\"{f}\", "))
        .unwrap_or_default();
    let footer = if theme.footer.is_empty() {
        String::new()
    } else {
        format!("<footer>{}</footer>\n", escape(&theme.footer))
    };
    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title}</title>
<style>
:root {{ --accent: {accent}; }}
body {{ font-family: {font}-apple-system, "Segoe UI", "Helvetica Neue", Arial, "Noto Sans", sans-serif; line-height: 1.6; color: #1b1b1b; max-width: 46rem; margin: 2rem auto; padding: 0 1rem; overflow-wrap: anywhere; }}
h1, h2, h3, h4, h5, h6 {{ color: var(--accent); line-height: 1.25; }}
h1.title {{ border-bottom: 2px solid var(--accent); padding-bottom: .3em; }}
a {{ color: var(--accent); }}
img {{ max-width: 100%; height: auto; }}
blockquote {{ margin: 1em 0; padding: .1em 1em; border-left: 4px solid var(--accent); color: #444; background: #f6f7f8; }}
code {{ font-family: ui-monospace, "SF Mono", Menlo, Consolas, monospace; font-size: .9em; background: #f2f3f4; padding: .1em .3em; border-radius: 3px; }}
pre {{ background: #f2f3f4; padding: .8em 1em; border-radius: 4px; overflow-x: auto; }}
pre code {{ background: none; padding: 0; }}
table {{ border-collapse: collapse; margin: 1em 0; }}
th, td {{ border: 1px solid #cfd3d7; padding: .35em .7em; vertical-align: top; }}
th {{ background: #eef0f2; }}
hr {{ border: 0; border-top: 1px solid #cfd3d7; margin: 1.5em 0; }}
.callout {{ margin: 1em 0; padding: .6em 1em; border-left: 4px solid var(--c); background: color-mix(in srgb, var(--c) 10%, white); border-radius: 4px; }}
.callout > .callout-title {{ font-weight: 700; color: var(--c); margin: 0; }}
.callout > :last-child {{ margin-bottom: 0; }}
footer {{ margin-top: 3rem; color: #777; font-size: .85em; }}
@media print {{ body {{ margin: 0; max-width: none; }} }}
</style>
</head>
<body>
<h1 class="title">{title}</h1>
{body}{footer}</body>
</html>
"#,
        title = escape(&doc.title),
        accent = theme.accent,
    )
}

struct Writer {
    assets: AssetLoader,
}

impl Writer {
    fn blocks(&mut self, blocks: &[Block]) -> String {
        blocks.iter().map(|b| self.block(b)).collect()
    }

    fn block(&mut self, b: &Block) -> String {
        match b {
            Block::Heading { level, content } => {
                // The document title is the h1, so note headings start at h2.
                let n = (*level).clamp(1, 5) + 1;
                format!("<h{n}>{}</h{n}>\n", self.inlines(content))
            }
            Block::Paragraph(c) => format!("<p>{}</p>\n", self.inlines(c)),
            Block::Quote(body) => format!("<blockquote>\n{}</blockquote>\n", self.blocks(body)),
            Block::Callout { kind, title, body } => {
                let label = title.clone().unwrap_or_else(|| capitalise(kind));
                format!(
                    "<div class=\"callout\" style=\"--c: {}\">\n<p class=\"callout-title\">{}</p>\n{}</div>\n",
                    callout_color(kind),
                    escape(&label),
                    self.blocks(body)
                )
            }
            Block::List {
                ordered,
                start,
                items,
            } => self.list(*ordered, *start, items),
            Block::Code { lang, text } => {
                let class = lang
                    .as_deref()
                    .filter(|l| {
                        l.chars()
                            .all(|c| c.is_ascii_alphanumeric() || "+-#_.".contains(c))
                    })
                    .map(|l| format!(" class=\"language-{l}\""))
                    .unwrap_or_default();
                format!("<pre><code{class}>{}</code></pre>\n", escape(text))
            }
            Block::Table(t) => self.table(t),
            Block::Rule => "<hr>\n".to_owned(),
        }
    }

    fn list(&mut self, ordered: bool, start: u64, items: &[ListItem]) -> String {
        let (open, close) = if ordered {
            let attr = if start != 1 {
                format!(" start=\"{start}\"")
            } else {
                String::new()
            };
            (format!("<ol{attr}>"), "</ol>")
        } else {
            ("<ul>".to_owned(), "</ul>")
        };
        let mut out = format!("{open}\n");
        for item in items {
            out.push_str("<li>");
            match item.checked {
                Some(true) => out.push_str("<input type=\"checkbox\" checked disabled> "),
                Some(false) => out.push_str("<input type=\"checkbox\" disabled> "),
                None => {}
            }
            // A one-paragraph item reads better without the <p> wrapper.
            match item.blocks.as_slice() {
                [Block::Paragraph(c)] => out.push_str(&self.inlines(c)),
                blocks => {
                    out.push('\n');
                    out.push_str(&self.blocks(blocks));
                }
            }
            out.push_str("</li>\n");
        }
        out.push_str(close);
        out.push('\n');
        out
    }

    fn table(&mut self, t: &Table) -> String {
        let n = t.columns();
        if n == 0 {
            return String::new();
        }
        let style = |i: usize| match t.aligns.get(i) {
            Some(Align::Right) => " style=\"text-align:right\"",
            Some(Align::Center) => " style=\"text-align:center\"",
            _ => "",
        };
        let mut out = String::from("<table>\n<thead><tr>");
        for i in 0..n {
            let c = t.header.get(i).map(|c| self.inlines(c)).unwrap_or_default();
            out.push_str(&format!("<th{}>{c}</th>", style(i)));
        }
        out.push_str("</tr></thead>\n<tbody>\n");
        for row in &t.rows {
            out.push_str("<tr>");
            for i in 0..n {
                let c = row.get(i).map(|c| self.inlines(c)).unwrap_or_default();
                out.push_str(&format!("<td{}>{c}</td>", style(i)));
            }
            out.push_str("</tr>\n");
        }
        out.push_str("</tbody>\n</table>\n");
        out
    }

    fn inlines(&mut self, content: &[Inline]) -> String {
        let mut out = String::new();
        for i in content {
            match i {
                Inline::Text(t) => out.push_str(&escape(t)),
                Inline::Emph(c) => out.push_str(&format!("<em>{}</em>", self.inlines(c))),
                Inline::Strong(c) => out.push_str(&format!("<strong>{}</strong>", self.inlines(c))),
                Inline::Strike(c) => out.push_str(&format!("<del>{}</del>", self.inlines(c))),
                Inline::Code(c) => out.push_str(&format!("<code>{}</code>", escape(c))),
                Inline::Link { url, content } => {
                    if safe_url(url) {
                        out.push_str(&format!(
                            "<a href=\"{}\">{}</a>",
                            escape(url),
                            self.inlines(content)
                        ));
                    } else {
                        out.push_str(&self.inlines(content));
                    }
                }
                Inline::Image { alt, src } => match self.assets.load(src) {
                    Some(a) => out.push_str(&format!(
                        "<img src=\"data:{};base64,{}\" alt=\"{}\">",
                        a.mime(),
                        STANDARD.encode(&a.bytes),
                        escape(alt)
                    )),
                    None => {
                        let label = if alt.is_empty() { "image" } else { alt };
                        out.push_str(&format!("<em>[{} unavailable]</em>", escape(label)));
                    }
                },
                Inline::SoftBreak => out.push('\n'),
                Inline::HardBreak => out.push_str("<br>\n"),
            }
        }
        out
    }
}

fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn html(md: &str) -> String {
        to_html(
            &Document::from_markdown("T <&> \"t\"", md),
            &Theme::default(),
            None,
        )
    }

    #[test]
    fn page_is_standalone_and_title_is_escaped() {
        let h = html("hello\n");
        assert!(h.starts_with("<!DOCTYPE html>"));
        assert!(h.contains("<title>T &lt;&amp;&gt; &quot;t&quot;</title>"));
        assert!(!h.contains("<script"));
    }

    #[test]
    fn note_text_cannot_inject_markup() {
        let h = html(
            "text <script>alert(1)</script> and `<b>` and [x](javascript:alert(1)) and \"quo\\\"te\"\n\n<img src=x onerror=alert(1)>\n",
        );
        assert!(!h.contains("<script>"), "{h}");
        assert!(!h.contains("onerror"), "raw html is dropped: {h}");
        assert!(!h.contains("javascript:"), "{h}");
        assert!(h.contains("&lt;b&gt;"));
    }

    #[test]
    fn structure_maps_to_elements() {
        let h = html(
            "## Sec\n\n> [!tip] Hi\n> body\n\n| a | b |\n|:-|-:|\n| 1 | 2 |\n\n3. x\n4. y\n\n- [x] done\n\n```rust\nlet a = 1 < 2;\n```\n",
        );
        assert!(
            h.contains("<h3>Sec</h3>"),
            "note headings sit below the title: {h}"
        );
        assert!(h.contains("class=\"callout\""));
        assert!(h.contains("<th style=\"text-align:right\">b</th>"));
        assert!(h.contains("<ol start=\"3\">"));
        assert!(h.contains("<input type=\"checkbox\" checked disabled>"));
        assert!(h.contains("class=\"language-rust\">let a = 1 &lt; 2;"));
    }

    #[test]
    fn images_are_embedded_only_from_inside_the_root() {
        let root = tempfile::tempdir().unwrap();
        let inside = root.path().join("a.png");
        std::fs::write(&inside, b"\x89PNG").unwrap();
        let outside = tempfile::tempdir().unwrap();
        let secret = outside.path().join("s.png");
        std::fs::write(&secret, b"secret").unwrap();
        let md = format!("![in]({}) ![out]({})\n", inside.display(), secret.display());
        let h = to_html(
            &Document::from_markdown("t", &md),
            &Theme::default(),
            Some(root.path()),
        );
        assert_eq!(h.matches("data:image/png;base64,").count(), 1, "{h}");
        assert!(h.contains("[out unavailable]"));
    }
}
