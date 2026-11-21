//! Everything a checker can say about a note, in one list.

use std::collections::HashMap;

use crate::block::{BlockType, Severity, TypedBlock, valid_id};
use crate::doc::{Document, MAX_DOC_BYTES, NodeKind};
use crate::info::Info;
use crate::reference::RangeRef;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    /// The block the problem is in, or `None` for the note as a whole (its frontmatter, size).
    pub node: Option<usize>,
    pub severity: Severity,
    pub code: &'static str,
    pub message: String,
}

impl Diagnostic {
    fn note(severity: Severity, code: &'static str, message: String) -> Self {
        Diagnostic {
            node: None,
            severity,
            code,
            message,
        }
    }
}

/// Checks a note: its size, its suite frontmatter keys, every typed block, and the ties between
/// blocks (duplicate ids, a tile that points at a sheet that is not there).
pub fn validate(doc: &Document) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    if doc.render().len() > MAX_DOC_BYTES {
        out.push(Diagnostic::note(
            Severity::Error,
            "note.too_large",
            format!(
                "the note is larger than {} MiB",
                MAX_DOC_BYTES / (1024 * 1024)
            ),
        ));
    }
    if !doc.frontmatter().is_empty()
        && let Err(e) = crate::frontmatter::parse_mapping(doc.frontmatter())
    {
        out.push(Diagnostic::note(
            Severity::Error,
            "frontmatter.invalid",
            e.to_string(),
        ));
    }
    for p in doc.meta().1 {
        out.push(Diagnostic::note(
            Severity::Error,
            "meta.invalid",
            format!("{}: {}", p.key, p.message),
        ));
    }

    let mut ids: HashMap<String, usize> = HashMap::new();
    let mut names: HashMap<String, usize> = HashMap::new();
    let mut has_local_sheet = false;
    let mut local_refs: Vec<(usize, String)> = Vec::new();
    let mut comments: Vec<(usize, String)> = Vec::new();

    for (i, node) in doc.nodes().iter().enumerate() {
        match node.kind() {
            NodeKind::Typed(ty) => match TypedBlock::parse(node.src()) {
                Err(e) => out.push(Diagnostic {
                    node: Some(i),
                    severity: Severity::Error,
                    code: "block.bad_header",
                    message: e.to_string(),
                }),
                Ok(b) => {
                    for p in b.validate() {
                        out.push(Diagnostic {
                            node: Some(i),
                            severity: p.severity,
                            code: p.code,
                            message: p.message,
                        });
                    }
                    if let Some(id) = b.info.id().filter(|id| valid_id(id))
                        && let Some(first) = ids.insert(id.to_string(), i)
                    {
                        out.push(Diagnostic {
                            node: Some(i),
                            severity: Severity::Error,
                            code: "block.duplicate_id",
                            message: format!("id `{id}` is already used by block {first}"),
                        });
                    }
                    if *ty == BlockType::Comment
                        && let Some(on) = b.info.get("on")
                        && crate::block::is_block_hash(on)
                    {
                        comments.push((i, on.to_string()));
                    }
                    if *ty == BlockType::Sheet {
                        has_local_sheet = true;
                        if let Some(name) = b.info.get("name")
                            && let Some(first) = names.insert(name.to_string(), i)
                        {
                            out.push(Diagnostic {
                                node: Some(i),
                                severity: Severity::Warning,
                                code: "sheet.duplicate_name",
                                message: format!("name `{name}` is also used by block {first}; a reference to it is ambiguous"),
                            });
                        }
                    }
                    if matches!(ty, BlockType::Tile | BlockType::Plot)
                        && let Some(from) = b.info.get("from")
                        && let Ok(r) = RangeRef::parse(from)
                        && r.note.is_none()
                    {
                        local_refs.push((i, from.to_string()));
                    }
                }
            },
            NodeKind::Code { lang: Some(lang) } => {
                let first = node.src().lines().next().unwrap_or("");
                if BlockType::parse(lang).is_some() {
                    // Our type, but not where a typed block may be (it starts after an indent).
                    out.push(Diagnostic {
                        node: Some(i),
                        severity: Severity::Info,
                        code: "block.indented",
                        message: format!("a `{lang}` block must start in the first column; this one is kept as code"),
                    });
                } else if let Some(info) = first
                    .trim_start_matches(['`', '~'])
                    .trim()
                    .strip_prefix(lang.as_str())
                    && info.contains('=')
                    && Info::parse(first.trim_start_matches(['`', '~'])).is_ok()
                {
                    // ```thing key=value is shaped like a typed block this version does not know.
                    out.push(Diagnostic {
                        node: Some(i),
                        severity: Severity::Info,
                        code: "block.unknown_type",
                        message: format!(
                            "`{lang}` is not a block type this version knows; it is kept as code"
                        ),
                    });
                }
            }
            NodeKind::List { .. } | NodeKind::Quote => {
                if let Some(ty) = nested_typed_fence(node.src()) {
                    out.push(Diagnostic {
                        node: Some(i),
                        severity: Severity::Info,
                        code: "block.nested",
                        message: format!(
                            "a `{}` block inside a list or quote is kept as code; typed blocks sit at the top level",
                            ty.name()
                        ),
                    });
                }
            }
            _ => {}
        }
    }
    for (i, on) in comments {
        let anchored = doc.nodes().iter().enumerate().any(|(j, n)| {
            j != i && n.kind() != &NodeKind::Typed(BlockType::Comment) && n.hash() == on
        });
        if !anchored {
            out.push(Diagnostic {
                node: Some(i),
                severity: Severity::Info,
                code: "comment.orphaned",
                message: format!(
                    "no block has the hash `{on}` any more; the block was edited or removed"
                ),
            });
        }
    }
    if !has_local_sheet {
        for (i, from) in local_refs {
            out.push(Diagnostic {
                node: Some(i),
                severity: Severity::Warning,
                code: "block.dangling_from",
                message: format!("from=`{from}` points at a sheet in this note, and there is none"),
            });
        }
    }
    out
}

