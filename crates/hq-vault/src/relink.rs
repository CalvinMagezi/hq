//! Keeping links pointing where they pointed when notes move.
//!
//! The rule is one sentence: after a move, every link must resolve to the same file it
//! resolved to before, where "the same file" follows the move. [`plan_moves`] reads every
//! link in the vault twice, once as the vault is and once as it will be, and rewrites the
//! target part of exactly the links whose meaning would change. That covers:
//!
//! - a note renamed (`[[Old]]` becomes `[[New]]`), with its heading, alias and embed mark kept;
//! - a note moved to another folder, where path-qualified wikilinks and markdown links break;
//! - a moved note's own relative markdown links, which break because its folder changed;
//! - a name that used to be unique and is not any more, where the link gets a longer spelling;
//! - links that mean either of two notes, which are never guessed at and are reported instead.
//!
//! [`plan_moves`] is pure. [`apply`] writes the plan, one note at a time under its lock, and
//! skips a note that changed since it was read rather than overwrite the change.

use crate::links::{
    Edit, LinkRef, NoteInput, NoteSet, Resolution, Syntax, aliases_of, apply_edits, extract,
    retarget,
};
use crate::note_lock::lock_note;
use crate::notes::write_atomic;
use crate::walk::walk;
use std::collections::{BTreeSet, HashMap};
use std::path::Path;

/// Notes larger than this are not read: the scan would cost more than a link is worth.
pub const MAX_NOTE_BYTES: u64 = 2 * 1024 * 1024;

/// Every note and attachment as it is now.
#[derive(Debug, Default)]
pub struct Snapshot {
    pub notes: Vec<SnapNote>,
    pub attachments: Vec<String>,
    /// Notes that could not be read (too large, not UTF-8). Links inside them are unknown.
    pub unread: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct SnapNote {
    pub path: String,
    pub text: String,
}

/// Read the vault the way a link resolves it.
pub fn snapshot(vault: &Path) -> Snapshot {
    let walked = walk(vault, false);
    let mut snap = Snapshot {
        attachments: walked.attachments,
        ..Snapshot::default()
    };
    for note in walked.notes {
        if note.size > MAX_NOTE_BYTES {
            snap.unread.push(note.rel);
            continue;
        }
        match std::fs::read_to_string(&note.abs) {
            Ok(text) => snap.notes.push(SnapNote {
                path: note.rel,
                text,
            }),
            Err(_) => snap.unread.push(note.rel),
        }
    }
    snap
}

/// Why a link was not kept pointing at its note.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Why {
    /// The name fits more than one note, so there is no telling which one the writer meant.
    Ambiguous,
    /// No spelling of the new location reaches the file.
    NoSpelling,
    /// The note holding the link could not be read.
    Unread,
    /// The note holding the link changed while the move ran.
    Changed,
}

/// A link a move could not keep.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct LeftLink {
    /// The note the link is in, by its new path.
    pub note: String,
    pub line: usize,
    /// The link as written.
    pub text: String,
    pub why: Why,
}

/// The new text of one note.
#[derive(Debug, Clone)]
pub struct Rewrite {
    pub old_path: String,
    pub new_path: String,
    pub before: String,
    pub after: String,
    /// How many links changed.
    pub links: usize,
    /// The same change as spans of `before`, so another plan can be combined with it.
    pub edits: Vec<Edit>,
}

#[derive(Debug, Default)]
pub struct Plan {
    pub rewrites: Vec<Rewrite>,
    pub left: Vec<LeftLink>,
    /// Notes that are not moving and link to something that is, by their current path.
    pub referrers: Vec<String>,
}

