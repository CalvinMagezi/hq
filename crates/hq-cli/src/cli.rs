//! The command line: arguments and subcommands, as `clap` reads them.

use clap::{Parser, Subcommand};
use std::path::PathBuf;

use crate::commands;
use crate::commands::adoption::OffersAction;
use crate::commands::backup::{BackupCommand, RestoreArgs};
use crate::commands::connector::ConnectorAction;
use crate::commands::edge::EdgeAction;
use crate::commands::net::NetAction;
use crate::commands::queue::QueueAction;
use crate::commands::skills::SkillsAction;

const EDGE_HELP: &str = "Run a blind public forwarder so a device that cannot be reached from the internet (a phone) can answer HTTPS on a name you own.

The edge listens on one TCP port (443). It reads only the server name in each TLS ClientHello, never decrypts, and passes the bytes to the device registered under that name over an iroh stream. The device ends TLS. A name that belongs to no registered device is closed with no answer.

Run it on a public machine you control, off by default. Set in config.yaml:

  edge:
    enabled: true
    domain: edge.example.com      # point *.edge.example.com at this machine
    relay:                        # optional: host an iroh relay for the household
      enabled: false
      hostname: relay.edge.example.com
      acme_email: you@example.com

Then: `hq edge id` (give this id to each device), `hq edge allow <device id>`, `hq edge run`. Full guide: docs/EDGE.md. Threats and limits: docs/security/EDGE.md.";

#[derive(Parser)]
#[command(name = "hq", version = commands::update::LONG_VERSION, about = "Agent-HQ: Local-first AI agent hub")]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub(crate) command: Option<Commands>,

    /// Config file path override
    #[arg(long, global = true)]
    pub(crate) config: Option<String>,

    /// Vault path override
    #[arg(long, global = true)]
    pub(crate) vault: Option<String>,
}

/// `hq env` subcommands.
#[derive(Subcommand)]
pub(crate) enum EnvAction {
    /// Save one provider key without the interactive walkthrough. The key is read from one line
    /// of standard input (hidden when typed at a terminal) or from `--from-env`, then the
    /// default model is tested.
    Set {
        /// openrouter, anthropic, google, cerebras, groq, deepseek or openai
        provider: String,
        /// Read the key from this environment variable instead of standard input
        #[arg(long, value_name = "VAR")]
        from_env: Option<String>,
    },
}

#[derive(Subcommand)]
pub(crate) enum Commands {
    // ─── Getting Started ─────────────────────────────────────────────
    /// Full install: scaffold vault, seed soul content, detect tools, write config
    #[command(alias = "setup")]
    Install {
        /// Vault path override
        #[arg(long)]
        vault_path: Option<String>,
        /// Non-interactive mode (auto-detect everything, no prompts)
        #[arg(long)]
        non_interactive: bool,
        /// Upgrade system files and guides to latest version (preserves MEMORY.md, PREFERENCES.md)
        #[arg(long)]
        upgrade: bool,
        /// Minimal install: scaffold + config only, skip guides
        #[arg(long)]
        minimal: bool,
    },

    /// Interactive onboarding walkthrough (API keys, tools, integrations)
    Onboard {
        /// Jump to a specific step (1-6)
        #[arg(long)]
        step: Option<u8>,
        /// Reset onboarding progress
        #[arg(long)]
        reset: bool,
        /// Read an existing coding agent's state (claude) and propose spaces, initiatives,
        /// preferences and project briefs from it
        #[arg(long, value_name = "AGENT")]
        from: Option<String>,
        /// With --from: print the proposal and write nothing
        #[arg(long)]
        dry_run: bool,
    },

