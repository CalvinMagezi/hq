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
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
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
    let names = NameIndex::new(&stems);
    let broken = dangling
        .into_iter()
        .map(|l| BrokenLink {
            link: content[l.span.clone()].to_string(),
            line: l.line,
            did_you_mean: names.nearest_name(&l.target),
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

/// The existing notes by name, for finding the one a misspelt link meant.
///
/// Built once per scan. A name more than three letters longer or shorter than the link's cannot
/// be a near miss, so notes are kept by the length of their folded name and only the lengths in
/// range are compared; the folded names are computed once, not at every comparison; and the
/// answer for a link target is remembered, since the same missing name is usually linked from
/// many places. Without this a vault with thousands of broken links compared every one of them
/// with every note.
pub(crate) struct NameIndex {
    by_len: BTreeMap<usize, Vec<NameEntry>>,
    asked: RefCell<HashMap<String, Option<String>>>,
}

struct NameEntry {
    name: Vec<char>,
    rel: String,
}

impl NameIndex {
    /// `stems` is `(file name without .md, path)` for every note.
    pub(crate) fn new(stems: &[(String, String)]) -> Self {
        let mut by_len: BTreeMap<usize, Vec<NameEntry>> = BTreeMap::new();
        for (stem, rel) in stems {
            let name: Vec<char> = fold(stem).chars().collect();
            by_len.entry(name.len()).or_default().push(NameEntry {
                name,
                rel: rel.clone(),
            });
        }
        Self {
            by_len,
            asked: RefCell::new(HashMap::new()),
        }
    }

    /// The stem of the one note whose name is nearest to `target`, if it is near enough and
    /// nothing else is as near. A guess is shown to the writer, never written into a note unless
    /// the person says so.
    pub(crate) fn nearest_name(&self, target: &str) -> Option<String> {
        self.nearest(target).map(|rel| stem_of(&rel))
    }

    /// The path of that note.
    pub(crate) fn nearest(&self, target: &str) -> Option<String> {
        let want = fold(target.rsplit('/').next().unwrap_or(target));
        let want = want.strip_suffix(".md").unwrap_or(&want).to_string();
        if let Some(known) = self.asked.borrow().get(&want) {
            return known.clone();
        }
        let found = self.search(&want);
        self.asked.borrow_mut().insert(want, found.clone());
        found
    }

    fn search(&self, want: &str) -> Option<String> {
        let want: Vec<char> = want.chars().collect();
        if want.len() < 4 {
            return None;
        }
        let budget = (want.len() / 4).clamp(1, 3);
        let mut best: Option<(usize, &str)> = None;
        let mut tie = false;
        let mut scratch = Scratch::default();
        for (_, entries) in self
            .by_len
            .range(want.len().saturating_sub(budget)..=want.len() + budget)
        {
            for e in entries {
                let Some(d) = distance_chars(&want, &e.name, budget, &mut scratch) else {
                    continue;
                };
                match best {
                    Some((bd, _)) if d > bd => {}
                    Some((bd, _)) if d == bd => tie = true,
                    _ => {
                        best = Some((d, &e.rel));
                        tie = false;
                    }
                }
            }
        }
        match (best, tie) {
            (Some((_, rel)), false) => Some(rel.to_string()),
            _ => None,
        }
    }
}

/// Two rows for the edit distance, kept between comparisons so each one allocates nothing.
#[derive(Default)]
struct Scratch {
    prev: Vec<usize>,
    cur: Vec<usize>,
}

/// Edit distance between `a` and `b` when it is at most `max`, else `None`. Stops as soon as
/// every cell of a row is over `max`, since no later row can come back under it.
fn distance_chars(a: &[char], b: &[char], max: usize, s: &mut Scratch) -> Option<usize> {
    if a.len().abs_diff(b.len()) > max {
        return None;
    }
    s.prev.clear();
    s.prev.extend(0..=b.len());
    for (i, ca) in a.iter().enumerate() {
        s.cur.clear();
        s.cur.push(i + 1);
        let mut row_min = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let sub = s.prev[j] + usize::from(ca != cb);
            let v = sub.min(s.prev[j + 1] + 1).min(s.cur[j] + 1);
            row_min = row_min.min(v);
            s.cur.push(v);
        }
        if row_min > max {
            return None;
        }
        std::mem::swap(&mut s.prev, &mut s.cur);
    }
    let d = s.prev[b.len()];
    (d <= max).then_some(d)
}

/// [`distance_chars`] for two strings.
#[cfg(test)]
fn distance(a: &str, b: &str, max: usize) -> Option<usize> {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    distance_chars(&a, &b, max, &mut Scratch::default())
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

    fn index(names: &[&str]) -> NameIndex {
        let stems: Vec<(String, String)> = names
            .iter()
            .map(|n| (n.to_string(), format!("Notebooks/{n}.md")))
            .collect();
        NameIndex::new(&stems)
    }

    #[test]
    fn the_index_finds_the_one_near_note_and_says_nothing_on_a_tie_or_a_far_name() {
        let idx = index(&["Quarterly Review", "Budget", "Plan A", "Plan B"]);
        assert_eq!(
            idx.nearest_name("Quarterly Reveiw").as_deref(),
            Some("Quarterly Review")
        );
        assert_eq!(
            idx.nearest("quarterly  reveiw.md"),
            None,
            "two spaces is one more edit than it allows"
        );
        assert_eq!(
            idx.nearest("Folder/Quarterly Reveiw.md").as_deref(),
            Some("Notebooks/Quarterly Review.md")
        );
        assert_eq!(idx.nearest("Plan C"), None, "two notes are as near");
        assert_eq!(idx.nearest("Zebra Crossing Notes"), None);
        assert_eq!(
            idx.nearest("Bud"),
            None,
            "a name under four letters is never guessed"
        );
    }

    #[test]
    fn a_name_far_in_length_is_never_compared_and_the_answer_is_the_same_asked_twice() {
        let idx = index(&["Quarterly Review", "Re"]);
        assert_eq!(
            idx.nearest("Quarterly Reviews").as_deref(),
            Some("Notebooks/Quarterly Review.md")
        );
        assert_eq!(
            idx.nearest("Quarterly Reviews"),
            idx.nearest("Quarterly Reviews")
        );
        // The same missing name asked about again is answered from memory.
        assert_eq!(idx.asked.borrow().len(), 1);
    }

    #[test]
    fn two_notes_with_the_same_name_are_a_tie_not_a_guess() {
        let stems = vec![
            (
                "Quarterly Review".to_string(),
                "Notebooks/A/Quarterly Review.md".to_string(),
            ),
            (
                "Quarterly Review".to_string(),
                "Notebooks/B/Quarterly Review.md".to_string(),
            ),
        ];
        assert_eq!(NameIndex::new(&stems).nearest("Quarterly Reveiw"), None);
    }

    #[test]
    fn distance_is_bounded() {
        assert_eq!(distance("review", "reveiw", 2), Some(2));
        assert_eq!(distance("review", "review", 1), Some(0));
        assert_eq!(distance("review", "budget", 2), None);
    }
}
