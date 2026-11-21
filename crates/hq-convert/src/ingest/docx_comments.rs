//! Word comments (`word/comments.xml`, with `word/commentsExtended.xml` for resolved and replies).
//!
//! Only read when `ConvertOptions::word_comments` is set. Each comment becomes a
//! [`Block::Comment`] right after the body block it sits on; [`ir::render_checked`] hands it to
//! the caller's [`ir::CommentCodec`], which writes a `comment` block anchored by that block's
//! hash. A comment with no block to sit on (its range
//! is in a header, a footer or a note, or it is never referenced) is not carried, and the report
//! counts it as a loss.

use std::collections::HashMap;

use roxmltree::Node;

use super::ir::{Block, DocComment};
use super::package::{attr, is, parse_xml};
use super::report::{LossKind, Losses};

const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";

/// Longest comment text and author name carried, as the suite's comment tools allow.
const MAX_TEXT_CHARS: usize = 4000;
const MAX_BY_CHARS: usize = 60;

#[derive(Debug, Clone)]
pub struct WordComment {
    pub id: String,
    pub comment: DocComment,
    /// Id of the comment this one answers, from `commentsExtended.xml`.
    parent: Option<String>,
}

fn local<'a>(n: Node<'a, '_>, name: &str) -> Option<&'a str> {
    n.attributes().find(|a| a.name() == name).map(|a| a.value())
}

/// The author as a label: no control characters, quotes or backticks, at most 60 characters.
pub fn clean_author(raw: &str) -> String {
    raw.chars()
        .filter(|c| !c.is_control() && !matches!(c, '"' | '`'))
        .take(MAX_BY_CHARS)
        .collect::<String>()
        .trim()
        .to_string()
}

/// `YYYY-MM-DD` of a Word timestamp such as `2026-11-20T10:15:00Z`; `None` when it is not a date.
fn iso_date(raw: &str) -> Option<String> {
    let day = raw.get(..10)?;
    chrono::NaiveDate::parse_from_str(day, "%Y-%m-%d")
        .ok()
        .map(|_| day.to_string())
}

fn paragraph_text(p: Node<'_, '_>) -> String {
    let mut out = String::new();
    for d in p.descendants().filter(|d| d.is_element()) {
        match (d.tag_name().namespace(), d.tag_name().name()) {
            (Some(W), "t") => out.push_str(d.text().unwrap_or("")),
            (Some(W), "tab") => out.push(' '),
            _ => {}
        }
    }
    out
}

pub fn parse(comments_xml: &str, extended_xml: Option<&str>, max: u32) -> Vec<WordComment> {
    let Ok(doc) = parse_xml(comments_xml, false, max) else {
        return Vec::new();
    };
    // paraId of the comment's paragraphs -> comment id, to follow `paraIdParent`.
    let mut para_to_id: HashMap<String, String> = HashMap::new();
    let mut raw: Vec<(String, DocComment, Vec<String>)> = Vec::new();
    for c in doc
        .root_element()
        .children()
        .filter(|n| is(*n, W, "comment"))
    {
        let Some(id) = attr(c, W, "id") else { continue };
        let paras: Vec<Node<'_, '_>> = c.children().filter(|n| is(*n, W, "p")).collect();
        let text = paras
            .iter()
            .map(|p| paragraph_text(*p))
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        let mut ids = Vec::new();
        for p in &paras {
            if let Some(pid) = local(*p, "paraId") {
                para_to_id.insert(pid.to_string(), id.to_string());
                ids.push(pid.to_string());
            }
        }
        let text: String = text.chars().take(MAX_TEXT_CHARS).collect();
        raw.push((
            id.to_string(),
            DocComment {
                by: clean_author(attr(c, W, "author").unwrap_or("")),
                at: attr(c, W, "date").and_then(iso_date),
                resolved: false,
                text,
            },
            ids,
        ));
    }

    // commentsExtended: done flag and parent, keyed by the paragraph id.
    let mut done: HashMap<String, bool> = HashMap::new();
    let mut parent_of: HashMap<String, String> = HashMap::new();
    if let Some(xml) = extended_xml
        && let Ok(ext) = parse_xml(xml, false, max)
    {
        for e in ext
            .root_element()
            .children()
            .filter(|n| n.is_element() && n.tag_name().name() == "commentEx")
        {
            let Some(pid) = local(e, "paraId") else {
                continue;
            };
            if matches!(local(e, "done"), Some("1" | "true")) {
                done.insert(pid.to_string(), true);
            }
            if let Some(parent) = local(e, "paraIdParent")
                && let Some(parent_id) = para_to_id.get(parent)
            {
                parent_of.insert(pid.to_string(), parent_id.clone());
            }
        }
    }

    raw.into_iter()
        .map(|(id, mut comment, para_ids)| {
            comment.resolved = para_ids.iter().any(|p| done.contains_key(p));
            let parent = para_ids.iter().find_map(|p| parent_of.get(p).cloned());
            WordComment {
                id,
                comment,
                parent,
            }
        })
        .collect()
}

/// The comment ids a body paragraph or table mentions, in document order, each once.
pub fn refs(n: Node<'_, '_>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for d in n.descendants().filter(|d| {
        d.is_element()
            && d.tag_name().namespace() == Some(W)
            && matches!(
                d.tag_name().name(),
                "commentReference" | "commentRangeStart" | "commentRangeEnd"
            )
    }) {
        if let Some(id) = attr(d, W, "id")
            && !out.iter().any(|x| x == id)
        {
            out.push(id.to_string());
        }
    }
    out
}

/// Puts each comment after the body block it sits on and counts what was carried and what could
/// not be placed. A reply with no range of its own goes where the comment it answers sits.
pub fn place(
    blocks: Vec<Block>,
    comments: &[WordComment],
    anchors: &[(usize, Vec<String>)],
    losses: &mut Losses,
) -> Vec<Block> {
    let mut at: HashMap<&str, usize> = HashMap::new();
    for (index, ids) in anchors {
        for id in ids {
            at.entry(id.as_str()).or_insert(*index);
        }
    }
    let by_id: HashMap<&str, &WordComment> = comments.iter().map(|c| (c.id.as_str(), c)).collect();
    let mut after: HashMap<usize, Vec<DocComment>> = HashMap::new();
    let (mut carried, mut lost) = (0usize, 0usize);
    for c in comments {
        if c.comment.text.trim().is_empty() {
            continue;
        }
        let mut cur = c;
        let mut found = at.get(cur.id.as_str()).copied();
        for _ in 0..8 {
            if found.is_some() {
                break;
            }
            let Some(parent) = cur.parent.as_deref().and_then(|p| by_id.get(p)) else {
                break;
            };
            cur = parent;
            found = at.get(cur.id.as_str()).copied();
        }
        match found.filter(|i| *i < blocks.len()) {
            Some(i) => {
                after.entry(i).or_default().push(c.comment.clone());
                carried += 1;
            }
            None => lost += 1,
        }
    }
    losses.hit_n(LossKind::CommentsCarried, carried);
    losses.hit_n(LossKind::CommentsUnanchored, lost);
    let mut out = Vec::with_capacity(blocks.len() + carried);
    for (i, b) in blocks.into_iter().enumerate() {
        out.push(b);
        if let Some(list) = after.remove(&i) {
            out.extend(list.into_iter().map(Block::Comment));
        }
    }
    out
}
