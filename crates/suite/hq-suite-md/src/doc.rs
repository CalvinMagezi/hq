//! The hqmd document: a note cut into top-level blocks, each keeping its own source text.
//!
//! The model is a segmentation of the file, not a re-serialisation of it. Every node holds the
//! exact text it was read from plus the blank space before it, so printing an unedited document
//! returns the input byte for byte, whatever the input was. That is the guarantee the rest of
//! the suite stands on: opening and saving a note changes nothing the user did not change.
//!
//! CommonMark decides where blocks begin and end (through `pulldown-cmark`, with tables,
//! footnotes, strikethrough and task lists on). Only fenced blocks whose header names one of the
//! suite's types get a richer view; everything else is carried as written.

use std::ops::Range;

use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag};

use crate::block::{self, BlockError, BlockType, TypedBlock};
use crate::frontmatter;
use crate::hash::{block_hash, revision_of};
use crate::meta::SuiteMeta;

/// Notes above this size are parsed but reported as an error by [`crate::validate`], and the
/// patch API refuses to grow one past it.
pub const MAX_DOC_BYTES: usize = 16 * 1024 * 1024;

/// What a top-level block is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeKind {
    Heading {
        level: u8,
    },
    Paragraph,
    List {
        ordered: bool,
    },
    Quote,
    /// An ordinary code block. `lang` is the first word of its header, if it has one.
    Code {
        lang: Option<String>,
    },
    /// A fenced block of one of the suite's types.
    Typed(BlockType),
    Table,
    Html,
    Rule,
    Footnote,
    /// Text the CommonMark reader does not report as a block, such as link reference
    /// definitions. Kept so nothing the user wrote can be lost.
    Verbatim,
}

/// One top-level block and the blank space before it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    pub(crate) lead: String,
    pub(crate) kind: NodeKind,
    /// The block's text, with no trailing whitespace.
    pub(crate) src: String,
}

impl Node {
    pub fn kind(&self) -> &NodeKind {
        &self.kind
    }

    pub fn src(&self) -> &str {
        &self.src
    }

    /// The whitespace that separated this node from the one before it.
    pub fn lead(&self) -> &str {
        &self.lead
    }

    pub fn hash(&self) -> String {
        block_hash(&self.src)
    }

    pub(crate) fn plain(kind: NodeKind, lead: String, src: String) -> Node {
        Node { lead, kind, src }
    }
}

/// A note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
    pub(crate) frontmatter: String,
    pub(crate) nodes: Vec<Node>,
    /// Whitespace after the last block (usually one newline).
    pub(crate) tail: String,
    pub(crate) eol: &'static str,
}

fn detect_eol(text: &str) -> &'static str {
    match text.find('\n') {
        Some(i) if i > 0 && text.as_bytes()[i - 1] == b'\r' => "\r\n",
        _ => "\n",
    }
}

fn classify(tag: &Tag<'_>, src: &str, at_line_start: bool) -> NodeKind {
    match tag {
        Tag::Heading { level, .. } => NodeKind::Heading {
            level: *level as u8,
        },
        Tag::Paragraph => NodeKind::Paragraph,
        Tag::List(start) => NodeKind::List {
            ordered: start.is_some(),
        },
        Tag::BlockQuote(_) => NodeKind::Quote,
        Tag::CodeBlock(CodeBlockKind::Fenced(info)) => {
            match at_line_start.then(|| block::detect(src)).flatten() {
                Some(ty) => NodeKind::Typed(ty),
                None => NodeKind::Code {
                    lang: info.split_whitespace().next().map(str::to_string),
                },
            }
        }
        Tag::CodeBlock(CodeBlockKind::Indented) => NodeKind::Code { lang: None },
        Tag::Table(_) => NodeKind::Table,
        Tag::HtmlBlock => NodeKind::Html,
        Tag::FootnoteDefinition(_) => NodeKind::Footnote,
        _ => NodeKind::Verbatim,
    }
}

