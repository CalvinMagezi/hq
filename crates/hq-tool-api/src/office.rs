//! What the places that classify the `office_file` tool need to know about it: its name and which
//! of its actions move an original. Kept here, with no dependency on the HQ Suite, so the owner
//! approval in `hq-agent` and the connector gate in `hq-mcp` can use them without building the
//! tool itself (`hq-tools-office`).

/// The tool's name, for the places that classify it (owner approval, the connector gate).
pub const OFFICE_TOOL: &str = "office_file";

/// Actions that never move an original: they read, write the new note or put things back.
pub const SAFE_ACTIONS: [&str; 4] = ["status", "shrink", "abandon", "undo"];

/// Whether a call with this `action` moves an original out of its place (and so asks the owner).
pub fn moves_original(action: &str) -> bool {
    !SAFE_ACTIONS.contains(&action)
}

/// Where an office tool's note writes go through, so the runtime can lock the note, keep the text
/// it replaces (the owner's Undo) and record who wrote it. `hq-tools` implements it over its
/// provenance recorder; with none, a tool writes directly (a bare test or a build with no database).
pub trait NoteWriteSeam: Send + Sync {
    /// Run `write` for the note `rel`, under the note's lock, with `args` naming the caller.
    fn write(
        &self,
        rel: &str,
        args: &serde_json::Value,
        write: &mut dyn FnMut() -> anyhow::Result<()>,
    ) -> anyhow::Result<()>;

    /// A new note `rel` was just made by the call with `args`: record who made it.
    fn created(&self, rel: &str, args: &serde_json::Value);
}

pub type SharedSeam = std::sync::Arc<dyn NoteWriteSeam>;

/// Told when an office tool changed a note: its vault path and the note's new revision. The web
/// server installs one so open pages can reload the note; with none, nothing is announced.
pub type NoteChangedListener = Box<dyn Fn(&str, &str) + Send + Sync>;

static NOTE_LISTENER: std::sync::Mutex<Option<NoteChangedListener>> = std::sync::Mutex::new(None);

/// Installs the one listener (a later call replaces it) or removes it with `None`.
pub fn set_note_listener(listener: Option<NoteChangedListener>) {
    if let Ok(mut slot) = NOTE_LISTENER.lock() {
        *slot = listener;
    }
}

/// Announces that the note `rel` was changed and is now at `revision`.
pub fn notify_note_changed(rel: &str, revision: &str) {
    if let Ok(slot) = NOTE_LISTENER.lock()
        && let Some(listener) = slot.as_ref()
    {
        listener(rel, revision);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_listener_hears_a_changed_note_until_it_is_removed() {
        use std::sync::{Arc, Mutex};
        let heard: Arc<Mutex<Vec<(String, String)>>> = Arc::default();
        let sink = heard.clone();
        set_note_listener(Some(Box::new(move |n, r| {
            sink.lock().unwrap().push((n.into(), r.into()));
        })));
        notify_note_changed("Notebooks/a.md", "r1");
        set_note_listener(None);
        notify_note_changed("Notebooks/a.md", "r2");
        assert_eq!(
            *heard.lock().unwrap(),
            vec![("Notebooks/a.md".to_string(), "r1".to_string())]
        );
    }

    #[test]
    fn only_confirm_moves_an_original() {
        for a in SAFE_ACTIONS {
            assert!(!moves_original(a), "{a}");
        }
        assert!(moves_original("confirm"));
        assert!(moves_original("anything-new"), "an unknown action asks");
    }
}
