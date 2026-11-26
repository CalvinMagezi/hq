//! Checking the links in one note against the rest of the vault.
//!
//! The write tools call [`broken_links_in`] on the text they are about to save, so the writer
//! hears about a link that leads nowhere while it can still fix it. [`broken_links`] does the
//! same for a note already on disk. Both resolve with [`crate::links`], so a link counts as
//! broken here exactly when the health report and a move would count it.

use crate::links::{LinkRef, NoteInput, NoteSet, Resolution, aliases_of, extract, fold};
use crate::walk::{Walked, walk};
use anyhow::{Result, bail};
use serde::Serialize;
use std::path::Path;

/// A link that reaches no note.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BrokenLink {
    /// The link as written.
    pub link: String,
    pub line: usize,
    /// An existing note whose name is close, when exactly one is.
    pub did_you_mean: Option<String>,
}

/// A link whose name fits more than one note.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AmbiguousLink {
    pub link: String,
    pub line: usize,
    pub candidates: Vec<String>,
}

/// How many bytes of a note are read to find its `aliases:`.
const ALIAS_HEAD_BYTES: usize = 4096;

/// The broken and ambiguous links in `content`, a note that is, or will be, at `from`.
/// Links in code, to the same note, to outside addresses and to attachments are not broken.
pub fn broken_links_in(
    vault: &Path,
    from: &str,
    content: &str,
) -> (Vec<BrokenLink>, Vec<AmbiguousLink>) {
    let links: Vec<LinkRef> = extract(content);
    if links.is_empty() {
        return (Vec::new(), Vec::new());
    }
    let walked = walk(vault, false);
    let mut set = note_set(&walked, false);
    let mut dangling: Vec<&LinkRef> = links
        .iter()
        .filter(|l| set.resolve(from, l) == Resolution::Dangling)
        .collect();
    if !dangling.is_empty() {
        // A name may be an alias, which only the notes' frontmatter says. Read it now that a
        // link needs it, not on every write.
        set = note_set(&walked, true);
        dangling.retain(|l| set.resolve(from, l) == Resolution::Dangling);
    }

    let stems: Vec<(String, String)> = walked
        .notes
        .iter()
        .map(|n| (stem_of(&n.rel), n.rel.clone()))
        .collect();
    let broken = dangling
        .into_iter()
        .map(|l| BrokenLink {
            link: content[l.span.clone()].to_string(),
            line: l.line,
            did_you_mean: did_you_mean(&l.target, &stems),
        })
        .collect();
    let ambiguous = links
        .iter()
        .filter_map(|l| match set.resolve(from, l) {
            Resolution::Ambiguous(candidates) => Some(AmbiguousLink {
                link: content[l.span.clone()].to_string(),
                line: l.line,
                candidates,
            }),
            _ => None,
        })
        .collect();
    (broken, ambiguous)
}

/// The same for the note at `rel`, read from disk.
pub fn broken_links(vault: &Path, rel: &str) -> Result<(Vec<BrokenLink>, Vec<AmbiguousLink>)> {
    let rel = crate::rel_path::RelPath::parse(rel)?.normalized();
    let abs = vault.join(&rel);
    if !abs.is_file() {
        bail!("no note at '{rel}'");
    }
    let content = std::fs::read_to_string(abs)?;
    Ok(broken_links_in(vault, &rel, &content))
}

fn note_set(walked: &Walked, with_aliases: bool) -> NoteSet {
    NoteSet::new(
        walked.notes.iter().map(|n| NoteInput {
            path: n.rel.clone(),
            aliases: if with_aliases {
                std::fs::read(&n.abs)
                    .map(|b| {
                        let head = &b[..b.len().min(ALIAS_HEAD_BYTES)];
                        aliases_of(&String::from_utf8_lossy(head))
                    })
                    .unwrap_or_default()
            } else {
                Vec::new()
            },
        }),
        walked.attachments.iter().cloned(),
    )
}

fn stem_of(rel: &str) -> String {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    name.strip_suffix(".md").unwrap_or(name).to_string()
}

