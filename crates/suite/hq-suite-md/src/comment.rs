//! Review comments: `comment` blocks anchored to another block by its hash.
//!
//! ````text
//! ```comment on=0123456789ab by="Sam" at=2026-11-20 state=open
//! Is this the final figure?
//! ```
//! ````
//!
//! `by` is free text a person typed, never an identity. A comment is a note to the people editing
//! the file, so no export carries one: [`strip_comments`] is what the writers call first.

use crate::block::{BlockType, TypedBlock};
use crate::doc::{Document, NodeKind};

/// Whether a comment is still waiting for an answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommentState {
    Open,
    Resolved,
}

/// A comment block read out of a note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Comment {
    /// Node index of the comment block itself.
    pub index: usize,
    /// Hash of the block it is anchored to.
    pub on: String,
    pub by: Option<String>,
    pub at: Option<String>,
    pub state: CommentState,
    pub text: String,
    /// No block has the hash `on` any more.
    pub orphaned: bool,
}

impl Comment {
    /// Reads a typed block as a comment. `None` when it is not one or has no usable `on`.
    pub fn from_block(index: usize, b: &TypedBlock) -> Option<Comment> {
        if b.ty != BlockType::Comment {
            return None;
        }
        let on = b
            .info
            .get("on")
            .filter(|o| crate::block::is_block_hash(o))?;
        Some(Comment {
            index,
            on: on.to_string(),
            by: b.info.get("by").map(str::to_string),
            at: b.info.get("at").map(str::to_string),
            state: if b.info.get("state") == Some("resolved") {
                CommentState::Resolved
            } else {
                CommentState::Open
            },
            text: b.body.clone(),
            orphaned: false,
        })
    }
}

fn is_comment(k: &NodeKind) -> bool {
    *k == NodeKind::Typed(BlockType::Comment)
}

/// Every comment in the note, in order, each marked orphaned when nothing carries its anchor.
pub fn comments(doc: &Document) -> Vec<Comment> {
    let mut out: Vec<Comment> = doc
        .typed_blocks()
        .filter_map(|(i, b)| Comment::from_block(i, &b.ok()?))
        .collect();
    for c in &mut out {
        c.orphaned = !doc
            .nodes()
            .iter()
            .enumerate()
            .any(|(j, n)| j != c.index && !is_comment(n.kind()) && n.hash() == c.on);
    }
    out
}

/// The text without its `comment` blocks. A text with none is returned as it was, byte for byte.
pub fn strip_comments(text: &str) -> String {
    let doc = Document::parse(text);
    if !doc.nodes().iter().any(|n| is_comment(n.kind())) {
        return text.to_string();
    }
    let mut out = String::from(doc.frontmatter());
    let mut first_lead: Option<&str> = None;
    let mut wrote_any = false;
    for n in doc.nodes() {
        first_lead.get_or_insert(n.lead());
        if is_comment(n.kind()) {
            continue;
        }
        out.push_str(if wrote_any {
            n.lead()
        } else {
            first_lead.unwrap_or("")
        });
        out.push_str(n.src());
        wrote_any = true;
    }
    out.push_str(&doc.tail);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::block_hash;

    #[test]
    fn comments_are_listed_and_orphans_found() {
        let para = "A paragraph.";
        let src = format!(
            "# T\n\n{para}\n\n```comment on={} by=Sam at=2026-11-20\nCheck.\n```\n\n```comment on=000000000000 state=resolved\nOld.\n```\n",
            block_hash(para)
        );
        let list = comments(&Document::parse(&src));
        assert_eq!(list.len(), 2);
        assert!(!list[0].orphaned && list[0].state == CommentState::Open);
        assert_eq!(list[0].by.as_deref(), Some("Sam"));
        assert!(list[1].orphaned && list[1].state == CommentState::Resolved);
    }

    #[test]
    fn stripping_removes_only_the_comments() {
        let src = "# T\n\nOne.\n\n```comment on=0123456789ab\nhm\n```\n\nTwo.\n";
        assert_eq!(strip_comments(src), "# T\n\nOne.\n\nTwo.\n");
        let plain = "# T\r\n\r\nOne.\r\n";
        assert_eq!(strip_comments(plain), plain);
        let first = "```comment on=0123456789ab\nhm\n```\n\nBody.\n";
        assert_eq!(strip_comments(first), "Body.\n");
    }
}
