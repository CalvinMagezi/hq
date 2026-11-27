//! What could be repaired in the notes, and which notes may be touched to do it.
//!
//! [`plan`] reads one snapshot of the vault and lists fixes, each with the exact text it would
//! change. It is pure: the same snapshot and context give the same list, and planning again
//! after a fix is applied gives nothing for it. Writing is `repair.rs`.
//!
//! Two tiers, and nothing else is a fix:
//! - **A**, safe to do without asking once the owner has allowed it: the one note a link
//!   names is certain. A link whose folder is out of date but whose name has exactly one note,
//!   a link to a note that was renamed, and a second block of properties at the top of a body.
//! - **B**, offered and done only when the owner says so: a good guess, not a certainty. The
//!   nearest name when exactly one is near, the same-folder note when a name fits several, and
//!   removing a note that repeats another word for word.
//!
//! A link to a note that is not there and that nothing resembles, a missing heading, and any
//! ambiguity that no folder settles are reported and never fixed.

use crate::check::NameIndex;
use crate::links::{
    Edit, LinkRef, NoteInput, NoteSet, Resolution, Syntax, aliases_of, extract, nearest, retarget,
};
use crate::relink::{Snapshot, backlink_index, plan_redirect, spell};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap, HashSet};

/// How sure a fix is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    A,
    B,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// The folder in a link is out of date; the name has one note.
    StalePath,
    /// The note a link names was renamed.
    RenamedNote,
    /// A second block of properties at the top of the text.
    NestedFrontmatter,
    /// The one note whose name is close to a misspelt link.
    NearMiss,
    /// A name that fits several notes, settled by the folder the link is in.
    SameFolder,
    /// A note that repeats another is removed and its links follow the one kept.
    MergeDuplicate,
}

/// One repair, with the text it would change.
#[derive(Debug, Clone, Serialize)]
pub struct Fix {
    /// Stable for the same change to the same text, so a screen can name it.
    pub id: String,
    pub tier: Tier,
    pub kind: Kind,
    /// The note the person sees it under: where the link is, or the note that would go.
    pub note: String,
    pub line: Option<usize>,
    /// The link as written, or the note removed.
    pub before: String,
    /// The link as it would read, or the note kept.
    pub after: String,
    /// Spans to change, by note.
    #[serde(skip)]
    pub edits: BTreeMap<String, Vec<Edit>>,
    /// Notes this moves to the trash.
    #[serde(skip)]
    pub trash: Vec<String>,
    /// The text each note had when this was planned. A note that changed since is left alone.
    #[serde(skip)]
    pub bases: BTreeMap<String, String>,
}

/// Why a note is left alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SkipReason {
    /// Outside `Notebooks/`, or a folder HQ itself uses.
    Protected,
    /// Written by HQ's own background work.
    MachineWritten,
    /// A sheet, deck or dashboard: its text holds structure only its editor may change.
    Suite,
    /// Pinned: it is loaded into every chat, so only its owner edits it.
    Pinned,
    /// An assistant wrote it and nobody has reviewed it.
    AgentUnreviewed,
    /// Changed a moment ago, so someone may still be editing it.
    RecentlyEdited,
    /// Too large to read safely.
    TooLarge,
    /// Removing it would leave links that cannot be pointed at the note kept.
    CannotRelink,
    /// The text changed since the fix was planned.
    Changed,
    /// Two fixes want the same words.
    Conflict,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Skipped {
    pub note: String,
    pub why: SkipReason,
}

/// What the planner is told that a snapshot cannot say.
#[derive(Debug, Clone, Default)]
pub struct Context {
    /// Notes renamed since the last scan: old path to new path.
    pub renames: HashMap<String, String>,
    /// Notes an assistant wrote and nobody reviewed.
    pub agent_unreviewed: HashSet<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Plan {
    pub fixes: Vec<Fix>,
    pub skipped: Vec<Skipped>,
}

impl Plan {
    /// How many fixes are tier A and tier B.
    pub fn counts(&self) -> (usize, usize) {
        let a = self.fixes.iter().filter(|f| f.tier == Tier::A).count();
        (a, self.fixes.len() - a)
    }

    /// The fixes with these ids, in plan order.
    pub fn select<'a>(&'a self, ids: &HashSet<String>) -> Vec<&'a Fix> {
        self.fixes.iter().filter(|f| ids.contains(&f.id)).collect()
    }

    /// Every tier A fix.
    pub fn tier_a(&self) -> Vec<&Fix> {
        self.fixes.iter().filter(|f| f.tier == Tier::A).collect()
    }
}

