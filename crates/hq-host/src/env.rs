//! What a pane's process may inherit. The host starts from an empty
//! environment and copies only these names, so markers and secrets of whatever
//! launched the host (another agent session, provider keys) never reach an
//! agent unless the caller passes them explicitly.

const ALLOWED_EXACT: &[&str] = &[
    "PATH",
    "HOME",
    "USER",
    "LOGNAME",
    "SHELL",
    "LANG",
    "LANGUAGE",
    "TMPDIR",
    "TZ",
    "COLORTERM",
    "SSH_AUTH_SOCK",
    "DISPLAY",
    "WAYLAND_DISPLAY",
];
const ALLOWED_PREFIX: &[&str] = &["LC_", "XDG_"];

/// The terminal type every pane gets: the emulator understands it.
const TERM_VALUE: &str = "xterm-256color";

fn allowed(name: &str) -> bool {
    ALLOWED_EXACT.contains(&name) || ALLOWED_PREFIX.iter().any(|p| name.starts_with(p))
}

/// The inherited part, from any variable source.
pub fn filter_env(vars: impl IntoIterator<Item = (String, String)>) -> Vec<(String, String)> {
    vars.into_iter().filter(|(k, _)| allowed(k)).collect()
}

/// Full environment for a pane: allowlisted parent variables, `TERM`, then the
/// caller's explicit extras (which win).
pub fn pane_env(extra: &[(String, String)]) -> Vec<(String, String)> {
    let mut env = filter_env(std::env::vars());
    env.push(("TERM".into(), TERM_VALUE.into()));
    for (k, v) in extra {
        env.retain(|(name, _)| name != k);
        env.push((k.clone(), v.clone()));
    }
    env
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pairs(names: &[&str]) -> Vec<(String, String)> {
        names
            .iter()
            .map(|n| (n.to_string(), "x".to_string()))
            .collect()
    }

    #[test]
    fn agent_markers_and_secrets_are_dropped() {
        let kept = filter_env(pairs(&[
            "CLAUDE_CODE_CHILD_SESSION",
            "CLAUDECODE",
            "CLAUDE_CODE_MESSAGING_TOKEN",
            "GITHUB_PERSONAL_ACCESS_TOKEN",
            "OPENROUTER_API_KEY",
            "HQ_SESSION_ID",
            "PATH",
            "LC_ALL",
            "XDG_RUNTIME_DIR",
        ]));
        let names: Vec<&str> = kept.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(names, ["PATH", "LC_ALL", "XDG_RUNTIME_DIR"]);
    }

    #[test]
    fn extras_win_and_term_is_set() {
        let env = pane_env(&[
            ("TERM".into(), "dumb".into()),
            ("HQ_SESSION_ID".into(), "hs-1".into()),
        ]);
        let get = |k: &str| {
            env.iter()
                .rev()
                .find(|(n, _)| n == k)
                .map(|(_, v)| v.as_str())
        };
        assert_eq!(get("TERM"), Some("dumb"));
        assert_eq!(get("HQ_SESSION_ID"), Some("hs-1"));
    }
}