/// Plan the rewrites for moving files. `moves` maps each old path to its new path: the notes
/// and attachments that move, with `.md` endings.
pub fn plan_moves(snap: &Snapshot, moves: &HashMap<String, String>) -> Plan {
    let map = |p: &str| moves.get(p).cloned().unwrap_or_else(|| p.to_string());
    let inputs: Vec<(String, Vec<String>)> = snap
        .notes
        .iter()
        .map(|n| (n.path.clone(), aliases_of(&n.text)))
        .collect();
    let old_set = NoteSet::new(
        inputs.iter().map(|(p, a)| NoteInput {
            path: p.clone(),
            aliases: a.clone(),
        }),
        snap.attachments.iter().cloned(),
    );
    let new_set = NoteSet::new(
        inputs.iter().map(|(p, a)| NoteInput {
            path: map(p),
            aliases: a.clone(),
        }),
        snap.attachments.iter().map(|a| map(a)),
    );

    let mut plan = Plan::default();
    let mut referrers: BTreeSet<String> = BTreeSet::new();

    for note in &snap.notes {
        let new_path = map(&note.path);
        let mut edits: Vec<Edit> = Vec::new();
        let mut left: Vec<LeftLink> = Vec::new();
        for link in extract(&note.text) {
            let expected = match old_set.resolve(&note.path, &link) {
                Resolution::Unique(p) => {
                    note_links_to_move(&mut referrers, note, &p, moves);
                    Resolution::Unique(map(&p))
                }
                Resolution::Attachment(p) => {
                    note_links_to_move(&mut referrers, note, &p, moves);
                    Resolution::Attachment(map(&p))
                }
                Resolution::Ambiguous(paths) => {
                    for p in &paths {
                        note_links_to_move(&mut referrers, note, p, moves);
                    }
                    let mapped: BTreeSet<String> = paths.iter().map(|p| map(p)).collect();
                    let same_meaning = match new_set.resolve(&new_path, &link) {
                        Resolution::Ambiguous(now) => {
                            now.into_iter().collect::<BTreeSet<_>>() == mapped
                        }
                        _ => false,
                    };
                    if !same_meaning {
                        left.push(left_link(&new_path, &note.text, &link, Why::Ambiguous));
                    }
                    continue;
                }
                _ => continue,
            };
            if new_set.resolve(&new_path, &link) == expected {
                continue;
            }
            match spell(&link, &old_set, &new_set, &note.path, &new_path, &expected) {
                Some(text) => edits.push(retarget(&link, &text)),
                None => left.push(left_link(&new_path, &note.text, &link, Why::NoSpelling)),
            }
        }
        plan.left.append(&mut left);
        if edits.is_empty() {
            continue;
        }
        match apply_edits(&note.text, &edits) {
            Some(after) if after != note.text => plan.rewrites.push(Rewrite {
                old_path: note.path.clone(),
                new_path,
                before: note.text.clone(),
                links: edits.len(),
                after,
                edits,
            }),
            _ => {}
        }
    }
    for path in &snap.unread {
        plan.left.push(LeftLink {
            note: map(path),
            line: 0,
            text: String::new(),
            why: Why::Unread,
        });
    }
    plan.referrers = referrers.into_iter().collect();
    plan
}

/// A note that is not itself moving and links to a file that is moving is a referrer.
fn note_links_to_move(
    referrers: &mut BTreeSet<String>,
    note: &SnapNote,
    target: &str,
    moves: &HashMap<String, String>,
) {
    if moves.contains_key(target) && !moves.contains_key(&note.path) {
        referrers.insert(note.path.clone());
    }
}

fn left_link(note: &str, text: &str, link: &LinkRef, why: Why) -> LeftLink {
    LeftLink {
        note: note.to_string(),
        line: link.line,
        text: text[link.span.clone()].to_string(),
        why,
    }
}

// ─── Who links here ──────────────────────────────────────────────────────────

/// A link from another note to a note.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Backlink {
    /// The note holding the link.
    pub path: String,
    pub line: usize,
    /// The link as written.
    pub link: String,
    /// The name fits other notes too, so this link may mean one of them.
    pub ambiguous: bool,
}

