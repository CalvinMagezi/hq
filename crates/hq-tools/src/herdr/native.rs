use super::transport::Transport;
use super::{
    AgentInfo, AgentStatus, AwaitingAgent, HerdrError, Host, HostBackend, HostEvent, HostEvents,
    INVALID_KEYS_CODE, LaunchRequest, Launched, PromptOutcome, shell_line, validate_keys,
};
use hq_host::{Client, ClientError};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

const HOST_NAME: &str = "native";
/// How long a prompt may go without the agent reacting before Enter is retried.
const SUBMIT_CONFIRM: Duration = Duration::from_secs(10);
/// A state must hold this long before a wait reports it, so a flicker is ignored.
const STABLE_MS: u64 = 700;
const DEFAULT_COMMAND_TIMEOUT: Duration = Duration::from_secs(15);
const DEFAULT_LAUNCH_BOUND: Duration = Duration::from_secs(60);
/// Silence after the first output that counts as "finished drawing".
const DRAWN_QUIET_MS: u64 = 600;
const DRAWN_POLL: Duration = Duration::from_millis(100);
const SETTLED: [AgentStatus; 2] = [AgentStatus::Idle, AgentStatus::Blocked];
const REACTED: [AgentStatus; 2] = [AgentStatus::Working, AgentStatus::Blocked];

/// The built-in agent host, reached over its control socket. A pane's name is
/// its workspace id and its pane id, so every herdr-shaped call maps onto it.
#[derive(Debug, Clone)]
pub struct NativeBackend {
    link: Link,
    launch_bound: Duration,
    command_timeout: Duration,
}

/// How HQ reaches the host.
#[derive(Debug, Clone)]
enum Link {
    /// The host's socket on this machine.
    Local(PathBuf),
    /// A host on another machine, through its `hq host gate` over ssh.
    Remote {
        name: String,
        transport: Transport,
        ssh_program: String,
    },
}

/// What a remote host runs for each request: the gate command pinned to the key.
pub const NATIVE_GATE_COMMAND: &str = "hq host gate";

