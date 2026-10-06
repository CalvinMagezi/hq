use super::{
    AgentInfo, AgentStatus, HerdrError, Host, HostBackend, INVALID_KEYS_CODE, LaunchRequest,
    Launched, PromptOutcome, shell_line, validate_keys,
};
use hq_host::{Client, ClientError};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

const HOST_NAME: &str = "native";
/// How long a prompt may go without the agent reacting before Enter is retried.
const SUBMIT_CONFIRM: Duration = Duration::from_secs(10);
/// A state must hold this long before a wait reports it, so a flicker is ignored.
const STABLE_MS: u64 = 700;
const DEFAULT_COMMAND_TIMEOUT: Duration = Duration::from_secs(15);
const DEFAULT_LAUNCH_BOUND: Duration = Duration::from_secs(60);
const SETTLED: [AgentStatus; 2] = [AgentStatus::Idle, AgentStatus::Blocked];
const REACTED: [AgentStatus; 2] = [AgentStatus::Working, AgentStatus::Blocked];

/// The built-in agent host, reached over its control socket. A pane's name is
/// its workspace id and its pane id, so every herdr-shaped call maps onto it.
#[derive(Debug, Clone)]
pub struct NativeBackend {
    dir: PathBuf,
    launch_bound: Duration,
    command_timeout: Duration,
}

impl NativeBackend {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            launch_bound: DEFAULT_LAUNCH_BOUND,
            command_timeout: DEFAULT_COMMAND_TIMEOUT,
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn call(&self, method: &str, params: Value) -> Result<Value, HerdrError> {
        self.call_within(method, params, self.command_timeout)
    }

    fn call_within(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, HerdrError> {
        let mut client = Client::connect(&self.dir).map_err(|e| self.error(e))?;
        client.set_timeout(Some(timeout));
        client.call(method, params).map_err(|e| self.error(e))
    }

    fn error(&self, e: ClientError) -> HerdrError {
        match e {
            ClientError::Remote { code, message } => HerdrError::Api { code, message },
            other => HerdrError::Unreachable {
                host: HOST_NAME.into(),
                detail: other.to_string(),
            },
        }
    }

    /// Waits for one of `until`; a wait that times out is an `Ok(None)`.
    fn wait_for(
        &self,
        target: &str,
        until: &[AgentStatus],
        timeout: Duration,
    ) -> Result<Option<AgentInfo>, HerdrError> {
        let states: Vec<&str> = until.iter().map(status_name).collect();
        let params = json!({
            "name": target, "until": "state", "states": states,
            "stable_ms": STABLE_MS, "timeout_ms": timeout.as_millis() as u64,
        });
        match self.call_within("agent.wait", params, timeout + self.command_timeout) {
            Ok(v) => Ok(parse_info(&v)),
            Err(HerdrError::Api { code, .. }) if code == "timeout" => Ok(None),
            Err(e) => Err(e),
        }
    }
}

/// The program and arguments to start. A wrapper command runs through a shell,
/// the same way herdr types it into a pane.
fn argv_for(req: &LaunchRequest) -> Vec<String> {
    match &req.command {
        Some(command) => vec!["sh".into(), "-c".into(), shell_line(command, &req.args)],
        None => std::iter::once(binary_for(&req.kind))
            .chain(req.args.iter().cloned())
            .collect(),
    }
}

fn binary_for(kind: &str) -> String {
    match kind {
        "cursor" => "cursor-agent".into(),
        other => other.into(),
    }
}

fn status_name(s: &AgentStatus) -> &'static str {
    match s {
        AgentStatus::Idle | AgentStatus::Done => "idle",
        AgentStatus::Working => "working",
        AgentStatus::Blocked => "blocked",
        AgentStatus::Unknown => "unknown",
    }
}

fn parse_status(state: Option<&str>) -> AgentStatus {
    match state {
        Some("idle") => AgentStatus::Idle,
        Some("working") => AgentStatus::Working,
        Some("blocked") => AgentStatus::Blocked,
        _ => AgentStatus::Unknown,
    }
}

