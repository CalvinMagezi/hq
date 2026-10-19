//! OpenDocument text (`.odt`) and presentations (`.odp`) to Markdown.
//!
//! Spreadsheets (`.ods`) go through the spreadsheet reader. The reader works
//! on `content.xml` and the style definitions, so bold, italic and code come
//! from the styles a paragraph or span names.

use std::collections::HashMap;

use roxmltree::{Document, Node};

use super::ir::{
    Block, Inline, ListItem, Parsed, Run, Table, clean_alt, collapse_ws, is_mono_font,
};
use super::package::{Package, attr, child, is, parse_xml};
use super::report::{LossKind, Losses};
use crate::types::ConvertError;

const OFFICE: &str = "urn:oasis:names:tc:opendocument:xmlns:office:1.0";
const TEXT: &str = "urn:oasis:names:tc:opendocument:xmlns:text:1.0";
const TABLE: &str = "urn:oasis:names:tc:opendocument:xmlns:table:1.0";
const DRAW: &str = "urn:oasis:names:tc:opendocument:xmlns:drawing:1.0";
const STYLE: &str = "urn:oasis:names:tc:opendocument:xmlns:style:1.0";
const FO: &str = "urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0";
const XLINK: &str = "http://www.w3.org/1999/xlink";
const SVG: &str = "urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0";
const PRES: &str = "urn:oasis:names:tc:opendocument:xmlns:presentation:1.0";
const DC: &str = "http://purl.org/dc/elements/1.1/";

pub fn read_text(pkg: &mut Package<'_>) -> Result<Parsed, ConvertError> {
    read(pkg, false)
}

pub fn read_presentation(pkg: &mut Package<'_>) -> Result<Parsed, ConvertError> {
    read(pkg, true)
}

fn read(pkg: &mut Package<'_>, presentation: bool) -> Result<Parsed, ConvertError> {
    let max = pkg.nodes_limit();
    let content = pkg.read_text("content.xml")?.ok_or_else(|| {
        ConvertError::InboundFailed("no content.xml: not an OpenDocument file".into())
    })?;
    if pkg
        .read_text("META-INF/manifest.xml")?
        .is_some_and(|m| m.contains("encryption-data"))
    {
        return Err(ConvertError::Encrypted(
            "this OpenDocument file is encrypted".into(),
        ));
    }
    let styles_xml = pkg.read_text("styles.xml")?;
    let meta_xml = pkg.read_text("meta.xml")?;
    let doc = parse_xml(&content, false, max)?;

    let mut ctx = Ctx::default();
    ctx.styles.load(&doc);
    ctx.lists.load(&doc);
    let styles_doc = styles_xml
        .as_ref()
        .and_then(|x| parse_xml(x, false, max).ok());
    if let Some(d) = &styles_doc {
        ctx.styles.load(d);
        ctx.lists.load(d);
    }

    let body = child(doc.root_element(), OFFICE, "body")
        .ok_or_else(|| ConvertError::InboundFailed("content.xml has no body".into()))?;
    let mut blocks = Vec::new();
    if presentation {
        let Some(pres) = child(body, OFFICE, "presentation") else {
            return Err(ConvertError::InboundFailed(
                "no presentation in this file".into(),
            ));
        };
        ctx.slides(pres, &mut blocks);
    } else {
        let Some(text) = child(body, OFFICE, "text") else {
            return Err(ConvertError::InboundFailed(
                "no text body in this file".into(),
            ));
        };
        ctx.blocks_into(text, &mut blocks);
    }
    if blocks.is_empty() {
        return Err(ConvertError::InboundFailed(
            "no text found in this document".into(),
        ));
    }
    if !ctx.notes.is_empty() {
        let defs: Vec<String> = ctx
            .notes
            .iter()
            .enumerate()
            .map(|(i, t)| {
                format!(
                    "[^{}]: {}",
                    i + 1,
                    super::ir::render(vec![Block::Para(t.clone())]).trim()
                )
            })
            .collect();
        ctx.losses.hit_n(LossKind::FootnotesAppended, defs.len());
        blocks.push(Block::Raw(defs.join("\n")));
    }
    let title = meta_xml.and_then(|x| {
        let d = parse_xml(&x, false, max).ok()?;
        let t = d
            .descendants()
            .find(|n| is(*n, DC, "title"))?
            .text()?
            .trim()
            .to_string();
        (!t.is_empty()).then_some(t)
    });
    Ok(Parsed {
        blocks,
        title,
        losses: ctx.losses,
    })
}

