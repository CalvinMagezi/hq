//! The golden corpus and the invariants hqmd promises (`docs/suite/HQMD.md`, "Guarantees").
//!
//! Every file under `tests/corpus/` is invented. `UPDATE_GOLDEN=1 cargo test -p hq-suite-md
//! --test corpus` rewrites the `.golden` files after a deliberate change; review the diff.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use hq_suite_md::{Document, NodeKind, Op, Patch, PatchError, Scalar, apply, validate};

fn corpus_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus")
}

fn corpus() -> Vec<(String, String)> {
    let mut files: Vec<_> = fs::read_dir(corpus_dir())
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "md"))
        .collect();
    files.sort();
    assert!(files.len() >= 8, "the corpus went missing");
    files
        .into_iter()
        .map(|p| {
            let name = p.file_stem().unwrap().to_string_lossy().into_owned();
            (name, fs::read_to_string(&p).unwrap())
        })
        .collect()
}

/// A readable outline of how a note was understood: blocks, typed headers, diagnostics.
fn outline(doc: &Document) -> String {
    let mut out = String::new();
    let (meta, problems) = doc.meta();
    if !doc.frontmatter().is_empty() {
        writeln!(out, "frontmatter: {} bytes", doc.frontmatter().len()).unwrap();
        writeln!(out, "meta: {meta:?}").unwrap();
        for p in problems {
            writeln!(out, "meta problem: {} {}", p.key, p.message).unwrap();
        }
    }
    for (i, n) in doc.nodes().iter().enumerate() {
        let first = n.src().lines().next().unwrap_or("");
        let first: String = first.chars().take(60).collect();
        writeln!(out, "{i:>2} {:?} | {first}", n.kind()).unwrap();
        if let NodeKind::Typed(_) = n.kind() {
            match doc.typed(i).unwrap() {
                Ok(b) => {
                    writeln!(out, "     header: {}", b.info.render()).unwrap();
                    writeln!(out, "     closed: {}", b.closed).unwrap();
                }
                Err(e) => writeln!(out, "     header error: {e}").unwrap(),
            }
        }
    }
    let mut diags = validate(doc);
    diags.sort_by_key(|d| (d.node, d.code));
    for d in diags {
        let at = d.node.map_or("note".to_string(), |n| format!("block {n}"));
        writeln!(
            out,
            "diag {:?} {} [{}] {}",
            d.severity, d.code, at, d.message
        )
        .unwrap();
    }
    out
}

#[test]
fn every_corpus_note_prints_back_exactly() {
    for (name, text) in corpus() {
        assert_eq!(Document::parse(&text).render(), text, "{name}");
    }
}

#[test]
fn golden_outlines_match() {
    let update = std::env::var_os("UPDATE_GOLDEN").is_some();
    for (name, text) in corpus() {
        let got = outline(&Document::parse(&text));
        let path = corpus_dir().join(format!("{name}.golden"));
        if update {
            fs::write(&path, &got).unwrap();
            continue;
        }
        let want = fs::read_to_string(&path)
            .unwrap_or_else(|_| panic!("{name}.golden is missing; run with UPDATE_GOLDEN=1"));
        assert_eq!(
            got, want,
            "{name}: outline changed (UPDATE_GOLDEN=1 to accept)"
        );
    }
}

#[test]
fn the_clean_notes_have_no_diagnostics_and_the_broken_one_has_many() {
    for (name, text) in corpus() {
        let diags = validate(&Document::parse(&text));
        match name.as_str() {
            "malformed" => assert!(diags.len() >= 8, "{diags:?}"),
            // Lossy imports and unknown blocks are legal, only damaged hqmd is reported.
            _ => assert!(
                !hq_suite_md::has_errors(&diags),
                "{name} should be clean: {diags:?}"
            ),
        }
    }
}

#[test]
fn replacing_every_block_with_itself_changes_nothing() {
    for (name, text) in corpus() {
        let doc = Document::parse(&text);
        for i in 0..doc.nodes().len() {
            let patch = Patch {
                base_revision: Some(doc.revision()),
                ops: vec![Op::Replace {
                    at: doc.addr(i).unwrap(),
                    markdown: doc.nodes()[i].src().to_string(),
                }],
            };
            match apply(&doc, &patch) {
                Ok(next) => assert_eq!(next.render(), text, "{name} block {i}"),
                // A block that cannot be read back from its own text is the only allowed refusal.
                Err(PatchError::Unstable | PatchError::NotOneBlock(_)) => {}
                Err(e) => panic!("{name} block {i}: {e:?}"),
            }
        }
    }
}

#[test]
fn an_edit_in_a_plain_editor_leaves_typed_blocks_alone() {
    // A person with no hqmd support rewrites the prose around the blocks.
    for (name, text) in corpus() {
        let doc = Document::parse(&text);
        let before: Vec<_> = doc
            .nodes()
            .iter()
            .filter(|n| matches!(n.kind(), NodeKind::Typed(_)))
            .map(|n| n.src().to_string())
            .collect();
        let edited = format!(
            "A new first line.\n\n{}\n\nAnd a last one.\n",
            text.trim_end()
        );
        let after_doc = Document::parse(&edited);
        let after: Vec<_> = after_doc
            .nodes()
            .iter()
            .filter(|n| matches!(n.kind(), NodeKind::Typed(_)))
            .map(|n| n.src().to_string())
            .collect();
        // Frontmatter must stay first to count, so only compare notes without it.
        if doc.frontmatter().is_empty() {
            assert_eq!(before, after, "{name}");
        }
    }
}

