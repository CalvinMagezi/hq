//! Runs one call to a remote host through ssh, under a hard deadline so a dead
//! host can never wedge a daemon sweep.

use super::AgentHostError;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const POLL_INTERVAL: Duration = Duration::from_millis(20);
const SSH_CONNECT_TIMEOUT_SECS: u32 = 8;

/// How long the shared master connection outlives its last client.
const SSH_MUX_PERSIST_SECS: u32 = 120;
/// A dead master (laptop asleep, network gone) notices after interval x count
/// seconds and exits, instead of leaving later calls to run into the deadline.
const SSH_ALIVE_INTERVAL_SECS: u32 = 15;
const SSH_ALIVE_COUNT_MAX: u32 = 2;
/// `%C` expands to a 40 character hash; ssh adds a 17 character temp suffix
/// while binding. Unix socket paths are limited to 103 bytes on macOS.
const MUX_SOCKET_NAME_LEN: usize = 1 + 40 + 17;
const MAX_SOCKET_PATH_LEN: usize = 103;
const MUX_DIR_MODE: u32 = 0o700;

/// ssh exits 255 for its own failures (no route, auth refused, host key), as
/// opposed to the remote command's exit status.
const SSH_FAILURE_EXIT: i32 = 255;

#[derive(Debug, Clone)]
pub(super) enum Transport {
    Ssh {
        target: String,
        port: Option<u16>,
        identity_file: Option<String>,
        gate_command: String,
        /// Private directory holding the shared control sockets; `None` means
        /// every call opens its own connection.
        mux_dir: Option<PathBuf>,
    },
}

#[derive(Debug)]
pub(super) struct RawOutput {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl Transport {
    pub(super) fn run_with_program(
        &self,
        ssh_program: &str,
        host: &str,
        args: &[String],
        timeout: Duration,
    ) -> Result<RawOutput, AgentHostError> {
        let unreachable = |detail: String| AgentHostError::Unreachable {
            host: host.to_string(),
            detail,
        };
        let started = Instant::now();
        let out = self
            .attempt(ssh_program, args, timeout)
            .map_err(unreachable)?;
        let out = match self {
            Transport::Ssh {
                mux_dir: Some(_), ..
            } if is_read_only(args) && mux_failed_before_session(&out) => {
                tracing::warn!(host, detail = %first_line(&out.stderr), "ssh multiplexing failed, retrying on a fresh connection");
                self.without_mux()
                    .attempt(ssh_program, args, timeout)
                    .map_err(unreachable)?
            }
            _ => out,
        };
        tracing::debug!(host, subcommand = ?loggable(args), elapsed_ms = started.elapsed().as_millis() as u64, exit = out.exit_code, "host call");
        if matches!(self, Transport::Ssh { .. }) && out.exit_code == SSH_FAILURE_EXIT {
            return Err(unreachable(first_line(&out.stderr)));
        }
        Ok(out)
    }

    fn attempt(
        &self,
        ssh_program: &str,
        args: &[String],
        timeout: Duration,
    ) -> Result<RawOutput, String> {
        let (mut command, stdin_payload) = self.command(ssh_program, args)?;
        run_with_deadline(&mut command, stdin_payload, timeout)
    }

    fn without_mux(&self) -> Transport {
        match self.clone() {
            Transport::Ssh {
                target,
                port,
                identity_file,
                gate_command,
                ..
            } => Transport::Ssh {
                target,
                port,
                identity_file,
                gate_command,
                mux_dir: None,
            },
        }
    }