// ----------------------------------------------------------------- styles

#[derive(Default, Clone)]
struct StyleProps {
    bold: Option<bool>,
    italic: Option<bool>,
    strike: Option<bool>,
    mono: Option<bool>,
    parent: Option<String>,
    /// The style name with ODF's `_20_` escapes read back as spaces, lower case.
    label: String,
}

#[derive(Default)]
struct Styles(HashMap<String, StyleProps>);

impl Styles {
    fn load(&mut self, doc: &Document<'_>) {
        for st in doc.descendants().filter(|n| is(*n, STYLE, "style")) {
            let Some(name) = attr(st, STYLE, "name") else {
                continue;
            };
            let tp = child(st, STYLE, "text-properties");
            let get = |k: &str| tp.and_then(|t| attr(t, FO, k));
            let bold =
                get("font-weight").map(|v| v == "bold" || v.parse::<u32>().is_ok_and(|n| n >= 600));
            let italic = get("font-style").map(|v| v == "italic" || v == "oblique");
            let strike = tp
                .and_then(|t| attr(t, STYLE, "text-line-through-style"))
                .map(|v| v != "none");
            let font =
                tp.and_then(|t| attr(t, STYLE, "font-name").or_else(|| attr(t, FO, "font-family")));
            let label = name.replace("_20_", " ").to_lowercase();
            let named_mono = ["preformatted", "source", "verbatim", "code", "teletype"]
                .iter()
                .any(|k| label.contains(k));
            self.0.insert(
                name.to_string(),
                StyleProps {
                    bold,
                    italic,
                    strike,
                    mono: font.map(is_mono_font).or(named_mono.then_some(true)),
                    parent: attr(st, STYLE, "parent-style-name").map(str::to_string),
                    label,
                },
            );
        }
    }

    fn lookup<T>(&self, name: &str, pick: impl Fn(&StyleProps) -> Option<T>) -> Option<T> {
        let mut cur = Some(name);
        for _ in 0..12 {
            let s = self.0.get(cur?)?;
            if let Some(v) = pick(s) {
                return Some(v);
            }
            cur = s.parent.as_deref();
        }
        None
    }

    fn bold(&self, n: &str) -> bool {
        self.lookup(n, |s| s.bold).unwrap_or(false)
    }
    fn italic(&self, n: &str) -> bool {
        self.lookup(n, |s| s.italic).unwrap_or(false)
    }
    fn strike(&self, n: &str) -> bool {
        self.lookup(n, |s| s.strike).unwrap_or(false)
    }
    fn mono(&self, n: &str) -> bool {
        self.lookup(n, |s| s.mono).unwrap_or(false)
    }
    fn is_title(&self, n: &str) -> bool {
        self.lookup(n, |s| (s.label == "title").then_some(()))
            .is_some()
    }
    fn is_quote(&self, n: &str) -> bool {
        self.lookup(n, |s| {
            (s.label.contains("quotation") || s.label.contains("quote")).then_some(())
        })
        .is_some()
    }
}

/// `text:list-style` definitions: which levels are numbered.
#[derive(Default)]
struct ListStyles(HashMap<String, Vec<bool>>);

