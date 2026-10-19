//! Word documents (`.docx`) to Markdown.
//!
//! Reads `word/document.xml` directly, with the styles, numbering, footnotes
//! and relationships it points to. Headings come from paragraph styles,
//! lists from numbering definitions, links from relationships and field codes.

use std::collections::HashMap;

use roxmltree::Node;

use super::ir::{Block, Inline, ListItem, Parsed, Run, Table, clean_alt, is_mono_font};
use super::package::{Package, attr, child, core_title, flag, is, parse_xml, read_rels};
use super::report::{LossKind, Losses};
use crate::types::ConvertError;

const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const MC: &str = "http://schemas.openxmlformats.org/markup-compatibility/2006";

pub fn read(pkg: &mut Package<'_>) -> Result<Parsed, ConvertError> {
    let max = pkg.nodes_limit();
    let document = pkg.read_text("word/document.xml")?.ok_or_else(|| {
        ConvertError::InboundFailed("no word/document.xml: not a Word document".into())
    })?;
    let rels = pkg
        .read_text("word/_rels/document.xml.rels")?
        .map(|x| read_rels(&x, max))
        .unwrap_or_default();
    let styles_xml = pkg.read_text("word/styles.xml")?;
    let numbering_xml = pkg.read_text("word/numbering.xml")?;
    let footnotes_xml = pkg.read_text("word/footnotes.xml")?;
    let endnotes_xml = pkg.read_text("word/endnotes.xml")?;
    let core_xml = pkg.read_text("docProps/core.xml")?;

    let mut ctx = Ctx {
        rels,
        styles: styles_xml
            .map(|x| Styles::parse(&x, max))
            .unwrap_or_default(),
        numbering: numbering_xml
            .map(|x| Numbering::parse(&x, max))
            .unwrap_or_default(),
        notes: HashMap::new(),
        cited: Vec::new(),
        losses: Losses::default(),
    };
    for xml in [footnotes_xml, endnotes_xml].into_iter().flatten() {
        ctx.load_notes(&xml, max);
    }

    let doc = parse_xml(&document, false, max)?;
    let body = child(doc.root_element(), W, "body")
        .ok_or_else(|| ConvertError::InboundFailed("word/document.xml has no body".into()))?;
    let mut blocks = ctx.blocks(body);

    if !ctx.cited.is_empty() {
        let mut defs = Vec::new();
        for (n, id) in ctx.cited.iter().enumerate() {
            if let Some(text) = ctx.notes.get(id) {
                let rendered = super::ir::render(vec![Block::Para(text.clone())]);
                defs.push(format!("[^{}]: {}", n + 1, rendered.trim()));
            }
        }
        ctx.losses.hit_n(LossKind::FootnotesAppended, defs.len());
        if !defs.is_empty() {
            blocks.push(Block::Raw(defs.join("\n")));
        }
    }

    let title = core_xml.and_then(|x| core_title(&x, max));
    Ok(Parsed {
        blocks,
        title,
        losses: ctx.losses,
    })
}

// ---------------------------------------------------------------- styles

#[derive(Default, Clone)]
struct Style {
    name: String,
    based_on: Option<String>,
    outline: Option<u8>,
    mono: bool,
    num: Option<(String, usize)>,
}

#[derive(Default)]
struct Styles(HashMap<String, Style>);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Role {
    Title,
    Heading(u8),
}

impl Styles {
    fn parse(xml: &str, max: u32) -> Styles {
        let mut map = HashMap::new();
        let Ok(doc) = parse_xml(xml, false, max) else {
            return Styles(map);
        };
        for s in doc.root_element().children().filter(|n| is(*n, W, "style")) {
            let Some(id) = attr(s, W, "styleId") else {
                continue;
            };
            let name = child(s, W, "name")
                .and_then(|n| attr(n, W, "val"))
                .unwrap_or("")
                .to_string();
            let ppr = child(s, W, "pPr");
            let name_lower = name.to_lowercase();
            let mono = child(s, W, "rPr")
                .and_then(|r| child(r, W, "rFonts"))
                .and_then(|f| attr(f, W, "ascii").or_else(|| attr(f, W, "hAnsi")))
                .is_some_and(is_mono_font)
                || [
                    "source code",
                    "verbatim",
                    "html code",
                    "html preformatted",
                    "macro text",
                    "plain text",
                ]
                .iter()
                .any(|k| name_lower.contains(k))
                || name_lower == "code";
            map.insert(
                id.to_string(),
                Style {
                    name,
                    based_on: child(s, W, "basedOn")
                        .and_then(|n| attr(n, W, "val"))
                        .map(str::to_string),
                    outline: ppr
                        .and_then(|p| child(p, W, "outlineLvl"))
                        .and_then(|n| attr(n, W, "val"))
                        .and_then(|v| v.parse::<u8>().ok())
                        .filter(|v| *v < 9)
                        .map(|v| v + 1),
                    mono,
                    num: ppr.and_then(|p| child(p, W, "numPr")).map(num_pr),
                },
            );
        }
        Styles(map)
    }