/// The largest note a fix will rewrite.
pub const MAX_NOTE_BYTES: u64 = crate::relink::MAX_NOTE_BYTES;

/// Folders whose notes are never rewritten.
const NEVER: &[&str] = &["_graph", "_system", "_threads", "_data", "_trash"];

/// Whether the text at `path` may be rewritten, judged from the path and the text alone. The
/// time since an edit is judged when writing, where the clock is.
pub fn may_touch(path: &str, text: &str, ctx: &Context) -> Result<(), SkipReason> {
    let top = path.split('/').next().unwrap_or("");
    if top != crate::walk::NOTES_DIR
        || NEVER.contains(&top)
        || crate::rel_path::INSTRUCTION_FOLDERS.contains(&top)
        || path.split('/').any(|seg| seg.starts_with('.'))
    {
        return Err(SkipReason::Protected);
    }
    if path.starts_with("Notebooks/Memories/") {
        return Err(SkipReason::MachineWritten);
    }
    if text.len() as u64 > MAX_NOTE_BYTES {
        return Err(SkipReason::TooLarge);
    }
    if ctx.agent_unreviewed.contains(path) {
        return Err(SkipReason::AgentUnreviewed);
    }
    let (fm, body) = hq_core::frontmatter_utils::split_frontmatter(text);
    if let Some(fm) = fm
        && let Ok(serde_yaml::Value::Mapping(map)) = serde_yaml::from_str::<serde_yaml::Value>(fm)
    {
        if map.contains_key("suite_kind") {
            return Err(SkipReason::Suite);
        }
        let pinned = map.get("pinned").is_some_and(|v| match v {
            serde_yaml::Value::Bool(b) => *b,
            serde_yaml::Value::String(s) => matches!(s.to_lowercase().as_str(), "true" | "yes"),
            _ => false,
        });
        if pinned {
            return Err(SkipReason::Pinned);
        }
    }
    const SUITE_FENCES: &[&str] = &["```sheet", "```tile", "```plot", "```chart", "```comment"];
    if body
        .lines()
        .any(|l| SUITE_FENCES.iter().any(|f| l.trim_start().starts_with(f)))
    {
        return Err(SkipReason::Suite);
    }
    Ok(())
}

/// The fingerprint of a text, the same as the search index uses (`b3:` and a BLAKE3 digest).
pub fn text_hash(text: &str) -> String {
    format!("b3:{}", blake3::hash(text.as_bytes()).to_hex())
}

fn fix_id(kind: Kind, note: &str, before: &str, after: &str) -> String {
    let mut h = blake3::Hasher::new();
    for part in [&format!("{kind:?}"), note, before, after] {
        h.update(part.as_bytes());
        h.update(&[0]);
    }
    h.finalize().to_hex()[..16].to_string()
}

/// Plan every fix the snapshot allows.
pub fn plan(snap: &Snapshot, ctx: &Context) -> Plan {
    plan_with(snap, ctx, &NameIndex::for_snapshot(snap))
}

/// [`plan`] with a name index built by the caller, to share it with the report.
pub fn plan_with(snap: &Snapshot, ctx: &Context, names: &NameIndex) -> Plan {
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
    let mut out = Plan::default();
    let mut skipped: BTreeMap<String, SkipReason> = BTreeMap::new();

    for note in &snap.notes {
        let touch = may_touch(&note.path, &note.text, ctx);
        let mut found: Vec<Fix> = Vec::new();

        if let Some(new_text) = crate::remediation::unnest(&note.text)
            && new_text != note.text
        {
            let edit = Edit {
                range: 0..note.text.len(),
                text: new_text,
            };
            found.push(make_fix(
                Tier::A,
                Kind::NestedFrontmatter,
                &note.path,
                None,
                String::new(),
                String::new(),
                BTreeMap::from([(note.path.clone(), vec![edit])]),
                vec![],
                &texts,
            ));
        }

        for link in extract(&note.text) {
            if link.in_frontmatter {
                continue;
            }
            let Some((tier, kind, to)) = choose(&set, names, ctx, &note.path, &link) else {
                continue;
            };
            let want = Resolution::Unique(to);
            let Some(spelled) = spell(&link, &set, &set, &note.path, &note.path, &want) else {
                continue;
            };
            let edit = retarget(&link, &spelled);
            let before = note.text[link.span.clone()].to_string();
            let after = format!(
                "{}{}{}",
                &note.text[link.span.start..link.target_span.start],
                edit.text,
                &note.text[link.target_span.end..link.span.end]
            );
            if after == before {
                continue;
            }
            found.push(make_fix(
                tier,
                kind,
                &note.path,
                Some(link.line),
                before,
                after,
                BTreeMap::from([(note.path.clone(), vec![edit])]),
                vec![],
                &texts,
            ));
        }

        match touch {
            Ok(()) => out.fixes.extend(found),
            Err(why) if !found.is_empty() => {
                skipped.entry(note.path.clone()).or_insert(why);
            }
            Err(_) => {}
        }
    }

    plan_duplicates(snap, ctx, &texts, &mut out, &mut skipped);

    out.fixes
        .sort_by(|a, b| (a.tier, &a.note, a.line, &a.id).cmp(&(b.tier, &b.note, b.line, &b.id)));
    out.skipped = skipped
        .into_iter()
        .map(|(note, why)| Skipped { note, why })
        .collect();
    out
}

