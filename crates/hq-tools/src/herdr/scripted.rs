//! A host driven by a script that answers the way the retired herdr CLI did
//! (JSON replies per command). It exists only as a test double: the harness
//! session and supervisor tests script a host's replies with it, and the real
//! backend is `NativeBackend`.

use super::backend::HostBackend;
use super::transport::{RawOutput, run_with_deadline};
use super::{
    AgentInfo, AgentStatus, HerdrError, Host, INVALID_KEYS_CODE, LaunchRequest, Launched, PromptOutcome,
    validate_keys,
};
use serde_json::Value;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

const MIN_WAIT_MS: u64 = 3_000;
const MAX_WAIT_MS: u64 = 300_000;

/// Extra time granted to the process beyond a Herdr-side wait, so Herdr's own
/// timeout error arrives instead of ours.
const WAIT_MARGIN: Duration = Duration::from_secs(15);

const READ_SOURCE: &str = "recent-unwrapped";

/// Herdr refuses a `recent` read of more than ~25 lines while an agent runs on
/// the alternate screen; `visible` has no such limit and is the live screen.
const VISIBLE_SOURCE: &str = "visible";
const AGENT_NOT_IDLE_CODE: &str = "agent_not_idle";

/// Herdr's own stall detector fires after five seconds without activity, so a
/// confirmation wait has to outlast it.
const SUBMIT_CONFIRM: Duration = Duration::from_secs(10);

/// Pane lines quoted when a wrapper never turns into a recognizable agent.
const UNDETECTED_SCREEN_LINES: usize = 20;

/// How often a wrapper launch asks whether Herdr has recognized an agent yet.
const DETECT_POLL: Duration = Duration::from_millis(500);


#[derive(Debug, Clone)]
pub struct ScriptedHost {
    name: String,
    binary: PathBuf,
    command_timeout: Duration,
    launch_bound: Duration,
    binary_preflight: bool,
}