impl ListStyles {
    fn load(&mut self, doc: &Document<'_>) {
        for ls in doc.descendants().filter(|n| is(*n, TEXT, "list-style")) {
            let Some(name) = attr(ls, STYLE, "name") else {
                continue;
            };
            let mut levels: Vec<bool> = Vec::new();
            for l in ls.children().filter(|c| c.is_element()) {
                let ordered = l.tag_name().name() == "list-level-style-number";
                let level = attr(l, TEXT, "level")
                    .and_then(|v| v.parse::<usize>().ok())
                    .unwrap_or(1);
                if levels.len() < level {
                    levels.resize(level, false);
                }
                levels[level - 1] = ordered;
            }
            self.0.insert(name.to_string(), levels);
        }
    }

    fn ordered(&self, style: Option<&str>, level: usize) -> bool {
        style
            .and_then(|s| self.0.get(s))
            .and_then(|l| l.get(level))
            .copied()
            .unwrap_or(false)
    }
}

// ----------------------------------------------------------------- reader

#[derive(Default, Clone)]
struct Fmt {
    bold: bool,
    italic: bool,
    strike: bool,
    code: bool,
    link: Option<String>,
}

#[derive(Default)]
struct Ctx {
    styles: Styles,
    lists: ListStyles,
    losses: Losses,
    notes: Vec<Vec<Inline>>,
}

impl Ctx {
    fn blocks_into(&mut self, parent: Node<'_, '_>, out: &mut Vec<Block>) {
        for n in parent.children().filter(|n| n.is_element()) {
            let ns = n.tag_name().namespace();
            match (ns, n.tag_name().name()) {
                (Some(TEXT), "h") => {
                    let level = attr(n, TEXT, "outline-level")
                        .and_then(|v| v.parse::<u8>().ok())
                        .unwrap_or(1);
                    let mut extra = Vec::new();
                    let inl = self.inlines(n, &Fmt::default(), &mut extra);
                    out.push(Block::Heading(level, inl));
                    out.extend(extra);
                }
                (Some(TEXT), "p") => self.paragraph(n, out),
                (Some(TEXT), "list") => {
                    let mut items = Vec::new();
                    let mut after = Vec::new();
                    let style = attr(n, TEXT, "style-name").map(str::to_string);
                    self.list(n, style.as_deref(), 0, &mut items, &mut after);
                    if !items.is_empty() {
                        out.push(Block::List(items));
                    }
                    out.extend(after);
                }
                (Some(TABLE), "table") => {
                    if let Some(t) = self.table(n) {
                        out.push(t);
                    }
                }
                (Some(TEXT), "section" | "index-body" | "index-title") => self.blocks_into(n, out),
                (
                    Some(TEXT),
                    "table-of-content" | "alphabetical-index" | "illustration-index"
                    | "table-index" | "bibliography",
                ) => self.losses.hit(LossKind::HiddenContentSkipped),
                (Some(DRAW), "frame") => {
                    let mut inl = Vec::new();
                    let mut extra = Vec::new();
                    self.frame(n, &mut inl, &mut extra);
                    if !inl.is_empty() {
                        out.push(Block::Para(inl));
                    }
                    out.extend(extra);
                }
                _ => {}
            }
        }
    }

    fn paragraph(&mut self, p: Node<'_, '_>, out: &mut Vec<Block>) {
        let style = attr(p, TEXT, "style-name").unwrap_or("");
        let base = Fmt {
            code: self.styles.mono(style),
            ..Fmt::default()
        };
        let mut extra = Vec::new();
        let inl = self.inlines(p, &base, &mut extra);
        let has = inl.iter().any(|i| match i {
            Inline::Run(r) => !r.text.trim().is_empty(),
            Inline::Image(_) | Inline::FootnoteRef(_) => true,
            Inline::Break => false,
        });
        if has {
            if self.styles.is_title(style) {
                out.push(Block::Heading(0, inl));
            } else if base.code
                || inl.iter().all(|i| match i {
                    Inline::Run(r) => r.text.trim().is_empty() || r.code,
                    Inline::Break => true,
                    _ => false,
                })
            {
                let mut text = String::new();
                for i in &inl {
                    match i {
                        Inline::Run(r) => text.push_str(&r.text),
                        Inline::Break => text.push('\n'),
                        _ => {}
                    }
                }
                out.push(Block::Code { lang: None, text });
            } else if self.styles.is_quote(style) {
                out.push(Block::Quote(vec![Block::Para(inl)]));
            } else {
                out.push(Block::Para(inl));
            }
        }
        out.extend(extra);
    }