fn parse_info(v: &Value) -> Option<AgentInfo> {
    let name = v.get("name")?.as_str()?.to_string();
    let exited = v.get("status").and_then(Value::as_str) == Some("exited");
    let status = if exited {
        AgentStatus::Unknown
    } else {
        parse_status(v.get("state").and_then(Value::as_str))
    };
    let text = |key: &str| v.get(key).and_then(Value::as_str).map(str::to_string);
    Some(AgentInfo {
        kind: text("agent").unwrap_or_default(),
        status,
        pane_id: name.clone(),
        workspace_id: name.clone(),
        cwd: text("cwd").unwrap_or_default(),
        title: text("title").filter(|t| !t.is_empty()),
        // The host has no change counter yet; age moves on every call, so
        // callers that notify per change must key on status instead.
        state_change_seq: 0,
        launch_pending: false,
        name: Some(name),
    })
}

impl HostBackend for NativeBackend {
    fn name(&self) -> &str {
        HOST_NAME
    }
    fn checks_binaries(&self) -> bool {
        true
    }
    fn launch_bound(&self) -> Duration {
        self.launch_bound
    }

    fn version(&self) -> Result<String, HerdrError> {
        let status = self.call("host.status", json!({}))?;
        Ok(status
            .get("host_version")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string())
    }

    fn agents(&self) -> Result<Vec<AgentInfo>, HerdrError> {
        let list = self.call("agent.list", json!({}))?;
        let agents = list.get("agents").and_then(Value::as_array);
        Ok(agents
            .map(|a| a.iter().filter_map(parse_info).collect())
            .unwrap_or_default())
    }