    /// Manage long-lived agent sessions: coding agents (claude-code, cursor, opencode, antigravity, pi...)
    /// and general-purpose agents (hermes, openclaw), which never take code work
    Sessions {
        /// Subcommand: list (default), spawn <harness>, status <id>, logs <id>, send <id>, stop <id>, resume <id>, stats [<root id>] [--task <id>], watch (one JSON line per event) or channel (a Claude Code channel server)
        #[arg(default_value = "list")]
        sub: String,
        /// Harness name (spawn), session id (status/logs/send/stop/resume) or team root (stats)
        arg: Option<String>,
        /// Prompt text (spawn/send/resume)
        #[arg(long)]
        prompt: Option<String>,
        /// Working directory (spawn)
        #[arg(long)]
        cwd: Option<String>,
        /// Short label (spawn)
        #[arg(long)]
        label: Option<String>,
        /// host to run on, e.g. a laptop from `agent_host.hosts` (spawn; default: this machine)
        #[arg(long)]
        host: Option<String>,
        /// What the session is for (spawn): code, monitor, research, review or general.
        /// Default: code for a coding agent, general for a general-purpose agent, which refuses code.
        #[arg(long)]
        purpose: Option<String>,
        /// Session id a monitor or reviewer watches (spawn; repeatable; needs --task)
        #[arg(long)]
        observe: Vec<String>,
        /// HQ task (id or display id) the session works on (spawn)
        #[arg(long)]
        task: Option<String>,
        /// Model to start the agent on, passed with its CLI's --model flag (spawn)
        #[arg(long)]
        model: Option<String>,
        /// Work on this branch in a git worktree of its own, so it never shares a working
        /// tree with another agent (spawn)
        #[arg(long)]
        branch: Option<String>,
        /// Start a new --branch at this ref instead of the folder's current commit (spawn)
        #[arg(long)]
        from: Option<String>,
        /// A path, relative to --cwd, the session writes to; repeat for more (spawn)
        #[arg(long)]
        owns: Vec<String>,
        /// Command policy to run under: builder, reviewer, one from agent_host.command_policies,
        /// or none (spawn; default: builder for code, reviewer for review)
        #[arg(long)]
        policy: Option<String>,
        /// Log lines to tail (logs)
        #[arg(long, default_value_t = 40)]
        lines: usize,
        #[command(flatten)]
        watch: Box<commands::sessions_watch::WatchArgs>,
    },

    /// The Agentic Stack: general-purpose agents that run the tasks assigned to them (config `stack:`)
    Stack {
        /// list (default) or run <specialist> <task>
        #[arg(default_value = "list")]
        sub: String,
        /// Specialist name (run)
        name: Option<String>,
        /// Task id or display id (run)
        task: Option<String>,
        /// Print JSON (list)
        #[arg(long)]
        json: bool,
    },

    /// How far HQ's verification of finished tasks can be trusted, the audits you answer, and sweeps that clear a queue
    Verify {
        /// status (default), rollout, audits, pull [n], agree <id>, disagree <id>, holds, veto <task>, briefing [hours], queue or sweep <verb>
        #[arg(default_value = "status")]
        sub: String,
        /// A count (pull), an audit id (agree, disagree), a task (veto) or a sweep verb
        /// (plan, list, show, run, pause, resume, cancel, apply, undo)
        arg: Option<String>,
        /// A sweep id (sweep show, run, pause, resume, cancel, apply, undo)
        extra: Option<String>,
        /// Why you disagree or veto (disagree, veto)
        #[arg(long)]
        note: Option<String>,
        /// Sweep over one initiative (sweep plan)
        #[arg(long)]
        initiative: Option<String>,
        /// Sweep over tasks that entered review on or after this date, YYYY-MM-DD (sweep plan)
        #[arg(long)]
        since: Option<String>,
        /// Confirm an apply that moves more than the configured threshold (sweep apply)
        #[arg(long)]
        confirm: bool,
        /// Print JSON (status, audits, holds, sweep list, sweep show)
        #[arg(long)]
        json: bool,
    },

    /// Run a WireGuard private plane for HQ: init, peer add|revoke|list, link add|remove|list, status, apply (`hq net --help`)
    Net {
        #[command(subcommand)]
        action: NetAction,
        /// Where hq keeps the plane's settings and peers (default: ~/.hq/net)
        #[arg(long, global = true, hide = true)]
        dir: Option<PathBuf>,
        /// The directory the root applier reads its input from (default: /var/lib/hq-net)
        #[arg(long, global = true, hide = true)]
        spool: Option<PathBuf>,
    },

    /// Run a blind public forwarder for your devices: run, id, allow, revoke, list (`hq edge --help`)
    #[command(long_about = EDGE_HELP)]
    Edge {
        #[command(subcommand)]
        action: EdgeAction,
    },

    /// Pair machines with a QR code and reach them by key: pair, join, serve, ping, peers, revoke (`hq bridge --help`)
    #[cfg(all(unix, feature = "bridge"))]
    #[command(subcommand)]
    Bridge(commands::bridge::BridgeAction),

    /// Add HQ to Claude Code and Claude Desktop through the paired hub: add, status, remove (`hq claude --help`)
    #[cfg(all(unix, feature = "bridge"))]
    #[command(subcommand)]
    Claude(commands::claude::ClaudeAction),

