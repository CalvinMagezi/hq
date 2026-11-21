//! Word comments as hqmd `comment` blocks.
//!
//! The importer (`hq-convert`) finds the comments and the block each sits on, but does not know
//! the note format. This codec is what it calls to write them: the block hash is the note's own,
//! and the block text is what `hq_suite_md` renders, so the two stay in step.

use hq_convert::ingest::{CommentCodec, WordComments};
use hq_suite_md::{BlockType, Document, TypedBlock};

/// Writes Word comments the way hqmd holds them.
#[derive(Debug, Clone, Copy, Default)]
pub struct HqmdComments;

impl CommentCodec for HqmdComments {
    fn last_block_hash(&self, markdown_part: &str) -> Option<String> {
        Document::parse(markdown_part)
            .nodes()
            .last()
            .map(|n| n.hash())
    }

    fn render_comment(
        &self,
        on: &str,
        by: &str,
        at: Option<&str>,
        resolved: bool,
        text: &str,
    ) -> String {
        let mut attrs: Vec<(&str, &str)> = vec![("on", on)];
        if !by.is_empty() {
            attrs.push(("by", by));
        }
        if let Some(at) = at {
            attrs.push(("at", at));
        }
        attrs.push(("state", if resolved { "resolved" } else { "open" }));
        TypedBlock::new(BlockType::Comment, &attrs, text).render("\n")
    }

    fn orphans(&self, markdown: &str) -> usize {
        hq_suite_md::comments(&Document::parse(markdown))
            .iter()
            .filter(|c| c.orphaned)
            .count()
    }
}

/// The value for `ConvertOptions::word_comments` that carries Word comments as `comment` blocks.
pub fn word_comment_codec() -> WordComments {
    WordComments::new(HqmdComments)
}
