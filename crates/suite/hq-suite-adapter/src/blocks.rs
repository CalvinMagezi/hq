//! Editing a note one block at a time: the blocks of a doc as the phone lists them, and an edit,
//! an insert or a delete that goes through the hqmd patch (so a block that changed since the phone
//! looked is refused, and nothing else in the note moves).

use std::path::Path;

use anyhow::{Context, Result, bail};
use hq_suite_md::{Addr, Document, NodeKind, Op, Patch, apply};
use hq_suite_store::{SuitePaths, fsx};
use serde::Serialize;

/// Most blocks one listing holds, so a hostile note cannot ask for unbounded memory.
const MAX_BLOCKS: usize = 2000;
/// Longest markdown one block edit may carry.
const MAX_BLOCK_CHARS: usize = 20_000;

#[derive(Debug, Clone, Serialize)]
pub struct BlockRow {
    pub index: usize,
    /// What the phone quotes back with an edit; it changes when the block's text does.
    pub hash: String,
    /// `heading 2`, `paragraph`, `list`, `quote`, `code`, `table`, `sheet`, ...
    pub kind: String,
    /// The block's markdown, as written.
    pub src: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct BlocksView {
    pub revision: String,
    pub blocks: Vec<BlockRow>,
}

fn kind_label(kind: &NodeKind) -> String {
    match kind {
        NodeKind::Heading { level } => format!("heading {level}"),
        NodeKind::Paragraph => "paragraph".into(),
        NodeKind::List { ordered: true } => "numbered list".into(),
        NodeKind::List { ordered: false } => "list".into(),
        NodeKind::Quote => "quote".into(),
        NodeKind::Code { lang: Some(l) } => format!("code ({l})"),
        NodeKind::Code { lang: None } => "code".into(),
        NodeKind::Typed(t) => t.name().to_string(),
        NodeKind::Table => "table".into(),
        NodeKind::Html => "html".into(),
        NodeKind::Rule => "rule".into(),
        NodeKind::Footnote => "footnote".into(),
        NodeKind::Verbatim => "text".into(),
    }
}

pub(crate) fn view_of(doc: &Document) -> BlocksView {
    let blocks = doc
        .nodes()
        .iter()
        .take(MAX_BLOCKS)
        .enumerate()
        .map(|(i, n)| BlockRow {
            index: i,
            hash: n.hash(),
            kind: kind_label(n.kind()),
            src: n.src().to_string(),
        })
        .collect();
    BlocksView {
        revision: doc.revision(),
        blocks,
    }
}

fn read(vault: &Path, note: &str) -> Result<(std::path::PathBuf, Document)> {
    let file = SuitePaths::new(vault).resolve(note)?;
    let text = std::fs::read_to_string(&file).with_context(|| format!("{note} cannot be read"))?;
    Ok((file, Document::parse(&text)))
}

/// The note's blocks, in order.
pub fn list(vault: &Path, note: &str) -> Result<BlocksView> {
    Ok(view_of(&read(vault, note)?.1))
}

pub(crate) fn check_markdown(markdown: &str) -> Result<()> {
    if markdown.trim().is_empty() {
        bail!("a block cannot be empty; delete it instead");
    }
    if markdown.chars().count() > MAX_BLOCK_CHARS {
        bail!("a block holds at most {MAX_BLOCK_CHARS} characters");
    }
    Ok(())
}

/// Applies one operation to the note as it is now and writes the result. The note's revision is
/// checked too, so an edit made from a listing that is out of date is refused.
fn change(
    vault: &Path,
    note: &str,
    op: impl FnOnce(&Document) -> Result<Op>,
) -> Result<BlocksView> {
    let (file, doc) = read(vault, note)?;
    let patch = Patch {
        base_revision: None,
        ops: vec![op(&doc)?],
    };
    let patched = apply(&doc, &patch).map_err(|e| anyhow::anyhow!("that edit was refused: {e}"))?;
    fsx::atomic_write(&file, patched.render().as_bytes())?;
    Ok(view_of(&patched))
}

/// Replaces block `index` (which must still hash to `hash`) with one block of `markdown`.
pub fn edit(
    vault: &Path,
    note: &str,
    index: usize,
    hash: &str,
    markdown: &str,
) -> Result<BlocksView> {
    check_markdown(markdown)?;
    change(vault, note, |_| {
        Ok(Op::Replace {
            at: Addr {
                index,
                hash: hash.to_string(),
            },
            markdown: markdown.to_string(),
        })
    })
}

/// Adds `markdown` after block `after` (which must still hash to `hash`), or at the top with `None`.
pub fn insert(
    vault: &Path,
    note: &str,
    after: Option<(usize, &str)>,
    markdown: &str,
) -> Result<BlocksView> {
    check_markdown(markdown)?;
    change(vault, note, |_| {
        Ok(Op::InsertAfter {
            at: after.map(|(index, hash)| Addr {
                index,
                hash: hash.to_string(),
            }),
            markdown: markdown.to_string(),
        })
    })
}

/// Removes block `index`, which must still hash to `hash`.
pub fn delete(vault: &Path, note: &str, index: usize, hash: &str) -> Result<BlocksView> {
    change(vault, note, |_| {
        Ok(Op::Delete {
            at: Addr {
                index,
                hash: hash.to_string(),
            },
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOTE: &str = "---\ntitle: Plan\nsuite_kind: doc\n---\n\n# Plan\n\nFirst paragraph.\n\n- one\n- two\n\nLast paragraph.\n";

    fn vault_with(note: &str) -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("Notebooks")).unwrap();
        std::fs::write(d.path().join("Notebooks/plan.md"), note).unwrap();
        d
    }

    fn text(d: &tempfile::TempDir) -> String {
        std::fs::read_to_string(d.path().join("Notebooks/plan.md")).unwrap()
    }

    #[test]
    fn the_blocks_are_listed_in_order_with_their_kind_and_the_frontmatter_is_not_one() {
        let d = vault_with(NOTE);
        let v = list(d.path(), "Notebooks/plan.md").unwrap();
        let kinds: Vec<&str> = v.blocks.iter().map(|b| b.kind.as_str()).collect();
        assert_eq!(kinds, ["heading 1", "paragraph", "list", "paragraph"]);
        assert_eq!(v.blocks[1].src, "First paragraph.");
        assert_eq!(v.blocks[2].src, "- one\n- two");
    }

    #[test]
    fn an_edit_changes_one_block_and_the_rest_of_the_note_is_byte_for_byte() {
        let d = vault_with(NOTE);
        let before = list(d.path(), "Notebooks/plan.md").unwrap();
        let b = &before.blocks[1];
        let after = edit(
            d.path(),
            "Notebooks/plan.md",
            b.index,
            &b.hash,
            "A **better** paragraph.",
        )
        .unwrap();
        assert_eq!(after.blocks[1].src, "A **better** paragraph.");
        assert_eq!(
            text(&d),
            NOTE.replace("First paragraph.", "A **better** paragraph.")
        );
    }

    #[test]
    fn a_stale_edit_is_refused_and_nothing_is_written() {
        let d = vault_with(NOTE);
        let v = list(d.path(), "Notebooks/plan.md").unwrap();
        let b = &v.blocks[1];
        edit(
            d.path(),
            "Notebooks/plan.md",
            b.index,
            &b.hash,
            "Changed elsewhere.",
        )
        .unwrap();
        let written = text(&d);
        assert!(edit(d.path(), "Notebooks/plan.md", b.index, &b.hash, "My edit").is_err());
        assert!(delete(d.path(), "Notebooks/plan.md", b.index, &b.hash).is_err());
        assert_eq!(text(&d), written);
    }

    #[test]
    fn a_block_is_inserted_after_another_or_at_the_top_and_deleted() {
        let d = vault_with(NOTE);
        let v = list(d.path(), "Notebooks/plan.md").unwrap();
        let h1 = &v.blocks[1];
        let added = insert(
            d.path(),
            "Notebooks/plan.md",
            Some((h1.index, &h1.hash)),
            "Inserted paragraph.",
        )
        .unwrap();
        assert_eq!(added.blocks[2].src, "Inserted paragraph.");
        let top = insert(d.path(), "Notebooks/plan.md", None, "## Summary").unwrap();
        assert_eq!(top.blocks[0].src, "## Summary");
        let last = top.blocks.last().unwrap();
        let gone = delete(d.path(), "Notebooks/plan.md", last.index, &last.hash).unwrap();
        assert_eq!(gone.blocks.len(), top.blocks.len() - 1);
        assert!(!text(&d).contains("Last paragraph."));
        assert!(
            text(&d).starts_with("---\ntitle: Plan\nsuite_kind: doc\n---\n"),
            "the frontmatter is untouched"
        );
    }

    #[test]
    fn nonsense_is_refused_with_a_reason_and_a_path_cannot_escape() {
        let d = vault_with(NOTE);
        let v = list(d.path(), "Notebooks/plan.md").unwrap();
        let b = &v.blocks[1];
        assert!(
            edit(d.path(), "Notebooks/plan.md", b.index, &b.hash, "   ").is_err(),
            "empty"
        );
        assert!(
            edit(
                d.path(),
                "Notebooks/plan.md",
                b.index,
                &b.hash,
                &"x".repeat(MAX_BLOCK_CHARS + 1)
            )
            .is_err(),
            "too long"
        );
        assert!(
            edit(
                d.path(),
                "Notebooks/plan.md",
                b.index,
                &b.hash,
                "one\n\ntwo"
            )
            .is_err(),
            "two blocks in place of one"
        );
        assert!(
            edit(d.path(), "Notebooks/plan.md", 99, "abc", "x").is_err(),
            "no such block"
        );
        assert!(list(d.path(), "../etc/passwd").is_err());
        assert_eq!(text(&d), NOTE);
    }
}
