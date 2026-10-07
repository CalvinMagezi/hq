use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Reserved host name for the machine HQ itself runs on.
pub const LOCAL_HOST: &str = "local";

/// Reserved host name for the built-in agent host (`hq host serve`). Set it as
/// `default_host` to start new sessions there; sessions already running keep
/// the host they were started on.
pub const NATIVE_HOST: &str = "native";

/// Where the built-in host keeps its socket, token and `session.json`:
/// `HQ_HOST_DIR` when set (the same variable panes get, so hooks and HQ agree),
/// else `~/.hq/run/host`. A unix socket path is limited to about 100 bytes, so a
/// long override can fail to bind.
pub fn native_host_dir() -> std::path::PathBuf {
    match std::env::var_os("HQ_HOST_DIR") {
        Some(dir) if !dir.is_empty() => dir.into(),
        _ => super::HqConfig::hq_dir().join("run").join("host"),
    }
}

/// Where coding-agent sessions run and how HQ reaches each machine's Herdr
/// (herdr.dev). With no `hosts` configured every session runs on this machine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HerdrConfig {
    /// Herdr binary on this machine; a bare name is resolved on PATH.
    #[serde(default = "default_binary")]
    pub binary: String,

    /// Named Herdr session on this machine. Unset targets the default session,
    /// which is the one a person sees when they open `herdr`.
    #[serde(default)]
    pub session: Option<String>,

    /// The HQ MCP endpoint a launched agent connects to with its own session
    /// token (for example `https://hq.example.ts.net:8444/mcp`), reachable from
    /// the machine the agent runs on. Unset leaves agents without it.
    #[serde(default)]
    pub agent_mcp_url: Option<String>,

    /// Host that new sessions start on when the caller names none: `local`, a
    /// configured remote, or `native` for the built-in host.
    #[serde(default = "default_host")]
    pub default_host: String,

    /// Remote machines reachable over SSH, keyed by host name.
    #[serde(default)]
    pub hosts: BTreeMap<String, HerdrHostConfig>,

    /// Reuse one ssh connection per remote host (ControlMaster) so each Herdr
    /// call skips the handshake. If the control socket cannot be set up, HQ
    /// falls back to a fresh connection per call. Set false to always do that.
    #[serde(default = "default_ssh_multiplex")]
    pub ssh_multiplex: bool,

    /// Seconds a launch waits for the agent to come up before the workspace is
    /// closed and the call fails. Clamped to 5..=50 so the answer arrives before
    /// an MCP client's roughly 60 second transport limit.
    #[serde(default = "default_launch_bound_secs")]
    pub launch_bound_secs: u64,

    /// Ceiling for any single Herdr call that is not an explicit wait.
    #[serde(default = "default_command_timeout_secs")]
    pub command_timeout_secs: u64,

    /// Custom launchers layered on a built-in harness, keyed by the name callers
    /// pass as `harness` (for example a wrapper script that selects an account).
    #[serde(default)]
    pub harness_profiles: BTreeMap<String, HarnessProfileConfig>,

    /// How often a web chat driving a session checks on it when nothing
    /// happened, to catch an agent that says it is working but is stuck.
    #[serde(default = "default_driver_checkin_minutes")]
    pub driver_checkin_minutes: u64,

    /// Whether a session a web chat starts watching starts with Drive on.
    /// Off keeps every new watch at updates only until the user flips the switch.
    #[serde(default = "default_drive_new_watches")]
    pub drive_new_watches: bool,

    /// Instructions (prompts, keys) the driver may send one session before Drive
    /// switches itself off, counted from the last time the user turned Drive on.
    /// Clamped to 1..=100 by `nudge_budget`.
    #[serde(default = "default_driver_nudge_budget")]
    pub driver_nudge_budget: u32,

    /// Key presses (permission dialogs, menus) the driver may send one session, counted like
    /// the instruction budget. Separate and larger, so a healthy task with many dialogs
    /// is not cut off. Clamped to 1..=500 by `key_allowance`.
    #[serde(default = "default_driver_key_allowance")]
    pub driver_key_allowance: u32,

    /// Running sessions started by an MCP client with no chat (spawn or handoff) at once.
    /// Clamped to 1..=50 by `mcp_started_session_cap`.
    #[serde(default = "default_max_mcp_started_sessions")]
    pub max_mcp_started_sessions: u32,

    /// Full-mode `hq_ask` questions that may wait for an answer at once. Clamped to 1..=10.
    #[serde(default = "default_max_full_asks")]
    pub max_full_asks: u32,

    /// Finished turns in a row with no new tool activity after which Drive
    /// switches itself off. Clamped to 2..=20 by `no_progress_limit`.
    #[serde(default = "default_driver_no_progress_limit")]
    pub driver_no_progress_limit: u32,

    /// Running sessions HQ may drive at once. A watch that would exceed it
    /// starts observation-only. Clamped to 1..=50 by `driven_session_cap`.
    #[serde(default = "default_max_driven_sessions")]
    pub max_driven_sessions: u32,

    /// Running sessions that chats started by `hq_ask` may own at once, which
    /// bounds an agent asking HQ to start agents. 0 refuses them. Clamped to 0..=20.
    #[serde(default = "default_max_ask_spawned_sessions")]
    pub max_ask_spawned_sessions: u32,

    /// Directories no session may start in, matched as case-insensitive
    /// substrings of the working directory after `.` and `..` are resolved
    /// (`/clients/acme` also blocks `/home/me/clients/acme/app`). Applies to
    /// `harness_session_spawn`, `harness_session_handoff` and resume. Empty by
    /// default. The match is on the path as written, so a symlink into a denied
    /// directory is not caught.
    #[serde(default)]
    pub spawn_cwd_deny: Vec<String>,

    /// When non-empty, a session started through the handoff-scoped MCP key
    /// (`AGENTHQ_HANDOFF_API_KEY`) must have a working directory in or under
    /// one of these absolute paths. Empty leaves that key unrestricted, which
    /// makes it equivalent to shell access on every configured host.
    #[serde(default)]
    pub handoff_cwd_allow: Vec<String>,
}