    /// Flattens a (possibly nested) list. Content that cannot sit in a list item
    /// (tables inside an item) is placed after the list in `after`.
    fn list(
        &mut self,
        list: Node<'_, '_>,
        style: Option<&str>,
        level: usize,
        items: &mut Vec<ListItem>,
        after: &mut Vec<Block>,
    ) {
        for item in list
            .children()
            .filter(|c| is(*c, TEXT, "list-item") || is(*c, TEXT, "list-header"))
        {
            let mut content: Vec<Inline> = Vec::new();
            let mut nested: Vec<(Node<'_, '_>, Option<String>)> = Vec::new();
            for c in item.children().filter(|c| c.is_element()) {
                match (c.tag_name().namespace(), c.tag_name().name()) {
                    (Some(TEXT), "p" | "h") => {
                        let mut extra = Vec::new();
                        let inl = self.inlines(c, &Fmt::default(), &mut extra);
                        if !inl.is_empty() && !content.is_empty() {
                            content.push(Inline::Break);
                        }
                        content.extend(inl);
                        after.extend(extra);
                    }
                    (Some(TEXT), "list") => {
                        nested.push((c, attr(c, TEXT, "style-name").map(str::to_string)));
                    }
                    (Some(TABLE), "table") => {
                        if let Some(t) = self.table(c) {
                            after.push(t);
                        }
                    }
                    _ => {}
                }
            }
            if content.iter().any(|i| !matches!(i, Inline::Break)) {
                items.push(ListItem {
                    level,
                    ordered: self.lists.ordered(style, level),
                    content,
                });
            }
            for (n, own) in nested {
                // A nested list without a style of its own continues its parent's.
                let s = own.as_deref().or(style);
                self.list(n, s, level + 1, items, after);
            }
        }
    }

    fn table(&mut self, t: Node<'_, '_>) -> Option<Block> {
        let mut table = Table::default();
        self.table_rows(t, &mut table);
        // Trailing empty rows and cells come from the spreadsheet-style grid.
        while table
            .rows
            .last()
            .is_some_and(|r| r.iter().all(|c| c.is_empty()))
        {
            table.rows.pop();
        }
        for row in &mut table.rows {
            while row.last().is_some_and(|c| c.is_empty()) {
                row.pop();
            }
        }
        (!table.rows.is_empty()).then_some(Block::Table(table))
    }

    fn table_rows(&mut self, parent: Node<'_, '_>, table: &mut Table) {
        for n in parent.children().filter(|c| c.is_element()) {
            match (n.tag_name().namespace(), n.tag_name().name()) {
                (Some(TABLE), "table-row") => {
                    let repeat = attr(n, TABLE, "number-rows-repeated")
                        .and_then(|v| v.parse::<usize>().ok())
                        .unwrap_or(1)
                        .clamp(1, 100);
                    let mut row: Vec<Vec<Inline>> = Vec::new();
                    for c in n.children().filter(|c| c.is_element()) {
                        let name = c.tag_name().name();
                        if !(is(c, TABLE, "table-cell") || is(c, TABLE, "covered-table-cell")) {
                            continue;
                        }
                        let cols = attr(c, TABLE, "number-columns-repeated")
                            .and_then(|v| v.parse::<usize>().ok())
                            .unwrap_or(1)
                            .clamp(1, 64);
                        let cell = if name == "covered-table-cell" {
                            Vec::new()
                        } else {
                            self.cell(c)
                        };
                        for _ in 0..cols {
                            row.push(cell.clone());
                        }
                    }
                    let empty = row.iter().all(|c| c.is_empty());
                    for _ in 0..if empty { 1 } else { repeat } {
                        table.rows.push(row.clone());
                    }
                }
                (Some(TABLE), "table-header-rows" | "table-rows" | "table-row-group") => {
                    self.table_rows(n, table)
                }
                _ => {}
            }
        }
    }

