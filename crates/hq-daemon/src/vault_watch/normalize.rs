//! Turns raw filesystem notifications into the small vocabulary the pipeline understands.
//!
//! A notification only says "this path may have changed". Nothing here trusts what an event
//! claims happened: the consumer reads the file again and acts on what it finds. These are pure
//! functions, so they are tested with hand-built events instead of real timing.

use std::path::Path;

use hq_db::search::{indexable_dir_path, indexable_rel_path};
use notify::Event;
use notify::event::{CreateKind, EventKind, ModifyKind, RemoveKind, RenameMode};

/// What the pipeline is told. Paths are vault-relative, `/`-separated, and already known to be
/// ones the search index covers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchEvent {
    /// A note appeared (or may have).
    Created(String),
    /// A note was written or replaced (or may have been).
    Changed(String),
    /// A note went away (or may have).
    Deleted(String),
    /// Both ends were reported together and both are notes the index covers.
    Renamed { from: String, to: String },
    /// A folder appeared, moved or went away. Moving a folder reports the folder, not the notes
    /// inside it, so everything under it has to be checked again.
    Folder(String),
    /// Events were lost, or the watcher cannot say what changed. Scan everything.
    Overflow,
}

/// The result of normalizing one notification.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Normalized {
    pub events: Vec<WatchEvent>,
    /// Paths the index does not cover (other folders, hidden files, editor temp files).
    pub ignored: u64,
}

/// What a reported path is, as far as the index is concerned.
enum Target {
    Note(String),
    Folder(String),
    Ignored,
}

fn relative(root: &Path, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(root).ok()?.to_str()?;
    Some(rel.replace('\\', "/"))
}

fn is_markdown(rel: &str) -> bool {
    Path::new(rel).extension().is_some_and(|e| e == "md")
}

/// Classify a path. `folder_hint` says the event kind itself named a folder; without it, a path
/// that is not a note is treated as a possible folder, since a rename does not say which.
fn classify(root: &Path, path: &Path, folder_hint: Option<bool>) -> Target {
    let Some(rel) = relative(root, path) else {
        return Target::Ignored;
    };
    if is_markdown(&rel) && folder_hint != Some(true) {
        return match indexable_rel_path(&rel) {
            Some(note) => Target::Note(note),
            None => Target::Ignored,
        };
    }
    if folder_hint == Some(false) {
        return Target::Ignored;
    }
    match indexable_dir_path(&rel) {
        Some(dir) => Target::Folder(dir),
        None => Target::Ignored,
    }
}

fn note_event(target: Target, as_note: fn(String) -> WatchEvent, out: &mut Normalized) {
    match target {
        Target::Note(rel) => out.events.push(as_note(rel)),
        Target::Folder(rel) => out.events.push(WatchEvent::Folder(rel)),
        Target::Ignored => out.ignored += 1,
    }
}