    /// Walks the `basedOn` chain, so "Section heading" based on "Heading 1" is a heading.
    fn chain<T>(&self, id: &str, mut pick: impl FnMut(&Style) -> Option<T>) -> Option<T> {
        let mut cur = Some(id);
        for _ in 0..12 {
            let style = self.0.get(cur?)?;
            if let Some(v) = pick(style) {
                return Some(v);
            }
            cur = style.based_on.as_deref();
        }
        None
    }

    fn role(&self, id: &str) -> Option<Role> {
        self.chain(id, |s| {
            let name = s.name.to_lowercase();
            if name == "title" {
                return Some(Role::Title);
            }
            if let Some(n) = s.outline {
                return Some(Role::Heading(n.min(6)));
            }
            name.strip_prefix("heading ")
                .and_then(|n| n.trim().parse::<u8>().ok())
                .map(|n| Role::Heading(n.clamp(1, 6)))
        })
    }

    fn is_mono(&self, id: &str) -> bool {
        self.chain(id, |s| s.mono.then_some(())).is_some()
    }

    fn is_quote(&self, id: &str) -> bool {
        self.chain(id, |s| {
            s.name.to_lowercase().contains("quote").then_some(())
        })
        .is_some()
    }

    fn numbering(&self, id: &str) -> Option<(String, usize)> {
        let name = self
            .0
            .get(id)
            .map(|s| s.name.to_lowercase())
            .unwrap_or_default();
        // "List Bullet 2" is the second level of its list, even when its numbering
        // definition carries no explicit level.
        let named_level = name
            .starts_with("list ")
            .then(|| name.chars().last().and_then(|c| c.to_digit(10)))
            .flatten()
            .map_or(0, |d| d.saturating_sub(1) as usize);
        match self.chain(id, |s| s.num.clone()) {
            Some((num, level)) => Some((num, if level == 0 { named_level } else { level })),
            // Built-in list styles are sometimes defined without a numbering link.
            None if name.starts_with("list bullet") => Some(("bullet".into(), named_level)),
            None if name.starts_with("list number") => Some(("decimal".into(), named_level)),
            None => None,
        }
    }
}

fn num_pr(np: Node<'_, '_>) -> (String, usize) {
    let id = child(np, W, "numId")
        .and_then(|n| attr(n, W, "val"))
        .unwrap_or("0")
        .to_string();
    let lvl = child(np, W, "ilvl")
        .and_then(|n| attr(n, W, "val"))
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    (id, lvl)
}

// ------------------------------------------------------------- numbering

#[derive(Default)]
struct Numbering {
    num_to_abstract: HashMap<String, String>,
    /// abstract id -> number format per level
    formats: HashMap<String, Vec<String>>,
}

impl Numbering {
    fn parse(xml: &str, max: u32) -> Numbering {
        let mut n = Numbering::default();
        let Ok(doc) = parse_xml(xml, false, max) else {
            return n;
        };
        for el in doc.root_element().children() {
            if is(el, W, "abstractNum") {
                let Some(id) = attr(el, W, "abstractNumId") else {
                    continue;
                };
                let mut levels: Vec<String> = Vec::new();
                for l in el.children().filter(|c| is(*c, W, "lvl")) {
                    let ilvl = attr(l, W, "ilvl")
                        .and_then(|v| v.parse::<usize>().ok())
                        .unwrap_or(levels.len());
                    let fmt = child(l, W, "numFmt")
                        .and_then(|f| attr(f, W, "val"))
                        .unwrap_or("bullet")
                        .to_string();
                    if levels.len() <= ilvl {
                        levels.resize(ilvl + 1, "bullet".into());
                    }
                    levels[ilvl] = fmt;
                }
                n.formats.insert(id.to_string(), levels);
            } else if is(el, W, "num")
                && let (Some(id), Some(abs)) = (
                    attr(el, W, "numId"),
                    child(el, W, "abstractNumId").and_then(|a| attr(a, W, "val")),
                )
            {
                n.num_to_abstract.insert(id.to_string(), abs.to_string());
            }
        }
        n
    }

    fn ordered(&self, num_id: &str, level: usize) -> bool {
        match num_id {
            // Synthetic ids from built-in list styles.
            "bullet" => return false,
            "decimal" => return true,
            _ => {}
        }
        self.num_to_abstract
            .get(num_id)
            .and_then(|a| self.formats.get(a))
            .and_then(|levels| levels.get(level))
            .is_some_and(|f| !matches!(f.as_str(), "bullet" | "none" | ""))
    }
}

