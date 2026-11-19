//! Alternate names for a coding-agent harness.
//!
//! Free of the session code so the quota gate can map a name to its lane without reaching up
//! into `harness_session`, which re-exports [`canonical_name`] from its spec table.

/// Aliases accepted in addition to each spec's canonical `harness` name
/// (relay/dispatch paths already map "agy"/"jetski" etc.; sessions match).
const ALIASES: &[(&str, &str)] = &[("agy", "antigravity")];

/// Map an alias to its canonical harness name; unknown names pass through.
pub fn canonical_name(harness: &str) -> &str {
    ALIASES
        .iter()
        .find(|(alias, _)| *alias == harness)
        .map(|(_, canonical)| *canonical)
        .unwrap_or(harness)
}