    /// Run or inspect the built-in agent host (long-lived coding agents in pseudo-terminals)
    Host {
        /// serve (run the host in this terminal), status, stop, lockdown [reason] (stop every agent and refuse new ones), unlock (lift a lockdown; only on this machine), install (run it as a login service), sign <path> (macOS: give an hq binary the stable local signature when this machine has one), join (print this machine's join code), add <code> (pair a machine, on the HQ side), check <host>, authorize (pin a remote key to the gate; with --gate-key, a server's key for the TCP gate), gate-key <host> (make this server's key for a host's TCP gate), report (used by agent hooks), or gate (the ssh forced command for remote access)
        #[arg(default_value = "status")]
        sub: String,
        /// add: the join code; check: the host name; join: the name for this machine; lockdown: the reason; sign: the binary to sign
        arg: Option<String>,
        /// Directory holding host.sock and operator.token (default: ~/.hq/run/host)
        #[arg(long)]
        dir: Option<std::path::PathBuf>,
        /// serve only: also start agents that are not under the process sandbox. Without it the host refuses them, whoever asks.
        #[arg(long)]
        allow_unsandboxed: bool,
        /// serve only: most agents running or waiting at once (default 64)
        #[arg(long)]
        max_agents: Option<usize>,
        /// serve only: most agents started within 10 minutes before the host locks itself (default 50)
        #[arg(long)]
        max_starts: Option<usize>,
        /// authorize only: the ssh public key to pin to `hq host gate`
        #[arg(long)]
        key: Option<String>,
        /// authorize only: a server's gate public key (from `hq host gate-key`) for the TCP gate
        #[arg(long)]
        gate_key: Option<String>,
        /// authorize only: the address the key may connect from (ssh `from=`, or the TCP gate's `from=`)
        #[arg(long)]
        from: Option<String>,
        /// relay and sandbox-init (run inside a Linux sandbox): the loopback port to serve
        #[arg(long)]
        port: Option<u16>,
        /// relay and sandbox-init: the unix socket the relay forwards to
        #[arg(long)]
        unix: Option<std::path::PathBuf>,
        /// sandbox-init: the agent command to run after `--`
        #[arg(last = true)]
        rest: Vec<String>,
        /// join: this machine's tailnet address when tailscale cannot be asked
        #[arg(long)]
        addr: Option<String>,
    },

    /// Internal: detached applier spawned by self_update_install
    #[command(name = "self-apply", hide = true)]
    SelfApply {
        /// Self-update run id to apply
        #[arg(long)]
        run_id: i64,
    },

    /// Internal: pre/post-restart broadcast, called by the deploy tooling
    #[command(name = "notify-restart", hide = true)]
    NotifyRestart {
        /// pre | post-ok | post-rolled-back
        phase: String,
        #[arg(long, default_value = "deploy")]
        reason: String,
        #[arg(long)]
        sha: Option<String>,
    },

    /// Check for, apply or roll back a signed release (pull-based updater)
    Update(commands::update::UpdateArgs),

    /// Internal: vault DB snapshot/restore/count, run as the service user by `hq update`
    #[command(name = "update-db", hide = true)]
    UpdateDb {
        /// snapshot <src> <dest> | restore <db> <snapshot> | count <db>
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },

    /// System health check (alias for doctor)
    Health,

    /// Print a one-time code that makes the first chat to present it the relay owner
    Pair {
        /// Relay to pair: telegram or discord
        #[arg(long, default_value = "telegram")]
        platform: String,
    },

    /// Diagnose common issues
    Doctor {
        /// List every outbound destination this config can reach, and (under `profile: lite`)
        /// exit non-zero if any is not allowed
        #[arg(long)]
        egress: bool,
        /// Read-only checks for a managed Windows computer (notes in a synced folder, tools, port)
        #[arg(long)]
        windows: bool,
    },

    /// Start the web app when you sign in (Windows, per user): on, off, status
    Autostart {
        #[arg(default_value = "status")]
        sub: String,
    },

    /// Move notes between HQ Lite and Full HQ: export <folder> | import <folder>
    Lite {
        sub: String,
        folder: Option<std::path::PathBuf>,
    },

    /// Encrypted backup and restore of the vault and its database: create, verify, restore, keygen, prune (`hq backup --help`)
    Backup {
        #[command(subcommand)]
        command: BackupCommand,
    },

    /// Restore an encrypted backup into a new or empty folder (the same as `hq backup restore`)
    Restore(RestoreArgs),

    /// Personal finance from mobile-money messages, on this machine only (`hq finance --help`)
    Finance {
        #[command(subcommand)]
        command: commands::finance::FinanceCommand,
    },