    fn command(
        &self,
        ssh_program: &str,
        args: &[String],
    ) -> Result<(Command, Option<Vec<u8>>), String> {
        match self {
            Transport::Ssh {
                target,
                port,
                identity_file,
                gate_command,
                mux_dir,
            } => {
                // Arguments travel as JSON on stdin so no remote shell ever
                // parses prompt text.
                let payload = serde_json::to_vec(args).map_err(|e| e.to_string())?;
                let mut c = Command::new(ssh_program);
                c.args(ssh_args(
                    target,
                    *port,
                    identity_file.as_deref(),
                    gate_command,
                    mux_dir.as_deref(),
                ));
                Ok((c, Some(payload)))
            }
        }
    }
}

pub(super) fn ssh_args(
    target: &str,
    port: Option<u16>,
    identity_file: Option<&str>,
    gate: &str,
    mux_dir: Option<&Path>,
) -> Vec<String> {
    let mut args = vec![
        "-o".to_string(),
        "BatchMode=yes".to_string(),
        "-o".to_string(),
        format!("ConnectTimeout={SSH_CONNECT_TIMEOUT_SECS}"),
        "-T".to_string(),
    ];
    if let Some(port) = port {
        args.extend(["-p".to_string(), port.to_string()]);
    }
    if let Some(key) = identity_file {
        args.extend(["-i".to_string(), key.to_string(), "-o".to_string()]);
        args.push("IdentitiesOnly=yes".to_string());
    }
    if let Some(dir) = mux_dir {
        args.extend(mux_options(dir));
    }
    args.push(target.to_string());
    args.push(gate.to_string());
    args
}

fn mux_options(dir: &Path) -> Vec<String> {
    [
        "ControlMaster=auto".to_string(),
        format!("ControlPersist={SSH_MUX_PERSIST_SECS}"),
        format!("ControlPath={}/%C", dir.display()),
        format!("ServerAliveInterval={SSH_ALIVE_INTERVAL_SECS}"),
        format!("ServerAliveCountMax={SSH_ALIVE_COUNT_MAX}"),
    ]
    .into_iter()
    .flat_map(|opt| ["-o".to_string(), opt])
    .collect()
}

/// The part of a request that is safe to log. A host request is words (`agent
/// get`); a native host's is a method and its params as JSON text, and the params
/// can hold a secret (an agent's session token), so only the method is kept.
fn loggable(args: &[String]) -> Vec<&str> {
    let take = match args.first() {
        Some(first) if first.contains('.') => 1,
        _ => 2,
    };
    args.iter().take(take).map(String::as_str).collect()
}

/// The host arguments that only look at state. Only these may be re-run, because
/// a retry of a write (send-text, a prompt, a launch) could apply it twice.
fn is_read_only(args: &[String]) -> bool {
    let mut words = args.iter().map(String::as_str);
    let mut first = words.next();
    if first == Some("--session") {
        words.next();
        first = words.next();
    }
    matches!(
        (first, words.next()),
        (Some("status"), _)
            | (Some("agent"), Some("get" | "list" | "read"))
            | (Some("host.status" | "events.poll" | "agent.get" | "agent.list" | "agent.read"), _)
    )
}

/// Complaints ssh makes before any session is opened on the master, so the
/// remote command cannot have run. A broken pipe mid-session ("read from
/// master failed") is deliberately absent: the gate may already have run.
const PRE_SESSION_MUX_ERRORS: [&str; 6] = [
    "control socket connect",
    "cannot bind to path",
    "too long for unix domain socket",
    "controlpath",
    "mux_client_hello_exchange",
    "session open refused",
];

fn mux_failed_before_session(out: &RawOutput) -> bool {
    if out.exit_code != SSH_FAILURE_EXIT {
        return false;
    }
    let err = out.stderr.to_lowercase();
    PRE_SESSION_MUX_ERRORS.iter().any(|m| err.contains(m))
}

/// Creates (or validates) the private directory for control sockets. Returns
/// `None`, so callers use plain connections, when it cannot be made safe.
pub(super) fn prepare_mux_dir(dir: &Path) -> Option<PathBuf> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    if dir.as_os_str().len() + MUX_SOCKET_NAME_LEN > MAX_SOCKET_PATH_LEN {
        tracing::warn!(dir = %dir.display(), "ssh control socket path would be too long, multiplexing disabled");
        return None;
    }
    let made = std::fs::DirBuilder::new()
        .recursive(true)
        .mode(MUX_DIR_MODE)
        .create(dir);
    let meta = made.and_then(|()| std::fs::symlink_metadata(dir));
    let ok = meta.is_ok_and(|m| {
        m.is_dir()
            && (m.permissions().mode() & 0o077 == 0
                || std::fs::set_permissions(dir, std::fs::Permissions::from_mode(MUX_DIR_MODE))
                    .is_ok())
    });
    if !ok {
        tracing::warn!(dir = %dir.display(), "cannot prepare ssh control socket directory, multiplexing disabled");
        return None;
    }
    Some(dir.to_path_buf())
}

fn first_line(text: &str) -> String {
    text.lines().next().unwrap_or("").trim().to_string()
}

pub(super) fn run_with_deadline(
    command: &mut Command,
    stdin_payload: Option<Vec<u8>>,
    timeout: Duration,
) -> Result<RawOutput, String> {
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|e| format!("cannot start {:?}: {e}", command.get_program()))?;

    if let Some(mut stdin) = child.stdin.take() {
        // A gate that rejects the call closes stdin early; the exit status
        // reports that, so a failed write is not an error here.
        let _ = stdin.write_all(&stdin_payload.unwrap_or_default());
    }
    let stdout = drain(child.stdout.take());
    let stderr = drain(child.stderr.take());

    let status = wait_or_kill(&mut child, timeout)?;
    Ok(RawOutput {
        exit_code: status.code().unwrap_or(-1),
        stdout: stdout.join().unwrap_or_default(),
        stderr: stderr.join().unwrap_or_default(),
    })
}