#[allow(clippy::too_many_arguments)]
fn make_fix(
    tier: Tier,
    kind: Kind,
    note: &str,
    line: Option<usize>,
    before: String,
    after: String,
    edits: BTreeMap<String, Vec<Edit>>,
    trash: Vec<String>,
    texts: &HashMap<&str, &str>,
) -> Fix {
    let bases = edits
        .keys()
        .chain(trash.iter())
        .filter_map(|p| texts.get(p.as_str()).map(|t| (p.clone(), text_hash(t))))
        .collect();
    Fix {
        id: fix_id(kind, note, &before, &after),
        tier,
        kind,
        note: note.to_string(),
        line,
        before,
        after,
        edits,
        trash,
        bases,
    }
}

/// What to do about one link, if anything: how sure, why, and the note it should reach.
fn choose(
    set: &NoteSet,
    names: &NameIndex,
    ctx: &Context,
    from: &str,
    link: &LinkRef,
) -> Option<(Tier, Kind, String)> {
    match set.resolve(from, link) {
        Resolution::Dangling => {
            if let Some(new) = renamed_to(link, &ctx.renames)
                && set.contains(&new)
            {
                return Some((Tier::A, Kind::RenamedNote, new));
            }
            let name = link.target.trim_end_matches('/').rsplit('/').next()?;
            if link.target.contains('/')
                && let Resolution::Unique(p) = set.resolve_target(from, Syntax::Wiki, name)
            {
                return Some((Tier::A, Kind::StalePath, p));
            }
            if !link.target.contains('/') {
                return names
                    .nearest(&link.target)
                    .map(|p| (Tier::B, Kind::NearMiss, p));
            }
            None
        }
        Resolution::Ambiguous(candidates) => {
            nearest(from, &candidates).map(|p| (Tier::B, Kind::SameFolder, p.to_string()))
        }
        _ => None,
    }
}

/// The note a link named before it was renamed, if the link names one of the renamed notes.
fn renamed_to(link: &LinkRef, renames: &HashMap<String, String>) -> Option<String> {
    let want = crate::links::fold(link.target.trim_end_matches(".md"));
    let mut hit: Option<&String> = None;
    for (old, new) in renames {
        let old_folded = crate::links::fold(old.trim_end_matches(".md"));
        let same = old_folded == want
            || old_folded.ends_with(&format!("/{want}"))
            || (!want.contains('/') && old_folded.rsplit('/').next() == Some(want.as_str()));
        if same {
            if hit.is_some_and(|h| h != new) {
                return None;
            }
            hit = Some(new);
        }
    }
    hit.cloned()
}