impl NativeBackend {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self::with_link(Link::Local(dir.into()))
    }

    /// A host on another machine named `name`, reached with `ssh`. The remote
    /// side runs `gate_command` (normally pinned in `authorized_keys`).
    pub fn remote(
        name: &str,
        target: &str,
        port: Option<u16>,
        identity_file: Option<String>,
        gate_command: &str,
        mux_dir: Option<PathBuf>,
    ) -> Self {
        Self::remote_with_program(name, target, port, identity_file, gate_command, mux_dir, "ssh")
    }

    /// Like `remote`, with the program that stands in for `ssh` (tests).
    pub fn remote_with_program(
        name: &str,
        target: &str,
        port: Option<u16>,
        identity_file: Option<String>,
        gate_command: &str,
        mux_dir: Option<PathBuf>,
        ssh_program: &str,
    ) -> Self {
        Self::with_link(Link::Remote {
            name: name.to_string(),
            transport: Transport::Ssh {
                target: target.to_string(),
                port,
                identity_file,
                gate_command: gate_command.to_string(),
                mux_dir,
            },
            ssh_program: ssh_program.to_string(),
        })
    }

    fn with_link(link: Link) -> Self {
        Self {
            link,
            launch_bound: DEFAULT_LAUNCH_BOUND,
            command_timeout: DEFAULT_COMMAND_TIMEOUT,
        }
    }

    pub fn with_launch_bound(mut self, bound: Duration) -> Self {
        self.launch_bound = bound;
        self
    }

    pub fn with_command_timeout(mut self, timeout: Duration) -> Self {
        self.command_timeout = timeout;
        self
    }

    /// The local socket directory; None for a remote host.
    pub fn dir(&self) -> Option<&Path> {
        match &self.link {
            Link::Local(dir) => Some(dir),
            Link::Remote { .. } => None,
        }
    }

    fn host_name(&self) -> &str {
        match &self.link {
            Link::Local(_) => HOST_NAME,
            Link::Remote { name, .. } => name,
        }
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
        match &self.link {
            Link::Local(dir) => {
                let mut client = Client::connect(dir).map_err(|e| self.error(e))?;
                client.set_timeout(Some(timeout));
                client.call(method, params).map_err(|e| self.error(e))
            }
            Link::Remote {
                name,
                transport,
                ssh_program,
            } => self.call_remote(transport, ssh_program, name, method, params, timeout),
        }
    }

    /// One request through the remote gate: it prints `{"result": ...}` or
    /// `{"error": {...}}` on stdout, and exits 64 with a message on stderr when
    /// it refuses the request.
    fn call_remote(
        &self,
        transport: &Transport,
        ssh_program: &str,
        name: &str,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, HerdrError> {
        let args = vec![method.to_string(), params.to_string()];
        let out = transport.run_with_program(ssh_program, name, &args, timeout)?;
        let unreachable = |detail: String| HerdrError::Unreachable {
            host: name.to_string(),
            detail,
        };
        if out.exit_code == hq_host::GATE_DENIED_EXIT {
            return Err(HerdrError::Api {
                code: "gate_denied".into(),
                message: out.stderr.trim().to_string(),
            });
        }
        if out.exit_code != 0 {
            return Err(unreachable(format!(
                "gate exited {}: {}",
                out.exit_code,
                out.stderr.lines().next().unwrap_or_default()
            )));
        }
        let reply: Value = serde_json::from_str(out.stdout.trim())
            .map_err(|e| unreachable(format!("unreadable gate reply: {e}")))?;
        if let Some(error) = reply.get("error") {
            let text = |k: &str| error.get(k).and_then(Value::as_str).unwrap_or_default();
            return match text("code") {
                "unreachable" => Err(unreachable(text("message").to_string())),
                code => Err(HerdrError::Api {
                    code: code.to_string(),
                    message: text("message").to_string(),
                }),
            };
        }
        reply
            .get("result")
            .cloned()
            .ok_or_else(|| unreachable("gate reply had no result".into()))
    }

    fn error(&self, e: ClientError) -> HerdrError {
        match e {
            ClientError::Remote { code, message } => HerdrError::Api { code, message },
            other => HerdrError::Unreachable {
                host: self.host_name().into(),
                detail: other.to_string(),
            },
        }
    }

    /// The request with the agent's reporting hooks added, so the host hears
    /// its state and conversation id from the agent itself. The host writes the
    /// settings file on the machine the agent runs on.
    fn with_hooks(&self, req: &LaunchRequest) -> LaunchRequest {
        let mut req = req.clone();
        let flags = self.hook_flags(&req.name, &req.kind);
        req.args.extend(flags.clone());
        if let Some(resume) = req.resume_args.as_mut() {
            resume.extend(flags);
        }
        req
    }

    /// Empty for other kinds, or when the host cannot write the file.
    fn hook_flags(&self, name: &str, kind: &str) -> Vec<String> {
        if kind != CLAUDE_KIND {
            return Vec::new();
        }
        let reply = self.call("agent.hook_flags", json!({ "name": name, "agent": kind }));
        let flags = reply.ok().and_then(|v| v.get("flags").cloned());
        flags
            .and_then(|f| serde_json::from_value(f).ok())
            .unwrap_or_default()
    }

    /// Whether the agent has drawn something and stopped for a moment. An empty
    /// screen reads as idle, so state alone cannot tell a CLI that is still
    /// starting from one that is ready (or waiting on a dialog).
    fn drawn(&self, name: &str, within: Duration) -> Result<bool, HerdrError> {
        let deadline = Instant::now() + within;
        loop {
            let info = self.call("agent.get", json!({ "name": name }))?;
            let seen = info.get("bytes_seen").and_then(Value::as_u64).unwrap_or(0);
            let quiet = info.get("quiet_ms").and_then(Value::as_u64).unwrap_or(0);
            if seen > 0 && quiet >= DRAWN_QUIET_MS {
                return Ok(true);
            }
            if Instant::now() >= deadline {
                return Ok(false);
            }
            std::thread::sleep(DRAWN_POLL);
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

const CLAUDE_KIND: &str = "claude";

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

fn parse_awaiting(v: &Value) -> Option<AwaitingAgent> {
    let keys = v.get("env_keys")?.as_array()?;
    Some(AwaitingAgent {
        name: v.get("name")?.as_str()?.to_string(),
        env_keys: keys
            .iter()
            .filter_map(|k| k.as_str().map(str::to_string))
            .collect(),
    })
}

fn parse_info(v: &Value) -> Option<AgentInfo> {
    let name = v.get("name")?.as_str()?.to_string();
    let exited = v.get("status").and_then(Value::as_str) == Some("exited");
    let done = v.get("done").and_then(Value::as_bool).unwrap_or(false);
    let status = match parse_status(v.get("state").and_then(Value::as_str)) {
        _ if exited => AgentStatus::Unknown,
        AgentStatus::Idle if done => AgentStatus::Done,
        other => other,
    };
    let text = |key: &str| v.get(key).and_then(Value::as_str).map(str::to_string);
    Some(AgentInfo {
        kind: text("agent").unwrap_or_default(),
        status,
        pane_id: name.clone(),
        workspace_id: name.clone(),
        cwd: text("cwd").unwrap_or_default(),
        title: text("title").filter(|t| !t.is_empty()),
        // Goes up on each state change; the supervisor alerts once per value.
        state_change_seq: v.get("state_seq").and_then(Value::as_u64).unwrap_or(0),
        launch_pending: false,
        agent_session_id: text("agent_session_id"),
        name: Some(name),
    })
}

impl HostBackend for NativeBackend {
    fn name(&self) -> &str {
        self.host_name()
    }
    fn checks_binaries(&self) -> bool {
        // HQ can only look at its own PATH. On a remote host a missing agent
        // binary shows up as the host's own `spawn_failed` error.
        matches!(self.link, Link::Local(_))
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
        let req = &self.with_hooks(req);
        let params = json!({
            "name": req.name, "argv": argv_for(req), "cwd": req.cwd,
            "agent": req.kind, "resume_argv": resume, "env": env,

        });
        let spawned = self.call("agent.spawn", params)?;
        // A kind without a rule file has no state to wait for.
        let detected = spawned.get("state").is_some_and(|s| !s.is_null());
        let began = Instant::now();
        let ready = match detected {
            false => true,
            true if !self.drawn(&req.name, req.start_timeout)? => false,
            true => match self.wait_for(
                &req.name,
                &SETTLED,
                req.start_timeout.saturating_sub(began.elapsed()),
            ) {
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

    fn poll_events(&self, after: Option<u64>, wait: Duration) -> Result<HostEvents, HerdrError> {
        let params = json!({ "after": after, "timeout_ms": wait.as_millis() as u64 });
        let v = self.call_within("events.poll", params, wait + self.command_timeout)?;
        let text = |e: &Value, k: &str| e.get(k).and_then(Value::as_str).map(str::to_string);
        let events = v.get("events").and_then(Value::as_array);
        Ok(HostEvents {
            events: events
                .map(|list| {
                    list.iter()
                        .filter_map(|e| {
                            Some(HostEvent {
                                seq: e.get("seq")?.as_u64()?,
                                name: text(e, "name")?,
                                kind: text(e, "kind")?,
                                state: text(e, "state"),
                            })
                        })
                        .collect()
                })
                .unwrap_or_default(),
            last_seq: v.get("last_seq").and_then(Value::as_u64).unwrap_or(0),
            lost: v.get("lost").and_then(Value::as_bool).unwrap_or(false),
        })
    }

    fn update_resume(&self, name: &str, kind: &str, args: Vec<String>) -> Result<(), HerdrError> {
        let mut argv = vec![binary_for(kind)];
        argv.extend(args);
        argv.extend(self.hook_flags(name, kind));
        self.call("agent.set_resume", json!({ "name": name, "argv": argv }))
            .map(|_| ())
    }

    fn awaiting(&self) -> Result<Vec<AwaitingAgent>, HerdrError> {
        let v = self.call("agent.awaiting", json!({}))?;
        let list = v.get("agents").and_then(Value::as_array);
        Ok(list
            .map(|a| a.iter().filter_map(parse_awaiting).collect())
            .unwrap_or_default())
    }

    fn resume_awaiting(&self, name: &str, env: Vec<(String, String)>) -> Result<(), HerdrError> {
        let env: serde_json::Map<String, Value> = env
            .into_iter()
            .map(|(k, v)| (k, Value::String(v)))
            .collect();
        self.call("agent.resume", json!({ "name": name, "env": env }))
            .map(|_| ())
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
