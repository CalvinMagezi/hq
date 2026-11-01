//! The far end of a remote connection. HQ reaches a host on another machine
//! over ssh, and the key it presents is pinned to one command, `hq host gate`.
//! That command reads one request from stdin, checks the method against an
//! allowlist, forwards it to the host's local socket and prints the reply. The
//! key can therefore do nothing but call the host API. That API can start any
//! command (`agent.spawn`), so the key is as powerful as a shell as the user who
//! runs the host: pin it to one source address and treat it like an ssh login.

use crate::client::{Client, ClientError};
use serde_json::{Value, json};
use std::path::Path;

/// Largest request read from stdin.
pub const MAX_GATE_INPUT: usize = 1_000_000;
/// Exit status for a refused request, as the host gate uses.
pub const GATE_DENIED_EXIT: i32 = 64;

/// Methods a remote caller may use. `host.stop` is left out (a remote key must
/// not end the host), and so is `agent.report`, which belongs to the agent's own
/// hooks on the machine where it runs. `host.lockdown` is in so HQ can stop a
/// machine's agents from anywhere; unlocking has no method at all.
const ALLOWED: [&str; 30] = [
    "host.status",
    "host.lockdown",
    "host.workspace",
    "host.dirs",
    "host.mkdir",
    "worktree.add",
    "worktree.remove",
    "worktree.status",
    "events.poll",
    "agent.spawn",
    "agent.list",
    "agent.get",
    "agent.read",
    "agent.read_since",
    "agent.pending",
    "agent.answer",
    "agent.head",
    "agent.send_text",
    "agent.paste",
    "agent.prompt",
    "agent.send_keys",
    "agent.resize",
    "agent.wait",
    "agent.kill",
    "agent.remove",
    "agent.awaiting",
    "agent.resume",
    "agent.set_resume",
    "agent.hook_flags",
    "agent.mcp_config",
];

#[derive(Debug, PartialEq, Eq)]
pub struct Denied(pub String);

/// The method and params from the request HQ sends: a JSON array of two
/// strings, the method name and the params as JSON text, so no shell ever
/// parses either.
pub fn parse_request(input: &str) -> Result<(String, Value), Denied> {
    if input.len() > MAX_GATE_INPUT {
        return Err(Denied("request too large".into()));
    }
    let parts: Vec<String> = serde_json::from_str(input)
        .map_err(|_| Denied("expected a JSON array of method and params".into()))?;
    let [method, params] = <[String; 2]>::try_from(parts)
        .map_err(|_| Denied("expected exactly a method and its params".into()))?;
    if !ALLOWED.contains(&method.as_str()) {
        return Err(Denied(format!(
            "method {method:?} is not allowed through the gate"
        )));
    }
    let params: Value =
        serde_json::from_str(&params).map_err(|_| Denied("params must be JSON".into()))?;
    if !params.is_object() {
        return Err(Denied("params must be a JSON object".into()));
    }
    Ok((method, params))
}

/// Runs the request against the host in `dir` and returns the reply as one
/// JSON line: `{"result": ...}` or `{"error": {"code", "message"}}`. A host
/// that cannot be reached is an error reply too, with code `unreachable`.
pub fn forward(dir: &Path, method: &str, params: Value) -> String {
    let reply = Client::connect(dir).and_then(|mut c| c.call(method, params));
    let body = match reply {
        Ok(result) => json!({ "result": result }),
        Err(ClientError::Remote { code, message }) => {
            json!({ "error": { "code": code, "message": message } })
        }
        Err(other) => json!({ "error": { "code": "unreachable", "message": other.to_string() } }),
    };
    body.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(method: &str, params: &str) -> String {
        serde_json::to_string(&[method, params]).unwrap()
    }

    #[test]
    fn a_remote_caller_can_lock_a_host_but_never_unlock_it() {
        assert!(parse_request(&request("host.lockdown", r#"{"reason":"r"}"#)).is_ok());
        assert!(parse_request(&request("host.unlock", "{}")).is_err());
    }

    #[test]
    fn an_allowed_method_with_object_params_is_accepted() {
        let (m, p) = parse_request(&request("agent.get", r#"{"name":"a"}"#)).unwrap();
        assert_eq!(m, "agent.get");
        assert_eq!(p["name"], "a");
    }

    #[test]
    fn a_paired_machines_dialogs_can_be_read_and_answered() {
        // A server HQ drives agents on a paired laptop through this gate, so everything the
        // session tools call must pass it: reading a dialog, answering it, following output.
        for method in [
            "agent.pending",
            "agent.answer",
            "agent.read_since",
            "agent.head",
            "worktree.add",
            "worktree.remove",
            "worktree.status",
        ] {
            assert!(
                parse_request(&request(method, r#"{"name":"a"}"#)).is_ok(),
                "{method}"
            );
        }
    }

    #[test]
    fn methods_a_remote_key_must_not_reach_are_refused() {
        for method in ["host.stop", "agent.report", "hello", "agent.nope", ""] {
            assert!(parse_request(&request(method, "{}")).is_err(), "{method}");
        }
    }

    #[test]
    fn malformed_requests_are_refused() {
        for bad in [
            "not json",
            "[]",
            r#"["agent.list"]"#,
            r#"["agent.list","{}","extra"]"#,
            r#"{"method":"agent.list"}"#,
            &request("agent.list", "[1]"),
            &request("agent.list", "not json"),
        ] {
            assert!(parse_request(bad).is_err(), "{bad}");
        }
        assert!(parse_request(&"x".repeat(MAX_GATE_INPUT + 1)).is_err());
    }

    #[test]
    fn a_host_that_is_not_running_gives_an_error_reply_not_a_crash() {
        let out = forward(Path::new("/nonexistent/run"), "agent.list", json!({}));
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["error"]["code"], "unreachable");
    }
}