/// Every link that reaches a note, keyed by the note it reaches. Built once, so asking about
/// many notes does not read the vault again. A link that fits several notes is listed under
/// each of them, marked, so a reader sees it rather than losing it. Links a note makes to
/// itself are left out. Each list is sorted by note, then line.
pub fn backlink_index(snap: &Snapshot) -> HashMap<String, Vec<Backlink>> {
    let set = NoteSet::new(
        snap.notes.iter().map(|n| NoteInput {
            path: n.path.clone(),
            aliases: aliases_of(&n.text),
        }),
        [],
    );
    let mut index: HashMap<String, Vec<Backlink>> = HashMap::new();
    for note in &snap.notes {
        for link in extract(&note.text) {
            let (targets, ambiguous) = match set.resolve(&note.path, &link) {
                Resolution::Unique(p) => (vec![p], false),
                Resolution::Ambiguous(ps) => (ps, true),
                _ => continue,
            };
            for target in targets.into_iter().filter(|t| *t != note.path) {
                index.entry(target).or_default().push(Backlink {
                    path: note.path.clone(),
                    line: link.line,
                    link: note.text[link.span.clone()].to_string(),
                    ambiguous,
                });
            }
        }
    }
    for list in index.values_mut() {
        list.sort_by(|a, b| (&a.path, a.line).cmp(&(&b.path, b.line)));
    }
    index
}

/// The links in other notes that reach `target`. For one note; use [`backlink_index`] for many.
pub fn backlinks(snap: &Snapshot, target: &str) -> Vec<Backlink> {
    backlink_index(snap).remove(target).unwrap_or_default()
}

// ─── Pointing links at another note ──────────────────────────────────────────

/// Rewrites that make every link to `gone` reach `keep` instead, for when `gone` is about to be
/// deleted because it repeats `keep`. Only links that name `gone` alone are rewritten; a link
/// that fit several notes is left as it is and reported in `left`. `referrers` are the notes
/// that link to `gone` (itself excluded).
pub fn plan_redirect(snap: &Snapshot, gone: &str, keep: &str) -> Plan {
    let inputs: Vec<(String, Vec<String>)> = snap
        .notes
        .iter()
        .map(|n| (n.path.clone(), aliases_of(&n.text)))
        .collect();
    let to_inputs = |skip: Option<&str>| {
        inputs
            .iter()
            .filter(move |(p, _)| Some(p.as_str()) != skip)
            .map(|(p, a)| NoteInput {
                path: p.clone(),
                aliases: a.clone(),
            })
            .collect::<Vec<_>>()
    };
    let old_set = NoteSet::new(to_inputs(None), snap.attachments.iter().cloned());
    let new_set = NoteSet::new(to_inputs(Some(gone)), snap.attachments.iter().cloned());
    let want = Resolution::Unique(keep.to_string());

    let mut plan = Plan::default();
    let mut referrers: BTreeSet<String> = BTreeSet::new();
    for note in snap.notes.iter().filter(|n| n.path != gone) {
        let mut edits: Vec<Edit> = Vec::new();
        for link in extract(&note.text) {
            match old_set.resolve(&note.path, &link) {
                Resolution::Unique(p) if p == gone => {
                    referrers.insert(note.path.clone());
                    if new_set.resolve(&note.path, &link) == want {
                        continue;
                    }
                    match spell(&link, &old_set, &new_set, &note.path, &note.path, &want) {
                        Some(text) => edits.push(retarget(&link, &text)),
                        None => plan.left.push(left_link(
                            &note.path,
                            &note.text,
                            &link,
                            Why::NoSpelling,
                        )),
                    }
                }
                Resolution::Ambiguous(ps) if ps.iter().any(|p| p == gone) => {
                    referrers.insert(note.path.clone());
                    plan.left
                        .push(left_link(&note.path, &note.text, &link, Why::Ambiguous));
                }
                _ => {}
            }
        }
        if edits.is_empty() {
            continue;
        }
        if let Some(after) = apply_edits(&note.text, &edits)
            && after != note.text
        {
            plan.rewrites.push(Rewrite {
                old_path: note.path.clone(),
                new_path: note.path.clone(),
                before: note.text.clone(),
                links: edits.len(),
                after,
                edits,
            });
        }
    }
    plan.referrers = referrers.into_iter().collect();
    plan
}

// ─── Spelling a target ───────────────────────────────────────────────────────