/// A launcher built on a built-in harness. It inherits that harness's resume,
/// trust and token behaviour and changes only how the CLI is started.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HarnessProfileConfig {
    /// Built-in harness this profile behaves like (`claude-code`, `cursor`, ...).
    pub base: String,

    /// Executable typed into the pane's shell instead of the base CLI, such as a
    /// wrapper script. It has to end up running the base CLI so Herdr can
    /// recognize the agent. Unset launches the base CLI through Herdr directly.
    #[serde(default)]
    pub command: Option<String>,

    /// Replaces the base harness's arguments on a fresh spawn. A resume uses the
    /// base harness's resume arguments.
    #[serde(default)]
    pub args: Option<Vec<String>>,

    /// Environment for the pane's shell, and so for the launched CLI.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

/// What runs agents on a remote machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HostKind {
    /// Herdr, driven through its CLI.
    #[default]
    Herdr,
    /// HQ's built-in host (`hq host serve`), driven through `hq host gate`.
    Native,
}

/// One remote machine running Herdr or HQ's built-in host, reached with `ssh`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HerdrHostConfig {
    /// Which kind of host runs there. Defaults to herdr.
    #[serde(default)]
    pub kind: HostKind,

    /// `user@address` for ssh, usually a Tailscale address or MagicDNS name.
    pub ssh: String,

    /// ssh port when the server does not listen on 22.
    #[serde(default)]
    pub port: Option<u16>,

    /// Private key HQ presents. Unset lets ssh choose its defaults.
    #[serde(default)]
    pub identity_file: Option<String>,

    /// Command run after login. For herdr it receives the herdr arguments as a
    /// JSON array on stdin (see `scripts/hq-herdr-gate`); for a native host it
    /// is `hq host gate` and gets a method and its params. A forced-command
    /// `authorized_keys` entry ignores this value.
    #[serde(default = "default_gate_command")]
    pub gate_command: String,

    /// Named Herdr session on that machine. Unset targets its default session.
    #[serde(default)]
    pub session: Option<String>,
}

