//! A slides note as cards.
//!
//! A slides note is ordinary markdown (`docs/suite/HQMD.md`, "Docs and slides"): the level 1
//! heading is the deck's title and each level 2 heading starts a card. A line `<!-- layout: NAME -->`
//! under a card's heading picks its layout. Text between the title and the first card is the title
//! card's subtitle. A horizontal rule splits a two-column card.

use hq_export::doc::{Block, Document, Inline, ListItem, plain_text};

/// Most cards one deck may hold, so a hostile note cannot ask for unbounded memory.
pub const MAX_CARDS: usize = 500;
/// Deepest list nesting a card shows.
const MAX_LEVEL: u8 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Layout {
    #[default]
    Content,
    TwoColumn,
}

impl Layout {
    /// The layout a name picks, or `None` for a name the suite does not have.
    pub fn named(name: &str) -> Option<Layout> {
        match name.trim().to_ascii_lowercase().as_str() {
            "content" | "title" => Some(Layout::Content),
            "two-column" | "two-columns" => Some(Layout::TwoColumn),
            _ => None,
        }
    }
}

/// A run of text with its emphasis.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Run {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub code: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    Bullet {
        level: u8,
        runs: Vec<Run>,
    },
    Number {
        level: u8,
        runs: Vec<Run>,
    },
    Paragraph(Vec<Run>),
    Code(String),
    Table(Vec<Vec<String>>),
    /// A linked picture, by the path the note wrote. Placed only if the caller can supply it.
    Picture {
        src: String,
        alt: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Card {
    pub title: String,
    pub layout: Layout,
    /// One column, or two when the layout is `two-column` (split at the first rule).
    pub columns: Vec<Vec<Item>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Deck {
    pub title: String,
    /// The title card's subtitle, if the note has text between its title and its first card.
    pub subtitle: Vec<Vec<Run>>,
    pub cards: Vec<Card>,
    /// Layout names a card asked for that the suite does not have (the card uses `content`).
    pub unknown_layouts: Vec<String>,
}

fn runs(
    inlines: &[Inline],
    out: &mut Vec<Run>,
    bold: bool,
    italic: bool,
    pictures: &mut Vec<(String, String)>,
) {
    for i in inlines {
        match i {
            Inline::Text(t) => out.push(Run {
                text: t.clone(),
                bold,
                italic,
                code: false,
            }),
            Inline::Code(t) => out.push(Run {
                text: t.clone(),
                bold,
                italic,
                code: true,
            }),
            Inline::Strong(c) => runs(c, out, true, italic, pictures),
            Inline::Emph(c) => runs(c, out, bold, true, pictures),
            Inline::Strike(c) => runs(c, out, bold, italic, pictures),
            Inline::Link { content, .. } => runs(content, out, bold, italic, pictures),
            Inline::Image { src, alt } => pictures.push((src.clone(), alt.clone())),
            Inline::SoftBreak => out.push(Run {
                text: " ".into(),
                bold,
                italic,
                code: false,
            }),
            Inline::HardBreak => out.push(Run {
                text: "\n".into(),
                bold,
                italic,
                code: false,
            }),
        }
    }
}

/// Puts the pictures found in a piece of text after it, as items of their own.
fn flush(pictures: &mut Vec<(String, String)>, out: &mut Vec<Item>) {
    for (src, alt) in pictures.drain(..) {
        out.push(Item::Picture { src, alt });
    }
}

fn run_list(inlines: &[Inline], pictures: &mut Vec<(String, String)>) -> Vec<Run> {
    let mut out = Vec::new();
    runs(inlines, &mut out, false, false, pictures);
    out
}

fn list_items(
    ordered: bool,
    items: &[ListItem],
    level: u8,
    out: &mut Vec<Item>,
    pictures: &mut Vec<(String, String)>,
) {
    for item in items {
        for block in &item.blocks {
            match block {
                Block::Paragraph(inlines) => {
                    let r = run_list(inlines, pictures);
                    out.push(if ordered {
                        Item::Number { level, runs: r }
                    } else {
                        Item::Bullet { level, runs: r }
                    });
                    flush(pictures, out);
                }
                Block::List {
                    ordered: inner,
                    items,
                    ..
                } => list_items(*inner, items, (level + 1).min(MAX_LEVEL), out, pictures),
                other => blocks_to_items(std::slice::from_ref(other), out, pictures),
            }
        }
    }
}

fn blocks_to_items(blocks: &[Block], out: &mut Vec<Item>, pictures: &mut Vec<(String, String)>) {
    for block in blocks {
        match block {
            Block::Paragraph(inlines) => {
                let r = run_list(inlines, pictures);
                if !r.is_empty() {
                    out.push(Item::Paragraph(r));
                }
                flush(pictures, out);
            }
            Block::Heading { content, .. } => {
                let r = run_list(content, pictures);
                if !r.is_empty() {
                    out.push(Item::Paragraph(
                        r.into_iter().map(|run| Run { bold: true, ..run }).collect(),
                    ));
                }
            }
            Block::List { ordered, items, .. } => list_items(*ordered, items, 0, out, pictures),
            Block::Quote(inner) => blocks_to_items(inner, out, pictures),
            Block::Callout { body, .. } => blocks_to_items(body, out, pictures),
            Block::Code { text, .. } => out.push(Item::Code(text.clone())),
            Block::Table(t) => {
                let mut rows = Vec::new();
                rows.push(t.header.iter().map(|c| plain_text(c)).collect());
                for r in &t.rows {
                    rows.push(r.iter().map(|c| plain_text(c)).collect());
                }
                out.push(Item::Table(rows));
            }
            Block::Rule => {}
        }
    }
}

/// The layout comment under a card's heading, and the markdown without it.
fn take_layout(lines: &[&str]) -> (Option<String>, String) {
    let mut name = None;
    let mut kept = Vec::new();
    let mut in_fence = false;
    for line in lines {
        let t = line.trim();
        if t.starts_with("```") || t.starts_with("~~~") {
            in_fence = !in_fence;
        }
        let comment = (!in_fence)
            .then(|| t.strip_prefix("<!--").and_then(|r| r.strip_suffix("-->")))
            .flatten()
            .map(str::trim)
            .and_then(|c| c.strip_prefix("layout:"))
            .map(|n| n.trim().to_string());
        match comment {
            Some(n) if name.is_none() => name = Some(n),
            _ => kept.push(*line),
        }
    }
    (name, kept.join("\n"))
}

/// Splits markdown into the title section and one section per level 2 heading, outside code fences.
fn sections(markdown: &str) -> (Vec<&str>, Vec<(String, Vec<&str>)>) {
    let mut head: Vec<&str> = Vec::new();
    let mut cards: Vec<(String, Vec<&str>)> = Vec::new();
    let mut in_fence = false;
    for line in markdown.lines() {
        let t = line.trim_start();
        if t.starts_with("```") || t.starts_with("~~~") {
            in_fence = !in_fence;
        }
        if !in_fence && let Some(title) = line.strip_prefix("## ") {
            cards.push((title.trim().to_string(), Vec::new()));
            continue;
        }
        match cards.last_mut() {
            Some((_, body)) => body.push(line),
            None => head.push(line),
        }
    }
    (head, cards)
}

impl Deck {
    /// The deck a slides note's markdown (without its frontmatter) describes. `fallback_title`
    /// names a deck that has no level 1 heading.
    pub fn from_markdown(fallback_title: &str, markdown: &str) -> Deck {
        let (head, card_sections) = sections(markdown);
        let mut deck = Deck {
            title: fallback_title.to_string(),
            ..Deck::default()
        };
        let mut pictures: Vec<(String, String)> = Vec::new();
        let head_md = head.join("\n");
        let head_doc = Document::from_markdown(fallback_title, &head_md);
        let mut subtitle_blocks = Vec::new();
        let mut titled = false;
        for block in &head_doc.blocks {
            match block {
                Block::Heading { level: 1, content } if !titled => {
                    deck.title = plain_text(content);
                    titled = true;
                }
                other => subtitle_blocks.push(other.clone()),
            }
        }
        let mut sub_items = Vec::new();
        blocks_to_items(&subtitle_blocks, &mut sub_items, &mut pictures);
        deck.subtitle = sub_items
            .into_iter()
            .filter_map(|i| match i {
                Item::Paragraph(r)
                | Item::Bullet { runs: r, .. }
                | Item::Number { runs: r, .. } => Some(r),
                _ => None,
            })
            .collect();
        for (title, body) in card_sections.into_iter().take(MAX_CARDS) {
            let (layout_name, md) = take_layout(&body);
            let layout = match layout_name.as_deref() {
                None => Layout::Content,
                Some(n) => Layout::named(n).unwrap_or_else(|| {
                    deck.unknown_layouts.push(n.to_string());
                    Layout::Content
                }),
            };
            let doc = Document::from_markdown(&title, &md);
            let mut columns: Vec<Vec<Item>> = vec![Vec::new()];
            for block in &doc.blocks {
                if matches!(block, Block::Rule) && layout == Layout::TwoColumn && columns.len() == 1
                {
                    columns.push(Vec::new());
                    continue;
                }
                let last = columns.last_mut().expect("one column at least");
                blocks_to_items(std::slice::from_ref(block), last, &mut pictures);
            }
            if layout == Layout::TwoColumn && columns.len() == 1 {
                columns.push(Vec::new());
            }
            deck.cards.push(Card {
                title,
                layout,
                columns,
            });
        }
        deck
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOTE: &str = "# Q4 review\n\nPrepared for the board.\n\n## Results\n\n- Revenue **up** 12%\n  - Uganda *strongest*\n- Costs flat\n\n## Plan\n<!-- layout: two-column -->\n\nFirst half\n\n---\n\n1. Hire\n2. Train\n\n## Numbers\n\n| item | qty |\n|---|---|\n| Sugar | 40 |\n\n```\ncode here\n```\n\n## Odd\n<!-- layout: carousel -->\n\nText ![pic](x.png)\n";

    #[test]
    fn a_note_becomes_a_title_card_and_one_card_per_heading() {
        let d = Deck::from_markdown("fallback", NOTE);
        assert_eq!(d.title, "Q4 review");
        assert_eq!(d.subtitle.len(), 1);
        assert_eq!(d.subtitle[0][0].text, "Prepared for the board.");
        let titles: Vec<&str> = d.cards.iter().map(|c| c.title.as_str()).collect();
        assert_eq!(titles, ["Results", "Plan", "Numbers", "Odd"]);
    }

    #[test]
    fn bullets_nest_and_keep_their_emphasis() {
        let d = Deck::from_markdown("x", NOTE);
        let items = &d.cards[0].columns[0];
        assert!(
            matches!(&items[0], Item::Bullet { level: 0, runs } if runs.iter().any(|r| r.bold && r.text == "up"))
        );
        assert!(
            matches!(&items[1], Item::Bullet { level: 1, runs } if runs.iter().any(|r| r.italic))
        );
        assert!(matches!(&items[2], Item::Bullet { level: 0, .. }));
    }

    #[test]
    fn a_two_column_card_splits_at_the_rule_and_an_unknown_layout_is_reported() {
        let d = Deck::from_markdown("x", NOTE);
        let plan = &d.cards[1];
        assert_eq!(plan.layout, Layout::TwoColumn);
        assert_eq!(plan.columns.len(), 2);
        assert!(matches!(&plan.columns[0][0], Item::Paragraph(_)));
        assert!(matches!(&plan.columns[1][0], Item::Number { .. }));
        assert_eq!(d.unknown_layouts, ["carousel"]);
        assert_eq!(d.cards[3].layout, Layout::Content);
        assert!(matches!(&d.cards[3].columns[0][1], Item::Picture { src, .. } if src == "x.png"));
    }

    #[test]
    fn tables_and_code_are_kept_and_headings_inside_a_fence_do_not_split() {
        let d = Deck::from_markdown(
            "x",
            "# T\n\n## A\n\n```\n## not a card\n```\n\n| a | b |\n|---|---|\n| 1 | 2 |\n",
        );
        assert_eq!(d.cards.len(), 1);
        let items = &d.cards[0].columns[0];
        assert!(matches!(&items[0], Item::Code(c) if c.contains("## not a card")));
        assert!(matches!(&items[1], Item::Table(rows) if rows.len() == 2 && rows[1] == ["1", "2"]));
    }

    #[test]
    fn a_note_with_no_title_uses_the_fallback_and_hostile_size_is_capped() {
        let many: String = (0..MAX_CARDS + 50)
            .map(|i| format!("## c{i}\n\ntext\n\n"))
            .collect();
        let d = Deck::from_markdown("Fallback", &many);
        assert_eq!(d.title, "Fallback");
        assert_eq!(d.cards.len(), MAX_CARDS);
    }
}
