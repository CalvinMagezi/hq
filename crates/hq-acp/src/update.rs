//! Typed views over `session/update` notifications and permission requests.
//! Each agent words tool calls differently (Hermes and Cursor send `kind` and `locations` or a
//! diff, agy sends `name` and `args`, Copilot puts a diff in `rawInput`), so file paths are
//! gathered from every place seen so far.

use serde_json::Value;

const PATH_KEYS: [&str; 5] = ["path", "Path", "file_path", "filePath", "fileName"];
const EDIT_TOOL_NAMES: [&str; 3] = ["write_file", "edit_file", "apply_patch"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolKind {
    Read,
    Edit,
    Execute,
    Other,
}

impl ToolKind {
    fn from_wire(kind: Option<&str>, name: Option<&str>) -> Self {
        match kind {
            Some("read") => Self::Read,
            Some("edit") => Self::Edit,
            Some("execute") => Self::Execute,
            _ if name.is_some_and(|n| EDIT_TOOL_NAMES.contains(&n)) => Self::Edit,
            _ => Self::Other,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolCall {
    pub id: String,
    pub kind: ToolKind,
    pub title: String,
    pub paths: Vec<String>,
    pub status: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Update {
    AgentText(String),
    Thought(String),
    ToolStart(ToolCall),
    ToolProgress {
        id: String,
        status: Option<String>,
        paths: Vec<String>,
    },
    Usage {
        size: Option<u64>,
        used: Option<u64>,
    },
    /// An update kind HQ does not act on (titles, commands, config changes, prompt echoes).
    Other(String),
}

/// `params` of a `session/update` notification.
pub fn parse_update(params: &Value) -> Option<Update> {
    let u = params.get("update")?;
    let tag = u.get("sessionUpdate")?.as_str()?;
    Some(match tag {
        "agent_message_chunk" => Update::AgentText(chunk_text(u)),
        "agent_thought_chunk" => Update::Thought(chunk_text(u)),
        "tool_call" => Update::ToolStart(tool_call(u)),
        "tool_call_update" => Update::ToolProgress {
            id: tool_id(u),
            status: str_field(u, "status"),
            paths: touched_paths(u),
        },
        "usage_update" => Update::Usage {
            size: u.get("size").and_then(Value::as_u64),
            used: u.get("used").and_then(Value::as_u64),
        },
        other => Update::Other(other.to_owned()),
    })
}

fn chunk_text(u: &Value) -> String {
    u.pointer("/content/text")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned()
}

fn str_field(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_owned)
}

fn tool_id(u: &Value) -> String {
    str_field(u, "toolCallId")
        .or_else(|| str_field(u, "id"))
        .unwrap_or_default()
}

pub(crate) fn tool_call(u: &Value) -> ToolCall {
    let name = str_field(u, "name");
    let title = str_field(u, "title")
        .or_else(|| name.clone())
        .unwrap_or_default();
    ToolCall {
        id: tool_id(u),
        kind: ToolKind::from_wire(u.get("kind").and_then(Value::as_str), name.as_deref()),
        title,
        paths: touched_paths(u),
        status: str_field(u, "status"),
    }
}

/// Every file path a tool call or permission request names, in first-seen order without repeats.
pub fn touched_paths(v: &Value) -> Vec<String> {
    let mut paths = Vec::new();
    let mut push = |p: &str| {
        if !p.is_empty() && !paths.iter().any(|x| x == p) {
            paths.push(p.to_owned());
        }
    };
    for loc in v
        .get("locations")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if let Some(p) = loc.get("path").and_then(Value::as_str) {
            push(p);
        }
    }
    for item in v
        .get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if let Some(p) = item.get("path").and_then(Value::as_str) {
            push(p);
        }
    }
    for args in ["rawInput", "args"] {
        for key in PATH_KEYS {
            if let Some(p) = v.pointer(&format!("/{args}/{key}")).and_then(Value::as_str) {
                push(p);
            }
        }
    }
    paths
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn agy_style_tool_call_is_an_edit_by_name() {
        let params = json!({"update": {"sessionUpdate": "tool_call", "toolCallId": "c1",
            "name": "write_file", "args": {"Path": "/work/a.txt", "Content": "x"}}});
        let Some(Update::ToolStart(t)) = parse_update(&params) else {
            panic!("no tool")
        };
        assert_eq!(t.kind, ToolKind::Edit);
        assert_eq!(t.paths, vec!["/work/a.txt"]);
        assert_eq!(t.title, "write_file");
    }

    #[test]
    fn locations_and_diffs_are_not_counted_twice() {
        let params = json!({"update": {"sessionUpdate": "tool_call_update", "toolCallId": "c1",
            "locations": [{"path": "/w/a"}], "content": [{"type": "diff", "path": "/w/a"}]}});
        let Some(Update::ToolProgress { paths, .. }) = parse_update(&params) else {
            panic!()
        };
        assert_eq!(paths, vec!["/w/a"]);
    }

    #[test]
    fn unknown_updates_are_kept_by_name() {
        let params = json!({"update": {"sessionUpdate": "plan"}});
        assert_eq!(parse_update(&params), Some(Update::Other("plan".into())));
        assert_eq!(parse_update(&json!({})), None);
    }
}