impl Document {
    /// Reads any text. Never fails: text that is not understood is kept as `Verbatim`.
    pub fn parse(text: &str) -> Document {
        let split = frontmatter::split(text);
        let body = split.body;
        let eol = detect_eol(text);

        let options = Options::ENABLE_TABLES
            | Options::ENABLE_FOOTNOTES
            | Options::ENABLE_STRIKETHROUGH
            | Options::ENABLE_TASKLISTS;
        let mut spans: Vec<(Range<usize>, NodeKind)> = Vec::new();
        let mut depth = 0usize;
        let mut open: Option<(Range<usize>, NodeKind)> = None;
        for (event, range) in Parser::new_ext(body, options).into_offset_iter() {
            match event {
                Event::Start(tag) => {
                    if depth == 0 {
                        let at_line_start =
                            range.start == 0 || body.as_bytes()[range.start - 1] == b'\n';
                        let kind = classify(&tag, &body[range.clone()], at_line_start);
                        open = Some((range, kind));
                    }
                    depth += 1;
                }
                Event::End(_) => {
                    depth = depth.saturating_sub(1);
                    if depth == 0
                        && let Some(done) = open.take()
                    {
                        spans.push(done);
                    }
                }
                Event::Rule if depth == 0 => spans.push((range, NodeKind::Rule)),
                _ => {}
            }
        }

        let mut nodes: Vec<Node> = Vec::with_capacity(spans.len());
        let mut cursor = 0usize;
        for (range, kind) in spans {
            // Ranges are monotonic for top-level blocks; guard anyway so a surprising event
            // stream can only lose structure, never text.
            if range.start < cursor {
                continue;
            }
            take_gap(body, &mut cursor, range.start, &mut nodes);
            let raw = &body[range.start..range.end];
            let src = raw.trim_end();
            nodes.push(Node::plain(
                kind,
                body[cursor..range.start].to_string(),
                src.to_string(),
            ));
            cursor = range.start + src.len();
        }
        let rest = &body[cursor..];
        let core = rest.trim();
        let tail = if core.is_empty() {
            rest.to_string()
        } else {
            // Text after the last block that the reader did not report.
            let lead_len = rest.len() - rest.trim_start().len();
            let end = lead_len + core.len();
            nodes.push(Node::plain(
                NodeKind::Verbatim,
                rest[..lead_len].to_string(),
                core.to_string(),
            ));
            rest[end..].to_string()
        };
        Document {
            frontmatter: split.raw.to_string(),
            nodes,
            tail,
            eol,
        }
    }

    /// The text of the document. For a document that was only parsed, exactly its input.
    pub fn render(&self) -> String {
        let mut out = String::with_capacity(self.len_hint());
        out.push_str(&self.frontmatter);
        for n in &self.nodes {
            out.push_str(&n.lead);
            out.push_str(&n.src);
        }
        out.push_str(&self.tail);
        out
    }

    fn len_hint(&self) -> usize {
        self.frontmatter.len()
            + self.tail.len()
            + self
                .nodes
                .iter()
                .map(|n| n.lead.len() + n.src.len())
                .sum::<usize>()
    }

    /// The raw frontmatter block, fences included, or an empty string.
    pub fn frontmatter(&self) -> &str {
        &self.frontmatter
    }

    pub fn nodes(&self) -> &[Node] {
        &self.nodes
    }

    pub fn node(&self, index: usize) -> Option<&Node> {
        self.nodes.get(index)
    }