fn default_binary() -> String {
    "herdr".to_string()
}

fn default_host() -> String {
    LOCAL_HOST.to_string()
}

fn default_ssh_multiplex() -> bool {
    true
}

fn default_gate_command() -> String {
    "hq-herdr-gate".to_string()
}

/// Bounds for `HerdrConfig::launch_bound_secs`.
pub const MIN_LAUNCH_BOUND_SECS: u64 = 5;
pub const MAX_LAUNCH_BOUND_SECS: u64 = 50;

fn default_launch_bound_secs() -> u64 {
    25
}

fn default_command_timeout_secs() -> u64 {
    30
}

fn default_driver_checkin_minutes() -> u64 {
    30
}

fn default_drive_new_watches() -> bool {
    true
}

pub const DEFAULT_DRIVER_NUDGE_BUDGET: u32 = 8;
pub const DEFAULT_DRIVER_NO_PROGRESS_LIMIT: u32 = 3;
pub const DEFAULT_MAX_DRIVEN_SESSIONS: u32 = 3;
pub const DEFAULT_MAX_ASK_SPAWNED_SESSIONS: u32 = 2;
pub const DEFAULT_DRIVER_KEY_ALLOWANCE: u32 = 40;
pub const DEFAULT_MAX_MCP_STARTED_SESSIONS: u32 = 3;
pub const DEFAULT_MAX_FULL_ASKS: u32 = 2;

fn default_driver_key_allowance() -> u32 {
    DEFAULT_DRIVER_KEY_ALLOWANCE
}

fn default_max_mcp_started_sessions() -> u32 {
    DEFAULT_MAX_MCP_STARTED_SESSIONS
}

fn default_max_full_asks() -> u32 {
    DEFAULT_MAX_FULL_ASKS
}

fn default_driver_nudge_budget() -> u32 {
    DEFAULT_DRIVER_NUDGE_BUDGET
}

fn default_driver_no_progress_limit() -> u32 {
    DEFAULT_DRIVER_NO_PROGRESS_LIMIT
}

fn default_max_driven_sessions() -> u32 {
    DEFAULT_MAX_DRIVEN_SESSIONS
}

fn default_max_ask_spawned_sessions() -> u32 {
    DEFAULT_MAX_ASK_SPAWNED_SESSIONS
}

impl HerdrConfig {
    pub fn nudge_budget(&self) -> i64 {
        i64::from(self.driver_nudge_budget.clamp(1, 100))
    }

    pub fn key_allowance(&self) -> i64 {
        i64::from(self.driver_key_allowance.clamp(1, 500))
    }

    pub fn mcp_started_session_cap(&self) -> i64 {
        i64::from(self.max_mcp_started_sessions.clamp(1, 50))
    }

    pub fn full_ask_cap(&self) -> i64 {
        i64::from(self.max_full_asks.clamp(1, 10))
    }

    pub fn no_progress_limit(&self) -> i64 {
        i64::from(self.driver_no_progress_limit.clamp(2, 20))
    }

    pub fn driven_session_cap(&self) -> i64 {
        i64::from(self.max_driven_sessions.clamp(1, 50))
    }

    pub fn ask_spawned_session_cap(&self) -> i64 {
        i64::from(self.max_ask_spawned_sessions.min(20))
    }
}

