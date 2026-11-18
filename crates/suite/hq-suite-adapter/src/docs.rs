//! `hq suite docx`, `hq suite pptx` and `hq suite check`: a record as a Word file, a slides note as
//! a PowerPoint file, and the check that the suite's own render reads back as the same record.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use hq_suite_md::{Document, frontmatter};
use hq_suite_store::{SuitePaths, fsx};

use crate::shrink::vault_relative;

/// The largest file `hq suite check` reads, the same cap the shrink applies to a document.
const MAX_CHECK_BYTES: u64 = 64 * 1024 * 1024;

/// A note read for rendering: where it is, its title and its body without the frontmatter.
struct NoteForRender {
    rel: String,
    title: String,
    body: String,
}

fn read_note(vault: &Path, note: &Path) -> Result<NoteForRender> {
    let rel = vault_relative(vault, note)?;
    if !rel.ends_with(".md") {
        bail!("{rel} is not a note (a .md file)");
    }
    let paths = SuitePaths::new(vault);
    let text = std::fs::read_to_string(paths.resolve(&rel)?)
        .with_context(|| format!("{rel} cannot be read"))?;
    let doc = Document::parse(&text);
    let title = frontmatter::get(doc.frontmatter(), "title").unwrap_or_else(|| {
        rel.rsplit('/')
            .next()
            .unwrap_or(&rel)
            .trim_end_matches(".md")
            .to_string()
    });
    let body = body_of(&text).to_string();
    Ok(NoteForRender { rel, title, body })
}

/// Where a render goes: the path asked for, or beside the note with the extension changed. Never a
/// file that is already there.
fn target_for(vault: &Path, rel: &str, out: Option<&Path>, extension: &str) -> Result<PathBuf> {
    let target = match out {
        Some(p) => p.to_path_buf(),
        None => SuitePaths::new(vault)
            .resolve(&format!("{}.{extension}", rel.trim_end_matches(".md")))?,
    };
    if target.exists() {
        bail!(
            "{} already exists; choose another name with --out",
            target.display()
        );
    }
    Ok(target)
}

/// `hq suite docx <note> [--out file]`: the note as a Word file, pictures included. Without `out`
/// the file is written beside the note, named after it, and never replaces one that is there.
pub fn render(vault: &Path, note: &Path, out: Option<&Path>) -> Result<PathBuf> {
    let n = read_note(vault, note)?;
    let dir = n.rel.rsplit_once('/').map_or("", |(d, _)| d);
    let rebased = hq_suite_docs::rebase_pictures(&n.body, dir);
    let bytes = hq_suite_docs::render_docx(&n.title, &rebased, Some(vault))?;
    let target = target_for(vault, &n.rel, out, "docx")?;
    fsx::atomic_write(&target, &bytes)?;
    Ok(target)
}

/// What `hq suite pptx` wrote and what it left out.
#[derive(Debug)]
pub struct SlidesMade {
    pub path: PathBuf,
    /// Cards after the title card.
    pub cards: usize,
    pub pictures_left_out: usize,
    pub unknown_layouts: Vec<String>,
}

/// `hq suite pptx <note> [--out file]`: the slides note as a native PowerPoint file. The level 1
/// heading is the deck title and each level 2 heading a card.
pub fn render_slides(vault: &Path, note: &Path, out: Option<&Path>) -> Result<SlidesMade> {
    let n = read_note(vault, note)?;
    let (deck, bytes) = hq_suite_slides::pptx_from_markdown(&n.title, &n.body)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let target = target_for(vault, &n.rel, out, "pptx")?;
    fsx::atomic_write(&target, &bytes)?;
    Ok(SlidesMade {
        path: target,
        cards: deck.cards.len(),
        pictures_left_out: deck.pictures_left_out,
        unknown_layouts: deck.unknown_layouts,
    })
}

