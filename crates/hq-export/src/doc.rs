//! The document model every writer consumes.
//!
//! A note is parsed once into [`Document`]; PDF, HTML, DOCX, XLSX and the rest
//! then walk the same tree, so a fix to how a table or a callout is understood
//! lands in every format at once.

use pulldown_cmark::{Alignment, CodeBlockKind, Event, Options, Parser, Tag, TagEnd};

/// A parsed note: a title plus a list of blocks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
    pub title: String,
    pub blocks: Vec<Block>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    Heading {
        level: u8,
        content: Vec<Inline>,
    },
    Paragraph(Vec<Inline>),
    Quote(Vec<Block>),
    /// An Obsidian-style `> [!kind] Title` block. `kind` is lowercased.
    Callout {
        kind: String,
        title: Option<String>,
        body: Vec<Block>,
    },
    List {
        ordered: bool,
        start: u64,
        items: Vec<ListItem>,
    },
    Code {
        lang: Option<String>,
        text: String,
    },
    Table(Table),
    Rule,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListItem {
    /// `Some` for task-list items (`- [ ]` / `- [x]`).
    pub checked: Option<bool>,
    pub blocks: Vec<Block>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Table {
    pub aligns: Vec<Align>,
    pub header: Vec<Vec<Inline>>,
    pub rows: Vec<Vec<Vec<Inline>>>,
}

impl Table {
    /// Number of columns, taken from the widest row so ragged tables stay safe.
    pub fn columns(&self) -> usize {
        self.rows
            .iter()
            .map(Vec::len)
            .chain([self.header.len(), self.aligns.len()])
            .max()
            .unwrap_or(0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Inline {
    Text(String),
    Emph(Vec<Inline>),
    Strong(Vec<Inline>),
    Strike(Vec<Inline>),
    Code(String),
    Link { url: String, content: Vec<Inline> },
    Image { alt: String, src: String },
    SoftBreak,
    HardBreak,
}

/// Flatten inline content to plain text, the way a cell or a title wants it.
pub fn plain_text(inlines: &[Inline]) -> String {
    let mut out = String::new();
    for inline in inlines {
        match inline {
            Inline::Text(t) | Inline::Code(t) => out.push_str(t),
            Inline::Emph(c) | Inline::Strong(c) | Inline::Strike(c) => out.push_str(&plain_text(c)),
            Inline::Link { content, .. } => out.push_str(&plain_text(content)),
            Inline::Image { alt, .. } => out.push_str(alt),
            Inline::SoftBreak => out.push(' '),
            Inline::HardBreak => out.push('\n'),
        }
    }
    out
}

impl Document {
    /// Parse already-prepared Markdown (see `hq_convert::note_pdf::prepare_note`).
    pub fn from_markdown(title: impl Into<String>, markdown: &str) -> Self {
        Document {
            title: title.into(),
            blocks: parse_blocks(markdown),
        }
    }

    /// Every table in the document, in reading order, with the nearest
    /// preceding heading as its name. Used by the tabular writers.
    pub fn tables(&self) -> Vec<(Option<String>, &Table)> {
        fn walk<'a>(
            blocks: &'a [Block],
            heading: &mut Option<String>,
            out: &mut Vec<(Option<String>, &'a Table)>,
        ) {
            for block in blocks {
                match block {
                    Block::Heading { content, .. } => *heading = Some(plain_text(content)),
                    Block::Table(t) => out.push((heading.clone(), t)),
                    Block::Quote(b) | Block::Callout { body: b, .. } => walk(b, heading, out),
                    Block::List { items, .. } => {
                        for item in items {
                            walk(&item.blocks, heading, out);
                        }
                    }
                    _ => {}
                }
            }
        }
        let mut out = Vec::new();
        walk(&self.blocks, &mut None, &mut out);
        out
    }

    /// Every fenced code block, in reading order.
    pub fn code_blocks(&self) -> Vec<(Option<&str>, &str)> {
        fn walk<'a>(blocks: &'a [Block], out: &mut Vec<(Option<&'a str>, &'a str)>) {
            for block in blocks {
                match block {
                    Block::Code { lang, text } => out.push((lang.as_deref(), text.as_str())),
                    Block::Quote(b) | Block::Callout { body: b, .. } => walk(b, out),
                    Block::List { items, .. } => {
                        for item in items {
                            walk(&item.blocks, out);
                        }
                    }
                    _ => {}
                }
            }
        }
        let mut out = Vec::new();
        walk(&self.blocks, &mut out);
        out
    }
}

enum Span {
    Emph,
    Strong,
    Strike,
}

enum Frame {
    Root(Vec<Block>),
    Para(Vec<Inline>),
    Heading(u8, Vec<Inline>),
    Quote(Vec<Block>),
    List {
        ordered: bool,
        start: u64,
        items: Vec<ListItem>,
    },
    Item {
        checked: Option<bool>,
        blocks: Vec<Block>,
        inlines: Vec<Inline>,
    },
    Code {
        lang: Option<String>,
        text: String,
    },
    Table {
        aligns: Vec<Align>,
        header: Vec<Vec<Inline>>,
        rows: Vec<Vec<Vec<Inline>>>,
        row: Vec<Vec<Inline>>,
    },
    Cell(Vec<Inline>),
    Span(Span, Vec<Inline>),
    Link {
        url: String,
        content: Vec<Inline>,
    },
    Image {
        src: String,
        alt: String,
    },
    /// Containers with no model equivalent (footnote bodies, raw HTML, ...).
    Ignore,
}

fn push_text(target: &mut Vec<Inline>, text: &str) {
    if let Some(Inline::Text(last)) = target.last_mut() {
        last.push_str(text);
    } else {
        target.push(Inline::Text(text.to_owned()));
    }
}

fn flush_item_inlines(blocks: &mut Vec<Block>, inlines: &mut Vec<Inline>) {
    if !inlines.is_empty() {
        blocks.push(Block::Paragraph(std::mem::take(inlines)));
    }
}

fn push_block(stack: &mut [Frame], block: Block) {
    match stack.last_mut() {
        Some(Frame::Root(blocks)) | Some(Frame::Quote(blocks)) => blocks.push(block),
        Some(Frame::Item {
            blocks, inlines, ..
        }) => {
            flush_item_inlines(blocks, inlines);
            blocks.push(block);
        }
        _ => {}
    }
}

fn push_inline(stack: &mut [Frame], inline: Inline) {
    let target = match stack.last_mut() {
        Some(Frame::Para(v))
        | Some(Frame::Heading(_, v))
        | Some(Frame::Cell(v))
        | Some(Frame::Span(_, v)) => v,
        Some(Frame::Link { content, .. }) => content,
        Some(Frame::Item { inlines, .. }) => inlines,
        Some(Frame::Image { alt, .. }) => {
            // Image alt text is plain text by definition.
            alt.push_str(&plain_text(&[inline]));
            return;
        }
        _ => return,
    };
    match inline {
        Inline::Text(t) => push_text(target, &t),
        other => target.push(other),
    }
}

fn parse_blocks(markdown: &str) -> Vec<Block> {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);

    let mut stack = vec![Frame::Root(Vec::new())];
    for event in Parser::new_ext(markdown, options) {
        match event {
            Event::Start(tag) => start(&mut stack, tag),
            Event::End(end) => finish(&mut stack, end),
            Event::Text(t) => match stack.last_mut() {
                Some(Frame::Code { text, .. }) => text.push_str(&t),
                _ => push_inline(&mut stack, Inline::Text(t.into_string())),
            },
            Event::Code(c) => push_inline(&mut stack, Inline::Code(c.into_string())),
            Event::SoftBreak => push_inline(&mut stack, Inline::SoftBreak),
            Event::HardBreak => push_inline(&mut stack, Inline::HardBreak),
            Event::Rule => push_block(&mut stack, Block::Rule),
            Event::TaskListMarker(done) => {
                if let Some(Frame::Item { checked, .. }) = stack.last_mut() {
                    *checked = Some(done);
                }
            }
            Event::FootnoteReference(label) => {
                push_inline(&mut stack, Inline::Text(format!("[{label}]")))
            }
            // Raw HTML never reaches a shareable document.
            _ => {}
        }
    }
    match stack.pop() {
        Some(Frame::Root(blocks)) => blocks,
        _ => Vec::new(),
    }
}

fn start(stack: &mut Vec<Frame>, tag: Tag<'_>) {
    let frame = match tag {
        Tag::Paragraph => Frame::Para(Vec::new()),
        Tag::Heading { level, .. } => Frame::Heading(level as u8, Vec::new()),
        Tag::BlockQuote(_) => Frame::Quote(Vec::new()),
        Tag::List(start) => Frame::List {
            ordered: start.is_some(),
            start: start.unwrap_or(1),
            items: Vec::new(),
        },
        Tag::Item => Frame::Item {
            checked: None,
            blocks: Vec::new(),
            inlines: Vec::new(),
        },
        Tag::CodeBlock(kind) => Frame::Code {
            lang: match kind {
                CodeBlockKind::Fenced(l) => l
                    .split_whitespace()
                    .next()
                    .map(str::to_owned)
                    .filter(|l| !l.is_empty()),
                CodeBlockKind::Indented => None,
            },
            text: String::new(),
        },
        Tag::Table(aligns) => Frame::Table {
            aligns: aligns
                .into_iter()
                .map(|a| match a {
                    Alignment::Right => Align::Right,
                    Alignment::Center => Align::Center,
                    _ => Align::Left,
                })
                .collect(),
            header: Vec::new(),
            rows: Vec::new(),
            row: Vec::new(),
        },
        // Header and body rows are folded into the Table frame.
        Tag::TableHead | Tag::TableRow => return,
        Tag::TableCell => Frame::Cell(Vec::new()),
        Tag::Emphasis => Frame::Span(Span::Emph, Vec::new()),
        Tag::Strong => Frame::Span(Span::Strong, Vec::new()),
        Tag::Strikethrough => Frame::Span(Span::Strike, Vec::new()),
        Tag::Link { dest_url, .. } => Frame::Link {
            url: dest_url.into_string(),
            content: Vec::new(),
        },
        Tag::Image { dest_url, .. } => Frame::Image {
            src: dest_url.into_string(),
            alt: String::new(),
        },
        _ => Frame::Ignore,
    };
    stack.push(frame);
}

fn finish(stack: &mut Vec<Frame>, end: TagEnd) {
    match end {
        TagEnd::TableHead => {
            if let Some(Frame::Table { header, row, .. }) = stack.last_mut() {
                *header = std::mem::take(row);
            }
            return;
        }
        TagEnd::TableRow => {
            if let Some(Frame::Table { rows, row, .. }) = stack.last_mut() {
                rows.push(std::mem::take(row));
            }
            return;
        }
        _ => {}
    }
    let Some(frame) = stack.pop() else { return };
    match frame {
        Frame::Para(content) => push_block(stack, Block::Paragraph(content)),
        Frame::Heading(level, content) => push_block(stack, Block::Heading { level, content }),
        Frame::Quote(blocks) => push_block(stack, into_quote_or_callout(blocks)),
        Frame::List {
            ordered,
            start,
            items,
        } => push_block(
            stack,
            Block::List {
                ordered,
                start,
                items,
            },
        ),
        Frame::Item {
            checked,
            mut blocks,
            mut inlines,
        } => {
            flush_item_inlines(&mut blocks, &mut inlines);
            if let Some(Frame::List { items, .. }) = stack.last_mut() {
                items.push(ListItem { checked, blocks });
            }
        }
        Frame::Code { lang, text } => push_block(
            stack,
            Block::Code {
                lang,
                text: text.trim_end_matches('\n').to_owned(),
            },
        ),
        Frame::Table {
            aligns,
            header,
            rows,
            ..
        } => push_block(
            stack,
            Block::Table(Table {
                aligns,
                header,
                rows,
            }),
        ),
        Frame::Cell(content) => {
            if let Some(Frame::Table { row, .. }) = stack.last_mut() {
                row.push(content);
            }
        }
        Frame::Span(kind, content) => push_inline(
            stack,
            match kind {
                Span::Emph => Inline::Emph(content),
                Span::Strong => Inline::Strong(content),
                Span::Strike => Inline::Strike(content),
            },
        ),
        Frame::Link { url, content } => push_inline(stack, Inline::Link { url, content }),
        Frame::Image { src, alt } => push_inline(stack, Inline::Image { alt, src }),
        Frame::Root(_) | Frame::Ignore => {}
    }
}

/// A quote whose first line is `[!kind] Title` is a callout.
fn into_quote_or_callout(mut blocks: Vec<Block>) -> Block {
    let Some(Block::Paragraph(first)) = blocks.first() else {
        return Block::Quote(blocks);
    };
    let Some(Inline::Text(lead)) = first.first() else {
        return Block::Quote(blocks);
    };
    // Owned, so the borrow of `blocks` ends before the paragraph is taken out.
    let lead = lead.clone();
    let Some(rest) = lead.strip_prefix("[!") else {
        return Block::Quote(blocks);
    };
    let Some(close) = rest.find(']') else {
        return Block::Quote(blocks);
    };
    let kind = rest[..close].trim().to_lowercase();
    if kind.is_empty() || !kind.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Block::Quote(blocks);
    }
    // `[!tip]+` and `[!tip]-` mark foldable callouts in Obsidian; folding means
    // nothing on paper.
    let after = rest[close + 1..].trim_start_matches(['+', '-']);

    let Block::Paragraph(inlines) = blocks.remove(0) else {
        unreachable!("checked above")
    };
    let split = inlines
        .iter()
        .position(|i| matches!(i, Inline::SoftBreak | Inline::HardBreak));
    let (title_part, body_part) = match split {
        Some(i) => (&inlines[..i], &inlines[i + 1..]),
        None => (&inlines[..], &inlines[inlines.len()..]),
    };
    let mut title_inlines: Vec<Inline> = Vec::new();
    push_text(&mut title_inlines, after);
    title_inlines.extend(title_part.iter().skip(1).cloned());
    let title = plain_text(&title_inlines).trim().to_owned();

    let mut body = Vec::new();
    if !body_part.is_empty() {
        body.push(Block::Paragraph(body_part.to_vec()));
    }
    body.extend(blocks);
    Block::Callout {
        kind,
        title: (!title.is_empty()).then_some(title),
        body,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(md: &str) -> Vec<Block> {
        Document::from_markdown("t", md).blocks
    }

    #[test]
    fn headings_paragraphs_and_inline_styles() {
        let blocks = parse("## Hi\n\nSome **bold** and *it* and `code` and ~~gone~~.\n");
        assert!(matches!(&blocks[0], Block::Heading { level: 2, .. }));
        let Block::Paragraph(p) = &blocks[1] else {
            panic!("expected paragraph")
        };
        assert!(p.iter().any(|i| matches!(i, Inline::Strong(_))));
        assert!(p.iter().any(|i| matches!(i, Inline::Emph(_))));
        assert!(p.contains(&Inline::Code("code".into())));
        assert!(p.iter().any(|i| matches!(i, Inline::Strike(_))));
    }

    #[test]
    fn tight_list_items_become_paragraphs() {
        let blocks = parse("- one\n- two\n  - nested\n");
        let Block::List { ordered, items, .. } = &blocks[0] else {
            panic!("expected list")
        };
        assert!(!ordered);
        assert_eq!(items.len(), 2);
        assert!(matches!(items[0].blocks[0], Block::Paragraph(_)));
        assert!(matches!(items[1].blocks[1], Block::List { .. }));
    }

    #[test]
    fn ordered_list_keeps_its_start_number() {
        let blocks = parse("3. c\n4. d\n");
        assert!(matches!(
            &blocks[0],
            Block::List {
                ordered: true,
                start: 3,
                ..
            }
        ));
    }

    #[test]
    fn task_items_carry_their_state() {
        let blocks = parse("- [x] done\n- [ ] todo\n");
        let Block::List { items, .. } = &blocks[0] else {
            panic!()
        };
        assert_eq!(items[0].checked, Some(true));
        assert_eq!(items[1].checked, Some(false));
    }

    #[test]
    fn tables_keep_alignment_header_and_rows() {
        let blocks = parse("| a | b |\n|:--|--:|\n| 1 | 2 |\n| 3 | 4 |\n");
        let Block::Table(t) = &blocks[0] else {
            panic!("expected table")
        };
        assert_eq!(t.aligns, vec![Align::Left, Align::Right]);
        assert_eq!(plain_text(&t.header[0]), "a");
        assert_eq!(t.rows.len(), 2);
        assert_eq!(plain_text(&t.rows[1][1]), "4");
        assert_eq!(t.columns(), 2);
    }

    #[test]
    fn callouts_are_recognised_with_title_and_body() {
        let blocks = parse("> [!tip] Remember\n> Bring the forms.\n");
        let Block::Callout { kind, title, body } = &blocks[0] else {
            panic!("expected callout, got {:?}", blocks[0])
        };
        assert_eq!(kind, "tip");
        assert_eq!(title.as_deref(), Some("Remember"));
        assert_eq!(body.len(), 1);
    }

    #[test]
    fn foldable_callout_without_title_is_still_a_callout() {
        let blocks = parse("> [!warning]-\n> Careful.\n");
        let Block::Callout { kind, title, body } = &blocks[0] else {
            panic!("expected callout")
        };
        assert_eq!(kind, "warning");
        assert!(title.is_none());
        assert_eq!(body.len(), 1);
    }

    #[test]
    fn ordinary_quotes_stay_quotes() {
        assert!(matches!(&parse("> just a quote\n")[0], Block::Quote(_)));
        assert!(matches!(&parse("> [not a callout]\n")[0], Block::Quote(_)));
    }

    #[test]
    fn code_blocks_keep_language_and_text() {
        let blocks = parse("```rust\nfn main() {}\n```\n");
        assert_eq!(
            blocks[0],
            Block::Code {
                lang: Some("rust".into()),
                text: "fn main() {}".into()
            }
        );
    }

    #[test]
    fn images_and_links_survive() {
        let blocks = parse("![alt text](pic.png) and [site](https://example.com)\n");
        let Block::Paragraph(p) = &blocks[0] else {
            panic!()
        };
        assert!(p.contains(&Inline::Image {
            alt: "alt text".into(),
            src: "pic.png".into()
        }));
        assert!(
            p.iter()
                .any(|i| matches!(i, Inline::Link { url, .. } if url == "https://example.com"))
        );
    }

    #[test]
    fn raw_html_is_dropped() {
        let blocks = parse("before <b>x</b> after\n\n<div>gone</div>\n");
        assert!(!format!("{blocks:?}").contains("div"));
    }

    #[test]
    fn tables_and_code_are_found_through_nesting() {
        let doc = Document::from_markdown(
            "t",
            "## Sites\n\n| a |\n|---|\n| 1 |\n\n> [!note]\n> ```py\n> x = 1\n> ```\n",
        );
        let tables = doc.tables();
        assert_eq!(tables.len(), 1);
        assert_eq!(tables[0].0.as_deref(), Some("Sites"));
        assert_eq!(doc.code_blocks()[0].0, Some("py"));
    }
}
