//! Claude Code hooks that let an agent tell the host what it is doing. They
//! are passed per launch with `--settings <file>`, so nothing is written to
//! the user's own Claude configuration.

use crate::token;
use serde_json::{Value, json};
use std::fs;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

const FILE_MODE: u32 = 0o600;
const HOOKS_DIR: &str = "hooks";
/// The events that say something about state or identify the conversation.
const CLAUDE_EVENTS: [&str; 4] = ["SessionStart", "UserPromptSubmit", "Stop", "Notification"];

/// Claude Code settings that run `report_command` on each event. The command
/// gets the event as JSON on stdin and must print nothing: output of a
/// SessionStart hook is handed to the model as context.
pub fn claude_settings(report_command: &str) -> Value {
    let silent = format!("{report_command} >/dev/null 2>&1 || true");
    let entry = json!([{ "hooks": [{ "type": "command", "command": silent }] }]);
    let hooks: serde_json::Map<String, Value> = CLAUDE_EVENTS
        .iter()
        .map(|event| (event.to_string(), entry.clone()))
        .collect();
    json!({ "hooks": hooks })
}

/// Writes `<dir>/hooks/<name>.json` (0600, in a 0700 directory) and returns
/// its path, ready for `claude --settings`.
pub fn write_claude_settings(
    dir: &Path,
    name: &str,
    report_command: &str,
) -> std::io::Result<PathBuf> {
    let hooks = dir.join(HOOKS_DIR);
    token::ensure_dir(&hooks)?;
    let path = hooks.join(format!("{name}.json"));
    let bytes = serde_json::to_vec_pretty(&claude_settings(report_command))
        .map_err(std::io::Error::other)?;
    let tmp = hooks.join(format!("{name}.json.tmp"));
    let _ = fs::remove_file(&tmp);
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(FILE_MODE)
        .open(&tmp)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    fs::rename(&tmp, &path)?;
    Ok(path)
}

/// The `agent.report` params for the JSON a Claude Code hook receives on
/// stdin, or None when it is not a hook payload.
pub fn report_params(hook_input: &str) -> Option<Value> {
    let v: Value = serde_json::from_str(hook_input).ok()?;
    let text = |key: &str| v.get(key).and_then(Value::as_str);
    Some(json!({
        "event": text("hook_event_name")?,
        "session_id": text("session_id"),
        "notification_type": text("notification_type"),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn every_event_runs_the_command_silently() {
        let s = claude_settings("/bin/hq host report");
        for event in CLAUDE_EVENTS {
            let cmd = &s["hooks"][event][0]["hooks"][0]["command"];
            assert_eq!(
                cmd, "/bin/hq host report >/dev/null 2>&1 || true",
                "{event}"
            );
        }
    }

    #[test]
    fn the_settings_file_is_private() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("run");
        let path = write_claude_settings(&dir, "a", "hq host report").unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let back: Value = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
        assert!(back["hooks"]["Stop"].is_array());
    }

    #[test]
    fn a_hook_payload_becomes_report_params() {
        let input = r#"{"hook_event_name":"Notification","session_id":"s1","notification_type":"permission_prompt","message":"x"}"#;
        let p = report_params(input).unwrap();
        assert_eq!(p["event"], "Notification");
        assert_eq!(p["session_id"], "s1");
        assert_eq!(p["notification_type"], "permission_prompt");
        assert!(report_params("not json").is_none());
        assert!(report_params(r#"{"session_id":"s"}"#).is_none());
    }
}