fn plan_duplicates(
    snap: &Snapshot,
    ctx: &Context,
    texts: &HashMap<&str, &str>,
    out: &mut Plan,
    skipped: &mut BTreeMap<String, SkipReason>,
) {
    let groups = crate::health::duplicate_groups(snap);
    if groups.is_empty() {
        return;
    }
    let inbound = backlink_index(snap);
    let linked = |p: &str| {
        inbound
            .get(p)
            .map(|l| {
                l.iter()
                    .filter(|b| !b.ambiguous)
                    .map(|b| b.path.as_str())
                    .collect::<HashSet<_>>()
                    .len()
            })
            .unwrap_or(0)
    };
    for group in groups {
        // Keep the note most others link to, then the shorter path, then the first by name.
        let Some(keep) = group
            .notes
            .iter()
            .min_by_key(|p| (std::cmp::Reverse(linked(p)), p.len(), (*p).clone()))
        else {
            continue;
        };
        for gone in group.notes.iter().filter(|p| *p != keep) {
            let Some(text) = texts.get(gone.as_str()) else {
                continue;
            };
            if let Err(why) = may_touch(gone, text, ctx) {
                skipped.entry(gone.clone()).or_insert(why);
                continue;
            }
            let redirect = plan_redirect(snap, gone, keep);
            if !redirect.left.is_empty() {
                skipped
                    .entry(gone.clone())
                    .or_insert(SkipReason::CannotRelink);
                continue;
            }
            let mut blocked = false;
            let mut edits: BTreeMap<String, Vec<Edit>> = BTreeMap::new();
            for rw in &redirect.rewrites {
                if let Err(why) = may_touch(&rw.old_path, &rw.before, ctx) {
                    skipped.entry(rw.old_path.clone()).or_insert(why);
                    blocked = true;
                    break;
                }
                edits.insert(rw.old_path.clone(), rw.edits.clone());
            }
            if blocked {
                continue;
            }
            out.fixes.push(make_fix(
                Tier::B,
                Kind::MergeDuplicate,
                gone,
                None,
                gone.clone(),
                keep.clone(),
                edits,
                vec![gone.clone()],
                texts,
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::links::apply_edits;
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

    /// The text of `note` after every fix in `plan` that edits it.
    fn applied(s: &Snapshot, plan: &Plan, note: &str) -> String {
        let text = &s.notes.iter().find(|n| n.path == note).unwrap().text;
        let edits: Vec<Edit> = plan
            .fixes
            .iter()
            .filter_map(|f| f.edits.get(note))
            .flatten()
            .cloned()
            .collect();
        apply_edits(text, &edits).unwrap()
    }

    const WORDS: &str = "enough words in this note to count as a real note and not be empty";

    /// A text no other test note has, so notes are not taken for repeats.
    fn own(tag: &str) -> String {
        format!("{WORDS}, and this one is about {tag}")
    }

    #[test]
    fn a_stale_folder_with_one_note_of_that_name_is_a_tier_a_fix() {
        let s = snap(
            &[
                ("Notebooks/New/Plan.md", WORDS),
                (
                    "Notebooks/L.md",
                    "see [[Old/Plan#Goals|goals]] and [t](Old/Plan.md)",
                ),
            ],
            &[],
        );
        let p = plan(&s, &Context::default());
        assert_eq!(p.fixes.len(), 2, "{:?}", p.fixes);
        assert!(
            p.fixes
                .iter()
                .all(|f| f.tier == Tier::A && f.kind == Kind::StalePath)
        );
        assert_eq!(
            applied(&s, &p, "Notebooks/L.md"),
            "see [[New/Plan#Goals|goals]] and [t](New/Plan.md)"
        );
        let wiki = p.fixes.iter().find(|f| f.before.starts_with("[[")).unwrap();
        assert_eq!(wiki.before, "[[Old/Plan#Goals|goals]]");
        assert_eq!(wiki.after, "[[New/Plan#Goals|goals]]");
        assert_eq!(wiki.line, Some(1));
    }

    #[test]
    fn a_renamed_note_is_followed_and_a_name_with_no_trace_is_not_guessed() {
        let s = snap(
            &[
                ("Notebooks/Budget 2026.md", WORDS),
                (
                    "Notebooks/L.md",
                    "[[Budget]] and [[Something Else Entirely]]",
                ),
            ],
            &[],
        );
        let ctx = Context {
            renames: HashMap::from([(
                "Notebooks/Budget.md".into(),
                "Notebooks/Budget 2026.md".into(),
            )]),
            ..Context::default()
        };
        let p = plan(&s, &ctx);
        assert_eq!(p.fixes.len(), 1);
        assert_eq!(
            (p.fixes[0].tier, p.fixes[0].kind),
            (Tier::A, Kind::RenamedNote)
        );
        assert_eq!(
            applied(&s, &p, "Notebooks/L.md"),
            "[[Budget 2026]] and [[Something Else Entirely]]"
        );
    }

    #[test]
    fn a_misspelling_is_offered_as_tier_b_only_when_one_note_is_near() {
        let s = snap(
            &[
                ("Notebooks/Quarterly Review.md", &own("reviews")),
                ("Notebooks/L.md", "[[Quarterly Reveiw]]"),
                ("Notebooks/Plan A.md", &own("plan a")),
                ("Notebooks/Plan B.md", &own("plan b")),
                ("Notebooks/M.md", "[[Plan C]]"),
            ],
            &[],
        );
        let p = plan(&s, &Context::default());
        assert_eq!(p.fixes.len(), 1, "{:?}", p.fixes);
        assert_eq!(
            (p.fixes[0].tier, p.fixes[0].kind),
            (Tier::B, Kind::NearMiss)
        );
        assert_eq!(applied(&s, &p, "Notebooks/L.md"), "[[Quarterly Review]]");
    }

    #[test]
    fn a_name_that_fits_two_notes_is_settled_only_by_the_folder_the_link_is_in() {
        let s = snap(
            &[
                ("Notebooks/A/Plan.md", &own("a")),
                ("Notebooks/B/Plan.md", &own("b")),
                ("Notebooks/A/Here.md", "[[Plan]]"),
                ("Notebooks/C/There.md", "[[Plan]]"),
            ],
            &[],
        );
        let p = plan(&s, &Context::default());
        assert_eq!(p.fixes.len(), 1);
        assert_eq!(
            (p.fixes[0].kind, p.fixes[0].note.as_str()),
            (Kind::SameFolder, "Notebooks/A/Here.md")
        );
        assert_eq!(applied(&s, &p, "Notebooks/A/Here.md"), "[[A/Plan]]");
    }

    #[test]
    fn a_second_block_of_properties_is_merged_as_tier_a() {
        let s = snap(
            &[(
                "Notebooks/N.md",
                "---\ntitle: T\n---\n\n---\ntags: [a]\n---\n\nbody words here",
            )],
            &[],
        );
        let p = plan(&s, &Context::default());
        assert_eq!(p.fixes.len(), 1);
        assert_eq!(
            (p.fixes[0].tier, p.fixes[0].kind),
            (Tier::A, Kind::NestedFrontmatter)
        );
        let after = applied(&s, &p, "Notebooks/N.md");
        assert_eq!(after.matches("---").count(), 2, "{after}");
        assert!(after.contains("tags"));
    }

    #[test]
    fn a_repeated_note_is_removed_and_its_links_follow_the_one_kept() {
        let text = "The same long paragraph about the quarterly numbers and what they mean.";
        let s = snap(
            &[
                ("Notebooks/Keep.md", text),
                ("Notebooks/Dup.md", text),
                ("Notebooks/L.md", "[[Keep]] twice [[Dup]] and [[Dup#Part]]"),
                ("Notebooks/M.md", "[[Keep]]"),
            ],
            &[],
        );
        let p = plan(&s, &Context::default());
        let merge = p
            .fixes
            .iter()
            .find(|f| f.kind == Kind::MergeDuplicate)
            .expect("a merge");
        assert_eq!(
            (merge.tier, merge.note.as_str(), merge.after.as_str()),
            (Tier::B, "Notebooks/Dup.md", "Notebooks/Keep.md")
        );
        assert_eq!(merge.trash, ["Notebooks/Dup.md"]);
        let edits = &merge.edits["Notebooks/L.md"];
        let l = &s
            .notes
            .iter()
            .find(|n| n.path == "Notebooks/L.md")
            .unwrap()
            .text;
        assert_eq!(
            apply_edits(l, edits).unwrap(),
            "[[Keep]] twice [[Keep]] and [[Keep#Part]]"
        );
    }

    #[test]
    fn notes_that_must_not_be_rewritten_are_left_alone_and_listed_with_the_reason() {
        let broken = "see [[Old/Plan]]";
        let s = snap(
            &[
                ("Notebooks/New/Plan.md", WORDS),
                (
                    "Notebooks/Pinned.md",
                    &format!("---\npinned: true\n---\n{broken}"),
                ),
                (
                    "Notebooks/Sheet.md",
                    &format!("---\nsuite_kind: sheet\n---\n{broken}"),
                ),
                (
                    "Notebooks/Fence.md",
                    &format!("```sheet\n| a |\n```\n{broken}"),
                ),
                ("Notebooks/Memories/Insight.md", broken),
                ("Notebooks/Agent.md", broken),
                ("_graph/Page.md", broken),
                ("Notebooks/Fine.md", broken),
            ],
            &[],
        );
        let ctx = Context {
            agent_unreviewed: HashSet::from(["Notebooks/Agent.md".to_string()]),
            ..Context::default()
        };
        let p = plan(&s, &ctx);
        assert_eq!(
            p.fixes.iter().map(|f| f.note.as_str()).collect::<Vec<_>>(),
            ["Notebooks/Fine.md"]
        );
        let why: BTreeMap<&str, SkipReason> =
            p.skipped.iter().map(|k| (k.note.as_str(), k.why)).collect();
        assert_eq!(why["Notebooks/Pinned.md"], SkipReason::Pinned);
        assert_eq!(why["Notebooks/Sheet.md"], SkipReason::Suite);
        assert_eq!(why["Notebooks/Fence.md"], SkipReason::Suite);
        assert_eq!(
            why["Notebooks/Memories/Insight.md"],
            SkipReason::MachineWritten
        );
        assert_eq!(why["Notebooks/Agent.md"], SkipReason::AgentUnreviewed);
        assert_eq!(why["_graph/Page.md"], SkipReason::Protected);
        assert_eq!(why.len(), 6);
    }

    #[test]
    fn a_duplicate_whose_links_sit_in_a_note_that_may_not_be_touched_is_not_removed() {
        let text = "The same long paragraph about the quarterly numbers and what they mean.";
        let s = snap(
            &[
                ("Notebooks/Keep.md", text),
                ("Notebooks/Dup.md", text),
                ("Notebooks/Pinned.md", "---\npinned: true\n---\n[[Dup]]"),
                // Two notes link to Keep, one to Dup, so Dup is the one that would go.
                ("Notebooks/M.md", "[[Keep]]"),
                ("Notebooks/N.md", "[[Keep]]"),
            ],
            &[],
        );
        let p = plan(&s, &Context::default());
        assert!(
            p.fixes.iter().all(|f| f.kind != Kind::MergeDuplicate),
            "{:?}",
            p.fixes
        );
        assert!(
            p.skipped
                .iter()
                .any(|k| k.note == "Notebooks/Pinned.md" && k.why == SkipReason::Pinned)
        );
    }

    #[test]
    fn planning_is_deterministic_and_ids_are_stable() {
        let s = snap(
            &[
                ("Notebooks/New/Plan.md", &own("plans")),
                ("Notebooks/L.md", "[[Old/Plan]] [[Quarterly Reveiw]]"),
                ("Notebooks/Quarterly Review.md", &own("reviews")),
            ],
            &[],
        );
        let ids = |p: &Plan| p.fixes.iter().map(|f| f.id.clone()).collect::<Vec<_>>();
        let a = plan(&s, &Context::default());
        let b = plan(&s, &Context::default());
        assert_eq!(ids(&a), ids(&b));
        assert_eq!(a.fixes.len(), 2);
        assert_eq!((a.fixes[0].tier, a.fixes[1].tier), (Tier::A, Tier::B));
        let unique: HashSet<_> = ids(&a).into_iter().collect();
        assert_eq!(unique.len(), 2);
    }

    #[test]
    fn after_a_fix_is_applied_there_is_nothing_left_to_plan_for_it() {
        let s = snap(
            &[
                ("Notebooks/New/Plan.md", WORDS),
                ("Notebooks/L.md", "see [[Old/Plan]]"),
            ],
            &[],
        );
        let p = plan(&s, &Context::default());
        let fixed = snap(
            &[
                ("Notebooks/New/Plan.md", WORDS),
                ("Notebooks/L.md", &applied(&s, &p, "Notebooks/L.md")),
            ],
            &[],
        );
        assert!(plan(&fixed, &Context::default()).fixes.is_empty());
    }

    #[test]
    fn a_link_in_code_or_properties_is_never_a_fix() {
        let s = snap(
            &[
                ("Notebooks/New/Plan.md", WORDS),
                (
                    "Notebooks/L.md",
                    "---\nrelated: \"[[Old/Plan]]\"\n---\n`[[Old/Plan]]`\n```\n[[Old/Plan]]\n```",
                ),
            ],
            &[],
        );
        assert!(plan(&s, &Context::default()).fixes.is_empty());
    }

    #[test]
    fn each_fix_remembers_the_text_it_was_planned_from() {
        let s = snap(
            &[
                ("Notebooks/New/Plan.md", WORDS),
                ("Notebooks/L.md", "[[Old/Plan]]"),
            ],
            &[],
        );
        let p = plan(&s, &Context::default());
        assert_eq!(
            p.fixes[0].bases["Notebooks/L.md"],
            text_hash("[[Old/Plan]]")
        );
    }
}