    fn cell(&mut self, c: Node<'_, '_>) -> Vec<Inline> {
        let mut out: Vec<Inline> = Vec::new();
        for p in c.children().filter(|x| x.is_element()) {
            match (p.tag_name().namespace(), p.tag_name().name()) {
                (Some(TEXT), "p" | "h") => {
                    let mut extra = Vec::new();
                    let inl = self.inlines(p, &Fmt::default(), &mut extra);
                    if !inl.is_empty() && !out.is_empty() {
                        out.push(Inline::Break);
                    }
                    out.extend(inl);
                }
                (Some(TEXT), "list") => {
                    for it in p.descendants().filter(|d| is(*d, TEXT, "p")) {
                        let mut extra = Vec::new();
                        let inl = self.inlines(it, &Fmt::default(), &mut extra);
                        if !inl.is_empty() {
                            if !out.is_empty() {
                                out.push(Inline::Break);
                            }
                            out.push(Inline::text("• "));
                            out.extend(inl);
                        }
                    }
                }
                (Some(TABLE), "table") => {
                    self.losses.hit(LossKind::NestedTablesFlattened);
                    let text = p
                        .descendants()
                        .filter(|d| is(*d, TEXT, "p"))
                        .map(|d| d.descendants().filter_map(|x| x.text()).collect::<String>())
                        .filter(|t| !t.trim().is_empty())
                        .collect::<Vec<_>>()
                        .join(" ");
                    if !text.is_empty() {
                        out.push(Inline::text(text));
                    }
                }
                _ => {}
            }
        }
        out
    }

    fn inlines(&mut self, parent: Node<'_, '_>, base: &Fmt, extra: &mut Vec<Block>) -> Vec<Inline> {
        let mut out = Vec::new();
        self.inline_children(parent, base, extra, &mut out);
        out
    }

    fn inline_children(
        &mut self,
        parent: Node<'_, '_>,
        base: &Fmt,
        extra: &mut Vec<Block>,
        out: &mut Vec<Inline>,
    ) {
        for n in parent.children() {
            if n.is_text() {
                let t = collapse_ws(n.text().unwrap_or(""));
                if !t.is_empty() {
                    out.push(Inline::Run(Run {
                        text: t,
                        bold: base.bold,
                        italic: base.italic,
                        strike: base.strike,
                        code: base.code,
                        link: base.link.clone(),
                    }));
                }
                continue;
            }
            if !n.is_element() {
                continue;
            }
            match (n.tag_name().namespace(), n.tag_name().name()) {
                (Some(TEXT), "span") => {
                    let mut f = base.clone();
                    if let Some(name) = attr(n, TEXT, "style-name") {
                        f.bold |= self.styles.bold(name);
                        f.italic |= self.styles.italic(name);
                        f.strike |= self.styles.strike(name);
                        f.code |= self.styles.mono(name);
                    }
                    self.inline_children(n, &f, extra, out);
                }
                (Some(TEXT), "a") => {
                    let mut f = base.clone();
                    f.link = attr(n, XLINK, "href")
                        .filter(|h| !h.starts_with('#') && !h.is_empty())
                        .map(str::to_string);
                    self.inline_children(n, &f, extra, out);
                }
                (Some(TEXT), "s") => {
                    let c = attr(n, TEXT, "c")
                        .and_then(|v| v.parse::<usize>().ok())
                        .unwrap_or(1)
                        .min(64);
                    out.push(Inline::Run(Run {
                        text: " ".repeat(c),
                        code: base.code,
                        ..Run::default()
                    }));
                }
                (Some(TEXT), "tab") => out.push(Inline::text(" ")),
                (Some(TEXT), "line-break") => out.push(Inline::Break),
                (Some(TEXT), "note") => {
                    let mut text = Vec::new();
                    if let Some(body) = child(n, TEXT, "note-body") {
                        for p in body.children().filter(|c| is(*c, TEXT, "p")) {
                            let mut ex = Vec::new();
                            let inl = self.inlines(p, &Fmt::default(), &mut ex);
                            if !text.is_empty() && !inl.is_empty() {
                                text.push(Inline::text(" "));
                            }
                            text.extend(inl);
                        }
                    }
                    self.notes.push(text);
                    out.push(Inline::FootnoteRef(self.notes.len().to_string()));
                }
                (Some(DRAW), "frame") => self.frame(n, out, extra),
                (
                    Some(TEXT),
                    "soft-page-break" | "bookmark" | "bookmark-start" | "bookmark-end"
                    | "reference-mark" | "change" | "change-start" | "change-end",
                ) => {}
                (Some(TEXT), "tracked-changes") => {}
                // Fields such as dates and page numbers keep their displayed text.
                (Some(TEXT), _) => self.inline_children(n, base, extra, out),
                _ => {}
            }
        }
    }