fn drain<R: Read + Send + 'static>(pipe: Option<R>) -> std::thread::JoinHandle<String> {
    std::thread::spawn(move || {
        let mut buf = String::new();
        if let Some(mut p) = pipe {
            let _ = p.read_to_string(&mut buf);
        }
        buf
    })
}

fn wait_or_kill(child: &mut Child, timeout: Duration) -> Result<std::process::ExitStatus, String> {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) => {}
            Err(e) => return Err(format!("wait failed: {e}")),
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("timed out after {}s", timeout.as_secs()));
        }
        std::thread::sleep(POLL_INTERVAL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssh_args_pin_the_key_and_never_prompt() {
        let args = ssh_args("me@100.64.0.1", None, Some("/k/id"), "hq host gate", None);
        assert!(args.contains(&"BatchMode=yes".to_string()));
        assert!(args.contains(&"IdentitiesOnly=yes".to_string()));
        assert_eq!(args[args.len() - 2], "me@100.64.0.1");
        assert_eq!(args.last().unwrap(), "hq host gate");
    }

    #[test]
    fn a_native_requests_params_never_reach_the_log() {
        let native = vec!["agent.mcp_config".to_string(), r#"{"token":"hqs_secret"}"#.to_string()];
        assert_eq!(loggable(&native), ["agent.mcp_config"]);
        let host_cfg = vec!["agent".to_string(), "get".to_string(), "x".to_string()];
        assert_eq!(loggable(&host_cfg), ["agent", "get"]);
    }

    #[test]
    fn a_non_default_port_is_passed_to_ssh() {
        let args = ssh_args("me@h", Some(2222), None, "gate", None);
        let at = args.iter().position(|a| a == "-p").expect("-p missing");
        assert_eq!(args[at + 1], "2222");
        assert!(!ssh_args("me@h", None, None, "gate", None).contains(&"-p".to_string()));
    }

    #[test]
    fn ssh_args_without_a_key_leave_identity_to_ssh() {
        let args = ssh_args("me@host", None, None, "gate", None);
        assert!(!args.contains(&"-i".to_string()));
    }

    #[test]
    fn plain_ssh_argv_is_exact() {
        let args = ssh_args("me@h", None, Some("/k/id"), "gate", None);
        assert_eq!(
            args,
            [
                "-o",
                "BatchMode=yes",
                "-o",
                "ConnectTimeout=8",
                "-T",
                "-i",
                "/k/id",
                "-o",
                "IdentitiesOnly=yes",
                "me@h",
                "gate"
            ]
        );
    }

    #[test]
    fn multiplexed_argv_adds_only_connection_options_before_the_target() {
        let dir = Path::new("/run/hq");
        let args = ssh_args("me@h", None, Some("/k/id"), "gate", Some(dir));
        let plain = ssh_args("me@h", None, Some("/k/id"), "gate", None);
        let target_at = plain.len() - 2;
        let mut expected = plain[..target_at].to_vec();
        expected.extend(
            [
                "-o",
                "ControlMaster=auto",
                "-o",
                "ControlPersist=120",
                "-o",
                "ControlPath=/run/hq/%C",
                "-o",
                "ServerAliveInterval=15",
                "-o",
                "ServerAliveCountMax=2",
            ]
            .map(String::from),
        );
        expected.extend(plain[target_at..].iter().cloned());
        assert_eq!(args, expected);
        // The gate model is untouched: same key pinning, batch mode, target and command.
        for must in ["BatchMode=yes", "IdentitiesOnly=yes", "-T", "-i", "/k/id"] {
            assert!(args.contains(&must.to_string()), "{must}");
        }
        assert_eq!(args[args.len() - 2], "me@h");
        assert_eq!(args.last().unwrap(), "gate");
    }

    #[test]
    fn mux_dir_is_private_and_rejects_paths_too_long_for_a_socket() {
        use std::os::unix::fs::PermissionsExt;
        // The default temp dir on macOS is already too long for a socket path.
        let tmp = tempfile::tempdir_in("/tmp").unwrap();
        let dir = tmp.path().join("ssh");
        assert_eq!(prepare_mux_dir(&dir).as_deref(), Some(dir.as_path()));
        assert_eq!(
            std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
            0o700
        );
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(prepare_mux_dir(&dir).is_some());
        assert_eq!(
            std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
            0o700
        );
        let long = tmp.path().join("x".repeat(90));
        assert!(prepare_mux_dir(&long).is_none());
    }

    fn failure(code: i32, err: &str) -> RawOutput {
        RawOutput {
            exit_code: code,
            stdout: String::new(),
            stderr: err.into(),
        }
    }

    fn argv(words: &[&str]) -> Vec<String> {
        words.iter().map(|w| w.to_string()).collect()
    }

    #[test]
    fn only_pre_session_control_socket_errors_trigger_the_fallback() {
        for err in [
            "Control socket connect(/x/y): Connection refused",
            "unix_listener: cannot bind to path /x/y.abc: No such file",
            "unix_listener: path \"/very/long\" too long for Unix domain socket",
            "ControlPath too long",
            "mux_client_hello_exchange: read from master failed: Broken pipe",
            "mux_client_request_session: session request failed: Session open refused by peer",
        ] {
            assert!(mux_failed_before_session(&failure(255, err)), "{err}");
        }
        for err in [
            "mux_client_request_session: read from master failed: Broken pipe",
            "mux_client_request_session: master session id: 2",
            "ssh: connect to host x port 22: Connection timed out",
        ] {
            assert!(!mux_failed_before_session(&failure(255, err)), "{err}");
        }
        assert!(!mux_failed_before_session(&failure(
            1,
            "Control socket connect"
        )));
    }

    #[test]
    fn only_read_commands_are_ever_retried() {
        for read in [
            &["status"][..],
            &["agent", "get", "a"],
            &["agent", "list"],
            &["agent", "read", "a", "--lines", "5"],
            &["--session", "s", "agent", "get", "a"],
        ] {
            assert!(is_read_only(&argv(read)), "{read:?}");
        }
        for write in [
            &["pane", "send-text", "p", "hi"][..],
            &["pane", "send-keys", "p", "enter"],
            &["agent", "prompt", "a", "x"],
            &["agent", "wait", "a"],
            &["agent", "start", "x"],
            &["workspace", "create"],
            &["--session", "s", "pane", "run", "p", "ls"],
            &[],
        ] {
            assert!(!is_read_only(&argv(write)), "{write:?}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_write_is_not_rerun_when_mux_fails_but_a_read_is() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("runs.log");
        let ssh = dir.path().join("ssh");
        std::fs::write(
            &ssh,
            format!(
                "#!/bin/sh\ncat >/dev/null\ncase \"$*\" in *ControlMaster*) echo run >> {log}; echo 'Control socket connect(/x): Connection refused' >&2; exit 255;; esac\necho run-plain >> {log}\nprintf ok\n",
                log = log.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&ssh, std::fs::Permissions::from_mode(0o755)).unwrap();
        let ssh_path = ssh.to_string_lossy().to_string();
        let t = Transport::Ssh {
            port: None,
            target: "me@h".into(),
            identity_file: None,
            gate_command: "gate".into(),
            mux_dir: Some(dir.path().to_path_buf()),
        };
        let runs = |t: &Transport, args: &[&str]| {
            let _ = std::fs::remove_file(&log);
            let out = t.run_with_program(&ssh_path, "h", &argv(args), Duration::from_secs(5));
            (
                out.is_ok(),
                std::fs::read_to_string(&log).unwrap_or_default(),
            )
        };
        let (ok, seen) = runs(&t, &["pane", "send-text", "p", "hi"]);
        assert!(!ok);
        assert_eq!(seen, "run\n", "a write must run exactly once");
        let (ok, seen) = runs(&t, &["agent", "read", "a"]);
        assert!(ok);
        assert_eq!(seen, "run\nrun-plain\n");
    }

    #[test]
    fn the_stdin_payload_reaches_the_command_and_its_output_comes_back() {
        let payload = br#"["agent","prompt","x","$(touch /tmp/never); `id`"]"#.to_vec();
        let out = run_with_deadline(
            &mut Command::new("/bin/cat"),
            Some(payload.clone()),
            Duration::from_secs(5),
        )
        .unwrap();
        assert_eq!(out.exit_code, 0);
        assert_eq!(out.stdout.as_bytes(), payload.as_slice());
    }

    #[test]
    fn a_missing_binary_is_an_error_not_a_panic() {
        let err = run_with_deadline(&mut Command::new("/nonexistent/ssh"), None, Duration::from_secs(2)).unwrap_err();
        assert!(!err.is_empty());
    }

    #[test]
    fn a_hung_process_is_killed_at_the_deadline() {
        let started = Instant::now();
        let mut command = Command::new("/bin/sleep");
        command.arg("30");
        let err = run_with_deadline(&mut command, None, Duration::from_millis(300)).unwrap_err();
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(err.contains("timed out"), "{err}");
    }
}