// ---------------------------------------------------------------- reader

struct Ctx {
    rels: HashMap<String, (String, bool)>,
    styles: Styles,
    numbering: Numbering,
    /// footnote/endnote id -> text
    notes: HashMap<String, Vec<Inline>>,
    /// note ids in order of first citation; the position is the displayed number
    cited: Vec<String>,
    losses: Losses,
}

#[derive(Default, Clone)]
struct Fmt {
    bold: bool,
    italic: bool,
    strike: bool,
    code: bool,
    link: Option<String>,
}

/// A complex field (`fldChar` begin to end) being read across several runs.
struct Field {
    instr: String,
    in_result: bool,
    link: Option<String>,
    /// Table of contents and similar generated text is not content.
    skip: bool,
}

#[derive(Default)]
struct Walk {
    fields: Vec<Field>,
}

impl Walk {
    fn suppressed(&self) -> bool {
        self.fields.iter().any(|f| !f.in_result || f.skip)
    }

    fn link(&self) -> Option<String> {
        self.fields
            .iter()
            .rev()
            .find_map(|f| f.in_result.then(|| f.link.clone()).flatten())
    }
}

fn is_hyperlink_instr(instr: &str) -> Option<Option<String>> {
    let t = instr.trim();
    let rest = t
        .strip_prefix("HYPERLINK")
        .or_else(|| t.strip_prefix("hyperlink"))?;
    if rest.contains("\\l ") || rest.contains("\\l\"") {
        return Some(None);
    }
    let url = match rest.find('"') {
        Some(a) => {
            let r = &rest[a + 1..];
            r.find('"').map(|b| r[..b].to_string())
        }
        None => rest.split_whitespace().next().map(str::to_string),
    };
    Some(url.filter(|u| !u.is_empty()))
}

impl Ctx {
    fn load_notes(&mut self, xml: &str, max: u32) {
        let Ok(doc) = parse_xml(xml, false, max) else {
            return;
        };
        for n in doc
            .root_element()
            .children()
            .filter(|n| is(*n, W, "footnote") || is(*n, W, "endnote"))
        {
            let Some(id) = attr(n, W, "id") else { continue };
            if matches!(
                attr(n, W, "type"),
                Some("separator" | "continuationSeparator" | "continuationNotice")
            ) {
                continue;
            }
            let mut text: Vec<Inline> = Vec::new();
            for p in n.children().filter(|c| is(*c, W, "p")) {
                let mut walk = Walk::default();
                let inl = self.inlines(p, &Fmt::default(), &mut walk, &mut Vec::new(), false);
                if !inl.is_empty() && !text.is_empty() {
                    text.push(Inline::text(" "));
                }
                text.extend(inl);
            }
            // The footnote's own reference mark would print its number again.
            text.retain(|i| !matches!(i, Inline::FootnoteRef(_)));
            self.notes.insert(id.to_string(), text);
        }
    }

    fn blocks(&mut self, parent: Node<'_, '_>) -> Vec<Block> {
        let mut out: Vec<Block> = Vec::new();
        self.blocks_into(parent, &mut out);
        out
    }

    fn blocks_into(&mut self, parent: Node<'_, '_>, out: &mut Vec<Block>) {
        for n in parent.children().filter(|n| n.is_element()) {
            let name = n.tag_name().name();
            let ns = n.tag_name().namespace();
            match (ns, name) {
                (Some(W), "p") => self.paragraph(n, out),
                (Some(W), "tbl") => {
                    if let Some(b) = self.table(n) {
                        out.push(b);
                    }
                }
                (Some(W), "sdt") => {
                    if let Some(c) = child(n, W, "sdtContent") {
                        self.blocks_into(c, out);
                    }
                }
                (Some(W), "ins" | "moveTo" | "customXml" | "smartTag") => self.blocks_into(n, out),
                (Some(W), "del" | "moveFrom") => self.losses.hit(LossKind::HiddenContentSkipped),
                (Some(MC), "AlternateContent") => {
                    if let Some(c) = child(n, MC, "Choice") {
                        self.blocks_into(c, out);
                    }
                }
                _ => {}
            }
        }
    }