/// A target for `link` that resolves to `expected` once the move is done, written the way the
/// link was written: a bare name stays a bare name for as long as that is enough, a path stays
/// a path, a relative markdown link stays relative. `None` when nothing reaches it.
pub(crate) fn spell(
    link: &LinkRef,
    old_set: &NoteSet,
    new_set: &NoteSet,
    from_old: &str,
    from_new: &str,
    expected: &Resolution,
) -> Option<String> {
    let (Resolution::Unique(path) | Resolution::Attachment(path)) = expected else {
        return None;
    };
    let had_ext = link.target.to_lowercase().ends_with(".md");
    let is_note = matches!(expected, Resolution::Unique(_));
    // The file as a link spells it: notes lose `.md` unless the link kept it.
    let with_ext = |p: &str| -> String {
        if is_note && !had_ext {
            p.strip_suffix(".md").unwrap_or(p).to_string()
        } else {
            p.to_string()
        }
    };
    let fits = |text: &str| -> bool {
        !text.is_empty()
            && !text.contains(['[', ']', '|', '#', '\n'])
            && new_set.resolve_target(from_new, link.syntax, text) == *expected
    };

    match link.syntax {
        Syntax::Wiki => {
            let shown = with_ext(path);
            let segs: Vec<&str> = shown.split('/').collect();
            let written = link.target.trim_matches('/').split('/').count();
            let start = if link.target.contains('/') {
                written
            } else {
                1
            };
            (start.min(segs.len())..=segs.len())
                .map(|k| segs[segs.len() - k..].join("/"))
                .find(|c| fits(c))
        }
        Syntax::Markdown => {
            let dir_new = from_new.rsplit_once('/').map_or("", |(d, _)| d);
            let target = link.target.trim_start_matches("./");
            let rooted = target.contains('/')
                && !target.starts_with("../")
                && old_set.knows_path(target)
                && !is_relative_hit(old_set, from_old, target);
            let relative = relative_path(dir_new, &with_ext(path));
            let root = with_ext(path);
            let name = with_ext(path.rsplit('/').next().unwrap_or(path));
            let order: [&String; 3] = if rooted {
                [&root, &relative, &name]
            } else if !target.contains('/') {
                [&name, &relative, &root]
            } else {
                [&relative, &root, &name]
            };
            order.into_iter().find(|c| fits(c)).cloned()
        }
    }
}

/// Whether `target`, read relative to the note at `from`, names a file in the set.
fn is_relative_hit(set: &NoteSet, from: &str, target: &str) -> bool {
    let dir = from.rsplit_once('/').map_or("", |(d, _)| d);
    let joined = if dir.is_empty() {
        target.to_string()
    } else {
        format!("{dir}/{target}")
    };
    set.knows_path(&joined)
}

/// `to` as a path from the folder `from_dir`: `../Sibling/Note.md`.
pub(crate) fn relative_path(from_dir: &str, to: &str) -> String {
    let from: Vec<&str> = from_dir.split('/').filter(|s| !s.is_empty()).collect();
    let to: Vec<&str> = to.split('/').collect();
    let common = from
        .iter()
        .zip(to.iter().take(to.len().saturating_sub(1)))
        .take_while(|(a, b)| a == b)
        .count();
    let mut out: Vec<&str> = vec![".."; from.len() - common];
    out.extend(&to[common..]);
    out.join("/")
}

// ─── Writing the plan ────────────────────────────────────────────────────────

/// What [`apply`] did.
#[derive(Debug, Default)]
pub struct Applied {
    /// New paths of the notes rewritten.
    pub rewritten: Vec<String>,
    pub left: Vec<LeftLink>,
}