/// The first fence inside a list or quote that opens one of our block types.
fn nested_typed_fence(src: &str) -> Option<BlockType> {
    src.lines().find_map(|line| {
        let rest = line.trim_start_matches([
            ' ', '\t', '>', '-', '*', '+', '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', '.',
        ]);
        crate::block::detect(rest)
    })
}

/// Whether anything in `diags` is an error.
pub fn has_errors(diags: &[Diagnostic]) -> bool {
    diags.iter().any(|d| d.severity == Severity::Error)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn codes(src: &str) -> Vec<&'static str> {
        validate(&Document::parse(src))
            .iter()
            .map(|d| d.code)
            .collect()
    }

    #[test]
    fn a_plain_note_is_clean() {
        assert!(
            codes("# Hello\n\nJust text, a [link](x) and `code`.\n\n```rust\nfn main() {}\n```\n")
                .is_empty()
        );
        assert!(codes("").is_empty());
    }

    #[test]
    fn a_good_dashboard_is_clean() {
        let src = "---\nsuite_kind: dashboard\n---\n\n```sheet name=Stock types=\"text,int\"\n| item | qty |\n|------|-----|\n| Sugar | 40 |\n```\n\n```tile from=B2:B2 fn=sum label=Total\n```\n";
        assert!(
            codes(src).is_empty(),
            "{:?}",
            validate(&Document::parse(src))
        );
    }

    #[test]
    fn block_problems_carry_their_position() {
        let d = Document::parse("para\n\n```tile fn=total\n```\n");
        let diags = validate(&d);
        assert!(diags.iter().all(|x| x.node == Some(1)));
        assert!(has_errors(&diags));
    }

    #[test]
    fn damaged_headers_are_errors_not_crashes() {
        assert_eq!(
            codes("```sheet name=\"open\n| a |\n```\n"),
            ["block.bad_header"]
        );
    }

    #[test]
    fn duplicate_ids_and_names_are_caught() {
        let src =
            "```sheet id=a name=s\n| x |\n|---|\n```\n\n```sheet id=a name=s\n| x |\n|---|\n```\n";
        let c = codes(src);
        assert!(
            c.contains(&"block.duplicate_id") && c.contains(&"sheet.duplicate_name"),
            "{c:?}"
        );
    }

    #[test]
    fn a_tile_over_a_missing_sheet_is_flagged() {
        assert_eq!(
            codes("```tile from=A1:A5 fn=sum\n```\n"),
            ["block.dangling_from"]
        );
        assert!(codes("```tile from=\"[[Other]]!A1:A5\" fn=sum\n```\n").is_empty());
    }

    #[test]
    fn unknown_typed_looking_blocks_get_a_note() {
        assert_eq!(codes("```kanban board=a\nx\n```\n"), ["block.unknown_type"]);
        assert!(codes("```rust\nfn x() {}\n```\n").is_empty());
    }

    #[test]
    fn typed_blocks_in_the_wrong_place_are_explained() {
        assert_eq!(
            codes(" ```sheet name=a\n | a |\n ```\n"),
            ["block.indented"]
        );
        assert_eq!(
            codes("> ```tile from=A1:A2 fn=sum\n> ```\n"),
            ["block.nested"]
        );
        assert_eq!(
            codes("- item\n\n  ```sheet name=a\n  | a |\n  ```\n"),
            ["block.nested"]
        );
    }

    #[test]
    fn bad_frontmatter_values_are_errors() {
        assert_eq!(
            codes("---\nsuite_kind: spreadsheet\n---\ntext\n"),
            ["meta.invalid"]
        );
        assert_eq!(
            codes("---\n: : :\n  - [\n---\ntext\n"),
            ["frontmatter.invalid"]
        );
    }
}
