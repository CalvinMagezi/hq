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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_confirm_moves_an_original() {
        for a in SAFE_ACTIONS {
            assert!(!moves_original(a), "{a}");
        }
        assert!(moves_original("confirm"));
        assert!(moves_original("anything-new"), "an unknown action asks");
    }
}
