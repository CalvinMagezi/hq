//! Whether a request loses something, decided by HQ's runtime from what the call is, never from
//! the model's words (N17). An approval of a losing request reads "Allow {verb}" in red, and no
//! automatic answer may say yes to it (W15).

use serde_json::Value;

/// The verbs an approval may name, in the words the apps show.
pub const VERBS: &[&str] = &[
    "delete",
    "remove",
    "stop",
    "sign out",
    "revoke",
    "disconnect",
    "drop",
    "overwrite",
];

/// Words in a tool's name or action that mean one of [`VERBS`].
const WORD_VERBS: &[(&str, &str)] = &[
    ("delete", "delete"),
    ("trash", "delete"),
    ("purge", "delete"),
    ("remove", "remove"),
    ("uninstall", "remove"),
    ("stop", "stop"),
    ("kill", "stop"),
    ("revoke", "revoke"),
    ("disconnect", "disconnect"),
    ("drop", "drop"),
    ("overwrite", "overwrite"),
    ("logout", "sign out"),
    ("signout", "sign out"),
];

/// Tools whose `command` argument is a shell command line.
const SHELL_TOOLS: &[&str] = &["bash"];
/// Tools whose `command` argument is a line of another program's words (`gws`).
const WORDS_TOOLS: &[&str] = &["google_workspace"];

/// The verb for a call to `tool` with `args`, or `None` when it loses nothing.
pub fn tool_verb(tool: &str, args: &Value) -> Option<&'static str> {
    let command = args.get("command").and_then(Value::as_str);
    if SHELL_TOOLS.contains(&tool) {
        return command.and_then(crate::command_policy::losing_verb);
    }
    if WORDS_TOOLS.contains(&tool)
        && let Some(verb) = command.and_then(words_verb)
    {
        return Some(verb);
    }
    // A remote server's own tool, called through `<name>_call`.
    let inner = args.get("tool").and_then(Value::as_str);
    let action = args.get("action").and_then(Value::as_str);
    [Some(tool), action, inner]
        .into_iter()
        .flatten()
        .find_map(name_verb)
}

/// The verb for a helper's dialog: a command dialog is judged as the command line it shows.
pub fn dialog_verb(kind: &str, command: Option<&str>) -> Option<&'static str> {
    match kind {
        "command" => command.and_then(crate::command_policy::losing_verb),
        _ => None,
    }
}

/// `task_delete`, `remove_file`, `deleteEvent`: any part of the name that is a losing word.
fn name_verb(name: &str) -> Option<&'static str> {
    name.split(|c: char| !c.is_ascii_alphanumeric())
        .flat_map(split_camel)
        .find_map(|part| word(&part))
}

/// Words up to the first flag: `gmail users messages trash --params ...`.
fn words_verb(line: &str) -> Option<&'static str> {
    line.split_whitespace()
        .take_while(|w| !w.starts_with('-'))
        .find_map(name_verb)
}

fn split_camel(part: &str) -> Vec<String> {
    let mut out = vec![String::new()];
    for c in part.chars() {
        if c.is_ascii_uppercase() && !out.last().is_some_and(String::is_empty) {
            out.push(String::new());
        }
        if let Some(last) = out.last_mut() {
            last.push(c.to_ascii_lowercase());
        }
    }
    out
}

fn word(part: &str) -> Option<&'static str> {
    WORD_VERBS
        .iter()
        .find(|(w, _)| *w == part)
        .map(|(_, verb)| *verb)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn hq_tools_lose_by_their_name_or_action() {
        assert_eq!(
            tool_verb("task_delete", &json!({"id": "t1"})),
            Some("delete")
        );
        assert_eq!(
            tool_verb("skill_manage", &json!({"action": "remove_file"})),
            Some("remove")
        );
        assert_eq!(
            tool_verb("slash_command_manage", &json!({"action": "delete"})),
            Some("delete")
        );
        assert_eq!(
            tool_verb("skill_manage", &json!({"action": "create"})),
            None
        );
        assert_eq!(tool_verb("write_file", &json!({"path": "a"})), None);
        assert_eq!(
            tool_verb("git_commit", &json!({"message": "delete old files"})),
            None
        );
    }

    #[test]
    fn a_remote_tool_loses_by_its_own_name() {
        assert_eq!(
            tool_verb("diagrams_call", &json!({"tool": "deleteDiagram"})),
            Some("delete")
        );
        assert_eq!(
            tool_verb("diagrams_call", &json!({"tool": "list_diagrams"})),
            None
        );
    }

    #[test]
    fn bash_loses_by_the_command_it_runs() {
        let bash = |c: &str| tool_verb("bash", &json!({ "command": c }));
        assert_eq!(bash("rm -rf build"), Some("delete"));
        assert_eq!(bash("sudo rm x"), Some("delete"));
        assert_eq!(bash("FOO=1 timeout 5 rm x"), Some("delete"));
        assert_eq!(bash("git -C repo clean -fdx"), Some("delete"));
        assert_eq!(bash("git push -f origin main"), Some("overwrite"));
        assert_eq!(bash("cargo test && git reset --hard"), Some("overwrite"));
        assert_eq!(bash("sh -c 'kill 42'"), Some("stop"));
        assert_eq!(bash("git push origin main"), None);
        assert_eq!(bash("echo rm"), None);
        assert_eq!(bash("ls -la"), None);
        // The model's words around a call never make it losing.
        assert_eq!(
            tool_verb(
                "bash",
                &json!({"command": "ls", "why": "delete everything"})
            ),
            None
        );
    }

    #[test]
    fn google_workspace_loses_by_its_method() {
        let gws = |c: &str| tool_verb("google_workspace", &json!({ "command": c }));
        assert_eq!(
            gws("gmail users messages trash --params {}"),
            Some("delete")
        );
        assert_eq!(gws("calendar events delete --params {}"), Some("delete"));
        assert_eq!(gws("drive files list --params {\"q\": \"delete\"}"), None);
    }

    #[test]
    fn a_command_dialog_is_judged_as_its_command() {
        assert_eq!(dialog_verb("command", Some("rm -r target")), Some("delete"));
        assert_eq!(dialog_verb("command", Some("cargo build")), None);
        assert_eq!(dialog_verb("edit", Some("src/main.rs")), None);
        assert_eq!(dialog_verb("question", None), None);
    }

    #[test]
    fn every_verb_is_one_the_apps_show() {
        for (_, verb) in WORD_VERBS {
            assert!(VERBS.contains(verb), "{verb}");
        }
        for cmd in [
            "rm a",
            "git rm a",
            "kill 1",
            "dropdb x",
            "gh auth logout",
            "git push -f",
        ] {
            let verb = crate::command_policy::losing_verb(cmd).unwrap();
            assert!(VERBS.contains(&verb), "{cmd}: {verb}");
        }
    }
}