    /// A picture, text box or embedded object anchored in text.
    fn frame(&mut self, f: Node<'_, '_>, inl: &mut Vec<Inline>, extra: &mut Vec<Block>) {
        if let Some(tb) = child(f, DRAW, "text-box") {
            let mut blocks = Vec::new();
            self.blocks_into(tb, &mut blocks);
            if !blocks.is_empty() {
                extra.extend(blocks);
                self.losses.hit(LossKind::TextBoxesInlined);
            }
        } else if child(f, DRAW, "image").is_some() {
            let alt = child(f, SVG, "title")
                .or_else(|| child(f, SVG, "desc"))
                .and_then(|n| n.text())
                .unwrap_or("");
            inl.push(Inline::Image(clean_alt(alt)));
            self.losses.hit(LossKind::ImagesOmitted);
        } else if child(f, DRAW, "object").is_some() || child(f, DRAW, "object-ole").is_some() {
            self.losses.hit(LossKind::EmbeddedObjectsOmitted);
        }
    }

    fn slides(&mut self, pres: Node<'_, '_>, out: &mut Vec<Block>) {
        let mut n = 0;
        for page in pres.children().filter(|c| is(*c, DRAW, "page")) {
            n += 1;
            let mut title: Option<String> = None;
            let mut body: Vec<Block> = Vec::new();
            let mut notes: Vec<Block> = Vec::new();
            for fr in page.children().filter(|c| is(*c, DRAW, "frame")) {
                let class = attr(fr, PRES, "class").unwrap_or("");
                let mut blocks = Vec::new();
                if let Some(tb) = child(fr, DRAW, "text-box") {
                    self.blocks_into(tb, &mut blocks);
                } else if let Some(t) = child(fr, TABLE, "table") {
                    if let Some(tb) = self.table(t) {
                        blocks.push(tb);
                    }
                } else if child(fr, DRAW, "image").is_some() {
                    self.losses.hit(LossKind::ImagesOmitted);
                    blocks.push(Block::Para(vec![Inline::Image(String::new())]));
                } else {
                    continue;
                }
                match class {
                    "title" => {
                        let text = blocks_text(&blocks);
                        if !text.is_empty() && title.is_none() {
                            title = Some(text);
                        }
                    }
                    "page-number" | "date-time" | "footer" | "header" => {}
                    _ => body.extend(blocks),
                }
            }
            if let Some(np) = child(page, PRES, "notes") {
                for fr in np.children().filter(|c| is(*c, DRAW, "frame")) {
                    if attr(fr, PRES, "class") == Some("notes")
                        && let Some(tb) = child(fr, DRAW, "text-box")
                    {
                        self.blocks_into(tb, &mut notes);
                    }
                }
            }
            let heading = match &title {
                Some(t) => format!("Slide {n}: {t}"),
                None => format!("Slide {n}"),
            };
            out.push(Block::Heading(2, vec![Inline::text(heading)]));
            out.extend(body);
            if !notes.is_empty() {
                out.push(Block::Heading(3, vec![Inline::text("Notes")]));
                out.extend(notes);
            }
        }
    }
}

