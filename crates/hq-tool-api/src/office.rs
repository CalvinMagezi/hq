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
