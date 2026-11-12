//! A permission request as a dialog HQ can show and answer, and the reply that answers it.

use crate::update::{ToolKind, touched_paths};
use serde_json::{Value, json};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptionKind {
    AllowOnce,
    AllowAlways,
    RejectOnce,
    RejectAlways,
    Other,
}

impl OptionKind {
    fn from_wire(kind: &str) -> Self {
        match kind {
            "allow_once" => Self::AllowOnce,
            "allow_always" => Self::AllowAlways,
            "reject_once" => Self::RejectOnce,
            "reject_always" => Self::RejectAlways,
            _ => Self::Other,
        }
    }

    /// Choices that widen what the agent may do later; only the owner picks these.
    pub fn remembers(self) -> bool {
        matches!(self, Self::AllowAlways | Self::RejectAlways)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DialogOption {
    pub id: String,
    pub label: String,
    pub kind: OptionKind,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Dialog {
    pub title: String,
    pub tool: ToolKind,
    pub paths: Vec<String>,
    pub command: Option<String>,
    pub options: Vec<DialogOption>,
}

/// `params` of a `session/request_permission` request.
pub fn parse_dialog(params: &Value) -> Dialog {
    let call = params.get("toolCall").unwrap_or(&Value::Null);
    let tool = match call.get("kind").and_then(Value::as_str) {
        Some("edit") => ToolKind::Edit,
        Some("execute") => ToolKind::Execute,
        Some("read") => ToolKind::Read,
        _ => ToolKind::Other,
    };
    let command = ["/rawInput/command", "/args/command"]
        .iter()
        .find_map(|p| call.pointer(p).and_then(Value::as_str))
        .map(str::to_owned);
    Dialog {
        title: call
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned(),
        tool,
        paths: touched_paths(call),
        command,
        options: options(params),
    }
}

fn options(params: &Value) -> Vec<DialogOption> {
    let list = params.get("options").and_then(Value::as_array);
    list.into_iter()
        .flatten()
        .filter_map(|o| {
            Some(DialogOption {
                id: o.get("optionId")?.as_str()?.to_owned(),
                label: o
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned(),
                kind: OptionKind::from_wire(o.get("kind").and_then(Value::as_str).unwrap_or("")),
            })
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Choice {
    Option(String),
    Cancelled,
}

/// The `result` of the permission request for `choice`.
pub fn outcome(choice: &Choice) -> Value {
    match choice {
        Choice::Option(id) => json!({"outcome": {"outcome": "selected", "optionId": id}}),
        Choice::Cancelled => json!({"outcome": {"outcome": "cancelled"}}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copilot_style_request_reads_options_paths_and_remember() {
        let params = json!({"options": [
            {"optionId": "allow_once", "kind": "allow_once", "name": "Allow once"},
            {"optionId": "allow_always", "kind": "allow_always", "name": "Always allow"},
            {"optionId": "reject_once", "kind": "reject_once", "name": "Deny"}],
            "toolCall": {"title": "Create file", "kind": "edit", "rawInput": {"fileName": "/w/hello.txt"}}});
        let d = parse_dialog(&params);
        assert_eq!(d.tool, ToolKind::Edit);
        assert_eq!(d.paths, vec!["/w/hello.txt"]);
        let remember: Vec<_> = d.options.iter().filter(|o| o.kind.remembers()).collect();
        assert_eq!(remember.len(), 1);
        assert_eq!(remember[0].id, "allow_always");
    }

    #[test]
    fn outcomes_follow_the_protocol_shape() {
        assert_eq!(
            outcome(&Choice::Option("allow".into())),
            json!({"outcome": {"outcome": "selected", "optionId": "allow"}})
        );
        assert_eq!(
            outcome(&Choice::Cancelled),
            json!({"outcome": {"outcome": "cancelled"}})
        );
    }
}