fn blocks_text(blocks: &[Block]) -> String {
    let mut parts = Vec::new();
    for b in blocks {
        match b {
            Block::Para(i) | Block::Heading(_, i) => parts.push(super::ir::plain_text(i)),
            Block::List(items) => {
                parts.extend(items.iter().map(|it| super::ir::plain_text(&it.content)))
            }
            _ => {}
        }
    }
    parts.retain(|p| !p.is_empty());
    parts.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingest::package::Limits;
    use crate::ingest::testutil::zip_of;

    const NS: &str = r#"xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" xmlns:xlink="http://www.w3.org/1999/xlink" xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0""#;

    fn odt(auto: &str, body: &str) -> String {
        let content = format!(
            r#"<?xml version="1.0"?><office:document-content {NS}><office:automatic-styles>{auto}</office:automatic-styles><office:body><office:text>{body}</office:text></office:body></office:document-content>"#
        );
        let bytes = zip_of(&[("content.xml", content.into_bytes())]);
        let mut pkg = Package::open(&bytes, &Limits::default()).unwrap();
        super::super::ir::render(read_text(&mut pkg).unwrap().blocks)
    }

    #[test]
    fn headings_spans_and_links() {
        let auto = r#"<style:style style:name="T1" style:family="text"><style:text-properties fo:font-weight="bold"/></style:style>
                      <style:style style:name="T2" style:family="text"><style:text-properties style:font-name="Courier New"/></style:style>"#;
        let body = r#"<text:h text:outline-level="1">Intro</text:h>
          <text:p>We saw <text:span text:style-name="T1">three</text:span> sites, see <text:a xlink:href="https://example.com/p">the plan</text:a> and run <text:span text:style-name="T2">seedctl</text:span>.</text:p>"#;
        assert_eq!(
            odt(auto, body),
            "# Intro\n\nWe saw **three** sites, see [the plan](https://example.com/p) and run `seedctl`.\n"
        );
    }

    #[test]
    fn nested_lists_use_the_list_style_for_numbering() {
        let auto = r#"<text:list-style style:name="L1"><text:list-level-style-number text:level="1"/><text:list-level-style-bullet text:level="2"/></text:list-style>"#;
        let body = r#"<text:list text:style-name="L1">
            <text:list-item><text:p>first</text:p><text:list><text:list-item><text:p>inner</text:p></text:list-item></text:list></text:list-item>
            <text:list-item><text:p>second</text:p></text:list-item></text:list>"#;
        assert_eq!(odt(auto, body), "1. first\n    - inner\n2. second\n");
    }

    #[test]
    fn tables_expand_repeated_cells_and_drop_trailing_empties() {
        let body = r#"<table:table>
          <table:table-row><table:table-cell><text:p>Site</text:p></table:table-cell><table:table-cell><text:p>Farmers</text:p></table:table-cell><table:table-cell table:number-columns-repeated="50"/></table:table-row>
          <table:table-row><table:table-cell><text:p>Mukono</text:p></table:table-cell><table:table-cell><text:p>42</text:p></table:table-cell></table:table-row>
          <table:table-row table:number-rows-repeated="1000"><table:table-cell table:number-columns-repeated="50"/></table:table-row></table:table>"#;
        assert_eq!(
            odt("", body),
            "| Site | Farmers |\n| --- | --- |\n| Mukono | 42 |\n"
        );
    }

    #[test]
    fn spaces_and_line_breaks() {
        let body = r#"<text:p>a<text:s text:c="3"/>b<text:line-break/>c</text:p>"#;
        assert_eq!(odt("", body), "a b\\\nc\n");
    }

    #[test]
    fn footnotes_become_numbered_definitions() {
        let body = r#"<text:p>Loss fell<text:note text:note-class="footnote"><text:note-citation>1</text:note-citation><text:note-body><text:p>Survey data.</text:p></text:note-body></text:note></text:p>"#;
        assert_eq!(odt("", body), "Loss fell[^1]\n\n[^1]: Survey data.\n");
    }
}