#[test]
fn insert_then_delete_restores_the_note() {
    for (name, text) in corpus() {
        let doc = Document::parse(&text);
        for i in 0..doc.nodes().len() {
            let ins = Patch {
                base_revision: None,
                ops: vec![Op::InsertAfter {
                    at: doc.addr(i),
                    markdown: "Inserted paragraph.".into(),
                }],
            };
            let Ok(with) = apply(&doc, &ins) else {
                continue;
            };
            // Go through text, as a saved note does.
            let reread = Document::parse(&with.render());
            let del = Patch {
                base_revision: None,
                ops: vec![Op::Delete {
                    at: reread.addr(i + 1).unwrap(),
                }],
            };
            let back = apply(&reread, &del).unwrap();
            assert_eq!(back.render(), text, "{name} after block {i}");
        }
    }
}

#[test]
fn setting_a_key_twice_is_the_same_as_once() {
    for (name, text) in corpus() {
        let doc = Document::parse(&text);
        let set = |d: &Document| {
            apply(
                d,
                &Patch {
                    base_revision: None,
                    ops: vec![Op::SetMeta {
                        key: "suite_saved_bytes".into(),
                        value: Scalar::Int(7),
                    }],
                },
            )
        };
        let once = match set(&doc) {
            Ok(d) => d,
            // A note whose frontmatter cannot take a one-line edit is refused, never mangled.
            Err(PatchError::Frontmatter(_)) => continue,
            Err(e) => panic!("{name}: {e:?}"),
        };
        let twice = set(&Document::parse(&once.render())).unwrap();
        assert_eq!(once.render(), twice.render(), "{name}");
        // Nothing but the one line changed.
        let a: Vec<&str> = text.lines().collect();
        let b_text = once.render();
        let b: Vec<&str> = b_text.lines().collect();
        let removed = a.iter().filter(|l| !b.contains(l)).count();
        assert!(removed <= 1, "{name}: {removed} lines changed");
    }
}

#[test]
fn crlf_versions_of_the_corpus_round_trip_and_agree_on_structure() {
    for (name, text) in corpus() {
        let crlf = text.replace('\n', "\r\n");
        let doc = Document::parse(&crlf);
        assert_eq!(doc.render(), crlf, "{name}");
        let lf = Document::parse(&text);
        assert_eq!(doc.nodes().len(), lf.nodes().len(), "{name}");
        let kinds = |d: &Document| {
            d.nodes()
                .iter()
                .map(|n| n.kind().clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(kinds(&doc), kinds(&lf), "{name}");
    }
}

/// A tiny deterministic generator so the fuzz run is the same everywhere.
struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }
    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[(self.next() as usize) % items.len()]
    }
}

#[test]
fn random_documents_round_trip_and_never_panic() {
    let fragments: Vec<&str> = vec![
        "# Heading",
        "Paragraph with *emphasis* and a [link](https://example.com).",
        "- a\n- b\n  - nested",
        "1. one\n2. two",
        "> quote\n> more",
        "```rust\nfn main() {}\n```",
        "```sheet name=s\n| a | b |\n|---|---|\n| 1 | 2 |\n```",
        "```sheet name=\"open\n| a |",
        "```tile from=A1:A3 fn=sum\n```",
        "~~~plot type=bar from=A1:B3 x=a y=b\n~~~",
        "| t | u |\n|---|---|\n| 1 | 2 |",
        "    indented code",
        "<div>html</div>",
        "<!-- comment -->",
        "---",
        "[ref]: https://example.com",
        "[^1]: footnote",
        "text[^1]",
        "\t",
        "```",
        "~~~",
        "***bold italic***",
        "Line one  \nLine two",
        "\u{1F600} emoji and \u{00e9}",
        "---\nkey: value\n---",
    ];
    let gaps = ["\n\n", "\n", "\n\n\n", "\r\n\r\n", " \n\n", "\n\t\n"];
    let mut rng = Lcg(0x5EED);
    for round in 0..400 {
        let mut text = String::new();
        if rng.next().is_multiple_of(3) {
            text.push_str("---\ntitle: fuzz\n---\n");
        }
        for _ in 0..(rng.next() % 12) {
            text.push_str(rng.pick(&fragments));
            text.push_str(rng.pick(&gaps));
        }
        let doc = Document::parse(&text);
        assert_eq!(doc.render(), text, "round {round}: {text:?}");
        let _ = validate(&doc);
        // Every block can be addressed, and replacing it with itself is either a no-op or refused.
        for i in 0..doc.nodes().len() {
            let patch = Patch {
                base_revision: None,
                ops: vec![Op::Replace {
                    at: doc.addr(i).unwrap(),
                    markdown: doc.nodes()[i].src().to_string(),
                }],
            };
            if let Ok(next) = apply(&doc, &patch) {
                assert_eq!(next.render(), text, "round {round} block {i}: {text:?}");
            }
        }
    }
}