    /// The line ending the document uses: `"\r\n"` if its first line break is one, else `"\n"`.
    pub fn eol(&self) -> &'static str {
        self.eol
    }

    /// An id for this exact text. A change anywhere gives a different one.
    pub fn revision(&self) -> String {
        revision_of(&self.render())
    }

    /// What the suite's frontmatter keys say about the note, and anything wrong with them.
    pub fn meta(&self) -> (SuiteMeta, Vec<crate::meta::MetaProblem>) {
        SuiteMeta::read(&self.frontmatter)
    }

    /// The typed view of node `index`, if it is a typed block.
    pub fn typed(&self, index: usize) -> Option<Result<TypedBlock, BlockError>> {
        let n = self.nodes.get(index)?;
        matches!(n.kind, NodeKind::Typed(_)).then(|| TypedBlock::parse(&n.src))
    }

    /// Every typed block, with its node index.
    pub fn typed_blocks(
        &self,
    ) -> impl Iterator<Item = (usize, Result<TypedBlock, BlockError>)> + '_ {
        self.nodes
            .iter()
            .enumerate()
            .filter(|(_, n)| matches!(n.kind, NodeKind::Typed(_)))
            .map(|(i, n)| (i, TypedBlock::parse(&n.src)))
    }

    /// Finds a typed block by its `id=` (or, failing that, `name=`).
    pub fn find_block(&self, wanted: &str) -> Option<(usize, TypedBlock)> {
        let mut by_name = None;
        for (i, parsed) in self.typed_blocks() {
            let Ok(b) = parsed else { continue };
            if b.info.id() == Some(wanted) {
                return Some((i, b));
            }
            if by_name.is_none() && b.info.get("name") == Some(wanted) {
                by_name = Some((i, b));
            }
        }
        by_name
    }

    /// `(index, hash)` for a node, the address an edit must quote.
    pub fn addr(&self, index: usize) -> Option<crate::patch::Addr> {
        self.nodes.get(index).map(|n| crate::patch::Addr {
            index,
            hash: n.hash(),
        })
    }
}

