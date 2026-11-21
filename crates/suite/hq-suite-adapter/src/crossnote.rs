//! Where a tile or a plot reads its cells from: its own note, or one other note of the same vault
//! (`from="[[Stock]]!D2:D40"`).
//!
//! The other note is found by its file name under the vault, or by a vault-relative path when the
//! name holds a `/`. It goes through `SuitePaths`, so nothing outside the vault can be named. The
//! reach is one note deep: only the other note's `sheet` blocks are read, so a tile or plot that
//! sits there never itself follows a third note, and a formula in a sheet reads its own note only.

use std::path::Path;

use anyhow::{Context, Result, bail};
use hq_suite_calc::Workbook;
use hq_suite_md::{BlockType, Document, RangeRef, TypedBlock};
use hq_suite_store::SuitePaths;

use crate::sheets::{NoteBook, sheet_of_range};

/// Largest note a tile or plot will read from elsewhere.
const MAX_NOTE_BYTES: u64 = 2 * 1024 * 1024;
/// Most folders and files looked at while searching for a note by name.
const MAX_VISITED: usize = 50_000;
const MAX_DEPTH: usize = 16;

/// The cells a range reads: the workbook of the note it lives in and the sheet's number in it.
pub(crate) struct Source {
    pub book: Workbook,
    pub sheet: usize,
}

/// The workbook and sheet `from` points at. `vault` is `None` when only the note's own text is at
/// hand (then a range in another note is refused, and says so).
pub(crate) fn source(
    vault: Option<&Path>,
    blocks: &[TypedBlock],
    note: &str,
    from: &RangeRef,
    what: &str,
) -> Result<Source> {
    let own = |from: &RangeRef| -> Result<Source> {
        let sheet = sheet_of_range(blocks, note, from, what)?;
        Ok(Source {
            book: NoteBook::new(blocks).book,
            sheet,
        })
    };
    let stem = stem_of(note);
    let Some(name) = from.note.as_deref().filter(|n| *n != stem && *n != note) else {
        return own(from);
    };
    let Some(vault) = vault else {
        bail!("{what} reads a sheet in another note only where the vault is at hand");
    };
    let (rel, doc) = load_other(vault, name)?;
    if rel == note {
        return own(from);
    }
    let other: Vec<TypedBlock> = doc.typed_blocks().filter_map(|(_, b)| b.ok()).collect();
    let wanted = from.block.as_deref();
    let sheet = other
        .iter()
        .filter(|b| b.ty == BlockType::Sheet)
        .position(|b| {
            wanted.is_none_or(|w| b.info.get("name") == Some(w) || b.info.get("id") == Some(w))
        })
        .with_context(|| match wanted {
            Some(w) => format!("the note `{name}` has no sheet called `{w}`"),
            None => format!("the note `{name}` has no sheet"),
        })?;
    Ok(Source {
        book: NoteBook::new(&other).book,
        sheet,
    })
}

fn stem_of(note: &str) -> &str {
    note.rsplit('/')
        .next()
        .unwrap_or(note)
        .trim_end_matches(".md")
}

/// Finds and reads the note a reference names.
fn load_other(vault: &Path, name: &str) -> Result<(String, Document)> {
    let paths = SuitePaths::new(vault);
    let name = name.trim().trim_end_matches(".md");
    if name.is_empty() || name.starts_with('.') || name.starts_with('~') {
        bail!("`{name}` is not a note name");
    }
    let rel = if name.contains('/') {
        let rel = format!("{name}.md");
        let real = paths
            .resolve(&rel)
            .map_err(|_| anyhow::anyhow!("`{name}` is not a note in this vault"))?;
        if !real.is_file() {
            bail!("there is no note called `{name}` in this vault");
        }
        rel
    } else {
        find_by_name(vault, name)?
    };
    let real = paths
        .resolve(&rel)
        .map_err(|_| anyhow::anyhow!("`{name}` is not a note in this vault"))?;
    let len = std::fs::metadata(&real)
        .with_context(|| format!("the note `{name}` cannot be read"))?
        .len();
    if len > MAX_NOTE_BYTES {
        bail!(
            "the note `{name}` is larger than {} MB, too big to read a range from",
            MAX_NOTE_BYTES / 1024 / 1024
        );
    }
    let text = std::fs::read_to_string(&real)
        .with_context(|| format!("the note `{name}` cannot be read"))?;
    Ok((rel, Document::parse(&text)))
}

/// The one note whose file name is `<name>.md` (case does not matter). Folders that start with a
/// dot or an underscore are the vault's own and are not searched; links are not followed.
fn find_by_name(vault: &Path, name: &str) -> Result<String> {
    let want = format!("{}.md", name.to_lowercase());
    let mut found: Vec<String> = Vec::new();
    let mut visited = 0usize;
    let mut stack = vec![(vault.to_path_buf(), String::new(), 0usize)];
    while let Some((dir, prefix, depth)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            visited += 1;
            if visited > MAX_VISITED {
                bail!(
                    "the vault is too large to look a note up by name; write its folder too, as [[Folder/{name}]]"
                );
            }
            let file = entry.file_name().to_string_lossy().into_owned();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if file.starts_with(['.', '_']) || kind.is_symlink() {
                continue;
            }
            let rel = format!("{prefix}{file}");
            if kind.is_dir() {
                if depth < MAX_DEPTH {
                    stack.push((entry.path(), format!("{rel}/"), depth + 1));
                }
            } else if kind.is_file() && file.to_lowercase() == want {
                found.push(rel);
            }
        }
    }
    found.sort();
    match found.len() {
        0 => bail!("there is no note called `{name}` in this vault"),
        1 => Ok(found.remove(0)),
        n => bail!(
            "{n} notes are called `{name}` ({}); write the folder too, as [[Folder/{name}]]",
            found.join(", ")
        ),
    }
}

#[cfg(test)]
#[path = "crossnote_tests.rs"]
mod tests;