    fn paragraph(&mut self, p: Node<'_, '_>, out: &mut Vec<Block>) {
        let ppr = child(p, W, "pPr");
        let style_id = ppr
            .and_then(|x| child(x, W, "pStyle"))
            .and_then(|x| attr(x, W, "val"));
        let direct_num = ppr.and_then(|x| child(x, W, "numPr")).map(num_pr);
        let direct_outline = ppr
            .and_then(|x| child(x, W, "outlineLvl"))
            .and_then(|x| attr(x, W, "val"))
            .and_then(|v| v.parse::<u8>().ok())
            .filter(|v| *v < 9);
        let role = style_id.and_then(|s| self.styles.role(s));
        let style_mono = style_id.is_some_and(|s| self.styles.is_mono(s));
        let style_quote = style_id.is_some_and(|s| self.styles.is_quote(s));

        let mut walk = Walk::default();
        let mut extra: Vec<Block> = Vec::new();
        let inlines = self.inlines(
            p,
            &Fmt {
                code: style_mono,
                ..Fmt::default()
            },
            &mut walk,
            &mut extra,
            true,
        );
        let has_content = inlines.iter().any(|i| match i {
            Inline::Run(r) => !r.text.trim().is_empty(),
            Inline::Image(_) | Inline::FootnoteRef(_) => true,
            Inline::Break => false,
        });

        if has_content {
            let num = direct_num
                .or_else(|| style_id.and_then(|s| self.styles.numbering(s)))
                .filter(|(id, _)| id != "0");
            match (role, direct_outline, num) {
                (Some(Role::Title), _, _) => out.push(Block::Heading(0, inlines)),
                (Some(Role::Heading(n)), _, _) => out.push(Block::Heading(n, inlines)),
                (None, Some(n), _) => out.push(Block::Heading(n + 1, inlines)),
                (None, None, Some((num_id, level))) => {
                    let item = ListItem {
                        level,
                        ordered: self.numbering.ordered(&num_id, level),
                        content: inlines,
                    };
                    match out.last_mut() {
                        Some(Block::List(items)) => items.push(item),
                        _ => out.push(Block::List(vec![item])),
                    }
                }
                (None, None, None) => {
                    let all_code = inlines.iter().all(|i| match i {
                        Inline::Run(r) => r.text.trim().is_empty() || r.code,
                        Inline::Break => true,
                        _ => false,
                    });
                    if all_code {
                        let mut text = String::new();
                        for i in &inlines {
                            match i {
                                Inline::Run(r) => text.push_str(&r.text),
                                Inline::Break => text.push('\n'),
                                _ => {}
                            }
                        }
                        out.push(Block::Code { lang: None, text });
                    } else if style_quote {
                        out.push(Block::Quote(vec![Block::Para(inlines)]));
                    } else {
                        out.push(Block::Para(inlines));
                    }
                }
            }
        }
        out.extend(extra);
    }

    /// The inline content of a paragraph or of a container inside one.
    fn inlines(
        &mut self,
        parent: Node<'_, '_>,
        base: &Fmt,
        walk: &mut Walk,
        extra: &mut Vec<Block>,
        top: bool,
    ) -> Vec<Inline> {
        let mut out: Vec<Inline> = Vec::new();
        self.inline_children(parent, base, walk, extra, &mut out);
        let _ = top;
        out
    }

    fn inline_children(
        &mut self,
        parent: Node<'_, '_>,
        base: &Fmt,
        walk: &mut Walk,
        extra: &mut Vec<Block>,
        out: &mut Vec<Inline>,
    ) {
        for n in parent.children().filter(|n| n.is_element()) {
            let ns = n.tag_name().namespace();
            match (ns, n.tag_name().name()) {
                (Some(W), "r") => self.run(n, base, walk, extra, out),
                (Some(W), "hyperlink") => {
                    let mut fmt = base.clone();
                    if let Some(id) = attr(n, R, "id")
                        && let Some((target, true)) = self.rels.get(id)
                    {
                        fmt.link = Some(target.clone());
                    }
                    self.inline_children(n, &fmt, walk, extra, out);
                }
                (Some(W), "fldSimple") => {
                    let mut fmt = base.clone();
                    if let Some(Some(url)) = attr(n, W, "instr").and_then(is_hyperlink_instr) {
                        fmt.link = Some(url);
                    }
                    let skip =
                        attr(n, W, "instr").is_some_and(|i| i.trim_start().starts_with("TOC"));
                    if !skip {
                        self.inline_children(n, &fmt, walk, extra, out);
                    }
                }
                (Some(W), "ins" | "smartTag" | "moveTo" | "customXml") => {
                    self.inline_children(n, base, walk, extra, out)
                }
                (Some(W), "sdt") => {
                    if let Some(c) = child(n, W, "sdtContent") {
                        self.inline_children(c, base, walk, extra, out);
                    }
                }
                (Some(W), "del" | "moveFrom") => self.losses.hit(LossKind::HiddenContentSkipped),
                (Some(MC), "AlternateContent") => {
                    if let Some(c) = child(n, MC, "Choice") {
                        self.inline_children(c, base, walk, extra, out);
                    }
                }
                (Some(ns), "oMath" | "oMathPara") if ns.contains("officeDocument/2006/math") => {
                    let text: String = n
                        .descendants()
                        .filter(|d| d.is_element() && d.tag_name().name() == "t")
                        .filter_map(|d| d.text())
                        .collect();
                    if !text.trim().is_empty() {
                        out.push(Inline::Run(Run {
                            text,
                            italic: true,
                            ..Run::default()
                        }));
                    }
                }
                _ => {}
            }
        }
    }

