//! HTML pages to Markdown.
//!
//! Scripts and styles are removed. Headings, lists, tables, links, code and
//! emphasis are kept. The whole body is read, not only the main article, so a
//! saved page keeps its content; navigation menus are dropped.

use encoding_rs::Encoding;
use scraper::{ElementRef, Html, Node};

use super::ir::{
    Block, Inline, ListItem, Parsed, Run, Table, clean_alt, collapse_ws, is_mono_font,
};
use super::report::{LossKind, Losses};
use super::text::decode;
use crate::types::ConvertError;

pub fn read(bytes: &[u8]) -> Result<Parsed, ConvertError> {
    let declared = declared_charset(bytes);
    let (text, guessed) = decode(bytes, declared.as_deref());
    let mut losses = Losses::default();
    if guessed {
        losses.note(
            LossKind::EncodingGuessed,
            "the page does not declare its encoding and is not UTF-8; read as Windows-1252",
        );
    }
    let doc = Html::parse_document(&text);
    let title = doc
        .select(&scraper::Selector::parse("title").expect("valid selector"))
        .next()
        .map(|t| {
            collapse_ws(&t.text().collect::<String>())
                .trim()
                .to_string()
        })
        .filter(|t| !t.is_empty());
    let root = ["main", "article", "body"]
        .iter()
        .filter_map(|s| {
            doc.select(&scraper::Selector::parse(s).expect("valid selector"))
                .next()
        })
        .next()
        .unwrap_or_else(|| doc.root_element());
    let mut w = Walker { losses };
    let mut blocks = Vec::new();
    let mut acc = Vec::new();
    w.children(root, &Fmt::default(), &mut acc, &mut blocks);
    Walker::flush(&mut acc, &mut blocks);
    Ok(Parsed {
        blocks,
        title,
        losses: w.losses,
    })
}

/// A charset named in the first kilobytes of the page.
fn declared_charset(bytes: &[u8]) -> Option<String> {
    if Encoding::for_bom(bytes).is_some() {
        return None;
    }
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(4096)]).to_lowercase();
    let at = head.find("charset")?;
    let rest = &head[at + "charset".len()..];
    let rest = rest.trim_start_matches([' ', '=', '"', '\'']);
    let label: String = rest
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    (!label.is_empty()).then_some(label)
}

#[derive(Default, Clone)]
struct Fmt {
    bold: bool,
    italic: bool,
    strike: bool,
    code: bool,
    link: Option<String>,
}

struct Walker {
    losses: Losses,
}

fn is_hidden(el: ElementRef<'_>) -> bool {
    let v = el.value();
    v.attr("hidden").is_some()
        || v.attr("aria-hidden") == Some("true") && v.name() != "body"
        || v.attr("style").is_some_and(|s| {
            let s: String = s
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect::<String>()
                .to_lowercase();
            s.contains("display:none") || s.contains("visibility:hidden")
        })
}

const BLOCKS: &[&str] = &[
    "p",
    "div",
    "section",
    "article",
    "main",
    "header",
    "footer",
    "aside",
    "figure",
    "figcaption",
    "ul",
    "ol",
    "li",
    "table",
    "pre",
    "blockquote",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "hr",
    "dl",
    "dt",
    "dd",
    "details",
    "summary",
    "form",
    "fieldset",
    "address",
    "center",
    "body",
];

impl Walker {
    /// Walks the children of `parent`, collecting inline content into `acc`
    /// and flushing it to a paragraph whenever a block element starts.
    fn children(
        &mut self,
        parent: ElementRef<'_>,
        fmt: &Fmt,
        acc: &mut Vec<Inline>,
        out: &mut Vec<Block>,
    ) {
        for child in parent.children() {
            match child.value() {
                Node::Text(t) => {
                    let text = collapse_ws(t);
                    if !text.trim().is_empty() || (!text.is_empty() && !acc.is_empty()) {
                        acc.push(Inline::Run(Run {
                            text,
                            bold: fmt.bold,
                            italic: fmt.italic,
                            strike: fmt.strike,
                            code: fmt.code,
                            link: fmt.link.clone(),
                        }));
                    }
                }
                Node::Element(_) => {
                    let Some(el) = ElementRef::wrap(child) else {
                        continue;
                    };
                    self.element(el, fmt, acc, out);
                }
                _ => {}
            }
        }
    }