    /// The HQ Suite (documents, storage): measure, shrink, undo (`hq suite --help`)
    Suite {
        #[command(subcommand)]
        command: commands::suite::SuiteCommand,
    },

    /// Set up API keys interactively, or set one with `hq env set <provider>`
    Env {
        #[command(subcommand)]
        action: Option<EnvAction>,
    },

    /// Tasks from a terminal (the same scope as the tasks-only MCP key), and task time reports
    #[command(alias = "tasks")]
    Task {
        #[command(subcommand)]
        cmd: commands::task::TaskCmd,
    },

    /// Link the GitHub Copilot seat this machine is signed in to
    Copilot {
        /// Subcommand: link, or init (write HQ's terminal instructions for coding agents)
        #[arg(default_value = "link")]
        sub: String,
        /// Use this model instead of the preference list
        #[arg(long)]
        model: Option<String>,
        /// Write the backend into the config file instead of printing it
        #[arg(long)]
        write: bool,
        /// With `init`: write AGENTS.md instead of .github/copilot-instructions.md
        #[arg(long)]
        agents: bool,
    },

    // ─── Chat & Agents ───────────────────────────────────────────────
    /// Interactive terminal chat with an LLM (default command)
    Chat {
        /// Model to use (e.g., anthropic/claude-sonnet-4)
        #[arg(short, long)]
        model: Option<String>,
        /// Launch fullscreen TUI mode
        #[arg(long)]
        tui: bool,
        /// Named permission preset bundling security profile + permission
        /// mode: read-only, workspace-write, or danger-full-access.
        #[arg(long)]
        permission_preset: Option<String>,
        /// Use a running daemon's backend instead of building one in-process.
        /// Bare `--server` means the configured loopback daemon; a URL targets
        /// another one (non-loopback needs https and HQ_WEB_AUTH_TOKEN).
        /// Without this flag chat runs in-process; HQ_CHAT_SERVER=auto opts
        /// into using a loopback daemon when one answers.
        #[arg(long, value_name = "URL", num_args = 0..=1, default_missing_value = "")]
        server: Option<String>,
        /// Always run in-process (the default), even with HQ_CHAT_SERVER=auto.
        #[arg(long, conflicts_with = "server")]
        local: bool,
        /// Run one turn with this prompt, print the reply, and exit.
        #[arg(short, long)]
        prompt: Option<String>,
    },

    // ─── Services ────────────────────────────────────────────────────
    /// Show vault status and system info
    #[command(alias = "s")]
    Status,

    /// Start HQ components
    Start {
        /// Component: all, daemon, relay, telegram
        #[arg(default_value = "all")]
        component: String,
    },

    /// Stop HQ components
    Stop {
        /// Component: all, daemon, relay, telegram
        #[arg(default_value = "all")]
        component: String,
    },

    /// Restart HQ components (stop + start)
    #[command(alias = "r")]
    Restart {
        /// Component: all, agent, daemon, relay
        #[arg(default_value = "all")]
        component: String,
    },

    // ─── Monitoring ──────────────────────────────────────────────────
    /// View recent log lines (journald under systemd, log files otherwise)
    #[command(alias = "l")]
    Logs {
        /// Service target: relay, daemon, all
        #[arg(default_value = "daemon")]
        target: String,
        /// Number of lines
        #[arg(short = 'n', long, short_alias = 'l', default_value = "30")]
        lines: usize,
        /// Keep streaming new lines
        #[arg(short, long)]
        follow: bool,
        /// Only error lines
        #[arg(long)]
        errors: bool,
    },

    /// Deprecated: `hq logs --errors`
    #[command(alias = "e", hide = true)]
    Errors {
        #[arg(default_value = "daemon")]
        target: String,
        #[arg(short, long, default_value = "20")]
        lines: usize,
    },

    /// Deprecated: `hq logs -f`
    #[command(alias = "f", hide = true)]
    Follow {
        #[arg(default_value = "daemon")]
        target: String,
    },

    /// Show all managed processes
    #[command(alias = "p")]
    Ps,

    // ─── Vault Operations ────────────────────────────────────────────
    /// Vault operations (list, read, write, export, export-pdf, inventory, import, stats, context)
    Vault {
        /// Subcommand: list, tree, read, write, export, export-pdf, inventory, import, stats, context
        #[arg(default_value = "stats")]
        sub: String,
        /// Additional arguments (options such as `-o` pass through to the subcommand)
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },

    /// Search vault notes
    Search {
        /// Search query
        query: Vec<String>,
        /// Max results
        #[arg(short, long, default_value = "20")]
        limit: usize,
        /// Print one JSON object instead of text
        #[arg(long)]
        json: bool,
    },

    /// Force a full rebuild of the vault's FTS search index
    Reindex,

    /// Show/manage memory and system context
    Memory {
        /// Subcommand: show, facts, add, soul, preferences, context
        #[arg(default_value = "show")]
        sub: String,
        /// Additional arguments
        args: Vec<String>,
    },

    /// Generate quality profiles (anti-slop)
    Profile(commands::profile::ProfileArgs),

    // ─── Agents ──────────────────────────────────────────────────────
    /// Inspect and maintain the inter-agent mailboxes
    Mailbox {
        /// Subcommand: list (default), archive
        #[arg(default_value = "list")]
        sub: String,
        /// Additional arguments: --older-than-days N, --dry-run
        #[arg(allow_hyphen_values = true)]
        args: Vec<String>,
    },

    /// List/inspect agent definitions
    Agents {
        /// Subcommand: list, show
        #[arg(default_value = "list")]
        sub: String,
        /// Additional arguments
        args: Vec<String>,
    },

    // ─── Configuration ───────────────────────────────────────────────
    /// Show/edit configuration
    Config {
        /// Config key to show or set
        key: Option<String>,
        /// Value to set
        value: Option<String>,
    },

    /// Show, install or remove the MCP config for Claude, Cursor, VS Code, Copilot, OpenCode, Antigravity
    Mcp {
        /// Subcommand: status, install, remove, doctor. Only install and remove change files
        #[arg(default_value = "status")]
        sub: String,
        /// Project directory for the project-level configs (default: current directory)
        path: Option<String>,
        /// Only this client: claude-desktop, claude-code, vscode, cursor, antigravity, copilot, opencode, project
        #[arg(long)]
        target: Option<String>,
        /// Only the per-user configs, skip the project files
        #[arg(long)]
        global: bool,
        /// What the installed stdio server may reach: full or tasks. Needs --target. Unset keeps
        /// the scope an existing entry has (full for a new one)
        #[arg(long, value_enum)]
        scope: Option<commands::cursor_mcp_config::ServeScope>,
        /// Point VS Code at a remote HQ's /mcp URL instead (the key is asked for, never written)
        #[arg(long)]
        url: Option<String>,
    },

    /// Deprecated: `hq mcp install --target project [path]`
    #[command(hide = true)]
    Link { path: Option<String> },

    /// Deprecated: `hq mcp install|status|remove --target project [path]`
    #[command(hide = true)]
    Cursor {
        #[command(subcommand)]
        action: CursorAction,
    },

    /// Start the MCP stdio server (used by Claude Desktop / editors)
    #[command(name = "mcp-serve", hide = true)]
    McpServe {
        /// Limit the server to a scope's tools: full (default) or tasks
        #[arg(long, value_enum, default_value = "full")]
        scope: commands::cursor_mcp_config::ServeScope,
        /// Relay to the `remote_mcp` entry with this name instead of serving this machine's
        /// vault (for a client that starts only stdio servers, such as Claude Desktop)
        #[arg(long, conflicts_with = "scope")]
        remote: Option<String>,
    },

    // ─── Background Daemon ───────────────────────────────────────────
    /// Daemon management (stop/status/logs)
    #[command(alias = "d")]
    Daemon {
        /// Subcommand: stop, status, logs
        #[arg(default_value = "status")]
        sub: String,
        /// Optional argument (e.g., number of log lines)
        arg: Option<String>,
    },

    // ─── Advanced ────────────────────────────────────────────────────
    /// Force-kill all processes
    #[command(alias = "k")]
    Kill,

    /// Remove stale locks and orphans
    #[command(alias = "c")]
    Clean,

    /// Manage service daemons (launchd/systemd)
    Service {
        /// Subcommand: install, uninstall, status
        #[arg(default_value = "status")]
        sub: String,
        /// Target: all, agent, relay, daemon, or lite (HQ Lite: stop the server, remove autostart)
        #[arg(default_value = "all")]
        target: String,
    },

    /// Remove service daemons (same as `hq service uninstall`)
    Uninstall {
        /// Target: all, agent, relay, daemon
        #[arg(default_value = "all")]
        target: String,
    },

    // ─── Tools & Diagrams ────────────────────────────────────────────
    /// Check/install CLI tools
    #[command(alias = "t")]
    Tools,