    fn run(
        &mut self,
        r: Node<'_, '_>,
        base: &Fmt,
        walk: &mut Walk,
        extra: &mut Vec<Block>,
        out: &mut Vec<Inline>,
    ) {
        let rpr = child(r, W, "rPr");
        if flag(rpr.and_then(|x| child(x, W, "vanish")), W) {
            self.losses.hit(LossKind::HiddenContentSkipped);
            return;
        }
        let mut fmt = base.clone();
        if flag(rpr.and_then(|x| child(x, W, "b")), W) {
            fmt.bold = true;
        }
        if flag(rpr.and_then(|x| child(x, W, "i")), W) {
            fmt.italic = true;
        }
        if flag(rpr.and_then(|x| child(x, W, "strike")), W)
            || flag(rpr.and_then(|x| child(x, W, "dstrike")), W)
        {
            fmt.strike = true;
        }
        if let Some(f) = rpr.and_then(|x| child(x, W, "rFonts"))
            && attr(f, W, "ascii")
                .or_else(|| attr(f, W, "hAnsi"))
                .is_some_and(is_mono_font)
        {
            fmt.code = true;
        }
        if let Some(sid) = rpr
            .and_then(|x| child(x, W, "rStyle"))
            .and_then(|x| attr(x, W, "val"))
        {
            let name = self
                .styles
                .0
                .get(sid)
                .map(|s| s.name.to_lowercase())
                .unwrap_or_default();
            if name == "strong" {
                fmt.bold = true;
            } else if name == "emphasis" {
                fmt.italic = true;
            } else if self.styles.is_mono(sid) {
                fmt.code = true;
            }
        }

        let mut text = String::new();
        let flush = |text: &mut String, fmt: &Fmt, walk: &Walk, out: &mut Vec<Inline>| {
            if text.is_empty() {
                return;
            }
            if !walk.suppressed() {
                let mut f = fmt.clone();
                if let Some(l) = walk.link() {
                    f.link = Some(l);
                }
                out.push(Inline::Run(Run {
                    text: std::mem::take(text),
                    bold: f.bold,
                    italic: f.italic,
                    strike: f.strike,
                    code: f.code,
                    link: f.link,
                }));
            } else {
                text.clear();
            }
        };

        for c in r.children().filter(|c| c.is_element()) {
            if c.tag_name().namespace() == Some(MC) && c.tag_name().name() == "AlternateContent" {
                if let Some(choice) = child(c, MC, "Choice") {
                    flush(&mut text, &fmt, walk, out);
                    self.run_content(choice, &fmt, walk, extra, out, &mut text);
                }
                continue;
            }
            self.run_content_one(c, &fmt, walk, extra, out, &mut text);
        }
        flush(&mut text, &fmt, walk, out);
    }

    fn run_content(
        &mut self,
        parent: Node<'_, '_>,
        fmt: &Fmt,
        walk: &mut Walk,
        extra: &mut Vec<Block>,
        out: &mut Vec<Inline>,
        text: &mut String,
    ) {
        for c in parent.children().filter(|c| c.is_element()) {
            self.run_content_one(c, fmt, walk, extra, out, text);
        }
    }

