//! What is wrong with the links and the notes themselves, found from one snapshot of the vault.
//!
//! [`scan`] is pure: it reads a [`Snapshot`] and returns a [`Report`]. The daemon, the
//! `hq vault health` command, the web and phone screens and the agent tool all show this one
//! report, so they cannot disagree about what is broken. Nothing here changes a note; the fix
//! planner (`fixes.rs`) reads the same report to say what could be repaired.
//!
//! Counts are exact. The lists of examples are capped at [`MAX_LISTED`] per class so a vault
//! with thousands of problems still gives a small report; the cap is a display limit, and the
//! planner reads the full set through [`scan_full`].

use crate::links::{LinkRef, NoteInput, NoteSet, Resolution, aliases_of, extract, fold};
use crate::relink::Snapshot;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};

/// Most examples listed per class in a [`Report`].
pub const MAX_LISTED: usize = 50;

/// A note body shorter than this, once trimmed, is "empty" and is not matched as a duplicate.
const MIN_DUPLICATE_CHARS: usize = 40;

/// A link that reaches no note.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeadLink {
    pub note: String,
    pub line: usize,
    /// The link as written.
    pub link: String,
    /// The name inside the link, without heading or label.
    pub target: String,
    pub did_you_mean: Option<String>,
}

/// A link that fits more than one note.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AmbiguousLink {
    pub note: String,
    pub line: usize,
    pub link: String,
    pub candidates: Vec<String>,
}

/// A link to a note that exists, to a heading or block that does not.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrokenAnchor {
    pub note: String,
    pub line: usize,
    pub link: String,
    /// The note the link reaches.
    pub target_note: String,
    /// The heading or `^block` that is not in it.
    pub anchor: String,
}

/// Notes whose text is the same once spacing is ignored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DuplicateGroup {
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FrontmatterProblem {
    /// Opens with `---` and never closes.
    Unclosed,
    /// The block between the lines is not valid YAML.
    InvalidYaml,
    /// A second block of properties sits at the top of the body.
    Nested,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrontmatterIssue {
    pub note: String,
    pub problem: FrontmatterProblem,
}

/// How the links in the vault came out.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkCounts {
    pub total: usize,
    /// Reach one note or a file.
    pub ok: usize,
    pub dead: usize,
    pub ambiguous: usize,
    pub broken_anchors: usize,
    /// Outside addresses and links to the same note: not checked.
    pub not_checked: usize,
}

/// Every count in a [`Report`], for a trend line and for the summary.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Counts {
    pub notes: usize,
    pub unread: usize,
    pub links: LinkCounts,
    pub loose: usize,
    pub duplicates: usize,
    pub empty: usize,
    pub frontmatter: usize,
}

