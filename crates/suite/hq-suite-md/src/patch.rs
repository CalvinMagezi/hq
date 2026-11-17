//! Edits to a note, as a list of operations that either all apply or none do.
//!
//! Every operation names a block by its position and the hash of its text at the time the caller
//! last looked (`Addr`). If the block has changed since, the edit is refused with `StaleBlock`
//! instead of landing on whatever is there now. All addresses in one patch refer to the document
//! as it was before the patch, so the order of operations never shifts what another one means.
//!
//! Text the patch does not touch is returned byte for byte.

use std::fmt;

use crate::doc::{Document, MAX_DOC_BYTES, Node};
use crate::frontmatter::{self, FrontmatterError, Scalar};

/// A block's address: where it is and what it said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Addr {
    pub index: usize,
    pub hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Op {
    /// Swap one block for new markdown, which must be exactly one block.
    Replace {
        at: Addr,
        markdown: String,
    },
    /// Add markdown after a block, or at the top with `at: None`. May be several blocks.
    InsertAfter {
        at: Option<Addr>,
        markdown: String,
    },
    Delete {
        at: Addr,
    },
    /// Set one flat frontmatter key.
    SetMeta {
        key: String,
        value: Scalar,
    },
    /// Remove one flat frontmatter key. A key that is not there is not an error.
    RemoveMeta {
        key: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Patch {
    /// If set, the patch applies only to the document with this revision.
    pub base_revision: Option<String>,
    pub ops: Vec<Op>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PatchError {
    StaleRevision {
        expected: String,
        actual: String,
    },
    NoSuchBlock(usize),
    StaleBlock {
        index: usize,
        expected: String,
        actual: String,
    },
    /// Two operations change the same block.
    Conflict(usize),
    /// The markdown to insert or substitute has no blocks.
    EmptyMarkdown,
    /// A replacement must be one block; this many were given.
    NotOneBlock(usize),
    /// Inserted markdown carries its own frontmatter.
    FrontmatterInBody,
    Frontmatter(FrontmatterError),
    TooLarge,
    /// Written out, the result would be read back as different blocks than the ones the edit
    /// made, for example a list placed right after another list, or text after a block whose
    /// closing fence is missing. Nothing is applied.
    Unstable,
}

impl fmt::Display for PatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PatchError::StaleRevision { .. } => write!(f, "the note changed since it was read"),
            PatchError::NoSuchBlock(i) => write!(f, "there is no block {i}"),
            PatchError::StaleBlock { index, .. } => {
                write!(f, "block {index} changed since it was read")
            }
            PatchError::Conflict(i) => write!(f, "two edits change block {i}"),
            PatchError::EmptyMarkdown => write!(f, "the new text has no content"),
            PatchError::NotOneBlock(n) => write!(f, "a replacement must be one block, got {n}"),
            PatchError::FrontmatterInBody => write!(f, "new text may not carry frontmatter"),
            PatchError::Frontmatter(e) => write!(f, "{e}"),
            PatchError::TooLarge => write!(f, "the note would be too large"),
            PatchError::Unstable => write!(
                f,
                "the edit would merge with or change the blocks around it when the note is read back"
            ),
        }
    }
}

impl std::error::Error for PatchError {}

impl From<FrontmatterError> for PatchError {
    fn from(e: FrontmatterError) -> Self {
        PatchError::Frontmatter(e)
    }
}

/// Gives `markdown` the document's line ending, so an edit cannot leave a file with mixed ones.
fn with_eol(markdown: &str, eol: &str) -> String {
    let unix = markdown.replace("\r\n", "\n");
    if eol == "\n" {
        unix
    } else {
        unix.replace('\n', eol)
    }
}

fn parse_fragment(markdown: &str, eol: &str) -> Result<Vec<Node>, PatchError> {
    let frag = Document::parse(&with_eol(markdown, eol));
    if !frag.frontmatter.is_empty() {
        return Err(PatchError::FrontmatterInBody);
    }
    if frag.nodes.is_empty() {
        return Err(PatchError::EmptyMarkdown);
    }
    Ok(frag.nodes)
}

#[derive(Default)]
struct Slot {
    replace: Option<Vec<Node>>,
    delete: bool,
    after: Vec<Vec<Node>>,
}

