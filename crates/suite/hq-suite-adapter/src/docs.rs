//! `hq suite docx` and `hq suite check`: a record as a Word file, and the check that the suite's
//! own render reads back as the same record.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use hq_suite_md::{Document, frontmatter};
use hq_suite_store::{SuitePaths, fsx};

use crate::shrink::vault_relative;

/// The largest file `hq suite check` reads, the same cap the shrink applies to a document.
const MAX_CHECK_BYTES: u64 = 64 * 1024 * 1024;

/// `hq suite docx <note> [--out file]`: the note as a Word file, pictures included. Without `out`
/// the file is written beside the note, named after it, and never replaces one that is there.
pub fn render(vault: &Path, note: &Path, out: Option<&Path>) -> Result<PathBuf> {
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
    let body = body_of(&text);
    let dir = rel.rsplit_once('/').map_or("", |(d, _)| d);
    let rebased = hq_suite_docs::rebase_pictures(body, dir);
    let bytes = hq_suite_docs::render_docx(&title, &rebased, Some(vault))?;
    let target = match out {
        Some(p) => p.to_path_buf(),
        None => paths.resolve(&format!("{}.docx", rel.trim_end_matches(".md")))?,
    };
    if target.exists() {
        bail!(
            "{} already exists; choose another name with --out",
            target.display()
        );
    }
    fsx::atomic_write(&target, &bytes)?;
    Ok(target)
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