/// Moves non-blank text between blocks that the reader skipped into its own `Verbatim` node.
fn take_gap(body: &str, cursor: &mut usize, upto: usize, nodes: &mut Vec<Node>) {
    let gap = &body[*cursor..upto];
    let core = gap.trim();
    if core.is_empty() {
        return;
    }
    let lead_len = gap.len() - gap.trim_start().len();
    nodes.push(Node::plain(
        NodeKind::Verbatim,
        gap[..lead_len].to_string(),
        core.to_string(),
    ));
    *cursor += lead_len + core.len();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(d: &Document) -> Vec<NodeKind> {
        d.nodes().iter().map(|n| n.kind().clone()).collect()
    }

    #[test]
    fn splits_a_note_into_blocks() {
        let d = Document::parse(
            "---\ntitle: T\n---\n# Title\n\nSome *text*.\n\n- a\n- b\n\n```rust\nfn x() {}\n```\n\n---\n\n> quote\n",
        );
        assert_eq!(
            kinds(&d),
            [
                NodeKind::Heading { level: 1 },
                NodeKind::Paragraph,
                NodeKind::List { ordered: false },
                NodeKind::Code {
                    lang: Some("rust".into())
                },
                NodeKind::Rule,
                NodeKind::Quote,
            ]
        );
        assert_eq!(d.frontmatter(), "---\ntitle: T\n---\n");
    }

    #[test]
    fn printing_returns_the_input_exactly() {
        let samples = [
            "",
            "\n",
            "# x",
            "# x\n",
            "\n\n# x\n\n\n\npara   \nmore\n\n\n",
            "---\na: 1\n---\n\nbody",
            "---\na: 1\n---\n",
            "[ref]: https://example.com\n\ntext [ref]\n",
            "text\n\n[ref]: https://example.com\n",
            "| a | b |\n|---|---|\n| 1 | 2 |\n",
            "para one\r\n\r\npara two\r\n",
            "```sheet\n| a |\n|---|\n| 1 |\n```\n",
            "```sheet\n| a |\n|---|\n| 1 |\n",
            "    indented code\n\n    more\n",
            "<!-- layout: two-column -->\n\n## Card\n",
            "1. one\n2. two\n   - nested\n\ntrailing   \n",
            "text with trailing spaces   \n\n\n",
            "\t\n\n   \nword\n",
            "---\nnot closed\n\ntext\n",
            "[^1]: note\n\ntext[^1]\n",
        ];
        for s in samples {
            assert_eq!(Document::parse(s).render(), s, "{s:?}");
        }
    }

    #[test]
    fn typed_blocks_are_found_only_at_the_top_level() {
        let src = "```sheet name=A\n| a |\n|---|\n| 1 |\n```\n\n- item\n\n  ```sheet name=B\n  | a |\n  ```\n\n> ```tile from=A1 fn=sum\n> ```\n\n ```sheet name=C\n | a |\n ```\n";
        let d = Document::parse(src);
        let typed: Vec<_> = d.typed_blocks().collect();
        assert_eq!(typed.len(), 1, "{:?}", kinds(&d));
        assert_eq!(typed[0].1.as_ref().unwrap().info.get("name"), Some("A"));
        assert_eq!(d.render(), src);
    }

    #[test]
    fn other_fenced_blocks_are_plain_code() {
        let src = "```sheet2 name=x\ndata\n```\n\n```fields\nname: text\n```\n";
        let d = Document::parse(src);
        assert!(d.typed_blocks().next().is_none());
        assert!(matches!(d.nodes()[0].kind(), NodeKind::Code { lang: Some(l) } if l == "sheet2"));
        assert_eq!(d.render(), src);
    }

    #[test]
    fn a_typed_block_with_a_damaged_header_is_kept_and_reported() {
        let src = "```sheet name=\"open\n| a |\n```\n";
        let d = Document::parse(src);
        assert!(matches!(
            d.nodes()[0].kind(),
            NodeKind::Typed(BlockType::Sheet)
        ));
        assert!(d.typed(0).unwrap().is_err());
        assert_eq!(d.render(), src);
    }

    #[test]
    fn link_definitions_are_not_lost() {
        let d = Document::parse("see [x]\n\n[x]: https://example.com \"T\"\n\nafter\n");
        assert!(d.nodes().iter().any(|n| n.src().starts_with("[x]:")));
        assert_eq!(
            d.render(),
            "see [x]\n\n[x]: https://example.com \"T\"\n\nafter\n"
        );
    }

    #[test]
    fn crlf_is_detected_and_kept() {
        let src = "# a\r\n\r\ntext\r\n";
        let d = Document::parse(src);
        assert_eq!(d.eol(), "\r\n");
        assert_eq!(d.render(), src);
        assert_eq!(Document::parse("a\nb").eol(), "\n");
    }

    #[test]
    fn find_block_prefers_id_over_name() {
        let d = Document::parse(
            "```sheet name=stock\n| a |\n|---|\n```\n\n```sheet id=stock\n| a |\n|---|\n```\n",
        );
        let (i, _) = d.find_block("stock").unwrap();
        assert_eq!(i, 1);
        assert!(d.find_block("nope").is_none());
    }

    #[test]
    fn revision_and_hashes_change_with_content() {
        let a = Document::parse("one\n\ntwo\n");
        let b = Document::parse("one\n\ntwo!\n");
        assert_ne!(a.revision(), b.revision());
        assert_eq!(a.node(0).unwrap().hash(), b.node(0).unwrap().hash());
        assert_ne!(a.node(1).unwrap().hash(), b.node(1).unwrap().hash());
        assert_eq!(a.addr(1).unwrap().index, 1);
        assert!(a.addr(2).is_none());
    }

    #[test]
    fn hostile_nesting_does_not_hang_or_lose_text() {
        let deep = format!("{}x\n", "> ".repeat(2000));
        assert_eq!(Document::parse(&deep).render(), deep);
        let lists = format!("{}x\n", "- ".repeat(2000));
        assert_eq!(Document::parse(&lists).render(), lists);
    }
}