impl ScriptedHost {
    /// A host that runs `binary` (a script standing in for the retired CLI) for every call.
    pub fn new(binary: impl Into<PathBuf>) -> Self {
        Self {
            name: "local".to_string(),
            binary: binary.into(),
            command_timeout: Duration::from_secs(10),
            launch_bound: Duration::from_secs(25),
            binary_preflight: true,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// Whether HQ can look for a harness binary itself. A remote host answers
    /// only through its gate, so its binaries are not visible from here.
    pub fn checks_binaries(&self) -> bool {
        self.binary_preflight
    }

    /// Longest a launch waits for the agent to come up.
    pub fn launch_bound(&self) -> Duration {
        self.launch_bound
    }

    pub fn with_launch_bound(mut self, bound: Duration) -> Self {
        self.launch_bound = bound;
        self
    }

    /// For tests that drive a fake herdr whose harness binaries do not exist.
    pub fn without_binary_preflight(mut self) -> Self {
        self.binary_preflight = false;
        self
    }

    /// Same host with a different ceiling for non-wait calls. The daemon's
    /// sweep uses a short one so a dead host cannot outlast its task timeout.
    pub fn with_command_timeout(mut self, timeout: Duration) -> Self {
        self.command_timeout = timeout;
        self
    }

    fn run(&self, args: &[&str], timeout: Duration) -> Result<RawOutput, HerdrError> {
        let mut command = Command::new(&self.binary);
        command.args(args);
        run_with_deadline(&mut command, None, timeout).map_err(|detail| HerdrError::Unreachable {
            host: self.name.clone(),
            detail,
        })
    }

    /// Herdr prints `{"id":..,"result":{..}}` on success and
    /// `{"error":{"code":..,"message":..}}` on failure.
    fn json(&self, args: &[&str], timeout: Duration) -> Result<Value, HerdrError> {
        let out = self.run(args, timeout)?;
        if out.exit_code != 0 {
            return Err(api_error(&out));
        }
        let value: Value =
            serde_json::from_str(out.stdout.trim()).map_err(|e| HerdrError::Api {
                code: "bad_response".into(),
                message: format!("not JSON ({e}): {}", truncate(&out.stdout, 200)),
            })?;
        Ok(value.get("result").cloned().unwrap_or(value))
    }

    fn text(&self, args: &[&str], timeout: Duration) -> Result<String, HerdrError> {
        let out = self.run(args, timeout)?;
        if out.exit_code != 0 {
            return Err(api_error(&out));
        }
        Ok(out.stdout)
    }

    pub fn version(&self) -> Result<String, HerdrError> {
        let out = self.text(&["status"], self.command_timeout)?;
        Ok(out
            .lines()
            .find_map(|l| l.trim().strip_prefix("version:"))
            .unwrap_or("unknown")
            .trim()
            .to_string())
    }

    pub fn agents(&self) -> Result<Vec<AgentInfo>, HerdrError> {
        let result = self.json(&["agent", "list"], self.command_timeout)?;
        Ok(result
            .get("agents")
            .and_then(Value::as_array)
            .map(|list| list.iter().filter_map(agent_from_json).collect())
            .unwrap_or_default())
    }

    /// `Ok(None)` means the host answered and no such agent exists.
    pub fn agent(&self, target: &str) -> Result<Option<AgentInfo>, HerdrError> {
        match self.json(&["agent", "get", target], self.command_timeout) {
            Ok(result) => Ok(result.get("agent").and_then(agent_from_json)),
            Err(e) if e.code() == Some("agent_not_found") => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Opens a workspace and starts the agent in its root pane. A workspace
    /// per session keeps each one closable without touching anyone else's.
    pub fn launch(&self, req: &LaunchRequest) -> Result<Launched, HerdrError> {
        let mut create = vec![
            "workspace",
            "create",
            "--cwd",
            &req.cwd,
            "--label",
            &req.label,
            "--no-focus",
        ];
        let env_pairs: Vec<String> = req.env.iter().map(|(k, v)| format!("{k}={v}")).collect();
        for pair in &env_pairs {
            create.extend(["--env", pair]);
        }
        let created = self.json(&create, self.command_timeout)?;
        let workspace_id = str_at(&created, &["workspace", "workspace_id"])?;
        let pane_id = str_at(&created, &["root_pane", "pane_id"])
            .map_err(|e| self.abandon(&workspace_id, e))?;

        let started = match &req.command {
            Some(command) => self.start_wrapped(req, &pane_id, command),
            None => self.start_native(req, &pane_id),
        };
        let result = started
            .and_then(|ready| self.launched(workspace_id.clone(), pane_id, &req.name, ready));
        result.map_err(|e| self.abandon(&workspace_id, e))
    }

    /// Closes a workspace whose launch failed and returns the error to report,
    /// saying so if the close failed too, so nothing is left behind silently.
    fn abandon(&self, workspace_id: &str, error: HerdrError) -> HerdrError {
        let Err(close) = self.close_workspace(workspace_id) else {
            return error;
        };
        let note =
            format!("; workspace {workspace_id} could not be closed ({close}), close it by hand");
        match error {
            HerdrError::Api { code, message } => HerdrError::Api {
                code,
                message: message + &note,
            },
            HerdrError::Unreachable { host, detail } => HerdrError::Unreachable {
                host,
                detail: detail + &note,
            },
        }
    }

    /// `herdr agent start`; `Ok(false)` when the agent is up but blocked on a
    /// dialog before its prompt.
    fn start_native(&self, req: &LaunchRequest, pane_id: &str) -> Result<bool, HerdrError> {
        let start_ms = clamp_ms(req.start_timeout).to_string();
        let mut start = vec![
            "agent",
            "start",
            &req.name,
            "--kind",
            &req.kind,
            "--pane",
            pane_id,
            "--timeout",
            &start_ms,
        ];
        if !req.args.is_empty() {
            start.push("--");
            start.extend(req.args.iter().map(String::as_str));
        }
        match self.json(&start, req.start_timeout + WAIT_MARGIN) {
            Ok(_) => Ok(true),
            Err(e) if e.code() == Some("agent_not_ready") => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// Types `command` and its arguments into the pane's shell (`pane run`
    /// presses Enter in the same write), waits for Herdr to recognize the agent
    /// that appears, and names it so every later call can address it.
    fn start_wrapped(
        &self,
        req: &LaunchRequest,
        pane_id: &str,
        command: &str,
    ) -> Result<bool, HerdrError> {
        let deadline = Instant::now() + req.start_timeout;
        let line = shell_line(command, &req.args);
        // `pane run` prints nothing on success, so there is no JSON to parse.
        self.text(&["pane", "run", pane_id, &line], self.command_timeout)?;
        if !self.await_agent(pane_id, deadline)? {
            let screen = self
                .read(pane_id, UNDETECTED_SCREEN_LINES)
                .unwrap_or_default();
            return Err(HerdrError::Api {
                code: "agent_not_detected".into(),
                message: format!(
                    "Herdr found no {} agent after running `{line}`; the command must end up running that CLI. Screen:\n{}",
                    req.kind,
                    screen.trim_end()
                ),
            });
        }
        self.json(
            &["agent", "rename", pane_id, &req.name],
            self.command_timeout,
        )?;
        let settle_for = deadline.saturating_duration_since(Instant::now());
        let until = [AgentStatus::Idle, AgentStatus::Done, AgentStatus::Blocked];
        match self.wait(&req.name, &until, settle_for) {
            Ok(agent) => Ok(matches!(
                agent.status,
                AgentStatus::Idle | AgentStatus::Done
            )),
            Err(e) if e.code() == Some("timeout") => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// True once Herdr reports any agent in the pane, false if `deadline`
    /// passes first. A status of `unknown` still counts: the agent is there,
    /// Herdr just has not classified it yet.
    fn await_agent(&self, pane_id: &str, deadline: Instant) -> Result<bool, HerdrError> {
        loop {
            if self.agent(pane_id)?.is_some() {
                return Ok(true);
            }
            if Instant::now() >= deadline {
                return Ok(false);
            }
            std::thread::sleep(DETECT_POLL);
        }
    }

    /// Polls until Herdr reports the agent past its launch (see
    /// `AgentInfo::is_started`) or `within` runs out, and returns the last
    /// observation either way. At least one poll is made.
    pub fn await_started(
        &self,
        name: &str,
        within: Duration,
    ) -> Result<Option<AgentInfo>, HerdrError> {
        let deadline = Instant::now() + within;
        loop {
            let seen = self.agent(name)?;
            if seen.as_ref().is_some_and(AgentInfo::is_started) || Instant::now() >= deadline {
                return Ok(seen);
            }
            std::thread::sleep(DETECT_POLL);
        }
    }

    fn launched(
        &self,
        workspace_id: String,
        pane_id: String,
        name: &str,
        ready: bool,
    ) -> Result<Launched, HerdrError> {
        let agent = self.agent(name)?;
        Ok(Launched {
            workspace_id,
            pane_id,
            agent,
            ready,
        })
    }

    /// Submits text to the agent's prompt without confirming it started a turn;
    /// see `submit` for that.
    pub fn prompt(
        &self,
        target: &str,
        text: &str,
        wait: Option<Duration>,
    ) -> Result<PromptOutcome, HerdrError> {
        let safe = option_safe(text);
        let mut args = vec!["agent", "prompt", target, &safe];
        let ms = wait.map(|d| clamp_ms(d).to_string());
        if let Some(ms) = &ms {
            args.extend(["--wait", "--timeout", ms]);
        }
        let process_timeout = wait.map_or(self.command_timeout, |d| d + WAIT_MARGIN);
        match self.json(&args, process_timeout) {
            Ok(result) => Ok(result
                .get("agent")
                .and_then(agent_from_json)
                .map_or(PromptOutcome::Submitted, PromptOutcome::Settled)),
            Err(HerdrError::Api { code, message }) if code == "agent_prompt_stalled" => {
                Ok(PromptOutcome::Stalled(message))
            }
            Err(HerdrError::Api { code, message }) if code == "timeout" => {
                Ok(PromptOutcome::TimedOut(message))
            }
            Err(e) => Err(e),
        }
    }

    /// Submits a prompt and confirms the agent reacted. `agent prompt` sends the
    /// text and Enter together, but a TUI that is not listening yet can swallow
    /// the Enter and leave the text sitting in its input box. Herdr reports that
    /// as a stall, so press Enter once more. On an empty input Enter does
    /// nothing, which makes the retry safe when the agent had simply finished.
    pub fn submit(&self, target: &str, text: &str) -> Result<PromptOutcome, HerdrError> {
        let safe = option_safe(text);
        let ms = clamp_ms(SUBMIT_CONFIRM).to_string();
        let args = [
            "agent",
            "prompt",
            target,
            &safe,
            "--wait",
            "--until",
            "working",
            "--until",
            "blocked",
            "--until",
            "done",
            "--timeout",
            &ms,
        ];
        match self.json(&args, SUBMIT_CONFIRM + WAIT_MARGIN) {
            Ok(_) => Ok(PromptOutcome::Submitted),
            Err(HerdrError::Api { code, .. }) if code == "agent_prompt_stalled" => {
                self.send_keys(target, &["enter".to_string()])?;
                Ok(PromptOutcome::Resubmitted)
            }
            Err(HerdrError::Api { code, message }) if code == "timeout" => {
                Ok(PromptOutcome::TimedOut(message))
            }
            Err(e) => Err(e),
        }
    }

    /// Logical keys (`enter`, `esc`, `down`, `ctrl+c`) for answering a dialog.
    pub fn send_keys(&self, target: &str, keys: &[String]) -> Result<(), HerdrError> {
        validate_keys(keys).map_err(|message| HerdrError::Api {
            code: INVALID_KEYS_CODE.to_string(),
            message,
        })?;
        let mut args = vec!["agent", "send-keys", target];
        args.extend(keys.iter().map(String::as_str));
        self.json(&args, self.command_timeout).map(|_| ())
    }

    /// Types text into a pane without pressing Enter.
    pub fn send_text(&self, pane_id: &str, text: &str) -> Result<(), HerdrError> {
        self.json(&["pane", "send-text", pane_id, text], self.command_timeout)
            .map(|_| ())
    }

    /// Recent output with soft wraps joined. Works for any pane id, so it also
    /// reads agents a person started.
    pub fn read(&self, target: &str, lines: usize) -> Result<String, HerdrError> {
        self.read_sourced(target, lines).map(|(text, _)| text)
    }

    /// `read` plus the herdr `--source` that produced the text. A working
    /// agent refuses a deep `recent` read (`agent_not_idle`), so that one
    /// error, and only it, is retried once as a `visible` read. Reads are
    /// idempotent, and the retry is a separate logical call from the
    /// transport's pre-session ssh retry, so neither multiplies the other.
    pub fn read_sourced(
        &self,
        target: &str,
        lines: usize,
    ) -> Result<(String, &'static str), HerdrError> {
        match self.read_from(target, lines, READ_SOURCE) {
            Ok(text) => Ok((text, READ_SOURCE)),
            Err(HerdrError::Api { code, .. }) if code == AGENT_NOT_IDLE_CODE => self
                .read_from(target, lines, VISIBLE_SOURCE)
                .map(|text| (text, VISIBLE_SOURCE)),
            Err(e) => Err(e),
        }
    }

    fn read_from(&self, target: &str, lines: usize, source: &str) -> Result<String, HerdrError> {
        let n = lines.to_string();
        self.text(
            &["agent", "read", target, "--source", source, "--lines", &n],
            self.command_timeout,
        )
    }

    pub fn wait(
        &self,
        target: &str,
        until: &[AgentStatus],
        timeout: Duration,
    ) -> Result<AgentInfo, HerdrError> {
        let ms = clamp_ms(timeout).to_string();
        let mut args = vec!["agent", "wait", target];
        for status in until {
            args.extend(["--until", status.as_str()]);
        }
        args.extend(["--timeout", &ms]);
        let result = self.json(&args, timeout + WAIT_MARGIN)?;
        result
            .get("agent")
            .and_then(agent_from_json)
            .ok_or_else(|| HerdrError::Api {
                code: "bad_response".into(),
                message: "wait returned no agent".into(),
            })
    }

    pub fn close_workspace(&self, workspace_id: &str) -> Result<(), HerdrError> {
        self.json(&["workspace", "close", workspace_id], self.command_timeout)
            .map(|_| ())
    }

    /// PID of the pane's shell, for callers that also track a local process.
    pub fn shell_pid(&self, pane_id: &str) -> Option<u32> {
        let result = self
            .json(
                &["pane", "process-info", "--pane", pane_id],
                self.command_timeout,
            )
            .ok()?;
        let pid = result.get("process_info")?.get("shell_pid")?.as_u64()?;
        u32::try_from(pid).ok()
    }
}

/// Text starting with `-` would be read as an option by Herdr's parser, so it
/// gets a leading space.
fn option_safe(text: &str) -> String {
    if text.starts_with('-') {
        format!(" {text}")
    } else {
        text.to_string()
    }
}

/// `command` is typed as written, so it may carry its own arguments or a `~`.
/// Arguments are quoted for a POSIX shell.
fn shell_line(command: &str, args: &[String]) -> String {
    let mut line = command.to_string();
    for arg in args {
        line.push(' ');
        line.push_str(&shell_quote(arg));
    }
    line
}

fn shell_quote(arg: &str) -> String {
    let plain = !arg.is_empty()
        && arg
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./:=@%+,".contains(c));
    if plain {
        arg.to_string()
    } else {
        format!("'{}'", arg.replace('\'', r"'\''"))
    }
}

fn clamp_ms(d: Duration) -> u64 {
    (d.as_millis() as u64).clamp(MIN_WAIT_MS, MAX_WAIT_MS)
}

fn truncate(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

fn str_at(v: &Value, path: &[&str]) -> Result<String, HerdrError> {
    let mut cur = v;
    for key in path {
        cur = cur.get(*key).unwrap_or(&Value::Null);
    }
    cur.as_str()
        .map(str::to_string)
        .ok_or_else(|| HerdrError::Api {
            code: "bad_response".into(),
            message: format!("missing {}", path.join(".")),
        })
}

fn api_error(out: &RawOutput) -> HerdrError {
    for stream in [&out.stderr, &out.stdout] {
        let parsed: Option<Value> = serde_json::from_str(stream.trim()).ok();
        let Some(err) = parsed.as_ref().and_then(|v| v.get("error")) else {
            continue;
        };
        return HerdrError::Api {
            code: err
                .get("code")
                .and_then(Value::as_str)
                .unwrap_or("error")
                .to_string(),
            message: err
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        };
    }
    HerdrError::Api {
        code: format!("exit_{}", out.exit_code),
        message: truncate(&format!("{} {}", out.stderr.trim(), out.stdout.trim()), 300),
    }
}


impl HostBackend for ScriptedHost {
    fn name(&self) -> &str {
        ScriptedHost::name(self)
    }
    fn checks_binaries(&self) -> bool {
        ScriptedHost::checks_binaries(self)
    }
    fn launch_bound(&self) -> Duration {
        ScriptedHost::launch_bound(self)
    }
    fn version(&self) -> Result<String, HerdrError> {
        ScriptedHost::version(self)
    }
    fn agents(&self) -> Result<Vec<AgentInfo>, HerdrError> {
        ScriptedHost::agents(self)
    }
    fn agent(&self, target: &str) -> Result<Option<AgentInfo>, HerdrError> {
        ScriptedHost::agent(self, target)
    }
    fn launch(&self, req: &LaunchRequest) -> Result<Launched, HerdrError> {
        ScriptedHost::launch(self, req)
    }
    fn await_started(&self, name: &str, within: Duration) -> Result<Option<AgentInfo>, HerdrError> {
        ScriptedHost::await_started(self, name, within)
    }
    fn prompt(
        &self,
        target: &str,
        text: &str,
        wait: Option<Duration>,
    ) -> Result<PromptOutcome, HerdrError> {
        ScriptedHost::prompt(self, target, text, wait)
    }
    fn submit(&self, target: &str, text: &str) -> Result<PromptOutcome, HerdrError> {
        ScriptedHost::submit(self, target, text)
    }
    fn send_keys(&self, target: &str, keys: &[String]) -> Result<(), HerdrError> {
        ScriptedHost::send_keys(self, target, keys)
    }
    fn send_text(&self, pane_id: &str, text: &str) -> Result<(), HerdrError> {
        ScriptedHost::send_text(self, pane_id, text)
    }
    fn read(&self, target: &str, lines: usize) -> Result<String, HerdrError> {
        ScriptedHost::read(self, target, lines)
    }
    fn read_sourced(
        &self,
        target: &str,
        lines: usize,
    ) -> Result<(String, &'static str), HerdrError> {
        ScriptedHost::read_sourced(self, target, lines)
    }
    fn wait(
        &self,
        target: &str,
        until: &[AgentStatus],
        timeout: Duration,
    ) -> Result<AgentInfo, HerdrError> {
        ScriptedHost::wait(self, target, until, timeout)
    }
    fn close_workspace(&self, workspace_id: &str) -> Result<(), HerdrError> {
        ScriptedHost::close_workspace(self, workspace_id)
    }
    fn shell_pid(&self, pane_id: &str) -> Option<u32> {
        ScriptedHost::shell_pid(self, pane_id)
    }
    fn with_launch_bound_dyn(&self, bound: Duration) -> Host {
        Arc::new(self.clone().with_launch_bound(bound))
    }
    fn with_command_timeout_dyn(&self, timeout: Duration) -> Host {
        Arc::new(self.clone().with_command_timeout(timeout))
    }
}

fn agent_from_json(v: &Value) -> Option<AgentInfo> {
    let text = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
    Some(AgentInfo {
        name: text("name"),
        kind: text("agent")?,
        status: AgentStatus::parse(v.get("agent_status")?.as_str()?),
        pane_id: text("pane_id")?,
        workspace_id: text("workspace_id")?,
        cwd: text("cwd").unwrap_or_default(),
        title: text("terminal_title_stripped"),
        state_change_seq: v
            .get("state_change_seq")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        launch_pending: v
            .get("launch_pending")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        agent_session_id: None,
    })
}
