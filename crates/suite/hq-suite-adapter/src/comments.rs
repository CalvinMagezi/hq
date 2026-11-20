//! Review comments on a note: list them, add one to a block, resolve one. A comment is a `comment`
//! block (`docs/suite/HQMD.md`) anchored to another block by its hash, so every change goes
//! through the hqmd patch: a block that changed since the caller looked is refused, and nothing
//! else in the note moves. `by` is a label the caller types, never an identity.

use std::path::Path;

use anyhow::{Context, Result, bail};
use hq_suite_md::{
    Addr, BlockType, CommentState, Document, NodeKind, Op, Patch, TypedBlock, apply, comments,
};
use hq_suite_store::{SuitePaths, fsx};
use serde::Serialize;

/// Longest comment, in characters.
const MAX_TEXT_CHARS: usize = 4000;
/// Longest `by` label, in characters.
const MAX_BY_CHARS: usize = 60;
/// Most comments one listing holds.
const MAX_COMMENTS: usize = 2000;

#[derive(Debug, Clone, Serialize)]
pub struct CommentRow {
    /// Position of the comment block, and what to quote back with [`resolve`].
    pub index: usize,
    pub hash: String,
    /// The hash of the block the comment is about.
    pub on: String,
    pub by: Option<String>,
    pub at: Option<String>,
    /// `open` or `resolved`.
    pub state: String,
    pub text: String,
    /// The block it was about has been edited or removed since.
    pub orphaned: bool,
    /// The first line of the block it is about, so a list can say what it points at.
    pub anchor_preview: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CommentsView {
    pub revision: String,
    pub comments: Vec<CommentRow>,
}

fn read(vault: &Path, note: &str) -> Result<(std::path::PathBuf, Document)> {
    let file = SuitePaths::new(vault).resolve(note)?;
    let text = std::fs::read_to_string(&file).with_context(|| format!("{note} cannot be read"))?;
    Ok((file, Document::parse(&text)))
}

fn view_of(doc: &Document) -> CommentsView {
    let rows = comments(doc)
        .into_iter()
        .take(MAX_COMMENTS)
        .map(|c| {
            let anchor_preview = doc
                .nodes()
                .iter()
                .find(|n| n.hash() == c.on && n.kind() != &NodeKind::Typed(BlockType::Comment))
                .map(|n| {
                    n.src()
                        .lines()
                        .next()
                        .unwrap_or("")
                        .chars()
                        .take(80)
                        .collect()
                });
            CommentRow {
                index: c.index,
                hash: doc.nodes()[c.index].hash(),
                on: c.on,
                by: c.by,
                at: c.at,
                state: match c.state {
                    CommentState::Open => "open",
                    CommentState::Resolved => "resolved",
                }
                .into(),
                text: c.text,
                orphaned: c.orphaned,
                anchor_preview,
            }
        })
        .collect();
    CommentsView {
        revision: doc.revision(),
        comments: rows,
    }
}

fn is_comment(doc: &Document, i: usize) -> bool {
    doc.nodes()[i].kind() == &NodeKind::Typed(BlockType::Comment)
}

fn patch_doc(doc: &Document, op: Op) -> Result<Document> {
    let patch = Patch {
        base_revision: Some(doc.revision()),
        ops: vec![op],
    };
    apply(doc, &patch).map_err(|e| anyhow::anyhow!("that edit was refused: {e}"))
}

/// The comments of a note already read, as [`list`] returns them.
pub fn view(doc: &Document) -> CommentsView {
    view_of(doc)
}

/// The note's comments, in the order they are written.
pub fn list(vault: &Path, note: &str) -> Result<CommentsView> {
    Ok(view_of(&read(vault, note)?.1))
}

/// Adds a comment on the block whose hash is `on`. If no block has that hash any more (it was
/// edited since the caller read it) nothing is written. `at` is an ISO 8601 date the caller
/// supplies; `by` is a free label. The comment goes after the block and after the comments
/// already there, so they read in the order they were made.
pub fn add(
    vault: &Path,
    note: &str,
    on: &str,
    by: &str,
    at: &str,
    text: &str,
) -> Result<CommentsView> {
    let (file, doc) = read(vault, note)?;
    let patched = add_in(&doc, on, by, at, text)?;
    fsx::atomic_write(&file, patched.render().as_bytes())?;
    Ok(view_of(&patched))
}

/// [`add`] over a note already read: the note as it would be, nothing written.
pub fn add_in(doc: &Document, on: &str, by: &str, at: &str, text: &str) -> Result<Document> {
    let text = text.trim();
    if text.is_empty() {
        bail!("a comment cannot be empty");
    }
    if text.chars().count() > MAX_TEXT_CHARS {
        bail!("a comment holds at most {MAX_TEXT_CHARS} characters");
    }
    if !hq_suite_md::block::is_block_hash(on) {
        bail!("`on` must be a block hash (12 lowercase hex characters)");
    }
    if chrono::NaiveDate::parse_from_str(at, "%Y-%m-%d").is_err() {
        bail!("`at` must be a date like 2026-11-20");
    }
    let by: String = by
        .chars()
        .filter(|c| !c.is_control() && !matches!(c, '"' | '`'))
        .take(MAX_BY_CHARS)
        .collect();
    let Some(anchor) =
        (0..doc.nodes().len()).find(|&i| !is_comment(doc, i) && doc.nodes()[i].hash() == on)
    else {
        bail!("that block changed since it was read, so the comment was not added");
    };
    let mut last = anchor;
    while last + 1 < doc.nodes().len() && is_comment(doc, last + 1) {
        last += 1;
    }
    let mut attrs = vec![("on", on)];
    if !by.trim().is_empty() {
        attrs.push(("by", by.trim()));
    }
    attrs.push(("at", at));
    attrs.push(("state", "open"));
    let block = TypedBlock::new(BlockType::Comment, &attrs, text);
    patch_doc(
        doc,
        Op::InsertAfter {
            at: doc.addr(last),
            markdown: block.render("\n"),
        },
    )
}

/// Marks the comment at `index` (which must still hash to `hash`) resolved. Its other attributes
/// and its text are kept as written, including any this version does not know.
pub fn resolve(vault: &Path, note: &str, index: usize, hash: &str) -> Result<CommentsView> {
    let (file, doc) = read(vault, note)?;
    let patched = resolve_in(&doc, index, hash)?;
    fsx::atomic_write(&file, patched.render().as_bytes())?;
    Ok(view_of(&patched))
}

/// [`resolve`] over a note already read: the note as it would be, nothing written.
pub fn resolve_in(doc: &Document, index: usize, hash: &str) -> Result<Document> {
    let node = doc
        .node(index)
        .with_context(|| format!("there is no block {index}"))?;
    if node.hash() != hash {
        bail!("that comment changed since it was read");
    }
    let Some(Ok(mut block)) = doc.typed(index) else {
        bail!("block {index} is not a comment");
    };
    if block.ty != BlockType::Comment {
        bail!("block {index} is not a comment");
    }
    if block.info.get("state") == Some("resolved") {
        return Ok(doc.clone());
    }
    match block.info.attrs.iter_mut().find(|a| a.key == "state") {
        Some(a) => *a = hq_suite_md::Attr::new("state", "resolved"),
        None => block
            .info
            .attrs
            .push(hq_suite_md::Attr::new("state", "resolved")),
    }
    patch_doc(
        doc,
        Op::Replace {
            at: Addr {
                index,
                hash: hash.to_string(),
            },
            markdown: block.render(doc.eol()),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use hq_suite_md::hash::block_hash;

    const PARA: &str = "The revenue figure is final.";

    fn note() -> String {
        format!("---\ntitle: Plan\nsuite_kind: doc\n---\n\n# Plan\n\n{PARA}\n\nLast paragraph.\n")
    }

    fn vault_with(text: &str) -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("Notebooks")).unwrap();
        std::fs::write(d.path().join("Notebooks/plan.md"), text).unwrap();
        d
    }

    fn text(d: &tempfile::TempDir) -> String {
        std::fs::read_to_string(d.path().join("Notebooks/plan.md")).unwrap()
    }

    const NOTE: &str = "Notebooks/plan.md";

    #[test]
    fn a_comment_is_added_after_its_block_and_nothing_else_moves() {
        let before = note();
        let d = vault_with(&before);
        let on = block_hash(PARA);
        let v = add(d.path(), NOTE, &on, "Sam", "2026-11-20", "Is this final?").unwrap();
        assert_eq!(v.comments.len(), 1);
        let c = &v.comments[0];
        assert_eq!((c.on.as_str(), c.state.as_str()), (on.as_str(), "open"));
        assert_eq!(c.by.as_deref(), Some("Sam"));
        assert!(!c.orphaned);
        assert_eq!(c.anchor_preview.as_deref(), Some(PARA));
        let after = text(&d);
        let expected = before.replace(
            &format!("{PARA}\n"),
            &format!("{PARA}\n\n```comment on={on} by=Sam at=2026-11-20 state=open\nIs this final?\n```\n"),
        );
        assert_eq!(after, expected, "only the new block was added");
        // A second comment on the same block follows the first.
        let v = add(d.path(), NOTE, &on, "Ola", "2026-11-21", "Agreed.").unwrap();
        let by: Vec<_> = v.comments.iter().map(|c| c.by.clone().unwrap()).collect();
        assert_eq!(by, ["Sam", "Ola"]);
        assert_eq!(list(d.path(), NOTE).unwrap().comments.len(), 2);
    }

    #[test]
    fn a_stale_anchor_or_bad_input_is_refused_and_writes_nothing() {
        let d = vault_with(&note());
        let before = text(&d);
        for (on, at, body) in [
            ("000000000000", "2026-11-20", "x"),
            (&block_hash(PARA)[..], "yesterday", "x"),
            (&block_hash(PARA)[..], "2026-11-20", "   "),
            ("short", "2026-11-20", "x"),
        ] {
            assert!(
                add(d.path(), NOTE, on, "Sam", at, body).is_err(),
                "{on} {at}"
            );
        }
        assert_eq!(text(&d), before);
    }

    #[test]
    fn resolving_changes_only_the_state_and_keeps_unknown_attributes() {
        let on = block_hash(PARA);
        let with = format!(
            "{}\n```comment on={on} by=\"Sam K\" mood=hopeful\nLook again.\n```\n",
            note()
        );
        let d = vault_with(&with);
        let row = list(d.path(), NOTE).unwrap().comments.remove(0);
        assert_eq!(row.state, "open");
        // A wrong hash is refused.
        assert!(resolve(d.path(), NOTE, row.index, "000000000000").is_err());
        let v = resolve(d.path(), NOTE, row.index, &row.hash).unwrap();
        assert_eq!(v.comments[0].state, "resolved");
        assert_eq!(
            text(&d),
            with.replace(
                "by=\"Sam K\" mood=hopeful",
                "by=\"Sam K\" mood=hopeful state=resolved"
            )
        );
        // Resolving again is a no-op, and a non-comment block is refused.
        let again = list(d.path(), NOTE).unwrap().comments.remove(0);
        resolve(d.path(), NOTE, again.index, &again.hash).unwrap();
        assert!(resolve(d.path(), NOTE, 0, &block_hash("# Plan")).is_err());
    }

    #[test]
    fn editing_the_block_orphans_its_comment() {
        let d = vault_with(&note());
        add(
            d.path(),
            NOTE,
            &block_hash(PARA),
            "Sam",
            "2026-11-20",
            "Check",
        )
        .unwrap();
        let edited = text(&d).replace(PARA, "The revenue figure is draft.");
        std::fs::write(d.path().join(NOTE), edited).unwrap();
        let v = list(d.path(), NOTE).unwrap();
        assert!(v.comments[0].orphaned && v.comments[0].anchor_preview.is_none());
    }
}