pub fn slides_text(m: &SlidesMade) -> String {
    let mut out = format!(
        "Wrote {} ({} card(s) after the title).\n",
        m.path.display(),
        m.cards
    );
    if m.pictures_left_out > 0 {
        out.push_str(&format!(
            "{} picture(s) are not in the PowerPoint file yet.\n",
            m.pictures_left_out
        ));
    }
    for name in &m.unknown_layouts {
        out.push_str(&format!(
            "The layout `{name}` is not one of content or two-column, so that card uses content.\n"
        ));
    }
    out
}

/// The note without its frontmatter block.
fn body_of(text: &str) -> &str {
    let parsed = Document::parse(text);
    let front = parsed.frontmatter();
    text.strip_prefix(front).unwrap_or(text)
}

/// `hq suite check <file>`: read a Word file into a record, render it, read the render, compare.
pub async fn check(file: &Path) -> Result<hq_suite_docs::Regenerated> {
    let meta =
        std::fs::metadata(file).with_context(|| format!("{} cannot be read", file.display()))?;
    if meta.len() > MAX_CHECK_BYTES {
        bail!("{} is too large to check", file.display());
    }
    let bytes = std::fs::read(file)?;
    Ok(hq_suite_docs::regenerate_check(&bytes).await?)
}

pub fn check_text(r: &hq_suite_docs::Regenerated) -> String {
    if r.equal {
        return format!(
            "The render reads back as the same record: {} blocks, with the same text, tables, lists, links and pictures.\n",
            r.blocks
        );
    }
    let mut out = String::from("The render does not read back as the same record:\n");
    for d in &r.differences {
        out.push_str(&format!("  {d}\n"));
    }
    out
}

#[cfg(test)]
mod slides_tests {
    use super::*;

    const DECK: &str = "---\ntitle: Board review\nsuite_kind: slides\n---\n\n# Board review\n\nPrepared for the board.\n\n## Results\n\n- Revenue **up** 12%\n  - Uganda strongest\n- Costs flat\n\n## Numbers\n\n| item | qty |\n|---|---|\n| Sugar | 40 |\n\n## Odd\n<!-- layout: carousel -->\n\nSomething ![pic](x.png)\n";

    fn vault_with_deck() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("Notebooks")).unwrap();
        std::fs::write(d.path().join("Notebooks/deck.md"), DECK).unwrap();
        d
    }

    #[tokio::test]
    async fn a_slides_note_becomes_a_powerpoint_file_the_suite_reads_back_as_the_same_cards() {
        let d = vault_with_deck();
        let made = render_slides(d.path(), &d.path().join("Notebooks/deck.md"), None).unwrap();
        assert_eq!(made.path, d.path().join("Notebooks/deck.pptx"));
        assert_eq!(made.cards, 3);
        assert_eq!(made.pictures_left_out, 1);
        assert_eq!(made.unknown_layouts, vec!["carousel".to_string()]);
        let bytes = std::fs::read(&made.path).unwrap();
        let read = hq_convert::ingest::convert_bytes(
            bytes,
            "pptx",
            None,
            &hq_convert::ingest::ConvertOptions::default(),
        )
        .await
        .unwrap();
        assert_eq!(read.title.as_deref(), Some("Board review"));
        for wanted in [
            "Board review",
            "Prepared for the board.",
            "Results",
            "Revenue",
            "up",
            "Uganda strongest",
            "Costs flat",
            "Numbers",
            "Sugar",
            "40",
            "Odd",
        ] {
            assert!(
                read.markdown.contains(wanted),
                "the file reads back without `{wanted}`:\n{}",
                read.markdown
            );
        }
        let text = slides_text(&made);
        assert!(text.contains("1 picture(s)") && text.contains("carousel"));
    }

    #[test]
    fn an_existing_file_is_never_replaced_and_a_non_note_is_refused() {
        let d = vault_with_deck();
        let note = d.path().join("Notebooks/deck.md");
        render_slides(d.path(), &note, None).unwrap();
        assert!(
            render_slides(d.path(), &note, None).is_err(),
            "the second run must not replace it"
        );
        std::fs::write(d.path().join("Notebooks/x.txt"), "x").unwrap();
        assert!(render_slides(d.path(), &d.path().join("Notebooks/x.txt"), None).is_err());
    }
}
