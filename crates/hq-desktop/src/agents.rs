//! The coding agents on this machine's host and which of them need the person, read from the host's
//! control socket. Display only: the app answers nothing and sends no notice, since alerts belong in
//! the owner's inbox.

use serde::Serialize;
use serde_json::{Value, json};
use std::path::Path;
use std::time::Duration;

/// How long one listing may take before the host is shown as unreachable.
const LIST_TIMEOUT: Duration = Duration::from_secs(2);
const MS_PER_SEC: u64 = 1000;

/// Where an agent stands for the person, most urgent first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Need {
    /// Waiting at a dialog or question.
    Blocked,
    /// Finished a turn and has not worked since.
    Done,
    Working,
    Idle,
    /// The agent kind has no rule file, so the host cannot tell.
    Unknown,
    /// Its process ended.
    Exited,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AgentView {
    pub name: String,
    /// The agent kind (`claude`, `codex`), when the host knows it.
    pub agent: Option<String>,
    pub need: Need,
    /// The terminal title the agent set, or empty.
    pub title: String,
    pub cwd: String,
    pub quiet_secs: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AgentsView {
    /// False when no host answers, which is not the same as a host with no agents.
    pub reachable: bool,
    pub agents: Vec<AgentView>,
}

/// Lists the host's agents. Blocking: run it off the async runtime.
pub fn read(host_dir: &Path) -> AgentsView {
    let listed = hq_host::Client::connect(host_dir).and_then(|mut client| {
        client.set_timeout(Some(LIST_TIMEOUT));
        client.call("agent.list", json!({}))
    });
    match listed {
        Ok(reply) => AgentsView {
            reachable: true,
            agents: from_list(&reply),
        },
        Err(_) => AgentsView {
            reachable: false,
            agents: Vec::new(),
        },
    }
}

/// The agents in an `agent.list` reply, those that need the person first. Entries without a name
/// are skipped.
pub fn from_list(reply: &Value) -> Vec<AgentView> {
    let mut agents: Vec<AgentView> = reply
        .get("agents")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(view)
        .collect();
    agents.sort_by(|a, b| a.need.cmp(&b.need).then_with(|| a.name.cmp(&b.name)));
    agents
}

fn view(entry: &Value) -> Option<AgentView> {
    let text = |key: &str| entry.get(key).and_then(Value::as_str);
    Some(AgentView {
        name: text("name")?.to_string(),
        agent: text("agent").map(str::to_string),
        need: need(text("status"), text("state"), entry.get("done")),
        title: text("title").unwrap_or_default().to_string(),
        cwd: text("cwd").unwrap_or_default().to_string(),
        quiet_secs: entry.get("quiet_ms").and_then(Value::as_u64).unwrap_or(0) / MS_PER_SEC,
    })
}

fn need(status: Option<&str>, state: Option<&str>, done: Option<&Value>) -> Need {
    if status == Some("exited") {
        return Need::Exited;
    }
    if state == Some("blocked") {
        return Need::Blocked;
    }
    if done.and_then(Value::as_bool) == Some(true) {
        return Need::Done;
    }
    match state {
        Some("working") => Need::Working,
        Some("idle") => Need::Idle,
        _ => Need::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, status: &str, state: Option<&str>, done: bool) -> Value {
        json!({ "name": name, "status": status, "state": state, "done": done, "quiet_ms": 61_500 })
    }

    #[test]
    fn agents_that_need_the_person_come_first() {
        let reply = json!({ "agents": [
            entry("b-idle", "running", Some("idle"), false),
            entry("gone", "exited", Some("blocked"), false),
            entry("a-working", "running", Some("working"), false),
            entry("asks", "running", Some("blocked"), true),
            entry("finished", "running", Some("idle"), true),
            entry("plain", "running", None, false),
            { "status": "running" },
        ]});
        let order: Vec<(String, Need)> = from_list(&reply)
            .into_iter()
            .map(|a| (a.name, a.need))
            .collect();
        assert_eq!(
            order,
            [
                ("asks".to_string(), Need::Blocked),
                ("finished".to_string(), Need::Done),
                ("a-working".to_string(), Need::Working),
                ("b-idle".to_string(), Need::Idle),
                ("plain".to_string(), Need::Unknown),
                ("gone".to_string(), Need::Exited),
            ]
        );
        assert_eq!(from_list(&reply)[0].quiet_secs, 61);
        assert!(from_list(&json!({})).is_empty());
    }

    #[test]
    fn no_host_is_unreachable_not_empty() {
        let dir = tempfile::tempdir().unwrap();
        let view = read(&dir.path().join("run"));
        assert!(!view.reachable);
        assert!(view.agents.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn a_real_host_lists_its_agents() {
        use std::sync::Arc;
        let dir = tempfile::tempdir().unwrap();
        let run = dir.path().join("run");
        let server = hq_host::Server::bind(&run, Arc::new(hq_host::Host::new())).unwrap();
        let stop = server.stop_handle();
        let thread = std::thread::spawn(move || server.serve());

        assert_eq!(
            read(&run),
            AgentsView {
                reachable: true,
                agents: vec![]
            }
        );
        hq_host::Client::connect(&run)
            .unwrap()
            .call(
                "agent.spawn",
                json!({ "name": "sleeper", "argv": ["sleep", "30"], "cwd": dir.path() }),
            )
            .unwrap();
        let view = read(&run);
        assert!(view.reachable);
        assert_eq!(view.agents.len(), 1);
        assert_eq!(view.agents[0].name, "sleeper");
        let _ = hq_host::Client::connect(&run)
            .unwrap()
            .call("agent.kill", json!({ "name": "sleeper" }));

        stop.stop();
        let _ = thread.join();
    }
}