    fn agent(&self, target: &str) -> Result<Option<AgentInfo>, HerdrError> {
        match self.call("agent.get", json!({ "name": target })) {
            Ok(v) => Ok(parse_info(&v)),
            Err(HerdrError::Api { code, .. }) if code == "agent_not_found" => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn launch(&self, req: &LaunchRequest) -> Result<Launched, HerdrError> {
        let env: serde_json::Map<String, Value> = req
            .env
            .iter()
            .map(|(k, v)| (k.clone(), Value::String(v.clone())))
            .collect();
        let resume = req.resume_args.as_ref().map(|args| {
            std::iter::once(binary_for(&req.kind))
                .chain(args.clone())
                .collect::<Vec<_>>()
        });
        let params = json!({
            "name": req.name, "argv": argv_for(req), "cwd": req.cwd,
            "agent": req.kind, "resume_argv": resume, "env": env,
        });
        let spawned = self.call("agent.spawn", params)?;
        // A kind without a rule file has no state to wait for.
        let detected = spawned.get("state").is_some_and(|s| !s.is_null());
        let ready = match detected {
            false => true,
            true => match self.wait_for(&req.name, &SETTLED, req.start_timeout) {
                Ok(Some(info)) => info.status == AgentStatus::Idle,
                Ok(None) => false,
                Err(e) => {
                    let _ = self.close_workspace(&req.name);
                    return Err(e);
                }
            },
        };
        Ok(Launched {
            workspace_id: req.name.clone(),
            pane_id: req.name.clone(),
            agent: self.agent(&req.name)?,
            ready,
        })
    }

    fn await_started(&self, name: &str, within: Duration) -> Result<Option<AgentInfo>, HerdrError> {
        match self.wait_for(name, &SETTLED, within)? {
            Some(info) => Ok(Some(info)),
            None => self.agent(name),
        }
    }

    fn prompt(
        &self,
        target: &str,
        text: &str,
        wait: Option<Duration>,
    ) -> Result<PromptOutcome, HerdrError> {
        self.call("agent.prompt", json!({ "name": target, "text": text }))?;
        let Some(limit) = wait else {
            return Ok(PromptOutcome::Submitted);
        };
        match self.wait_for(target, &SETTLED, limit)? {
            Some(info) => Ok(PromptOutcome::Settled(info)),
            None => Ok(PromptOutcome::TimedOut(format!(
                "{target} did not settle within {}s",
                limit.as_secs()
            ))),
        }
    }

    fn submit(&self, target: &str, text: &str) -> Result<PromptOutcome, HerdrError> {
        self.call("agent.prompt", json!({ "name": target, "text": text }))?;
        if self.wait_for(target, &REACTED, SUBMIT_CONFIRM)?.is_some() {
            return Ok(PromptOutcome::Submitted);
        }
        // An agent that was not listening yet can swallow the Enter. On an
        // empty input a second Enter does nothing, so the retry is safe.
        self.send_keys(target, &["enter".to_string()])?;
        Ok(PromptOutcome::Resubmitted)
    }

    fn send_keys(&self, target: &str, keys: &[String]) -> Result<(), HerdrError> {
        validate_keys(keys).map_err(|message| HerdrError::Api {
            code: INVALID_KEYS_CODE.to_string(),
            message,
        })?;
        self.call("agent.send_keys", json!({ "name": target, "keys": keys }))
            .map(|_| ())
    }

    fn send_text(&self, pane_id: &str, text: &str) -> Result<(), HerdrError> {
        self.call("agent.send_text", json!({ "name": pane_id, "text": text }))
            .map(|_| ())
    }

    fn read(&self, target: &str, lines: usize) -> Result<String, HerdrError> {
        self.read_sourced(target, lines).map(|(text, _)| text)
    }

    fn read_sourced(
        &self,
        target: &str,
        lines: usize,
    ) -> Result<(String, &'static str), HerdrError> {
        let v = self.call(
            "agent.read",
            json!({ "name": target, "source": "recent_unwrapped", "lines": lines }),
        )?;
        let text = v.get("text").and_then(Value::as_str).unwrap_or_default();
        Ok((text.to_string(), "recent-unwrapped"))
    }

    fn wait(
        &self,
        target: &str,
        until: &[AgentStatus],
        timeout: Duration,
    ) -> Result<AgentInfo, HerdrError> {
        self.wait_for(target, until, timeout)?
            .ok_or_else(|| HerdrError::Api {
                code: "timeout".into(),
                message: format!(
                    "{target} did not reach the state within {}s",
                    timeout.as_secs()
                ),
            })
    }

    fn close_workspace(&self, workspace_id: &str) -> Result<(), HerdrError> {
        match self.call("agent.remove", json!({ "name": workspace_id })) {
            Err(HerdrError::Api { code, .. }) if code == "agent_not_found" => Ok(()),
            other => other.map(|_| ()),
        }
    }

    fn shell_pid(&self, pane_id: &str) -> Option<u32> {
        let v = self.call("agent.get", json!({ "name": pane_id })).ok()?;
        v.get("pid")?.as_u64().map(|p| p as u32)
    }

    fn with_launch_bound_dyn(&self, bound: Duration) -> Host {
        Arc::new(Self {
            launch_bound: bound,
            ..self.clone()
        })
    }
    fn with_command_timeout_dyn(&self, timeout: Duration) -> Host {
        Arc::new(Self {
            command_timeout: timeout,
            ..self.clone()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(kind: &str, command: Option<&str>) -> LaunchRequest {
        LaunchRequest {
            name: "a".into(),
            kind: kind.into(),
            cwd: "/tmp".into(),
            label: String::new(),
            env: vec![],
            args: vec!["--flag".into(), "two words".into()],
            command: command.map(str::to_string),
            resume_args: None,
            start_timeout: Duration::from_secs(1),
        }
    }

    #[test]
    fn a_kind_runs_its_own_binary_with_the_args() {
        assert_eq!(
            argv_for(&req("claude", None)),
            ["claude", "--flag", "two words"]
        );
        assert_eq!(argv_for(&req("cursor", None))[0], "cursor-agent");
    }

    #[test]
    fn a_wrapper_command_runs_through_a_shell_with_quoted_args() {
        let argv = argv_for(&req("claude", Some("my-wrapper")));
        assert_eq!(argv[..2], ["sh", "-c"]);
        assert_eq!(argv[2], "my-wrapper --flag 'two words'");
    }

    #[test]
    fn states_map_and_an_exited_agent_is_unknown() {
        assert_eq!(parse_status(Some("blocked")), AgentStatus::Blocked);
        assert_eq!(parse_status(None), AgentStatus::Unknown);
        let exited = json!({"name": "a", "status": "exited", "state": "idle"});
        assert_eq!(parse_info(&exited).unwrap().status, AgentStatus::Unknown);
    }
}