/// Applies `patch` to `doc`, returning the new document. `doc` is not changed.
pub fn apply(doc: &Document, patch: &Patch) -> Result<Document, PatchError> {
    if let Some(expected) = &patch.base_revision {
        let actual = doc.revision();
        if *expected != actual {
            return Err(PatchError::StaleRevision {
                expected: expected.clone(),
                actual,
            });
        }
    }
    let eol = doc.eol();
    let n = doc.nodes.len();
    let mut slots: Vec<Slot> = (0..n).map(|_| Slot::default()).collect();
    let mut at_top: Vec<Vec<Node>> = Vec::new();
    let mut meta_ops: Vec<(&str, Option<&Scalar>)> = Vec::new();

    let check = |at: &Addr| -> Result<(), PatchError> {
        let node = doc
            .nodes
            .get(at.index)
            .ok_or(PatchError::NoSuchBlock(at.index))?;
        let actual = node.hash();
        if actual != at.hash {
            return Err(PatchError::StaleBlock {
                index: at.index,
                expected: at.hash.clone(),
                actual,
            });
        }
        Ok(())
    };

    for op in &patch.ops {
        match op {
            Op::Replace { at, markdown } => {
                check(at)?;
                let slot = &mut slots[at.index];
                if slot.replace.is_some() || slot.delete {
                    return Err(PatchError::Conflict(at.index));
                }
                // Replacing a block with its own text is a no-op, even in a note whose line
                // endings are mixed and would otherwise be normalised.
                if markdown.trim_end() == doc.nodes[at.index].src {
                    slot.replace = Some(vec![doc.nodes[at.index].clone()]);
                    continue;
                }
                let nodes = parse_fragment(markdown, eol)?;
                if nodes.len() != 1 {
                    return Err(PatchError::NotOneBlock(nodes.len()));
                }
                slot.replace = Some(nodes);
            }
            Op::Delete { at } => {
                check(at)?;
                let slot = &mut slots[at.index];
                if slot.replace.is_some() || slot.delete {
                    return Err(PatchError::Conflict(at.index));
                }
                slot.delete = true;
            }
            Op::InsertAfter { at, markdown } => {
                let nodes = parse_fragment(markdown, eol)?;
                match at {
                    Some(at) => {
                        check(at)?;
                        slots[at.index].after.push(nodes);
                    }
                    None => at_top.push(nodes),
                }
            }
            Op::SetMeta { key, value } => meta_ops.push((key, Some(value))),
            Op::RemoveMeta { key } => meta_ops.push((key, None)),
        }
    }

    let blank = format!("{eol}{eol}");
    let first_lead = doc
        .nodes
        .first()
        .map(|n| n.lead.clone())
        .unwrap_or_default();
    let mut out: Vec<Node> = Vec::with_capacity(n);
    let push_inserted = |out: &mut Vec<Node>, group: &Vec<Node>| {
        for (i, node) in group.iter().enumerate() {
            let mut node = node.clone();
            // The first inserted block is set off by a blank line; the rest keep the spacing
            // they were written with.
            if i == 0 {
                node.lead = blank.clone();
            }
            out.push(node);
        }
    };
    for group in &at_top {
        push_inserted(&mut out, group);
    }
    // The first original block follows what was inserted above it, so it is set off from it.
    let mut sep_next = !at_top.is_empty();
    for (i, original) in doc.nodes.iter().enumerate() {
        let slot = &slots[i];
        let kept = if let Some(replacement) = &slot.replace {
            let mut node = replacement[0].clone();
            node.lead = original.lead.clone();
            Some(node)
        } else if !slot.delete {
            Some(original.clone())
        } else {
            None
        };
        if let Some(mut node) = kept {
            if std::mem::take(&mut sep_next) {
                node.lead = blank.clone();
            }
            out.push(node);
        }
        for group in &slot.after {
            push_inserted(&mut out, group);
        }
    }
    // Whatever opens the note keeps the spacing the note opened with, so deleting or inserting
    // at the top never leaves a stray blank line or removes a deliberate one.
    if let Some(first) = out.first_mut() {
        first.lead = first_lead;
    }

    let mut frontmatter = doc.frontmatter.clone();
    for (key, value) in meta_ops {
        frontmatter = match value {
            Some(v) => frontmatter::set(&frontmatter, key, v, eol)?,
            None => frontmatter::remove(&frontmatter, key)?,
        };
    }

    let result = Document {
        frontmatter,
        nodes: out,
        tail: doc.tail.clone(),
        eol: doc.eol,
    };
    let text = result.render();
    if text.len() > MAX_DOC_BYTES {
        return Err(PatchError::TooLarge);
    }
    // The result must read back as the blocks we built. This is what lets a caller treat the
    // returned document and the text on disk as the same thing.
    let again = Document::parse(&text);
    let same = again.frontmatter == result.frontmatter
        && again.nodes.len() == result.nodes.len()
        && again
            .nodes
            .iter()
            .zip(&result.nodes)
            .all(|(a, b)| a.kind == b.kind && a.src == b.src);
    if !same {
        return Err(PatchError::Unstable);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOTE: &str = "---\ntitle: T\n---\n# Title\n\nFirst.\n\nSecond.\n";

    fn addr(d: &Document, i: usize) -> Addr {
        d.addr(i).unwrap()
    }

    fn run(d: &Document, ops: Vec<Op>) -> Result<String, PatchError> {
        apply(
            d,
            &Patch {
                base_revision: None,
                ops,
            },
        )
        .map(|d| d.render())
    }

    #[test]
    fn an_empty_patch_changes_nothing() {
        let d = Document::parse(NOTE);
        assert_eq!(run(&d, vec![]).unwrap(), NOTE);
    }

    #[test]
    fn replace_touches_only_its_block() {
        let d = Document::parse(NOTE);
        let out = run(
            &d,
            vec![Op::Replace {
                at: addr(&d, 1),
                markdown: "Changed.".into(),
            }],
        )
        .unwrap();
        assert_eq!(out, "---\ntitle: T\n---\n# Title\n\nChanged.\n\nSecond.\n");
    }

    #[test]
    fn delete_keeps_the_spacing_of_the_rest() {
        let d = Document::parse(NOTE);
        assert_eq!(
            run(&d, vec![Op::Delete { at: addr(&d, 1) }]).unwrap(),
            "---\ntitle: T\n---\n# Title\n\nSecond.\n"
        );
        assert_eq!(
            run(&d, vec![Op::Delete { at: addr(&d, 0) }]).unwrap(),
            "---\ntitle: T\n---\nFirst.\n\nSecond.\n"
        );
        assert_eq!(
            run(&d, vec![Op::Delete { at: addr(&d, 2) }]).unwrap(),
            "---\ntitle: T\n---\n# Title\n\nFirst.\n"
        );
    }

    #[test]
    fn insert_after_and_at_top() {
        let d = Document::parse(NOTE);
        let out = run(
            &d,
            vec![
                Op::InsertAfter {
                    at: Some(addr(&d, 1)),
                    markdown: "Between.".into(),
                },
                Op::InsertAfter {
                    at: None,
                    markdown: "Top.".into(),
                },
            ],
        )
        .unwrap();
        assert_eq!(
            out,
            "---\ntitle: T\n---\nTop.\n\n# Title\n\nFirst.\n\nBetween.\n\nSecond.\n"
        );
    }

    #[test]
    fn several_inserts_after_one_block_keep_their_order() {
        let d = Document::parse(NOTE);
        let out = run(
            &d,
            vec![
                Op::InsertAfter {
                    at: Some(addr(&d, 2)),
                    markdown: "A.".into(),
                },
                Op::InsertAfter {
                    at: Some(addr(&d, 2)),
                    markdown: "B.".into(),
                },
            ],
        )
        .unwrap();
        assert!(out.ends_with("Second.\n\nA.\n\nB.\n"), "{out}");
    }

    #[test]
    fn an_insert_may_carry_several_blocks() {
        let d = Document::parse("one\n");
        let out = run(
            &d,
            vec![Op::InsertAfter {
                at: Some(addr(&d, 0)),
                markdown: "## H\n\ntext".into(),
            }],
        )
        .unwrap();
        assert_eq!(out, "one\n\n## H\n\ntext\n");
    }

    #[test]
    fn stale_edits_are_refused() {
        let d = Document::parse(NOTE);
        let stale = Addr {
            index: 1,
            hash: "000000000000".into(),
        };
        assert!(matches!(
            run(
                &d,
                vec![Op::Replace {
                    at: stale,
                    markdown: "x".into()
                }]
            ),
            Err(PatchError::StaleBlock { index: 1, .. })
        ));
        assert_eq!(
            run(
                &d,
                vec![Op::Delete {
                    at: Addr {
                        index: 9,
                        hash: String::new()
                    }
                }]
            ),
            Err(PatchError::NoSuchBlock(9))
        );
        let p = Patch {
            base_revision: Some("0".repeat(16)),
            ops: vec![],
        };
        assert!(matches!(
            apply(&d, &p),
            Err(PatchError::StaleRevision { .. })
        ));
        let ok = Patch {
            base_revision: Some(d.revision()),
            ops: vec![],
        };
        assert!(apply(&d, &ok).is_ok());
    }

    #[test]
    fn conflicting_edits_are_refused_whole() {
        let d = Document::parse(NOTE);
        let a = addr(&d, 1);
        assert_eq!(
            run(
                &d,
                vec![
                    Op::Replace {
                        at: a.clone(),
                        markdown: "x".into()
                    },
                    Op::Delete { at: a },
                ]
            ),
            Err(PatchError::Conflict(1))
        );
    }

    #[test]
    fn a_failed_patch_changes_nothing() {
        let d = Document::parse(NOTE);
        let before = d.render();
        let _ = run(
            &d,
            vec![
                Op::Replace {
                    at: addr(&d, 1),
                    markdown: "ok".into(),
                },
                Op::Replace {
                    at: Addr {
                        index: 2,
                        hash: "bad".into(),
                    },
                    markdown: "x".into(),
                },
            ],
        );
        assert_eq!(d.render(), before);
    }

    #[test]
    fn replacements_must_be_one_non_empty_block_without_frontmatter() {
        let d = Document::parse(NOTE);
        let at = addr(&d, 1);
        assert_eq!(
            run(
                &d,
                vec![Op::Replace {
                    at: at.clone(),
                    markdown: "  \n".into()
                }]
            ),
            Err(PatchError::EmptyMarkdown)
        );
        assert_eq!(
            run(
                &d,
                vec![Op::Replace {
                    at: at.clone(),
                    markdown: "a\n\nb".into()
                }]
            ),
            Err(PatchError::NotOneBlock(2))
        );
        assert_eq!(
            run(
                &d,
                vec![Op::InsertAfter {
                    at: Some(at),
                    markdown: "---\nk: v\n---\nx".into()
                }]
            ),
            Err(PatchError::FrontmatterInBody)
        );
    }

    #[test]
    fn set_meta_edits_one_line() {
        let d = Document::parse(NOTE);
        let out = run(
            &d,
            vec![Op::SetMeta {
                key: "suite_kind".into(),
                value: Scalar::Str("doc".into()),
            }],
        )
        .unwrap();
        assert_eq!(
            out,
            "---\ntitle: T\nsuite_kind: doc\n---\n# Title\n\nFirst.\n\nSecond.\n"
        );
        assert!(matches!(
            run(
                &d,
                vec![Op::SetMeta {
                    key: "bad key".into(),
                    value: Scalar::Int(1)
                }]
            ),
            Err(PatchError::Frontmatter(_))
        ));
    }

    #[test]
    fn a_meta_key_can_be_removed_and_removing_an_absent_one_changes_nothing() {
        let d = Document::parse(
            "---\ntitle: T\nsuite_original_sink: usb\nsuite_kind: doc\n---\n# Title\n",
        );
        let out = run(
            &d,
            vec![Op::RemoveMeta {
                key: "suite_original_sink".into(),
            }],
        )
        .unwrap();
        assert_eq!(out, "---\ntitle: T\nsuite_kind: doc\n---\n# Title\n");
        let again = Document::parse(&out);
        assert_eq!(
            run(
                &again,
                vec![Op::RemoveMeta {
                    key: "suite_original_sink".into()
                }]
            )
            .unwrap(),
            out
        );
    }

    #[test]
    fn crlf_documents_stay_crlf() {
        let d = Document::parse("one\r\n\r\ntwo\r\n");
        let out = run(
            &d,
            vec![Op::InsertAfter {
                at: Some(addr(&d, 0)),
                markdown: "a\nb".into(),
            }],
        )
        .unwrap();
        assert_eq!(out, "one\r\n\r\na\r\nb\r\n\r\ntwo\r\n");
        assert!(!out.replace("\r\n", "").contains('\n'));
    }

    #[test]
    fn a_typed_block_can_be_replaced_by_a_built_one() {
        use crate::block::{BlockType, TypedBlock};
        let d = Document::parse("```tile from=A1:A3 fn=sum\n```\n");
        let new =
            TypedBlock::new(BlockType::Tile, &[("from", "A1:A9"), ("fn", "avg")], "").render("\n");
        let out = run(
            &d,
            vec![Op::Replace {
                at: addr(&d, 0),
                markdown: new,
            }],
        )
        .unwrap();
        assert_eq!(out, "```tile from=\"A1:A9\" fn=avg\n```\n");
    }

    #[test]
    fn edits_that_would_merge_with_their_neighbours_are_refused() {
        let d = Document::parse("- a\n- b\n");
        assert_eq!(
            run(
                &d,
                vec![Op::InsertAfter {
                    at: Some(addr(&d, 0)),
                    markdown: "- c".into()
                }]
            ),
            Err(PatchError::Unstable)
        );
        let open = Document::parse("```tile from=A1:A3 fn=sum\n");
        assert_eq!(
            run(
                &open,
                vec![Op::InsertAfter {
                    at: Some(addr(&open, 0)),
                    markdown: "after".into()
                }]
            ),
            Err(PatchError::Unstable)
        );
    }

    #[test]
    fn deleting_everything_leaves_the_frontmatter_and_tail() {
        let d = Document::parse("---\na: 1\n---\nonly\n");
        assert_eq!(
            run(&d, vec![Op::Delete { at: addr(&d, 0) }]).unwrap(),
            "---\na: 1\n---\n\n"
        );
    }
}