/// Write the rewrites. Call after the files have moved: each note is read at its new path.
/// A note that no longer holds the text the plan was made from is left as it is, and its links
/// are reported, so an edit made while the move ran is never overwritten.
pub fn apply(vault: &Path, plan: &Plan) -> Applied {
    let mut done = Applied::default();
    for rw in &plan.rewrites {
        let path = vault.join(&rw.new_path);
        // One note lock at a time; never held while waiting on another.
        let _held = lock_note(&rw.new_path);
        let unchanged = std::fs::read_to_string(&path).is_ok_and(|now| now == rw.before);
        if unchanged && write_atomic(&path, rw.after.as_bytes()).is_ok() {
            done.rewritten.push(rw.new_path.clone());
        } else {
            done.left.push(LeftLink {
                note: rw.new_path.clone(),
                line: 0,
                text: String::new(),
                why: Why::Changed,
            });
        }
    }
    done
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

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

    fn mv(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect()
    }

    fn after<'a>(plan: &'a Plan, new_path: &str) -> &'a str {
        &plan
            .rewrites
            .iter()
            .find(|r| r.new_path == new_path)
            .unwrap_or_else(|| panic!("{new_path} was not rewritten"))
            .after
    }

    #[test]
    fn a_rename_follows_every_spelling_of_the_link() {
        let s = snap(
            &[
                ("Notebooks/Old.md", "# Old"),
                (
                    "Notebooks/Linker.md",
                    "[[Old]] [[old#Goals|the goals]] ![[Old]] [[Old.md]] [[Notebooks/Old]] [t](Old.md#Part) [u](<Old.md>)",
                ),
            ],
            &[],
        );
        let plan = plan_moves(&s, &mv(&[("Notebooks/Old.md", "Notebooks/New Name.md")]));
        assert_eq!(
            after(&plan, "Notebooks/Linker.md"),
            "[[New Name]] [[New Name#Goals|the goals]] ![[New Name]] [[New Name.md]] \
             [[Notebooks/New Name]] [t](New%20Name.md#Part) [u](<New Name.md>)"
        );
        assert_eq!(plan.referrers, ["Notebooks/Linker.md"]);
        assert!(plan.left.is_empty());
    }

    #[test]
    fn a_link_in_code_is_never_touched() {
        let s = snap(
            &[
                ("Notebooks/Old.md", ""),
                (
                    "Notebooks/L.md",
                    "```\n[[Old]]\n```\nand `[[Old]]` and [[Old]]",
                ),
            ],
            &[],
        );
        let plan = plan_moves(&s, &mv(&[("Notebooks/Old.md", "Notebooks/New.md")]));
        assert_eq!(
            after(&plan, "Notebooks/L.md"),
            "```\n[[Old]]\n```\nand `[[Old]]` and [[New]]"
        );
    }

    #[test]
    fn a_move_to_another_folder_leaves_bare_names_alone_and_fixes_paths() {
        let s = snap(
            &[
                ("Notebooks/A/Plan.md", ""),
                (
                    "Notebooks/L.md",
                    "[[Plan]] [[A/Plan]] [t](A/Plan.md) [u](Notebooks/A/Plan.md)",
                ),
                ("Notebooks/Sub/R.md", "[v](../A/Plan.md)"),
            ],
            &[],
        );
        let plan = plan_moves(&s, &mv(&[("Notebooks/A/Plan.md", "Notebooks/B/Plan.md")]));
        assert_eq!(
            after(&plan, "Notebooks/L.md"),
            "[[Plan]] [[B/Plan]] [t](B/Plan.md) [u](Notebooks/B/Plan.md)"
        );
        assert_eq!(after(&plan, "Notebooks/Sub/R.md"), "[v](../B/Plan.md)");
    }

    #[test]
    fn a_moved_note_keeps_its_own_relative_links() {
        let s = snap(
            &[
                (
                    "Notebooks/A/Mover.md",
                    "[x](../Other.md) [y](Sibling.md) [[Other]] ![](pic.png)",
                ),
                ("Notebooks/A/Sibling.md", ""),
                ("Notebooks/Other.md", ""),
            ],
            &["Notebooks/A/pic.png"],
        );
        let plan = plan_moves(
            &s,
            &mv(&[("Notebooks/A/Mover.md", "Notebooks/Deep/Er/Mover.md")]),
        );
        assert_eq!(
            after(&plan, "Notebooks/Deep/Er/Mover.md"),
            // Name-only links (`Sibling.md`, `pic.png`) still find their file by name, so only
            // the link that climbs with `../` needs to change.
            "[x](../../Other.md) [y](Sibling.md) [[Other]] ![](pic.png)"
        );
        assert!(
            plan.referrers.is_empty(),
            "the moved note is not its own referrer"
        );
    }

    #[test]
    fn moving_a_whole_folder_fixes_path_links_into_it_and_leaves_inner_links_alone() {
        let s = snap(
            &[
                ("Notebooks/Old/One.md", "[a](Two.md) [[Two]] [b](../Out.md)"),
                ("Notebooks/Old/Two.md", ""),
                ("Notebooks/Out.md", "[[Old/One]] [c](Old/Two.md) [[Two]]"),
            ],
            &["Notebooks/Old/pic.png"],
        );
        let plan = plan_moves(
            &s,
            &mv(&[
                ("Notebooks/Old/One.md", "Notebooks/New/One.md"),
                ("Notebooks/Old/Two.md", "Notebooks/New/Two.md"),
                ("Notebooks/Old/pic.png", "Notebooks/New/pic.png"),
            ]),
        );
        // Inner relative links still hold, since both ends moved together.
        assert!(
            plan.rewrites
                .iter()
                .all(|r| r.new_path != "Notebooks/New/Two.md")
        );
        assert_eq!(
            after(&plan, "Notebooks/Out.md"),
            "[[New/One]] [c](New/Two.md) [[Two]]"
        );
        assert!(
            plan.rewrites
                .iter()
                .all(|r| r.new_path != "Notebooks/New/One.md")
        );
    }

    #[test]
    fn a_name_that_stops_being_unique_gets_a_longer_spelling() {
        let s = snap(
            &[
                ("Notebooks/A/Plan.md", ""),
                ("Notebooks/B/Other.md", ""),
                ("Notebooks/L.md", "[[Plan]] and [[Other]]"),
            ],
            &[],
        );
        // Renaming B/Other to B/Plan makes [[Plan]] mean two notes.
        let plan = plan_moves(&s, &mv(&[("Notebooks/B/Other.md", "Notebooks/B/Plan.md")]));
        assert_eq!(after(&plan, "Notebooks/L.md"), "[[A/Plan]] and [[B/Plan]]");
    }

    #[test]
    fn a_link_that_meant_either_of_two_notes_is_left_and_reported() {
        let s = snap(
            &[
                ("Notebooks/A/Plan.md", ""),
                ("Notebooks/B/Plan.md", ""),
                ("Notebooks/L.md", "see [[Plan]]"),
            ],
            &[],
        );
        let plan = plan_moves(
            &s,
            &mv(&[("Notebooks/A/Plan.md", "Notebooks/A/Plan v2.md")]),
        );
        assert!(plan.rewrites.is_empty());
        assert_eq!(plan.left.len(), 1);
        assert_eq!(plan.left[0].why, Why::Ambiguous);
        assert_eq!(plan.left[0].text, "[[Plan]]");
        assert_eq!(plan.left[0].line, 1);
    }

    #[test]
    fn an_ambiguous_link_whose_meaning_does_not_change_is_not_reported() {
        let s = snap(
            &[
                ("Notebooks/A/Plan.md", ""),
                ("Notebooks/B/Plan.md", ""),
                ("Notebooks/L.md", "see [[Plan]]"),
            ],
            &[],
        );
        let plan = plan_moves(&s, &mv(&[("Notebooks/A/Plan.md", "Notebooks/C/Plan.md")]));
        assert!(plan.rewrites.is_empty());
        assert!(plan.left.is_empty());
    }

    #[test]
    fn dangling_outside_and_same_note_links_are_not_touched() {
        let s = snap(
            &[
                ("Notebooks/Old.md", ""),
                (
                    "Notebooks/L.md",
                    "[[Nowhere]] [w](https://example.com/Old.md) [[#Top]] [x](#top)",
                ),
            ],
            &[],
        );
        let plan = plan_moves(&s, &mv(&[("Notebooks/Old.md", "Notebooks/New.md")]));
        assert!(plan.rewrites.is_empty() && plan.left.is_empty());
    }

    #[test]
    fn an_unread_note_is_reported_not_skipped_silently() {
        let mut s = snap(&[("Notebooks/Old.md", "")], &[]);
        s.unread.push("Notebooks/Huge.md".into());
        let plan = plan_moves(&s, &mv(&[("Notebooks/Old.md", "Notebooks/New.md")]));
        assert_eq!(plan.left.len(), 1);
        assert_eq!(plan.left[0].why, Why::Unread);
    }

    #[test]
    fn a_second_plan_after_applying_the_first_has_nothing_to_do() {
        let s = snap(
            &[
                ("Notebooks/Old.md", ""),
                ("Notebooks/L.md", "[[Old]] [t](Old.md)"),
            ],
            &[],
        );
        let moves = mv(&[("Notebooks/Old.md", "Notebooks/Sub/New.md")]);
        let plan = plan_moves(&s, &moves);
        let moved = snap(
            &[
                ("Notebooks/Sub/New.md", ""),
                ("Notebooks/L.md", after(&plan, "Notebooks/L.md")),
            ],
            &[],
        );
        assert!(plan_moves(&moved, &HashMap::new()).rewrites.is_empty());
    }

    #[test]
    fn apply_writes_the_plan_and_skips_a_note_edited_in_between() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for (rel, text) in [
            ("Notebooks/Old.md", "# Old"),
            ("Notebooks/Linker.md", "see [[Old]]"),
            ("Notebooks/Edited.md", "see [[Old]]"),
        ] {
            let p = root.join(rel);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, text).unwrap();
        }
        let s = snapshot(root);
        let plan = plan_moves(&s, &mv(&[("Notebooks/Old.md", "Notebooks/New.md")]));
        fs::rename(root.join("Notebooks/Old.md"), root.join("Notebooks/New.md")).unwrap();
        // Someone edits one note after the plan was made.
        fs::write(
            root.join("Notebooks/Edited.md"),
            "see [[Old]] and my new thoughts",
        )
        .unwrap();

        let done = apply(root, &plan);
        assert_eq!(done.rewritten, ["Notebooks/Linker.md"]);
        assert_eq!(done.left.len(), 1);
        assert_eq!(done.left[0].why, Why::Changed);
        assert_eq!(
            fs::read_to_string(root.join("Notebooks/Linker.md")).unwrap(),
            "see [[New]]"
        );
        assert_eq!(
            fs::read_to_string(root.join("Notebooks/Edited.md")).unwrap(),
            "see [[Old]] and my new thoughts"
        );
    }

    #[test]
    fn backlinks_find_every_spelling_and_mark_the_ambiguous_ones() {
        let s = snap(
            &[
                ("Notebooks/A/Plan.md", "# Plan"),
                ("Notebooks/B/Plan.md", ""),
                (
                    "Notebooks/L.md",
                    "[[A/Plan#Goals]] [t](A/Plan.md) [[Plan]]\n```\n[[A/Plan]]\n```",
                ),
                ("Notebooks/A/Self.md", "[[Plan]]"),
            ],
            &[],
        );
        let found = backlinks(&s, "Notebooks/A/Plan.md");
        let shown: Vec<(&str, usize, bool)> = found
            .iter()
            .map(|b| (b.path.as_str(), b.line, b.ambiguous))
            .collect();
        assert_eq!(
            shown,
            [
                ("Notebooks/A/Self.md", 1, true),
                ("Notebooks/L.md", 1, false),
                ("Notebooks/L.md", 1, false),
                ("Notebooks/L.md", 1, true),
            ]
        );
    }

    #[test]
    fn relative_paths_climb_and_descend() {
        assert_eq!(relative_path("Notebooks/A", "Notebooks/A/x.md"), "x.md");
        assert_eq!(
            relative_path("Notebooks/A", "Notebooks/B/x.md"),
            "../B/x.md"
        );
        assert_eq!(
            relative_path("Notebooks/A/B/C", "Notebooks/x.md"),
            "../../../x.md"
        );
        assert_eq!(relative_path("", "Notebooks/x.md"), "Notebooks/x.md");
    }
}