/// The one existing note whose name is nearest to `target`, if it is near enough and nothing
/// else is as near. A guess is shown to the writer, never written into a note.
fn did_you_mean(target: &str, stems: &[(String, String)]) -> Option<String> {
    let want = fold(target.rsplit('/').next().unwrap_or(target));
    let want = want.strip_suffix(".md").unwrap_or(&want);
    if want.chars().count() < 4 {
        return None;
    }
    let budget = (want.chars().count() / 4).clamp(1, 3);
    let mut best: Option<(usize, &str)> = None;
    let mut tie = false;
    for (stem, rel) in stems {
        let d = distance(want, &fold(stem), budget);
        let Some(d) = d else { continue };
        match best {
            Some((bd, _)) if d > bd => {}
            Some((bd, _)) if d == bd => tie = true,
            _ => {
                best = Some((d, rel));
                tie = false;
            }
        }
    }
    match (best, tie) {
        (Some((_, rel)), false) => Some(stem_of(rel)),
        _ => None,
    }
}

/// Edit distance between `a` and `b` when it is at most `max`, else `None`.
fn distance(a: &str, b: &str, max: usize) -> Option<usize> {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    if a.len().abs_diff(b.len()) > max {
        return None;
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut cur = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            let sub = prev[j] + usize::from(ca != cb);
            cur.push(sub.min(prev[j + 1] + 1).min(cur[j] + 1));
        }
        prev = cur;
    }
    let d = prev[b.len()];
    (d <= max).then_some(d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn vault(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (rel, text) in files {
            let p = dir.path().join(rel);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, text).unwrap();
        }
        dir
    }

    #[test]
    fn only_links_that_reach_nothing_are_broken() {
        let v = vault(&[
            ("Notebooks/Plan.md", ""),
            ("Notebooks/Person.md", "---\naliases: [Jo]\n---\n"),
            ("_attachments/pic.png", "x"),
            ("Notebooks/A/Twin.md", ""),
            ("Notebooks/B/Twin.md", ""),
        ]);
        let text = "[[Plan#Goals]] [[Jo]] ![[pic.png]] [[#here]] [w](https://e.com) \
                    `[[Code]]` [[Twin]] [[Missing]]";
        let (broken, ambiguous) = broken_links_in(v.path(), "Notebooks/New.md", text);
        assert_eq!(broken.len(), 1);
        assert_eq!(broken[0].link, "[[Missing]]");
        assert_eq!(ambiguous.len(), 1);
        assert_eq!(ambiguous[0].link, "[[Twin]]");
        assert_eq!(ambiguous[0].candidates.len(), 2);
    }

    #[test]
    fn a_near_miss_gets_a_suggestion_when_one_note_is_nearest() {
        let v = vault(&[
            ("Notebooks/Quarterly Review.md", ""),
            ("Notebooks/Budget.md", ""),
        ]);
        let (broken, _) = broken_links_in(
            v.path(),
            "Notebooks/x.md",
            "[[Quarterly Reveiw]] [[Zzzzzzzz]]",
        );
        assert_eq!(broken[0].did_you_mean.as_deref(), Some("Quarterly Review"));
        assert_eq!(broken[1].did_you_mean, None);
    }

    #[test]
    fn two_equally_near_notes_give_no_suggestion() {
        let v = vault(&[("Notebooks/Plan A.md", ""), ("Notebooks/Plan B.md", "")]);
        let (broken, _) = broken_links_in(v.path(), "Notebooks/x.md", "[[Plan C]]");
        assert_eq!(broken[0].did_you_mean, None);
    }

    #[test]
    fn a_note_with_no_links_costs_no_walk_and_reports_nothing() {
        let v = vault(&[("Notebooks/a.md", "")]);
        assert_eq!(
            broken_links_in(v.path(), "Notebooks/a.md", "plain text"),
            (vec![], vec![])
        );
    }

    #[test]
    fn broken_links_reads_a_note_from_disk_and_refuses_paths_outside_the_vault() {
        let v = vault(&[("Notebooks/a.md", "see [[Nope]]")]);
        let (broken, _) = broken_links(v.path(), "Notebooks/a.md").unwrap();
        assert_eq!(broken.len(), 1);
        assert_eq!(broken[0].line, 1);
        assert!(broken_links(v.path(), "../x.md").is_err());
        assert!(broken_links(v.path(), "Notebooks/missing.md").is_err());
    }

    #[test]
    fn distance_is_bounded() {
        assert_eq!(distance("review", "reveiw", 2), Some(2));
        assert_eq!(distance("review", "review", 1), Some(0));
        assert_eq!(distance("review", "budget", 2), None);
    }
}