impl Counts {
    /// Things a person could act on: dead and ambiguous links, broken anchors, duplicate groups
    /// and frontmatter problems. Loose and empty notes are reported but are not damage.
    pub fn problems(&self) -> usize {
        self.links.dead
            + self.links.ambiguous
            + self.links.broken_anchors
            + self.duplicates
            + self.frontmatter
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Report {
    pub counts: Counts,
    pub dead_links: Vec<DeadLink>,
    pub ambiguous_links: Vec<AmbiguousLink>,
    pub broken_anchors: Vec<BrokenAnchor>,
    /// Notes nothing links to and that link to nothing.
    pub loose: Vec<String>,
    pub duplicates: Vec<DuplicateGroup>,
    /// Notes with nothing in them.
    pub empty: Vec<String>,
    pub frontmatter: Vec<FrontmatterIssue>,
    /// Notes that could not be read: too large, or not text.
    pub unread: Vec<String>,
}

/// The report with each list capped at [`MAX_LISTED`]. The counts are exact.
pub fn scan(snap: &Snapshot) -> Report {
    let mut report = scan_full(snap);
    report.cap();
    report
}

/// The kinds of item an owner can ignore, as stored.
pub mod kind {
    pub const DEAD_LINK: &str = "dead_link";
    pub const AMBIGUOUS_LINK: &str = "ambiguous_link";
    pub const BROKEN_ANCHOR: &str = "broken_anchor";
    pub const LOOSE: &str = "loose";
    pub const EMPTY: &str = "empty";
    pub const DUPLICATE: &str = "duplicate";
    pub const FRONTMATTER: &str = "frontmatter";
}

/// What the owner chose to leave alone: `(kind, key)` pairs from [`kind`] and the `key()` of
/// an item.
pub type Ignored = HashSet<(String, String)>;

impl DeadLink {
    /// Names this link in the note it is written in, so a repeat elsewhere is a different item.
    pub fn key(&self) -> String {
        format!("{}|{}", self.note, self.link)
    }
}

impl AmbiguousLink {
    pub fn key(&self) -> String {
        format!("{}|{}", self.note, self.link)
    }
}

impl BrokenAnchor {
    pub fn key(&self) -> String {
        format!("{}|{}", self.note, self.link)
    }
}

impl DuplicateGroup {
    /// The notes in the group, which is what the owner is deciding about.
    pub fn key(&self) -> String {
        self.notes.join("|")
    }
}

impl Report {
    /// Keep at most [`MAX_LISTED`] examples in each list. The counts are not changed.
    pub fn cap(&mut self) {
        self.dead_links.truncate(MAX_LISTED);
        self.ambiguous_links.truncate(MAX_LISTED);
        self.broken_anchors.truncate(MAX_LISTED);
        self.loose.truncate(MAX_LISTED);
        self.duplicates.truncate(MAX_LISTED);
        self.empty.truncate(MAX_LISTED);
        self.frontmatter.truncate(MAX_LISTED);
        self.unread.truncate(MAX_LISTED);
    }

    /// Drop what the owner chose to ignore and count again. Call on a [`scan_full`] report,
    /// before [`Report::cap`], so a capped list is not mistaken for the whole.
    pub fn apply_ignores(&mut self, ignored: &Ignored) {
        if ignored.is_empty() {
            return;
        }
        let has = |k: &str, key: String| ignored.contains(&(k.to_string(), key));
        self.dead_links.retain(|d| !has(kind::DEAD_LINK, d.key()));
        self.ambiguous_links
            .retain(|a| !has(kind::AMBIGUOUS_LINK, a.key()));
        self.broken_anchors
            .retain(|b| !has(kind::BROKEN_ANCHOR, b.key()));
        self.loose.retain(|p| !has(kind::LOOSE, p.clone()));
        self.empty.retain(|p| !has(kind::EMPTY, p.clone()));
        self.duplicates.retain(|g| !has(kind::DUPLICATE, g.key()));
        self.frontmatter
            .retain(|f| !has(kind::FRONTMATTER, f.note.clone()));
        self.recount();
    }

    /// Keep only what concerns the note at `path`: links written in it, a duplicate group it is
    /// in, its own frontmatter, and whether it is loose or empty. For checking one note just
    /// written. Call on a [`scan_full`] report.
    pub fn only_note(&mut self, path: &str) {
        self.dead_links.retain(|d| d.note == path);
        self.ambiguous_links.retain(|a| a.note == path);
        self.broken_anchors.retain(|b| b.note == path);
        self.loose.retain(|p| p == path);
        self.empty.retain(|p| p == path);
        self.duplicates
            .retain(|g| g.notes.iter().any(|n| n == path));
        self.frontmatter.retain(|f| f.note == path);
        self.unread.retain(|p| p == path);
        self.recount();
    }

    /// The counts that are the length of a list, taken from the lists. The totals of links and
    /// notes are left as the scan found them.
    fn recount(&mut self) {
        self.counts.links.dead = self.dead_links.len();
        self.counts.links.ambiguous = self.ambiguous_links.len();
        self.counts.links.broken_anchors = self.broken_anchors.len();
        self.counts.loose = self.loose.len();
        self.counts.empty = self.empty.len();
        self.counts.duplicates = self.duplicates.len();
        self.counts.frontmatter = self.frontmatter.len();
        self.counts.unread = self.unread.len();
    }
}

/// The report with every example listed.
pub fn scan_full(snap: &Snapshot) -> Report {
    let texts: HashMap<&str, &str> = snap
        .notes
        .iter()
        .map(|n| (n.path.as_str(), n.text.as_str()))
        .collect();
    let set = NoteSet::new(
        snap.notes.iter().map(|n| NoteInput {
            path: n.path.clone(),
            aliases: aliases_of(&n.text),
        }),
        snap.attachments.iter().cloned(),
    );
    let stems: Vec<(String, String)> = snap
        .notes
        .iter()
        .map(|n| (stem_of(&n.path), n.path.clone()))
        .collect();

    let mut r = Report {
        unread: snap.unread.clone(),
        ..Report::default()
    };
    let mut inbound: HashSet<&str> = HashSet::new();
    let mut has_outbound: HashSet<&str> = HashSet::new();
    let mut headings_of: HashMap<String, HashSet<String>> = HashMap::new();

    for note in &snap.notes {
        for link in extract(&note.text) {
            r.counts.links.total += 1;
            match set.resolve(&note.path, &link) {
                Resolution::Unique(target) => {
                    r.counts.links.ok += 1;
                    if target != note.path {
                        has_outbound.insert(&note.path);
                        if let Some(t) = texts.get_key_value(target.as_str()) {
                            inbound.insert(t.0);
                        }
                    }
                    if let Some(anchor) = missing_anchor(&link, &target, &texts, &mut headings_of) {
                        r.counts.links.broken_anchors += 1;
                        r.broken_anchors.push(BrokenAnchor {
                            note: note.path.clone(),
                            line: link.line,
                            link: note.text[link.span.clone()].to_string(),
                            target_note: target,
                            anchor,
                        });
                    }
                }
                Resolution::Attachment(_) => {
                    r.counts.links.ok += 1;
                    has_outbound.insert(&note.path);
                }
                Resolution::External | Resolution::SameNote => r.counts.links.not_checked += 1,
                Resolution::Ambiguous(candidates) => {
                    r.counts.links.ambiguous += 1;
                    has_outbound.insert(&note.path);
                    for c in &candidates {
                        if let Some(t) = texts.get_key_value(c.as_str()) {
                            inbound.insert(t.0);
                        }
                    }
                    r.ambiguous_links.push(AmbiguousLink {
                        note: note.path.clone(),
                        line: link.line,
                        link: note.text[link.span.clone()].to_string(),
                        candidates,
                    });
                }
                Resolution::Dangling => {
                    r.counts.links.dead += 1;
                    r.dead_links.push(DeadLink {
                        note: note.path.clone(),
                        line: link.line,
                        link: note.text[link.span.clone()].to_string(),
                        did_you_mean: did_you_mean(&link.target, &stems),
                        target: link.target,
                    });
                }
            }
        }
    }

    for note in &snap.notes {
        let body = body_of(&note.text);
        if body.trim().is_empty() {
            r.empty.push(note.path.clone());
        } else if !inbound.contains(note.path.as_str())
            && !has_outbound.contains(note.path.as_str())
        {
            r.loose.push(note.path.clone());
        }
        if let Some(problem) = frontmatter_problem(&note.text) {
            r.frontmatter.push(FrontmatterIssue {
                note: note.path.clone(),
                problem,
            });
        }
    }
    r.duplicates = duplicate_groups(snap);

    // The same vault always gives the same report, whatever order the notes were listed in.
    r.dead_links
        .sort_by(|a, b| (&a.note, a.line).cmp(&(&b.note, b.line)));
    r.ambiguous_links
        .sort_by(|a, b| (&a.note, a.line).cmp(&(&b.note, b.line)));
    r.broken_anchors
        .sort_by(|a, b| (&a.note, a.line).cmp(&(&b.note, b.line)));
    r.frontmatter.sort_by(|a, b| a.note.cmp(&b.note));
    r.loose.sort();
    r.empty.sort();

    r.counts.notes = snap.notes.len();
    r.counts.unread = snap.unread.len();
    r.counts.loose = r.loose.len();
    r.counts.empty = r.empty.len();
    r.counts.duplicates = r.duplicates.len();
    r.counts.frontmatter = r.frontmatter.len();
    r
}

// ─── Parts ───────────────────────────────────────────────────────────────────

fn stem_of(path: &str) -> String {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.strip_suffix(".md").unwrap_or(name).to_string()
}

fn body_of(text: &str) -> &str {
    hq_core::frontmatter_utils::split_frontmatter(text).1
}

/// The heading or block a link names in a note that exists but lacks it, if any.
fn missing_anchor(
    link: &LinkRef,
    target: &str,
    texts: &HashMap<&str, &str>,
    cache: &mut HashMap<String, HashSet<String>>,
) -> Option<String> {
    if link.heading.is_none() && link.block.is_none() {
        return None;
    }
    let text = texts.get(target)?;
    if let Some(block) = &link.block {
        let needle = format!("^{block}");
        let has = text
            .lines()
            .any(|l| l.trim_end().ends_with(&needle) || l.trim() == needle);
        return (!has).then(|| format!("^{block}"));
    }
    let heading = link.heading.as_ref()?;
    let known = cache
        .entry(target.to_string())
        .or_insert_with(|| headings(text));
    // `A#B#C` is a heading inside a heading: only the last part is looked up.
    let last = heading.rsplit('#').next().unwrap_or(heading);
    (!known.contains(&anchor_key(last))).then(|| heading.clone())
}

/// The headings of a note, as [`anchor_key`] compares them. Headings in code are not headings.
fn headings(text: &str) -> HashSet<String> {
    let mut out = HashSet::new();
    let mut fence: Option<char> = None;
    for line in body_of(text).lines() {
        let t = line.trim_start();
        if let Some(c) = t.chars().next().filter(|c| matches!(c, '`' | '~')) {
            let run = t.chars().take_while(|x| *x == c).count();
            if run >= 3 {
                fence = match fence {
                    Some(open) if open == c => None,
                    None => Some(c),
                    other => other,
                };
                continue;
            }
        }
        if fence.is_some() {
            continue;
        }
        let hashes = t.chars().take_while(|c| *c == '#').count();
        if (1..=6).contains(&hashes) && t[hashes..].starts_with(' ') {
            out.insert(anchor_key(t[hashes..].trim().trim_end_matches('#')));
        }
    }
    out
}

/// A heading as a link compares it: letters and digits only, any case. A link to `Q3 plan?`
/// finds the heading `Q3 plan`, the way an editor's own heading links do.
fn anchor_key(heading: &str) -> String {
    fold(heading)
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect()
}

fn frontmatter_problem(text: &str) -> Option<FrontmatterProblem> {
    if text.starts_with("---\n") || text.starts_with("---\r\n") {
        let (fm, body) = hq_core::frontmatter_utils::split_frontmatter(text);
        let Some(fm) = fm else {
            return Some(FrontmatterProblem::Unclosed);
        };
        if serde_yaml::from_str::<serde_yaml::Value>(fm).is_err() {
            return Some(FrontmatterProblem::InvalidYaml);
        }
        if crate::remediation::has_nested_frontmatter(body) {
            return Some(FrontmatterProblem::Nested);
        }
    }
    None
}

/// Notes with the same words, whatever the spacing, in groups of two or more. Short notes are
/// not matched: two notes that both say "TODO" are not a duplicate.
pub(crate) fn duplicate_groups(snap: &Snapshot) -> Vec<DuplicateGroup> {
    let mut by_text: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for note in &snap.notes {
        let normal: String = body_of(&note.text)
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase();
        if normal.chars().count() >= MIN_DUPLICATE_CHARS {
            by_text.entry(normal).or_default().push(note.path.clone());
        }
    }
    let mut groups: Vec<DuplicateGroup> = by_text
        .into_values()
        .filter(|g| g.len() > 1)
        .map(|mut notes| {
            notes.sort();
            DuplicateGroup { notes }
        })
        .collect();
    groups.sort_by(|a, b| a.notes.cmp(&b.notes));
    groups
}

/// The one note whose name is nearest to `target`, if it is near enough and nothing is as near.
fn did_you_mean(target: &str, stems: &[(String, String)]) -> Option<String> {
    crate::check::nearest_name(target, stems)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::relink::SnapNote;

    fn snap(notes: &[(&str, &str)], attachments: &[&str]) -> Snapshot {
        Snapshot {
            notes: notes
                .iter()
                .map(|(p, t)| SnapNote {
                    path: p.to_string(),
                    text: t.to_string(),
                })
                .collect(),
            attachments: attachments.iter().map(|a| a.to_string()).collect(),
            unread: vec![],
        }
    }

    #[test]
    fn a_healthy_vault_reports_nothing_to_do() {
        let s = snap(
            &[
                (
                    "Notebooks/A.md",
                    "see [[B#Goals]] and ![[pic.png]] and [x](https://e.com)",
                ),
                (
                    "Notebooks/B.md",
                    "# Goals\n\nwords that make this a real note, with enough text",
                ),
            ],
            &["_attachments/pic.png"],
        );
        let r = scan(&s);
        assert_eq!(r.counts.problems(), 0, "{r:?}");
        assert_eq!(r.counts.links.total, 3);
        assert_eq!(r.counts.links.ok, 2);
        assert_eq!(r.counts.links.not_checked, 1);
        assert_eq!(r.counts.loose, 0);
    }

    #[test]
    fn dead_links_count_only_what_reaches_nothing_and_say_where() {
        let s = snap(
            &[
                (
                    "Notebooks/Quarterly Review.md",
                    "real note with enough words to not be empty",
                ),
                (
                    "Notebooks/L.md",
                    "ok [[Quarterly Review]]\nbad [[Quarterly Reveiw]]\n```\n[[In Code]]\n```",
                ),
            ],
            &[],
        );
        let r = scan(&s);
        assert_eq!(r.counts.links.dead, 1);
        assert_eq!(r.dead_links[0].note, "Notebooks/L.md");
        assert_eq!(r.dead_links[0].line, 2);
        assert_eq!(r.dead_links[0].link, "[[Quarterly Reveiw]]");
        assert_eq!(
            r.dead_links[0].did_you_mean.as_deref(),
            Some("Quarterly Review")
        );
    }

    #[test]
    fn a_name_two_notes_share_is_ambiguous_not_dead() {
        let s = snap(
            &[
                (
                    "Notebooks/A/Plan.md",
                    "plan a with enough words in it to count",
                ),
                (
                    "Notebooks/B/Plan.md",
                    "plan b is different but also long enough",
                ),
                ("Notebooks/L.md", "[[Plan]]"),
            ],
            &[],
        );
        let r = scan(&s);
        assert_eq!(r.counts.links.ambiguous, 1);
        assert_eq!(r.counts.links.dead, 0);
        assert_eq!(r.ambiguous_links[0].candidates.len(), 2);
        assert_eq!(
            r.counts.loose, 0,
            "both Plans are linked to, so neither is loose"
        );
    }

    #[test]
    fn a_heading_or_block_that_is_not_there_is_a_broken_anchor() {
        let s = snap(
            &[
                (
                    "Notebooks/T.md",
                    "# Real heading\n\n```\n# not a heading\n```\n\ntext ^blk1\n",
                ),
                (
                    "Notebooks/L.md",
                    "[[T#Real heading]] [[T#real  HEADING?]] [[T#Gone]] [[T#not a heading]] [[T#^blk1]] [[T#^nope]]",
                ),
            ],
            &[],
        );
        let r = scan(&s);
        let anchors: Vec<&str> = r.broken_anchors.iter().map(|a| a.anchor.as_str()).collect();
        assert_eq!(anchors, ["Gone", "not a heading", "^nope"]);
        assert_eq!(r.counts.links.broken_anchors, 3);
    }

    #[test]
    fn loose_and_empty_notes_are_reported_but_are_not_problems() {
        let s = snap(
            &[
                (
                    "Notebooks/Lonely.md",
                    "a note about something that nobody links to yet",
                ),
                ("Notebooks/Blank.md", "---\ntitle: Blank\n---\n\n"),
                ("Notebooks/A.md", "[[B]]"),
                (
                    "Notebooks/B.md",
                    "linked from A and so not loose at all, with words",
                ),
            ],
            &[],
        );
        let r = scan(&s);
        assert_eq!(r.loose, ["Notebooks/Lonely.md"]);
        assert_eq!(r.empty, ["Notebooks/Blank.md"]);
        assert_eq!(r.counts.problems(), 0);
    }

    #[test]
    fn exact_duplicates_are_found_through_spacing_and_case_and_short_notes_are_not() {
        let text = "The same long paragraph about the quarterly numbers and what they mean.";
        let s = snap(
            &[
                (
                    "Notebooks/One.md",
                    &format!("---\ntitle: One\n---\n\n{text}"),
                ),
                (
                    "Notebooks/Two.md",
                    &format!("{}\n\n", text.to_uppercase().replace(' ', "  ")),
                ),
                ("Notebooks/Three.md", "TODO"),
                ("Notebooks/Four.md", "TODO"),
            ],
            &[],
        );
        let r = scan(&s);
        assert_eq!(r.duplicates.len(), 1);
        assert_eq!(
            r.duplicates[0].notes,
            ["Notebooks/One.md", "Notebooks/Two.md"]
        );
    }

    #[test]
    fn frontmatter_problems_are_told_apart() {
        let s = snap(
            &[
                ("Notebooks/Unclosed.md", "---\ntitle: x\nno end"),
                ("Notebooks/Bad.md", "---\ntitle: [unclosed\n---\nbody text"),
                (
                    "Notebooks/Nested.md",
                    "---\ntitle: T\n---\n\n---\ntags: [a]\n---\n\nbody",
                ),
                (
                    "Notebooks/Fine.md",
                    "---\ntitle: T\n---\nbody that is long enough to be a real note",
                ),
                (
                    "Notebooks/Rule.md",
                    "intro\n\n---\n\nbelow a horizontal rule, which is not frontmatter",
                ),
            ],
            &[],
        );
        let r = scan(&s);
        let got: Vec<(&str, FrontmatterProblem)> = r
            .frontmatter
            .iter()
            .map(|f| (f.note.as_str(), f.problem))
            .collect();
        assert_eq!(
            got,
            [
                ("Notebooks/Bad.md", FrontmatterProblem::InvalidYaml),
                ("Notebooks/Nested.md", FrontmatterProblem::Nested),
                ("Notebooks/Unclosed.md", FrontmatterProblem::Unclosed),
            ]
        );
    }

    #[test]
    fn the_lists_are_capped_and_the_counts_are_not() {
        let mut notes = vec![("Notebooks/Hub.md".to_string(), String::new())];
        let links: String = (0..120).map(|i| format!("[[Missing {i}]] ")).collect();
        notes[0].1 = links;
        let s = Snapshot {
            notes: notes
                .into_iter()
                .map(|(path, text)| SnapNote { path, text })
                .collect(),
            ..Snapshot::default()
        };
        let capped = scan(&s);
        assert_eq!(capped.counts.links.dead, 120);
        assert_eq!(capped.dead_links.len(), MAX_LISTED);
        assert_eq!(scan_full(&s).dead_links.len(), 120);
    }

    #[test]
    fn ignored_items_leave_the_lists_and_the_counts_and_nothing_else_does() {
        let s = snap(
            &[
                (
                    "Notebooks/A.md",
                    "[[Gone One]] [[Gone Two]] and enough words to not be empty",
                ),
                (
                    "Notebooks/Lonely.md",
                    "a note nobody links to, with enough words in it",
                ),
            ],
            &[],
        );
        let mut r = scan_full(&s);
        assert_eq!(r.counts.links.dead, 2);
        // `A.md` links only to notes that are not there, so it is as loose as the other.
        assert_eq!(r.counts.loose, 2);
        let ignored: Ignored = [
            (kind::DEAD_LINK.to_string(), r.dead_links[0].key()),
            (kind::LOOSE.to_string(), "Notebooks/Lonely.md".to_string()),
            (
                kind::DEAD_LINK.to_string(),
                "Notebooks/Other.md|[[Gone Two]]".to_string(),
            ),
        ]
        .into_iter()
        .collect();
        r.apply_ignores(&ignored);
        assert_eq!(r.counts.links.dead, 1);
        assert_eq!(r.dead_links[0].link, "[[Gone Two]]");
        assert_eq!(r.counts.loose, 1);
        assert_eq!(r.loose, ["Notebooks/A.md"]);
        assert_eq!(r.counts.links.total, 2, "an ignored link is still a link");
    }

    #[test]
    fn a_report_can_be_narrowed_to_one_note() {
        let s = snap(
            &[
                (
                    "Notebooks/A.md",
                    "[[Gone]] with enough words to not be empty at all",
                ),
                (
                    "Notebooks/B.md",
                    "[[Also Gone]] with enough words to not be empty either",
                ),
            ],
            &[],
        );
        let mut r = scan_full(&s);
        r.only_note("Notebooks/A.md");
        assert_eq!(r.counts.links.dead, 1);
        assert_eq!(r.dead_links[0].link, "[[Gone]]");
        assert_eq!(r.counts.problems(), 1);
        let mut clean = scan_full(&s);
        clean.only_note("Notebooks/Missing.md");
        assert_eq!(clean.counts.problems(), 0);
    }

    #[test]
    fn a_report_survives_being_stored_as_json() {
        let s = snap(
            &[(
                "Notebooks/A.md",
                "[[Missing]] with enough words so it is a note",
            )],
            &[],
        );
        let r = scan(&s);
        let back: Report = serde_json::from_str(&serde_json::to_string(&r).unwrap()).unwrap();
        assert_eq!(back, r);
    }

    #[test]
    fn unread_notes_are_carried_through() {
        let mut s = snap(&[("Notebooks/A.md", "text")], &[]);
        s.unread.push("Notebooks/Huge.md".into());
        let r = scan(&s);
        assert_eq!(r.counts.unread, 1);
        assert_eq!(r.unread, ["Notebooks/Huge.md"]);
    }
}
