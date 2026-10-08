//! Agent flags that skip approval prompts. HQ launches nothing with them, and an agent saved by
//! an older build is brought back without them.

const ALWAYS: &[&str] = &[
    "--dangerously-skip-permissions",
    "--allow-dangerously-skip-permissions",
    "--dangerously-bypass-approvals-and-sandbox",
    "--dangerously-bypass-hook-trust",
    "--full-auto",
    "--approve-mcps",
];

/// Run-everything switches that only mean that for cursor; other tools use `--force` for other things.
const CURSOR_ONLY: &[&str] = &["--force", "--yolo"];

const PERMISSION_MODE: &str = "--permission-mode";
const MANUAL_MODE: &str = "manual";
/// Claude permission modes that answer approvals without a person.
const SELF_APPROVING_MODES: &[&str] = &["bypassPermissions", "auto"];

/// A word as an argument: quotes and brackets a shell wrapper puts around it removed.
fn bare(word: &str) -> &str {
    word.trim_matches(|c: char| matches!(c, '\'' | '"' | '(' | ')' | ';'))
}

/// The flag name of `word`, without a `=value`.
fn flag_name(word: &str) -> &str {
    let word = bare(word);
    word.split_once('=').map_or(word, |(name, _)| name)
}

fn listed(name: &str, agent: Option<&str>) -> Option<&'static str> {
    ALWAYS.iter().copied().find(|f| *f == name).or_else(|| {
        (agent == Some("cursor"))
            .then(|| CURSOR_ONLY.iter().copied().find(|f| *f == name))
            .flatten()
    })
}

fn self_approving_mode(value: &str) -> bool {
    SELF_APPROVING_MODES.contains(&bare(value))
}

/// The first bypass flag among `words` (a command line split on whitespace, or an argv): a listed
/// flag with or without `=value`, quoted or not, or `--permission-mode` set to a mode that
/// approves by itself. `agent` is the kind of agent the command starts, when known.
pub fn find_bypass(words: &[&str], agent: Option<&str>) -> Option<String> {
    for (i, word) in words.iter().enumerate() {
        let name = flag_name(word);
        if let Some(flag) = listed(name, agent) {
            return Some(flag.to_string());
        }
        if name != PERMISSION_MODE {
            continue;
        }
        let inline = bare(word).split_once('=').map(|(_, v)| v);
        let value = inline.or_else(|| words.get(i + 1).copied());
        if value.is_some_and(self_approving_mode) {
            return Some(format!(
                "{PERMISSION_MODE} {}",
                value.map(bare).unwrap_or_default()
            ));
        }
    }
    None
}

/// `argv` with every bypass flag removed and a self-approving permission mode turned into `manual`.
pub fn strip_bypass(argv: &[String], agent: Option<&str>) -> Vec<String> {
    let mut out = Vec::with_capacity(argv.len());
    let mut words = argv.iter().peekable();
    while let Some(word) = words.next() {
        let name = flag_name(word);
        if listed(name, agent).is_some() {
            continue;
        }
        if name == PERMISSION_MODE {
            let inline = word.split_once('=').map(|(_, v)| v.to_string());
            let value = inline
                .clone()
                .or_else(|| words.peek().map(|v| (*v).clone()));
            if value.as_deref().is_some_and(self_approving_mode) {
                if inline.is_none() {
                    words.next();
                }
                out.push(PERMISSION_MODE.to_string());
                out.push(MANUAL_MODE.to_string());
                continue;
            }
        }
        out.push(word.clone());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    #[test]
    fn listed_flags_are_found_bare_quoted_and_with_a_value() {
        for line in [
            "claude --dangerously-skip-permissions",
            "sh -c 'claude --dangerously-skip-permissions'",
            "tool --full-auto=true",
            "claude --permission-mode bypassPermissions",
            "claude --permission-mode=auto",
            "claude --permission-mode \"bypassPermissions\"",
        ] {
            let words: Vec<&str> = line.split_whitespace().collect();
            assert!(find_bypass(&words, Some("claude")).is_some(), "{line}");
        }
    }

    #[test]
    fn force_is_a_bypass_only_for_cursor() {
        let words = ["wrapper", "--force"];
        assert!(find_bypass(&words, Some("claude")).is_none());
        assert_eq!(
            find_bypass(&words, Some("cursor")).as_deref(),
            Some("--force")
        );
    }

    #[test]
    fn manual_and_unrelated_flags_pass() {
        let words = [
            "claude",
            "--permission-mode",
            "manual",
            "--model",
            "x",
            "--allowedTools",
            "Read",
        ];
        assert!(find_bypass(&words, Some("claude")).is_none());
    }

    #[test]
    fn stripping_removes_flags_and_rewrites_a_self_approving_mode() {
        assert_eq!(
            strip_bypass(&argv("--dangerously-skip-permissions -c"), None),
            argv("-c")
        );
        assert_eq!(
            strip_bypass(&argv("--permission-mode bypassPermissions -c"), None),
            argv("--permission-mode manual -c")
        );
        assert_eq!(
            strip_bypass(&argv("--permission-mode=auto"), None),
            argv("--permission-mode manual")
        );
        assert_eq!(
            strip_bypass(&argv("--force --resume x"), Some("cursor")),
            argv("--resume x")
        );
        assert_eq!(
            strip_bypass(&argv("--force --resume x"), Some("claude")),
            argv("--force --resume x")
        );
    }
}