    fn run_content_one(
        &mut self,
        c: Node<'_, '_>,
        fmt: &Fmt,
        walk: &mut Walk,
        extra: &mut Vec<Block>,
        out: &mut Vec<Inline>,
        text: &mut String,
    ) {
        let tag = c.tag_name().name();
        let in_w = c.tag_name().namespace() == Some(W);
        match (in_w, tag) {
            (true, "t") => {
                if !walk.suppressed() {
                    text.push_str(c.text().unwrap_or(""));
                }
            }
            (true, "tab" | "ptab") => text.push('\t'),
            (true, "noBreakHyphen") => text.push('-'),
            (true, "br" | "cr") => {
                let kind = attr(c, W, "type").unwrap_or("textWrapping");
                if matches!(kind, "textWrapping") && !walk.suppressed() {
                    self.flush_text(text, fmt, walk, out);
                    out.push(Inline::Break);
                }
            }
            (true, "instrText") => {
                if let Some(f) = walk.fields.last_mut()
                    && !f.in_result
                {
                    f.instr.push_str(c.text().unwrap_or(""));
                }
            }
            (true, "fldChar") => {
                self.flush_text(text, fmt, walk, out);
                match attr(c, W, "fldCharType") {
                    Some("begin") => walk.fields.push(Field {
                        instr: String::new(),
                        in_result: false,
                        link: None,
                        skip: false,
                    }),
                    Some("separate") => {
                        if let Some(f) = walk.fields.last_mut() {
                            f.in_result = true;
                            let instr = f.instr.trim().to_string();
                            if let Some(url) = is_hyperlink_instr(&instr) {
                                f.link = url;
                            }
                            f.skip = instr.starts_with("TOC") || instr.starts_with("PAGEREF");
                        }
                    }
                    Some("end") => {
                        walk.fields.pop();
                    }
                    _ => {}
                }
            }
            (true, "footnoteReference" | "endnoteReference") => {
                self.flush_text(text, fmt, walk, out);
                if let Some(id) = attr(c, W, "id")
                    && self.notes.contains_key(id)
                    && !walk.suppressed()
                {
                    let pos = match self.cited.iter().position(|x| x == id) {
                        Some(p) => p,
                        None => {
                            self.cited.push(id.to_string());
                            self.cited.len() - 1
                        }
                    };
                    out.push(Inline::FootnoteRef((pos + 1).to_string()));
                }
            }
            (true, "drawing" | "pict" | "object") => {
                self.flush_text(text, fmt, walk, out);
                if !walk.suppressed() {
                    self.drawing(c, extra, out);
                }
            }
            _ => {}
        }
    }

    fn flush_text(&self, text: &mut String, fmt: &Fmt, walk: &Walk, out: &mut Vec<Inline>) {
        if text.is_empty() {
            return;
        }
        if !walk.suppressed() {
            let mut f = fmt.clone();
            if let Some(l) = walk.link() {
                f.link = Some(l);
            }
            out.push(Inline::Run(Run {
                text: std::mem::take(text),
                bold: f.bold,
                italic: f.italic,
                strike: f.strike,
                code: f.code,
                link: f.link,
            }));
        } else {
            text.clear();
        }
    }

    /// A picture, text box, chart or diagram anchored in a run.
    fn drawing(&mut self, d: Node<'_, '_>, extra: &mut Vec<Block>, out: &mut Vec<Inline>) {
        // Text boxes hold real content: place it after the paragraph that anchors it.
        let boxes: Vec<Node<'_, '_>> = d
            .descendants()
            .filter(|n| n.is_element() && n.tag_name().name() == "txbxContent")
            .collect();
        if !boxes.is_empty() {
            for b in boxes {
                // Nested boxes are reached through their parent.
                if b.ancestors()
                    .skip(1)
                    .any(|a| a.is_element() && a.tag_name().name() == "txbxContent")
                {
                    continue;
                }
                let blocks = self.blocks(b);
                extra.extend(blocks);
                self.losses.hit(LossKind::TextBoxesInlined);
            }
            return;
        }
        let has = |name: &str| {
            d.descendants()
                .any(|n| n.is_element() && n.tag_name().name() == name)
        };
        if has("chart") || has("relIds") {
            self.losses.hit(LossKind::ChartsOmitted);
        } else if has("blip") || has("imagedata") {
            let alt = d
                .descendants()
                .find(|n| n.is_element() && n.tag_name().name() == "docPr")
                .and_then(|n| n.attribute("descr").or_else(|| n.attribute("title")))
                .unwrap_or("")
                .to_string();
            out.push(Inline::Image(clean_alt(&alt)));
            self.losses.hit(LossKind::ImagesOmitted);
        } else if has("oleObj") || tag_is(d, "object") {
            self.losses.hit(LossKind::EmbeddedObjectsOmitted);
        }
    }