/// Normalize one notification. `root` is the vault directory the paths are relative to.
pub fn normalize(root: &Path, event: &Event) -> Normalized {
    let mut out = Normalized::default();
    if event.need_rescan() {
        out.events.push(WatchEvent::Overflow);
        return out;
    }

    match &event.kind {
        EventKind::Access(_) => out.ignored += event.paths.len() as u64,
        EventKind::Create(kind) => {
            let hint = match kind {
                CreateKind::Folder => Some(true),
                CreateKind::File => Some(false),
                _ => None,
            };
            for path in &event.paths {
                note_event(classify(root, path, hint), WatchEvent::Created, &mut out);
            }
        }
        EventKind::Remove(kind) => {
            let hint = match kind {
                RemoveKind::Folder => Some(true),
                RemoveKind::File => Some(false),
                _ => None,
            };
            for path in &event.paths {
                note_event(classify(root, path, hint), WatchEvent::Deleted, &mut out);
            }
        }
        EventKind::Modify(ModifyKind::Name(RenameMode::Both)) if event.paths.len() == 2 => {
            let from = classify(root, &event.paths[0], None);
            let to = classify(root, &event.paths[1], None);
            match (from, to) {
                (Target::Note(from), Target::Note(to)) => {
                    out.events.push(WatchEvent::Renamed { from, to })
                }
                (from, to) => {
                    note_event(from, WatchEvent::Deleted, &mut out);
                    note_event(to, WatchEvent::Created, &mut out);
                }
            }
        }
        // One end of a rename, or a rename the backend could not pair: the path may now exist
        // or not, so say it changed and let the consumer look.
        EventKind::Modify(ModifyKind::Name(_)) => {
            for path in &event.paths {
                note_event(classify(root, path, None), WatchEvent::Changed, &mut out);
            }
        }
        // Writes, metadata changes and backends that cannot say more. Only notes matter here: a
        // folder's own content or metadata changing reports its children separately.
        EventKind::Modify(_) | EventKind::Any | EventKind::Other => {
            for path in &event.paths {
                match classify(root, path, Some(false)) {
                    Target::Note(rel) => out.events.push(WatchEvent::Changed(rel)),
                    _ => out.ignored += 1,
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use notify::event::{AccessKind, DataChange, Flag, MetadataKind};

    use super::*;

    const ROOT: &str = "/vault";

    fn root() -> PathBuf {
        PathBuf::from(ROOT)
    }

    fn event(kind: EventKind, paths: &[&str]) -> Event {
        let mut e = Event::new(kind);
        for p in paths {
            e = e.add_path(PathBuf::from(format!("{ROOT}/{p}")));
        }
        e
    }

    fn norm(kind: EventKind, paths: &[&str]) -> Normalized {
        normalize(&root(), &event(kind, paths))
    }

    fn only(n: Normalized) -> WatchEvent {
        assert_eq!(n.events.len(), 1, "{n:?}");
        n.events.into_iter().next().unwrap()
    }

    #[test]
    fn creating_writing_and_removing_a_note() {
        let create = EventKind::Create(CreateKind::File);
        assert_eq!(
            only(norm(create, &["Notebooks/a.md"])),
            WatchEvent::Created("Notebooks/a.md".into())
        );
        let write = EventKind::Modify(ModifyKind::Data(DataChange::Content));
        assert_eq!(
            only(norm(write, &["Notebooks/P/a.md"])),
            WatchEvent::Changed("Notebooks/P/a.md".into())
        );
        let remove = EventKind::Remove(RemoveKind::File);
        assert_eq!(
            only(norm(remove, &["Notebooks/a.md"])),
            WatchEvent::Deleted("Notebooks/a.md".into())
        );
    }

    #[test]
    fn a_replace_style_save_is_a_rename_over_the_note() {
        // Editors and git write a temp file, then rename it over the note.
        let both = EventKind::Modify(ModifyKind::Name(RenameMode::Both));
        let n = norm(both, &["Notebooks/.a.md.tmp", "Notebooks/a.md"]);
        assert_eq!(n.events, vec![WatchEvent::Created("Notebooks/a.md".into())]);
        assert_eq!(n.ignored, 1);
    }

    #[test]
    fn a_paired_rename_between_two_notes() {
        let both = EventKind::Modify(ModifyKind::Name(RenameMode::Both));
        assert_eq!(
            only(norm(both, &["Notebooks/a.md", "Notebooks/b.md"])),
            WatchEvent::Renamed {
                from: "Notebooks/a.md".into(),
                to: "Notebooks/b.md".into()
            }
        );
    }

    #[test]
    fn renaming_a_note_to_something_that_is_not_a_note_deletes_it() {
        let both = EventKind::Modify(ModifyKind::Name(RenameMode::Both));
        let n = norm(both, &["Notebooks/a.md", "Notebooks/a.md.bak"]);
        // The new name is not a note, so it is checked as a possible folder, which costs nothing.
        assert_eq!(
            n.events,
            vec![
                WatchEvent::Deleted("Notebooks/a.md".into()),
                WatchEvent::Folder("Notebooks/a.md.bak".into())
            ]
        );
    }

    #[test]
    fn one_end_of_an_unpaired_rename_says_changed() {
        let from = EventKind::Modify(ModifyKind::Name(RenameMode::From));
        let to = EventKind::Modify(ModifyKind::Name(RenameMode::To));
        assert_eq!(
            only(norm(from, &["Notebooks/a.md"])),
            WatchEvent::Changed("Notebooks/a.md".into())
        );
        assert_eq!(
            only(norm(to, &["Notebooks/b.md"])),
            WatchEvent::Changed("Notebooks/b.md".into())
        );
    }

    #[test]
    fn folders_are_reported_as_folders() {
        assert_eq!(
            only(norm(
                EventKind::Create(CreateKind::Folder),
                &["Notebooks/New"]
            )),
            WatchEvent::Folder("Notebooks/New".into())
        );
        assert_eq!(
            only(norm(
                EventKind::Remove(RemoveKind::Folder),
                &["Notebooks/Old"]
            )),
            WatchEvent::Folder("Notebooks/Old".into())
        );
        let both = EventKind::Modify(ModifyKind::Name(RenameMode::Both));
        let n = norm(both, &["Notebooks/A", "Notebooks/B"]);
        assert_eq!(
            n.events,
            vec![
                WatchEvent::Folder("Notebooks/A".into()),
                WatchEvent::Folder("Notebooks/B".into())
            ]
        );
    }

    #[test]
    fn a_folder_with_dots_in_its_name_is_still_a_folder() {
        assert_eq!(
            only(norm(
                EventKind::Create(CreateKind::Folder),
                &["Notebooks/v1.2.md"]
            )),
            WatchEvent::Folder("Notebooks/v1.2.md".into())
        );
    }

    #[test]
    fn data_changes_to_things_that_are_not_notes_are_ignored() {
        let write = EventKind::Modify(ModifyKind::Data(DataChange::Any));
        let n = norm(write, &["Notebooks/image.png", "Notebooks/Sub"]);
        assert!(n.events.is_empty());
        assert_eq!(n.ignored, 2);
        let create_file = EventKind::Create(CreateKind::File);
        assert!(norm(create_file, &["Notebooks/a.txt"]).events.is_empty());
    }

    #[test]
    fn hidden_trash_meta_and_outside_paths_are_ignored() {
        let write = EventKind::Modify(ModifyKind::Data(DataChange::Content));
        for path in [
            "Notebooks/.hidden.md",
            "Notebooks/.obsidian/x.md",
            "Notebooks/_trash/x.md",
            "Notebooks/_meta.md",
            "_system/SOUL.md",
            "Other/a.md",
        ] {
            let n = norm(write, &[path]);
            assert!(n.events.is_empty(), "{path}: {n:?}");
            assert_eq!(n.ignored, 1, "{path}");
        }
        // A path outside the vault root altogether.
        let stray = Event::new(write).add_path(PathBuf::from("/elsewhere/Notebooks/a.md"));
        assert!(normalize(&root(), &stray).events.is_empty());
    }

    #[test]
    fn metadata_and_access_events_do_not_cause_work() {
        let access = EventKind::Access(AccessKind::Any);
        assert!(norm(access, &["Notebooks/a.md"]).events.is_empty());
        // A touch changes mtime only; the consumer's content hash makes it free.
        let meta = EventKind::Modify(ModifyKind::Metadata(MetadataKind::WriteTime));
        assert_eq!(
            only(norm(meta, &["Notebooks/a.md"])),
            WatchEvent::Changed("Notebooks/a.md".into())
        );
    }

    #[test]
    fn a_rescan_request_becomes_overflow_and_nothing_else() {
        let e = Event::new(EventKind::Other)
            .add_path(PathBuf::from(format!("{ROOT}/Notebooks/a.md")))
            .set_flag(Flag::Rescan);
        assert_eq!(normalize(&root(), &e).events, vec![WatchEvent::Overflow]);
    }

    #[test]
    fn an_unknown_event_on_a_note_means_look_again() {
        assert_eq!(
            only(norm(EventKind::Any, &["Notebooks/a.md"])),
            WatchEvent::Changed("Notebooks/a.md".into())
        );
        assert_eq!(
            only(norm(EventKind::Other, &["Notebooks/a.md"])),
            WatchEvent::Changed("Notebooks/a.md".into())
        );
    }
}