    /// Manage local models (status, setup, recommend)
    Models {
        /// Subcommand: status, setup, recommend
        #[arg(default_value = "status")]
        sub: String,
    },

    // ─── Lane quotas ─────────────────────────────────────────────────
    /// Show what is left of each coding-agent lane's allowance, per host
    Quota {
        /// Print the readings as JSON
        #[arg(long)]
        json: bool,
        /// Ask the hosts now instead of using a recent stored reading
        #[arg(long)]
        refresh: bool,
        /// Only this lane (claude, codex, copilot, agy, cursor, or a profile name)
        #[arg(long)]
        lane: Option<String>,
        /// Only this host (see `hq host list`)
        #[arg(long)]
        host: Option<String>,
    },

    // ─── Sandbox grants ──────────────────────────────────────────────
    /// List or revoke the sandbox access helpers asked HQ for
    Grants {
        /// Subcommand: list, revoke, profile
        #[arg(default_value = "list")]
        sub: String,
        /// For revoke: a grant id, or a session id to revoke all of its grants. For profile: the
        /// session id (or, with --task, the task id) to bind
        target: Option<String>,
        /// For profile: the access profile's name from agent_host.grants.profiles, or `none`
        name: Option<String>,
        /// For profile: `target` is a task id, so every session on it gets the profile
        #[arg(long)]
        task: bool,
        /// Print the grants as JSON
        #[arg(long)]
        json: bool,
    },

    // ─── Usage ───────────────────────────────────────────────────────
    /// Show LLM cost and tokens from recorded calls
    Usage {
        /// Subcommand: summary, daily
        #[arg(default_value = "summary")]
        sub: String,
    },

    // ─── Web Dashboard ───────────────────────────────────────────────
    /// Host the web UI: start it, open the browser (`hq web --help` for more)
    #[command(alias = "pwa", alias = "dashboard")]
    Web(commands::web::WebArgs),

    // ─── Developer Workflow ───────────────────────────────────────
    /// Show LLM cost and tokens by model (same as `hq usage summary`)
    Cost,

    /// Show LLM cost and tokens by day and model (same as `hq usage daily`)
    Summary,

    /// Shortcut tool management and smoke tests
    Shortcuts {
        /// Subcommand: list, test
        #[arg(default_value = "list")]
        sub: String,
    },

    /// Manage vault skills
    #[command(alias = "skill", subcommand)]
    Skills(SkillsAction),

    /// List/clear the value-bus notification queue
    #[command(alias = "q", subcommand)]
    Queue(QueueAction),

    /// The claude.ai connector: status, allow a new connection, revoke clients
    #[command(subcommand)]
    Connector(ConnectorAction),

    /// Connect your own GitHub App so HQ Coder can work on your repositories
    #[command(subcommand)]
    Github(commands::github::GithubAction),

    /// Show what the structured-decision gates did (counts, scores, cost, what was hidden)
    Decisions {
        /// How many days back to summarize
        #[arg(long, default_value = "7")]
        days: u32,
        /// Only this gate, e.g. memory_turn
        #[arg(long)]
        site: Option<String>,
    },

    /// Questions with a recommended default and a deadline: create, list, answer
    #[command(subcommand)]
    Decide(commands::decide::DecideAction),

    /// The few lanes of work you chose to move forward: list, add, park, remove
    #[command(subcommand)]
    Focus(commands::focus::FocusAction),

    /// Tidy the inbox: `hq inbox tidy` counts (and with --apply moves) stale helper requests,
    /// `hq inbox restore` brings them back
    #[command(subcommand)]
    Inbox(commands::inbox::InboxAction),

    /// Domains HQ may fetch without asking you (never skips the untrusted-text marking)
    #[command(subcommand)]
    Trust(commands::trust::TrustAction),

    /// How HQ is used on this machine: capabilities, corrections, return visits (counts only)
    Adoption {
        /// Print JSON
        #[arg(long)]
        json: bool,
    },

    /// The "I can also do this for you" offers: `hq offers replay` tunes them on past chats
    #[command(subcommand)]
    Offers(OffersAction),

    /// Show version and build info
    Version,
}

#[derive(Subcommand)]
pub(crate) enum CursorAction {
    Link {
        path: Option<String>,
        /// Ignored; kept so old scripts still parse.
        #[arg(long)]
        gitignore_rules: bool,
    },
    Status {
        path: Option<String>,
    },
    Refresh {
        path: Option<String>,
    },
    Unlink {
        path: Option<String>,
    },
}