    fn flush(acc: &mut Vec<Inline>, out: &mut Vec<Block>) {
        let has = acc.iter().any(|i| match i {
            Inline::Run(r) => !r.text.trim().is_empty(),
            Inline::Image(_) => true,
            _ => false,
        });
        if has {
            out.push(Block::Para(std::mem::take(acc)));
        } else {
            acc.clear();
        }
    }

    fn element(
        &mut self,
        el: ElementRef<'_>,
        fmt: &Fmt,
        acc: &mut Vec<Inline>,
        out: &mut Vec<Block>,
    ) {
        let name = el.value().name();
        if matches!(
            name,
            "script" | "style" | "noscript" | "template" | "head" | "iframe" | "object" | "embed"
        ) {
            if matches!(name, "script" | "style") {
                self.losses.hit(LossKind::ScriptsRemoved);
            }
            return;
        }
        if matches!(
            name,
            "nav" | "input" | "button" | "select" | "textarea" | "svg" | "canvas"
        ) {
            return;
        }
        if is_hidden(el) {
            self.losses.hit(LossKind::HiddenContentSkipped);
            return;
        }
        match name {
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                Self::flush(acc, out);
                let level = name[1..].parse::<u8>().unwrap_or(1);
                let mut inl = Vec::new();
                self.children(el, fmt, &mut inl, &mut Vec::new());
                out.push(Block::Heading(level, inl));
            }
            "p" | "figcaption" | "dt" | "summary" | "address" => {
                Self::flush(acc, out);
                let mut inl = Vec::new();
                let mut inner = Vec::new();
                let f = if name == "dt" {
                    Fmt {
                        bold: true,
                        ..fmt.clone()
                    }
                } else {
                    fmt.clone()
                };
                self.children(el, &f, &mut inl, &mut inner);
                Self::flush(&mut inl, out);
                out.extend(inner);
            }
            "ul" | "ol" => {
                Self::flush(acc, out);
                let mut items = Vec::new();
                let mut after = Vec::new();
                self.list(el, 0, &mut items, &mut after);
                if !items.is_empty() {
                    out.push(Block::List(items));
                }
                out.extend(after);
            }
            "table" => {
                Self::flush(acc, out);
                if let Some(t) = self.table(el) {
                    out.push(t);
                }
            }
            "pre" => {
                Self::flush(acc, out);
                let text: String = el.text().collect();
                let lang = el
                    .select(&scraper::Selector::parse("code").expect("valid selector"))
                    .next()
                    .and_then(|c| c.value().attr("class"))
                    .and_then(|c| {
                        c.split_whitespace()
                            .find_map(|k| k.strip_prefix("language-"))
                    })
                    .map(str::to_string);
                out.push(Block::Code { lang, text });
            }
            "blockquote" => {
                Self::flush(acc, out);
                let mut inl = Vec::new();
                let mut blocks = Vec::new();
                self.children(el, fmt, &mut inl, &mut blocks);
                Self::flush(&mut inl, &mut blocks);
                if !blocks.is_empty() {
                    out.push(Block::Quote(blocks));
                }
            }
            "hr" => {
                Self::flush(acc, out);
                out.push(Block::Rule);
            }
            "br" => acc.push(Inline::Break),
            "img" => {
                let alt = clean_alt(el.value().attr("alt").unwrap_or(""));
                acc.push(Inline::Image(alt));
                self.losses.hit(LossKind::ImagesOmitted);
            }
            "a" => {
                let mut f = fmt.clone();
                f.link = el
                    .value()
                    .attr("href")
                    .map(str::trim)
                    .filter(|h| {
                        !h.is_empty()
                            && !h.starts_with('#')
                            && !h.to_lowercase().starts_with("javascript:")
                            && !h.to_lowercase().starts_with("data:")
                    })
                    .map(str::to_string);
                self.children(el, &f, acc, out);
            }
            "strong" | "b" => self.children(
                el,
                &Fmt {
                    bold: true,
                    ..fmt.clone()
                },
                acc,
                out,
            ),
            "em" | "i" | "cite" | "var" => self.children(
                el,
                &Fmt {
                    italic: true,
                    ..fmt.clone()
                },
                acc,
                out,
            ),
            "del" | "s" | "strike" => self.children(
                el,
                &Fmt {
                    strike: true,
                    ..fmt.clone()
                },
                acc,
                out,
            ),
            "code" | "kbd" | "samp" | "tt" => self.children(
                el,
                &Fmt {
                    code: true,
                    ..fmt.clone()
                },
                acc,
                out,
            ),
            n if BLOCKS.contains(&n) => {
                Self::flush(acc, out);
                let mut inl = Vec::new();
                self.children(el, fmt, &mut inl, out);
                Self::flush(&mut inl, out);
            }
            _ => {
                // An inline wrapper such as <span>; a monospace font in its style is code.
                let mono = el
                    .value()
                    .attr("style")
                    .is_some_and(|s| s.to_lowercase().contains("font-family") && is_mono_font(s));
                let f = if mono {
                    Fmt {
                        code: true,
                        ..fmt.clone()
                    }
                } else {
                    fmt.clone()
                };
                self.children(el, &f, acc, out);
            }
        }
    }

    fn list(
        &mut self,
        list: ElementRef<'_>,
        level: usize,
        items: &mut Vec<ListItem>,
        after: &mut Vec<Block>,
    ) {
        let ordered = list.value().name() == "ol";
        for li in list.children().filter_map(ElementRef::wrap) {
            match li.value().name() {
                "li" => {
                    let mut inl: Vec<Inline> = Vec::new();
                    let mut nested: Vec<ElementRef<'_>> = Vec::new();
                    let mut blocks: Vec<Block> = Vec::new();
                    for c in li.children() {
                        match c.value() {
                            Node::Text(t) => {
                                let text = collapse_ws(t);
                                if !text.trim().is_empty() || (!text.is_empty() && !inl.is_empty())
                                {
                                    inl.push(Inline::text(text));
                                }
                            }
                            Node::Element(e) => {
                                let Some(el) = ElementRef::wrap(c) else {
                                    continue;
                                };
                                if matches!(e.name(), "ul" | "ol") {
                                    nested.push(el);
                                } else if matches!(e.name(), "p" | "div") {
                                    // Paragraphs inside an item read as one line of the item.
                                    let mut part = Vec::new();
                                    self.children(el, &Fmt::default(), &mut part, &mut blocks);
                                    if !part.is_empty() && !inl.is_empty() {
                                        inl.push(Inline::Break);
                                    }
                                    inl.extend(part);
                                } else if matches!(e.name(), "table" | "pre") {
                                    self.element(el, &Fmt::default(), &mut Vec::new(), after);
                                } else {
                                    self.element(el, &Fmt::default(), &mut inl, &mut blocks);
                                }
                            }
                            _ => {}
                        }
                    }
                    after.extend(blocks);
                    if inl
                        .iter()
                        .any(|i| matches!(i, Inline::Run(r) if !r.text.trim().is_empty()))
                    {
                        items.push(ListItem {
                            level,
                            ordered,
                            content: inl,
                        });
                    }
                    for n in nested {
                        self.list(n, level + 1, items, after);
                    }
                }
                "ul" | "ol" => self.list(li, level + 1, items, after),
                _ => {}
            }
        }
    }

    fn table(&mut self, t: ElementRef<'_>) -> Option<Block> {
        let mut table = Table::default();
        let row_sel = scraper::Selector::parse("tr").expect("valid selector");
        for tr in t.select(&row_sel) {
            // Rows of a nested table belong to that table.
            if tr
                .ancestors()
                .filter_map(ElementRef::wrap)
                .find(|a| a.value().name() == "table")
                .map(|a| a.id())
                != Some(t.id())
            {
                continue;
            }
            let mut row: Vec<Vec<Inline>> = Vec::new();
            for cell in tr
                .children()
                .filter_map(ElementRef::wrap)
                .filter(|c| matches!(c.value().name(), "td" | "th"))
            {
                let span = cell
                    .value()
                    .attr("colspan")
                    .and_then(|v| v.parse::<usize>().ok())
                    .unwrap_or(1)
                    .clamp(1, 64);
                let mut inl: Vec<Inline> = Vec::new();
                let mut blocks: Vec<Block> = Vec::new();
                self.children(cell, &Fmt::default(), &mut inl, &mut blocks);
                for b in blocks {
                    match b {
                        Block::Para(p) => {
                            if !inl.is_empty() {
                                inl.push(Inline::Break);
                            }
                            inl.extend(p);
                        }
                        Block::Table(_) => self.losses.hit(LossKind::NestedTablesFlattened),
                        _ => {}
                    }
                }
                row.push(inl);
                for _ in 1..span {
                    row.push(Vec::new());
                }
            }
            if !row.is_empty() {
                table.rows.push(row);
            }
        }
        (!table.rows.is_empty()).then_some(Block::Table(table))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn md(html: &str) -> String {
        super::super::ir::render(read(html.as_bytes()).unwrap().blocks)
    }

    #[test]
    fn headings_emphasis_links_and_entities() {
        assert_eq!(
            md(
                "<html><body><h1>Update</h1><p>We reached <strong>three</strong> sites, see <a href=\"https://example.com/r\">the report</a>. Fish &amp; chips.</p></body></html>"
            ),
            "# Update\n\nWe reached **three** sites, see [the report](https://example.com/r). Fish & chips.\n"
        );
    }

    #[test]
    fn tables_keep_their_structure_and_spans() {
        assert_eq!(
            md(
                "<table><thead><tr><th colspan=2>Progress</th></tr><tr><th>Site</th><th>Farmers</th></tr></thead><tbody><tr><td>Mukono</td><td>42</td></tr></tbody></table>"
            ),
            "| Progress |  |\n| --- | --- |\n| Site | Farmers |\n| Mukono | 42 |\n"
        );
    }

    #[test]
    fn nested_lists_and_numbering() {
        assert_eq!(
            md(
                "<ul><li>Survey</li><li>Install<ul><li>Bags</li></ul></li></ul><ol><li>First</li><li>Second</li></ol>"
            ),
            "- Survey\n- Install\n    - Bags\n\n1. First\n2. Second\n"
        );
    }

    #[test]
    fn scripts_styles_and_hidden_content_are_dropped() {
        assert_eq!(
            md(
                "<body><p>Keep</p><script>alert(1)</script><style>p{}</style><div style=\"display:none\">secret</div><nav>menu</nav></body>"
            ),
            "Keep\n"
        );
    }

    #[test]
    fn code_blocks_keep_their_language() {
        assert_eq!(
            md(
                "<pre><code class=\"language-python\">print(1)\n</code></pre><p>Use <code>seedctl</code>.</p>"
            ),
            "```python\nprint(1)\n```\n\nUse `seedctl`.\n"
        );
    }

    #[test]
    fn a_declared_legacy_charset_is_honoured() {
        let page =
            b"<html><head><meta charset=\"windows-1252\"></head><body><p>caf\xe9</p></body></html>";
        let p = read(page).unwrap();
        assert_eq!(super::super::ir::render(p.blocks), "caf\u{e9}\n");
        assert!(p.losses.into_warnings().is_empty());
    }

    #[test]
    fn mixed_text_and_blocks_split_into_paragraphs() {
        assert_eq!(
            md("<div>Intro text<p>Para</p>tail</div>"),
            "Intro text\n\nPara\n\ntail\n"
        );
    }

    #[test]
    fn loose_text_in_the_body_is_kept() {
        assert_eq!(
            md("<html><body>some content<img alt=\"my_image\" src=\"x\"></body></html>"),
            "some content *[image: my_image]*\n"
        );
    }

    #[test]
    fn quotes_wrap_their_paragraphs() {
        assert_eq!(
            md("<blockquote><p>Seed saved is food saved.</p></blockquote>"),
            "> Seed saved is food saved.\n"
        );
    }
}