    fn table(&mut self, tbl: Node<'_, '_>) -> Option<Block> {
        let rows: Vec<Node<'_, '_>> = tbl.children().filter(|c| is(*c, W, "tr")).collect();
        if rows.is_empty() {
            return None;
        }
        // A one-cell table is a boxed paragraph or callout, not tabular data.
        if rows.len() == 1 {
            let cells: Vec<Node<'_, '_>> = rows[0].children().filter(|c| is(*c, W, "tc")).collect();
            if cells.len() == 1 {
                let inner = self.blocks(cells[0]);
                if inner.is_empty() {
                    return None;
                }
                return Some(Block::Quote(inner));
            }
        }
        let mut table = Table::default();
        for tr in rows {
            let mut row: Vec<Vec<Inline>> = Vec::new();
            for tc in tr.children().filter(|c| is(*c, W, "tc")) {
                let tcpr = child(tc, W, "tcPr");
                let span = tcpr
                    .and_then(|x| child(x, W, "gridSpan"))
                    .and_then(|x| attr(x, W, "val"))
                    .and_then(|v| v.parse::<usize>().ok())
                    .unwrap_or(1)
                    .clamp(1, 64);
                let continues = tcpr
                    .and_then(|x| child(x, W, "vMerge"))
                    .is_some_and(|m| attr(m, W, "val") != Some("restart"));
                let cell = if continues { Vec::new() } else { self.cell(tc) };
                row.push(cell);
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

    fn cell(&mut self, tc: Node<'_, '_>) -> Vec<Inline> {
        let mut out: Vec<Inline> = Vec::new();
        for n in tc.children().filter(|c| c.is_element()) {
            match (n.tag_name().namespace(), n.tag_name().name()) {
                (Some(W), "p") => {
                    let ppr = child(n, W, "pPr");
                    let listed = ppr
                        .and_then(|x| child(x, W, "numPr"))
                        .map(num_pr)
                        .is_some_and(|(id, _)| id != "0");
                    let mut walk = Walk::default();
                    let mut extra = Vec::new();
                    let mut inl = self.inlines(n, &Fmt::default(), &mut walk, &mut extra, false);
                    for b in extra {
                        if let Block::Para(p) = b {
                            inl.extend(p);
                        }
                    }
                    let has = inl.iter().any(|i| !matches!(i, Inline::Break));
                    if !has {
                        continue;
                    }
                    if !out.is_empty() {
                        out.push(Inline::Break);
                    }
                    if listed {
                        out.push(Inline::text("• "));
                    }
                    out.extend(inl);
                }
                (Some(W), "tbl") => {
                    self.losses.hit(LossKind::NestedTablesFlattened);
                    let text = n
                        .descendants()
                        .filter(|d| is(*d, W, "t"))
                        .filter_map(|d| d.text())
                        .collect::<Vec<_>>()
                        .join(" ");
                    if !text.trim().is_empty() {
                        if !out.is_empty() {
                            out.push(Inline::Break);
                        }
                        out.push(Inline::text(text));
                    }
                }
                (Some(W), "sdt") => {
                    if let Some(c) = child(n, W, "sdtContent") {
                        let more = self.cell(c);
                        if !more.is_empty() {
                            if !out.is_empty() {
                                out.push(Inline::Break);
                            }
                            out.extend(more);
                        }
                    }
                }
                _ => {}
            }
        }
        out
    }
}

fn tag_is(n: Node<'_, '_>, name: &str) -> bool {
    n.is_element() && n.tag_name().name() == name
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingest::package::Limits;
    use crate::ingest::testutil::zip_of;

    const NS: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships""#;

    fn docx(body: &str, extra: &[(&str, &str)]) -> Vec<u8> {
        let doc = format!(
            r#"<?xml version="1.0"?><w:document {NS}><w:body>{body}</w:body></w:document>"#
        );
        let mut entries: Vec<(&str, Vec<u8>)> = vec![("word/document.xml", doc.into_bytes())];
        for (n, d) in extra {
            entries.push((n, d.as_bytes().to_vec()));
        }
        zip_of(&entries)
    }

    fn md(bytes: &[u8]) -> String {
        let mut pkg = Package::open(bytes, &Limits::default()).unwrap();
        let parsed = read(&mut pkg).unwrap();
        super::super::ir::render(parsed.blocks)
    }

    #[test]
    fn headings_emphasis_and_hyperlinks() {
        let styles = format!(
            r#"<w:styles {NS}><w:style w:styleId="Heading1"><w:name w:val="heading 1"/></w:style><w:style w:styleId="Title"><w:name w:val="Title"/></w:style></w:styles>"#
        );
        let rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Target="https://example.com/plan" TargetMode="External"/></Relationships>"#;
        let body = r#"
          <w:p><w:pPr><w:pStyle w:val="Title"/></w:pPr><w:r><w:t>Report</w:t></w:r></w:p>
          <w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>Intro</w:t></w:r></w:p>
          <w:p><w:r><w:t xml:space="preserve">We saw </w:t></w:r><w:r><w:rPr><w:b/></w:rPr><w:t>three</w:t></w:r>
              <w:r><w:t xml:space="preserve"> sites, see </w:t></w:r>
              <w:hyperlink r:id="rId1"><w:r><w:t>the plan</w:t></w:r></w:hyperlink><w:r><w:t>.</w:t></w:r></w:p>"#;
        let bytes = docx(
            body,
            &[
                ("word/styles.xml", &styles),
                ("word/_rels/document.xml.rels", rels),
            ],
        );
        assert_eq!(
            md(&bytes),
            "# Report\n\n## Intro\n\nWe saw **three** sites, see [the plan](https://example.com/plan).\n"
        );
    }

    #[test]
    fn numbered_and_bulleted_lists_nest_by_level() {
        let numbering = format!(
            r#"<w:numbering {NS}>
              <w:abstractNum w:abstractNumId="0"><w:lvl w:ilvl="0"><w:numFmt w:val="bullet"/></w:lvl><w:lvl w:ilvl="1"><w:numFmt w:val="bullet"/></w:lvl></w:abstractNum>
              <w:abstractNum w:abstractNumId="1"><w:lvl w:ilvl="0"><w:numFmt w:val="decimal"/></w:lvl></w:abstractNum>
              <w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num><w:num w:numId="2"><w:abstractNumId w:val="1"/></w:num></w:numbering>"#
        );
        let item = |id: u8, lvl: u8, t: &str| {
            format!(
                r#"<w:p><w:pPr><w:numPr><w:ilvl w:val="{lvl}"/><w:numId w:val="{id}"/></w:numPr></w:pPr><w:r><w:t>{t}</w:t></w:r></w:p>"#
            )
        };
        let body = [
            item(1, 0, "one"),
            item(1, 1, "inner"),
            item(2, 0, "first"),
            item(2, 0, "second"),
        ]
        .concat();
        let bytes = docx(&body, &[("word/numbering.xml", &numbering)]);
        assert_eq!(md(&bytes), "- one\n    - inner\n1. first\n2. second\n");
    }

    #[test]
    fn tables_keep_cells_and_spans() {
        let body = r#"<w:tbl>
          <w:tr><w:tc><w:tcPr><w:gridSpan w:val="2"/></w:tcPr><w:p><w:r><w:t>Summary</w:t></w:r></w:p></w:tc></w:tr>
          <w:tr><w:tc><w:p><w:r><w:t>Item</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>Cost</w:t></w:r></w:p></w:tc></w:tr>
          <w:tr><w:tc><w:p><w:r><w:t>Bags</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>6,000</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"#;
        assert_eq!(
            md(&docx(body, &[])),
            "| Summary |  |\n| --- | --- |\n| Item | Cost |\n| Bags | 6,000 |\n"
        );
    }

    #[test]
    fn field_code_hyperlinks_and_code_runs() {
        let body = r#"<w:p>
          <w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText xml:space="preserve"> HYPERLINK "https://example.com/a" </w:instrText></w:r>
          <w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>site</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r>
          <w:r><w:t xml:space="preserve"> and </w:t></w:r>
          <w:r><w:rPr><w:rFonts w:ascii="Consolas"/></w:rPr><w:t>seedctl</w:t></w:r></w:p>"#;
        assert_eq!(
            md(&docx(body, &[])),
            "[site](https://example.com/a) and `seedctl`\n"
        );
    }

    #[test]
    fn deleted_text_and_toc_fields_are_left_out() {
        let body = r#"<w:p><w:r><w:t>Keep</w:t></w:r><w:del><w:r><w:delText>gone</w:delText></w:r></w:del></w:p>
          <w:p><w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText> TOC \o "1-3" </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>Contents 1</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r></w:p>"#;
        assert_eq!(md(&docx(body, &[])), "Keep\n");
    }

    #[test]
    fn footnotes_are_numbered_by_first_citation() {
        let notes = format!(
            r#"<w:footnotes {NS}><w:footnote w:id="5"><w:p><w:r><w:t>Source: survey.</w:t></w:r></w:p></w:footnote></w:footnotes>"#
        );
        let body = r#"<w:p><w:r><w:t>Seed loss</w:t></w:r><w:r><w:footnoteReference w:id="5"/></w:r></w:p>"#;
        assert_eq!(
            md(&docx(body, &[("word/footnotes.xml", &notes)])),
            "Seed loss[^1]\n\n[^1]: Source: survey.\n"
        );
    }

    #[test]
    fn a_one_cell_table_is_a_quote() {
        let body = r#"<w:tbl><w:tr><w:tc><w:p><w:r><w:t>Note this</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"#;
        assert_eq!(md(&docx(body, &[])), "> Note this\n");
    }

    #[test]
    fn a_file_without_a_body_part_is_not_a_word_document() {
        let bytes = zip_of(&[("other.xml", b"<a/>".to_vec())]);
        let mut pkg = Package::open(&bytes, &Limits::default()).unwrap();
        assert!(read(&mut pkg).is_err());
    }
}