impl Default for HerdrConfig {
    fn default() -> Self {
        Self {
            binary: default_binary(),
            session: None,
            agent_mcp_url: None,
            default_host: default_host(),
            hosts: BTreeMap::new(),
            ssh_multiplex: default_ssh_multiplex(),
            launch_bound_secs: default_launch_bound_secs(),
            command_timeout_secs: default_command_timeout_secs(),
            harness_profiles: BTreeMap::new(),
            driver_checkin_minutes: default_driver_checkin_minutes(),
            drive_new_watches: default_drive_new_watches(),
            driver_nudge_budget: default_driver_nudge_budget(),
            driver_no_progress_limit: default_driver_no_progress_limit(),
            driver_key_allowance: default_driver_key_allowance(),
            max_mcp_started_sessions: default_max_mcp_started_sessions(),
            max_full_asks: default_max_full_asks(),
            max_driven_sessions: default_max_driven_sessions(),
            max_ask_spawned_sessions: default_max_ask_spawned_sessions(),
            spawn_cwd_deny: Vec::new(),
            handoff_cwd_allow: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_config_runs_everything_locally() {
        let cfg: HerdrConfig = serde_yaml::from_str("{}").unwrap();
        assert_eq!(cfg.default_host, LOCAL_HOST);
        assert!(cfg.hosts.is_empty());
        assert_eq!(cfg.binary, "herdr");
        assert!(
            cfg.drive_new_watches,
            "new watches drive unless the config opts out"
        );
        let off: HerdrConfig = serde_yaml::from_str("drive_new_watches: false").unwrap();
        assert!(!off.drive_new_watches);
        assert!(
            cfg.spawn_cwd_deny.is_empty(),
            "nothing is denied unless configured"
        );
        let deny: HerdrConfig = serde_yaml::from_str("spawn_cwd_deny: [/secret]").unwrap();
        assert_eq!(deny.spawn_cwd_deny, ["/secret"]);
    }

    #[test]
    fn ssh_multiplex_defaults_on_and_can_be_disabled() {
        let cfg: HerdrConfig = serde_yaml::from_str("{}").unwrap();
        assert!(cfg.ssh_multiplex);
        let off: HerdrConfig = serde_yaml::from_str("ssh_multiplex: false").unwrap();
        assert!(!off.ssh_multiplex);
    }

    #[test]
    fn drive_limits_have_defaults_and_clamp() {
        let cfg: HerdrConfig = serde_yaml::from_str("{}").unwrap();
        assert_eq!(
            (
                cfg.nudge_budget(),
                cfg.no_progress_limit(),
                cfg.driven_session_cap(),
                cfg.ask_spawned_session_cap()
            ),
            (8, 3, 3, 2)
        );
        let wild: HerdrConfig = serde_yaml::from_str(
            "driver_nudge_budget: 0\ndriver_no_progress_limit: 1\nmax_driven_sessions: 0\nmax_ask_spawned_sessions: 999",
        )
        .unwrap();
        assert_eq!(
            (
                wild.nudge_budget(),
                wild.no_progress_limit(),
                wild.driven_session_cap(),
                wild.ask_spawned_session_cap()
            ),
            (1, 2, 1, 20)
        );
        assert_eq!((cfg.key_allowance(), cfg.mcp_started_session_cap(), cfg.full_ask_cap()), (40, 3, 2));
        let wild2: HerdrConfig =
            serde_yaml::from_str("driver_key_allowance: 0\nmax_mcp_started_sessions: 0\nmax_full_asks: 99").unwrap();
        assert_eq!((wild2.key_allowance(), wild2.mcp_started_session_cap(), wild2.full_ask_cap()), (1, 1, 10));
        let huge: HerdrConfig = serde_yaml::from_str("driver_nudge_budget: 5000").unwrap();
        assert_eq!(huge.nudge_budget(), 100);
    }

    #[test]
    fn remote_host_takes_gate_default() {
        let cfg: HerdrConfig =
            serde_yaml::from_str("hosts:\n  laptop:\n    ssh: me@100.64.0.1\n").unwrap();
        let laptop = &cfg.hosts["laptop"];
        assert_eq!(laptop.ssh, "me@100.64.0.1");
        assert_eq!(laptop.gate_command, "hq-herdr-gate");
        assert!(laptop.identity_file.is_none());
    }

    #[test]
    fn a_profile_needs_only_its_base() {
        let cfg: HerdrConfig = serde_yaml::from_str(
            "harness_profiles:\n  wrapped:\n    base: claude-code\n    command: my-wrapper\n    env:\n      A: b\n",
        )
        .unwrap();
        let profile = &cfg.harness_profiles["wrapped"];
        assert_eq!(profile.base, "claude-code");
        assert_eq!(profile.command.as_deref(), Some("my-wrapper"));
        assert!(profile.args.is_none());
        assert_eq!(profile.env["A"], "b");
    }
}
