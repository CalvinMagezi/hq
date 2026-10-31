use anyhow::Result;
use clap::{Parser, Subcommand};
use hq_core::config::HqConfig;
use std::path::PathBuf;
use tracing_subscriber::{EnvFilter, fmt};

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod android_runtime;
mod commands;
mod render;
use commands::backup::{BackupCommand, RestoreArgs};
use commands::connector::ConnectorAction;
use commands::edge::EdgeAction;
use commands::net::NetAction;
use commands::queue::QueueAction;
use commands::skills::SkillsAction;

#[derive(Parser)]
#[command(name = "hq", version = commands::update::LONG_VERSION, about = "Agent-HQ: Local-first AI agent hub")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Config file path override
    #[arg(long, global = true)]
    config: Option<String>,

    /// Vault path override
    #[arg(long, global = true)]
    vault: Option<String>,
}

/// `hq env` subcommands.
#[derive(Subcommand)]
enum EnvAction {
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
enum Commands {
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
        /// Subcommand: list (default), spawn <harness>, status <id>, logs <id>, send <id>, stop <id>, resume <id>, stats [<root id>] [--task <id>]
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

    /// Install MCP server config for Claude, Cursor, VS Code, Copilot, OpenCode, Antigravity
    Mcp {
        /// Subcommand: install, status, remove, doctor
        #[arg(default_value = "install")]
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

    /// Show what the structured-decision gates did (counts, scores, cost, what was hidden)
    Decisions {
        /// How many days back to summarize
        #[arg(long, default_value = "7")]
        days: u32,
        /// Only this gate, e.g. email_fyi, memory_turn
        #[arg(long)]
        site: Option<String>,
    },

    /// Show version and build info
    Version,
}

#[derive(Subcommand)]
enum CursorAction {
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

fn project_scope(path: Option<String>) -> commands::mcp::Scope {
    commands::mcp::Scope {
        target: Some("project".into()),
        path: path.map(PathBuf::from),
        ..Default::default()
    }
}

fn main() -> Result<()> {
    android_runtime::init();
    // Load .env files BEFORE spawning any tokio threads, so set_var is safe.
    // `update` and `update-db` run as root from a timer: a .env in the current
    // directory must not be able to redirect them.
    let privileged_command = matches!(
        std::env::args().nth(1).as_deref(),
        Some("update" | "update-db")
    );
    // An agent's hook runs inside its sandbox, where the user's env files and
    // config are unreadable on purpose; it needs neither. Nor does `host sign`, which `hq update`
    // runs on a build it has not installed yet and which must not read a config or env file
    // that could redirect it.
    let args: Vec<String> = std::env::args().skip(1).take(2).collect();
    let hook_command = matches!(args.as_slice(), [host, sub] if host == "host" && (sub == "report" || sub == "gate" || sub == "sign"));
    if !privileged_command && !hook_command {
        if let Ok(home) = std::env::var("HOME") {
            load_env_file(&std::path::PathBuf::from(home).join(".env.local"));
        }
        load_env_file(&std::path::PathBuf::from(".env.local"));
        load_env_file(&std::path::PathBuf::from(".env"));
    }
    apply_config_flag()?;
    hq_core::daemon_restart::process_started_at();

    let result = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(async_main());
    if result.is_ok() && hq_core::daemon_restart::restart_requested() {
        std::process::exit(hq_core::daemon_restart::RESTART_EXIT_CODE);
    }
    result
}

async fn async_main() -> Result<()> {
    init_tracing();
    commands::update::register_build_info();
    let cli = Cli::parse();

    // The updater runs as root from a systemd timer: it must not load the
    // vault config (and risk creating root-owned files under the vault).
    match cli.command {
        Some(Commands::Update(args)) => std::process::exit(commands::update::run(args).await),
        Some(Commands::UpdateDb { args }) => std::process::exit(commands::update::run_db(&args)),
        // `hq net` keeps its own files under ~/.hq/net and needs no vault or config.
        Some(Commands::Net { action, dir, spool }) => {
            return commands::net::run(action, dir, spool);
        }
        // `hq edge` reads only its own `edge:` section, and may run on a machine with no vault.
        Some(Commands::Edge { action }) => {
            return commands::edge::run(action).await;
        }
        _ => {}
    }

    // The host's hook reporter and ssh gate need no config, and the reporter
    // runs where the config is unreadable. `sign` runs under `hq update`, where a Lite
    // profile or a bad config must not turn into a warning on every macOS update.
    if let Some(Commands::Host {
        sub,
        arg,
        dir,
        allow_unsandboxed,
        max_agents,
        max_starts,
        key,
        gate_key,
        from,
        port,
        unix,
        rest,
        addr,
    }) = &cli.command
        && matches!(
            sub.as_str(),
            "report" | "gate" | "relay" | "sandbox-init" | "lockdown" | "unlock" | "sign"
        )
    {
        return commands::host::run(commands::host::HostArgs {
            sub: sub.clone(),
            arg: arg.clone(),
            addr: addr.clone(),
            dir: dir.clone(),
            allow_unsandboxed: *allow_unsandboxed,
            max_agents: *max_agents,
            max_starts: *max_starts,
            key: key.clone(),
            gate_key: gate_key.clone(),
            from: from.clone(),
            port: *port,
            unix: unix.clone(),
            rest: rest.clone(),
        })
        .await;
    }

    // `--config` was exported as HQ_CONFIG_PATH in main(), so this and every
    // other loader in the process read the same file.
    let mut config = HqConfig::load()?;
    if let Some(vault) = cli.vault {
        config.vault_path = vault.into();
    }

    let Some(command) = cli.command else {
        // Bare `hq` is chat, which Lite does not have.
        if config.profile.is_lite() {
            anyhow::bail!(
                "chat is not part of HQ Lite (profile: lite). Try `hq doctor --egress` or `hq web`."
            );
        }
        require_scaffolded_vault(&config);
        return run_chat(&config, None, false, None).await;
    };
    dispatch(command, &config).await
}

/// The value of `--config PATH` / `--config=PATH` in `args`, last one wins.
/// Read from the raw args because it has to happen before the runtime exists.
fn config_flag(args: &[String]) -> Option<String> {
    let mut found = None;
    let mut iter = args.iter().skip(1);
    while let Some(arg) = iter.next() {
        if arg == "--" {
            break;
        }
        if let Some(value) = arg.strip_prefix("--config=") {
            found = Some(value.to_string());
        } else if arg == "--config" {
            found = iter.next().cloned();
        }
    }
    found
}

/// Make `--config` authoritative: it must name an existing file, and it is
/// exported as HQ_CONFIG_PATH so `HqConfig::load()`, the cwd deny list, the web
/// server's per-turn loads and `hq doctor` all read that file. Called from
/// `main()` before any thread exists.
fn apply_config_flag() -> Result<()> {
    let Some(raw) = config_flag(&std::env::args().collect::<Vec<_>>()) else {
        return Ok(());
    };
    let path = std::path::absolute(&raw)?;
    if !path.is_file() {
        anyhow::bail!("--config {raw}: no such file");
    }
    // SAFETY: called from main() before the tokio runtime is built, so no other threads exist yet.
    unsafe { std::env::set_var("HQ_CONFIG_PATH", &path) };
    Ok(())
}

/// The log filter `hq edge` starts with: its own messages at info, the transport libraries at
/// warn, because they log full device endpoint ids otherwise.
const EDGE_LOG_FILTER: &str = "info,iroh=warn,iroh_relay=warn,iroh_base=warn,noq=warn,noq_proto=warn,portmapper=warn,tokio_rustls_acme=warn";

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

/// MCP stdio uses stdout for JSON-RPC, so `mcp-serve` logs to stderr; chat
/// (bare `hq` or `hq chat`) silences library logs. Both are read from the raw
/// args because tracing must be set up before clap parses.
fn init_tracing() {
    let args: Vec<String> = std::env::args().collect();
    let is_mcp = args.iter().any(|a| a == "mcp-serve");
    let is_chat = args.len() <= 1 || args.iter().any(|a| a == "chat");
    // The tunnel peer runs as a child whose output nobody may be reading; it reports through its
    // status file and event log, and a pipe nobody drains would block it once it filled.
    let is_tunnel_run = args.get(1).is_some_and(|a| a == "net")
        && args.iter().any(|a| a == "tunnel")
        && args.iter().any(|a| a == "run");
    // `hq bridge run` talks to the phone app in JSON lines on stdout; its logs must stay out of it.
    let is_bridge_run = args.windows(2).any(|w| w[0] == "bridge" && w[1] == "run");
    // `hq task` and `--json` print data for another program: logs go to stderr, quietly.
    let before_dashes = args.iter().skip(1).take_while(|a| *a != "--");
    let is_data = before_dashes.clone().take(4).any(|a| a == "task")
        || (args
            .get(1)
            .is_some_and(|a| a == "search" || a == "vault" || a == "web")
            && before_dashes.clone().any(|a| a == "--json"))
        || (args.get(1).is_some_and(|a| a == "web") && before_dashes.clone().any(|a| a == "link"));
    // iroh logs full endpoint ids at info; the edge keeps them out of its log unless asked.
    let is_edge = args
        .iter()
        .skip(1)
        .take_while(|a| *a != "--")
        .any(|a| a == "edge");
    // Reports print their own findings; startup and migration logs would bury them.
    let is_report = args
        .get(1)
        .is_some_and(|a| matches!(a.as_str(), "doctor" | "health" | "ps" | "status"));
    let default_level = if is_chat || is_tunnel_run {
        "off"
    } else if is_edge {
        EDGE_LOG_FILTER
    } else if is_data || is_bridge_run || is_report {
        "warn"
    } else {
        "info"
    };
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_level));
    if is_mcp || is_data || is_tunnel_run || is_bridge_run {
        fmt()
            .with_env_filter(filter)
            .with_target(false)
            .with_writer(std::io::stderr)
            .init();
    } else {
        fmt().with_env_filter(filter).with_target(false).init();
    }
}

/// Bare `hq` defaults to chat, and only that implicit default is gated; an
/// explicit command (including `hq chat`) runs and fails with its own message.
/// Without this, an unscaffolded vault gets a silent `create_dir_all` and the
/// user lands in chat against nothing, never told `hq install` was needed.
fn require_scaffolded_vault(config: &HqConfig) {
    if config.vault_path.join("_system/SOUL.md").exists() {
        return;
    }
    eprintln!();
    eprintln!("  No vault found at {}", config.vault_path.display());
    eprintln!("  Run `hq install` first to set it up.");
    eprintln!();
    std::process::exit(1);
}

async fn run_chat(
    config: &HqConfig,
    model: Option<String>,
    tui: bool,
    permission_preset: Option<String>,
) -> Result<()> {
    let preset = permission_preset
        .as_deref()
        .map(parse_permission_preset)
        .transpose()?;
    let flag = commands::chat::ServerFlag::from_cli(false, None, commands::chat::auto_from_env());
    commands::chat::start(config, model, tui, preset, flag, None).await
}

fn parse_permission_preset(name: &str) -> Result<hq_core::types::PermissionPreset> {
    use hq_core::types::PermissionPreset;
    PermissionPreset::from_name(name).ok_or_else(|| {
        let valid: Vec<&str> = PermissionPreset::all().iter().map(|p| p.name()).collect();
        anyhow::anyhow!(
            "Unknown permission preset '{name}'. Valid presets: {}",
            valid.join(", ")
        )
    })
}

async fn run_cursor(config: &HqConfig, action: CursorAction) -> Result<()> {
    let (old, sub, path) = match action {
        CursorAction::Link { path, .. } => ("cursor link", "install", path),
        CursorAction::Refresh { path } => ("cursor refresh", "install", path),
        CursorAction::Status { path } => ("cursor status", "status", path),
        CursorAction::Unlink { path } => ("cursor unlink", "remove", path),
    };
    commands::mcp::run_deprecated(config, old, sub, project_scope(path)).await
}

/// One arm per subcommand, each a single call into `commands::*`: a routing
/// table, so it stays one match rather than being split by topic.
/// How `profile: lite` treats a command.
enum LiteRule {
    /// Local only: always runs (diagnostics, configuration, local reads and process control).
    Open,
    /// Not part of Lite: refused under it.
    NotInLite,
    /// Runs under Lite only when nothing configured sends data to another service.
    Checked,
}

fn lite_rule(command: &Commands) -> LiteRule {
    match command {
        Commands::Chat { .. }
        | Commands::Sessions { .. }
        | Commands::Stack { .. }
        | Commands::Host { .. }
        | Commands::SelfApply { .. }
        | Commands::Agents { .. }
        | Commands::Models { .. } => LiteRule::NotInLite,
        #[cfg(all(unix, feature = "bridge"))]
        Commands::Bridge(_) => LiteRule::NotInLite,
        Commands::Install { .. }
        | Commands::Update(_)
        | Commands::UpdateDb { .. }
        | Commands::Health
        | Commands::Doctor { egress: true, .. }
        | Commands::Doctor { windows: true, .. }
        | Commands::Autostart { .. }
        | Commands::Lite { .. }
        | Commands::Backup { .. }
        | Commands::Restore(_)
        | Commands::Env { .. }
        | Commands::Task { .. }
        | Commands::Verify { .. }
        | Commands::Copilot { .. }
        | Commands::Status
        | Commands::Stop { .. }
        | Commands::Restart { .. }
        | Commands::Logs { .. }
        | Commands::Errors { .. }
        | Commands::Follow { .. }
        | Commands::Ps
        | Commands::Vault { .. }
        | Commands::Search { .. }
        | Commands::Config { .. }
        | Commands::Mcp { .. }
        | Commands::Link { .. }
        | Commands::Cursor { .. }
        | Commands::Kill
        | Commands::Clean
        | Commands::Service { .. }
        | Commands::Uninstall { .. }
        | Commands::Tools
        | Commands::Cost
        | Commands::Summary
        | Commands::Version => LiteRule::Open,
        // `hq web status` and `hq web stop` only manage the process.
        Commands::Web(args) if !args.serves() => LiteRule::Open,
        _ => LiteRule::Checked,
    }
}

async fn dispatch(command: Commands, config: &HqConfig) -> Result<()> {
    // HQ Lite: some commands are not part of it, and any command that could send data to
    // another service stops while something that would is configured.
    match lite_rule(&command) {
        LiteRule::Open => {}
        LiteRule::NotInLite if config.profile.is_lite() => {
            let advice = if cfg!(windows) {
                "Use Full HQ in WSL2 (docs/WINDOWS.md) for chat, coding-agent sessions and hosts."
            } else {
                "Use the full profile for chat, coding-agent sessions and hosts."
            };
            anyhow::bail!("this command is not part of HQ Lite (profile: lite). {advice}");
        }
        LiteRule::NotInLite => {}
        LiteRule::Checked => hq_core::config::enforce_lite(config, &hq_core::config::process_env)?,
    }
    match command {
        // Getting Started
        Commands::Install {
            vault_path,
            non_interactive,
            upgrade,
            minimal,
        } => commands::install::run(vault_path, non_interactive, upgrade, minimal).await,
        Commands::Onboard {
            from: Some(from),
            dry_run,
            ..
        } => commands::onboard_import::run(&from, dry_run),
        Commands::Onboard { step, reset, .. } => commands::onboard::run(config, step, reset).await,
        Commands::Sessions {
            sub,
            arg,
            prompt,
            cwd,
            label,
            host,
            purpose,
            observe,
            task,
            model,
            branch,
            owns,
            policy,
            lines,
        } => {
            commands::sessions::run(
                config,
                &sub,
                arg,
                prompt,
                commands::sessions::SpawnOpts {
                    cwd,
                    label,
                    host,
                    purpose,
                    observe,
                    task,
                    model,
                    branch,
                    owns,
                    policy,
                },
                lines,
            )
            .await
        }
        Commands::Stack {
            sub,
            name,
            task,
            json,
        } => commands::stack::run(config, &sub, name, task, json).await,
        Commands::Verify {
            sub,
            arg,
            extra,
            note,
            initiative,
            since,
            confirm,
            json,
        } => {
            commands::verify::run(
                config,
                commands::verify::Opts {
                    sub,
                    arg,
                    extra,
                    note,
                    json,
                    initiative,
                    since,
                    confirm,
                },
            )
            .await
        }
        Commands::Host {
            sub,
            arg,
            dir,
            allow_unsandboxed,
            max_agents,
            max_starts,
            key,
            gate_key,
            from,
            port,
            unix,
            rest,
            addr,
        } => {
            commands::host::run(commands::host::HostArgs {
                sub,
                arg,
                dir,
                allow_unsandboxed,
                max_agents,
                max_starts,
                key,
                gate_key,
                from,
                port,
                unix,
                rest,
                addr,
            })
            .await
        }
        Commands::SelfApply { run_id } => commands::self_apply::run(config, run_id).await,
        Commands::NotifyRestart { phase, reason, sha } => {
            commands::notify_restart::run(config, &phase, &reason, sha.as_deref()).await
        }
        Commands::Update(_)
        | Commands::UpdateDb { .. }
        | Commands::Net { .. }
        | Commands::Edge { .. } => {
            unreachable!("handled before config load")
        }
        Commands::Health => commands::health::run(config).await,
        Commands::Doctor { windows: true, .. } => commands::lite::doctor_windows(config),
        Commands::Doctor { egress: true, .. } => commands::doctor::run_egress(config),
        Commands::Doctor { egress: false, .. } => commands::doctor::run(config).await,
        Commands::Autostart { sub } => commands::lite::autostart(&sub),
        Commands::Lite { sub, folder } => commands::lite::lite(config, &sub, folder.as_deref()),
        Commands::Backup { command } => commands::backup::run(config, command),
        Commands::Restore(args) => commands::backup::restore(config, &args),
        Commands::Pair { platform } => commands::pair::run(config, &platform),
        Commands::Env { action: None } => commands::env::run(config).await,
        Commands::Env {
            action: Some(EnvAction::Set { provider, from_env }),
        } => commands::env::set(config, &provider, from_env.as_deref()).await,
        Commands::Task { cmd } => commands::task::run(config, cmd).await,
        Commands::Copilot {
            sub,
            model,
            write,
            agents,
        } => commands::copilot::run(config, &sub, model.as_deref(), write, agents).await,

        // Chat
        Commands::Chat {
            model,
            tui,
            permission_preset,
            server,
            local,
            prompt,
        } => {
            let flag = commands::chat::ServerFlag::from_cli(
                local,
                server,
                commands::chat::auto_from_env(),
            );
            let preset = permission_preset
                .as_deref()
                .map(parse_permission_preset)
                .transpose()?;
            commands::chat::start(config, model, tui, preset, flag, prompt).await
        }

        // Services
        Commands::Status => commands::status::run(config).await,
        Commands::Start { component } => commands::start::run(config, &component).await,
        Commands::Stop { component } => commands::stop::run(config, &component).await,
        Commands::Restart { component } => commands::restart::run(config, &component).await,

        // Monitoring
        Commands::Logs {
            target,
            lines,
            follow,
            errors,
        } => commands::logs::run(&target, lines, follow, errors),
        Commands::Errors { target, lines } => commands::logs::run(&target, lines, false, true),
        Commands::Follow { target } => commands::logs::run(&target, 30, true, false),
        Commands::Ps => commands::ps::run(config).await,

        // Vault Operations
        Commands::Vault { sub, args } => commands::vault::run(config, &sub, &args).await,
        Commands::Search { query, limit, json } => {
            commands::search::run(config, &query.join(" "), limit, json).await
        }
        Commands::Reindex => commands::search::reindex(config).await,
        Commands::Memory { sub, args } => commands::memory::run(config, &sub, &args).await,
        Commands::Profile(args) => commands::profile::run(config, &args).await,

        // Agents
        Commands::Mailbox { sub, args } => commands::mailbox::run(config, &sub, &args).await,
        Commands::Agents { sub, args } => commands::agents::run(config, &sub, &args).await,

        // Configuration
        Commands::Config { key, value } => {
            commands::config::run(config, key.as_deref(), value.as_deref()).await
        }
        Commands::Mcp {
            sub,
            path,
            target,
            global,
            scope: access,
            url,
        } => {
            let path = path.map(PathBuf::from);
            let scope = commands::mcp::Scope {
                target,
                global,
                path,
                access,
                url,
            };
            commands::mcp::run(config, &sub, scope).await
        }
        Commands::McpServe {
            remote: Some(name), ..
        } => commands::mcp_relay::run(config, &name).await,
        Commands::McpServe { scope, .. } => commands::mcp_serve::run(config, scope).await,
        Commands::Link { path } => {
            commands::mcp::run_deprecated(config, "link", "install", project_scope(path)).await
        }
        Commands::Cursor { action } => run_cursor(config, action).await,

        // Daemon
        Commands::Daemon { sub, arg } => commands::daemon::run(config, &sub, arg.as_deref()).await,

        // Advanced
        Commands::Kill => commands::kill::run(config).await,
        Commands::Clean => commands::clean::run(config).await,
        Commands::Service { sub, target } => commands::service::run(config, &sub, &target).await,
        Commands::Uninstall { target } if target == "lite" => {
            commands::lite::uninstall_lite(config)
        }
        Commands::Uninstall { target } => {
            commands::service::run(config, "uninstall", &target).await
        }

        // Tools, models and usage
        Commands::Tools => commands::tools::run(config).await,
        Commands::Models { sub } => commands::models::run(config, &sub).await,
        Commands::Usage { sub } => commands::usage::run(config, &sub).await,
        Commands::Cost => commands::usage::run(config, "summary").await,
        Commands::Summary => commands::usage::run(config, "daily").await,

        // Web Dashboard
        Commands::Web(args) => commands::web::run(config, args).await,

        // Developer Workflow
        Commands::Shortcuts { sub } => commands::shortcuts::run(config, &sub).await,
        Commands::Skills(action) => commands::skills::run(config, action).await,
        Commands::Queue(action) => commands::queue::run(config, action).await,
        Commands::Connector(action) => commands::connector::run(config, action),
        #[cfg(all(unix, feature = "bridge"))]
        Commands::Bridge(action) => commands::bridge::run(config, action).await,
        Commands::Decisions { days, site } => {
            commands::decisions::run(config, days, site.as_deref())
        }

        Commands::Version => {
            println!("hq {} (rust)", commands::update::VERSION);
            Ok(())
        }
    }
}

/// Load environment variables from a file (simple .env format).
/// Only sets vars that are not already set in the environment.
fn load_env_file(path: &std::path::Path) {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return,
    };
    for line in content.lines() {
        let line = line.trim();
        // Skip comments and empty lines
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // Parse KEY=VALUE (with optional `export` prefix and quotes)
        let line = line.strip_prefix("export ").unwrap_or(line);
        if let Some((key, value)) = line.split_once('=') {
            let key = key.trim();
            let value = value.trim();
            // Strip surrounding quotes
            let value = value
                .strip_prefix('"')
                .and_then(|v| v.strip_suffix('"'))
                .or_else(|| value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')))
                .unwrap_or(value);
            // Only set if not already in environment.
            // SAFETY: called from main() before the tokio runtime is built,
            // so no other threads exist yet.
            if std::env::var(key).is_err() {
                unsafe { std::env::set_var(key, value) };
            }
        }
    }
}

#[cfg(test)]
mod config_flag_tests {
    use super::config_flag;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn finds_both_spellings_and_stops_at_double_dash() {
        assert_eq!(
            config_flag(&args(&["hq", "--config", "/a.yaml", "doctor"])).as_deref(),
            Some("/a.yaml")
        );
        assert_eq!(
            config_flag(&args(&["hq", "doctor", "--config=/b.yaml"])).as_deref(),
            Some("/b.yaml")
        );
        assert_eq!(
            config_flag(&args(&["hq", "chat", "--", "--config", "/c.yaml"])),
            None
        );
        assert_eq!(config_flag(&args(&["hq", "doctor"])), None);
    }
}

#[cfg(test)]
mod lite_rule_tests {
    use super::{Cli, LiteRule, lite_rule};
    use clap::Parser;

    fn rule(argv: &[&str]) -> &'static str {
        let cli = Cli::try_parse_from(std::iter::once("hq").chain(argv.iter().copied())).unwrap();
        match lite_rule(&cli.command.expect("a subcommand")) {
            LiteRule::Open => "open",
            LiteRule::NotInLite => "not-in-lite",
            LiteRule::Checked => "checked",
        }
    }

    #[test]
    fn commands_that_serve_run_a_daemon_or_call_out_are_checked() {
        for argv in [
            &["start"][..],
            &["start", "all"],
            &["daemon", "run"],
            &["mcp-serve"],
            &["web"],
            &["web", "--detach"],
            &["reindex"],
            &["memory", "stats"],
            &["pair"],
            &["doctor"],
            &["usage"],
        ] {
            assert_eq!(rule(argv), "checked", "{argv:?}");
        }
    }

    #[test]
    fn chat_sessions_and_hosts_are_not_in_lite() {
        for argv in [
            &["chat"][..],
            &["sessions", "list"],
            &["host", "status"],
            &["agents", "list"],
            &["models"],
        ] {
            assert_eq!(rule(argv), "not-in-lite", "{argv:?}");
        }
    }

    #[test]
    fn diagnostics_configuration_and_process_control_always_run() {
        for argv in [
            &["doctor", "--egress"][..],
            &["config"],
            &["env"],
            &["copilot", "link"],
            &["autostart", "status"],
            &["lite", "export", "x"],
            &["backup", "keygen", "x"],
            &["backup", "verify", "x"],
            &["restore", "x", "--into", "y"],
            &["doctor", "--windows"],
            &["uninstall", "lite"],
            &["task", "list"],
            &["search", "x", "--json"],
            &["mcp", "install"],
            &["mcp", "status"],
            &["status"],
            &["stop"],
            &["web", "status"],
            &["web", "stop"],
            &["vault", "list"],
            &["search", "tasks"],
            &["version"],
        ] {
            assert_eq!(rule(argv), "open", "{argv:?}");
        }
    }
}

#[cfg(test)]
mod net_args_tests {
    use super::Cli;
    use clap::Parser;

    #[cfg(all(unix, feature = "bridge"))]
    #[test]
    fn bridge_commands_parse_and_no_option_takes_a_secret_or_a_code() {
        for argv in [
            &["hq", "bridge", "pair"][..],
            &[
                "hq",
                "bridge",
                "pair",
                "--ttl",
                "60",
                "--relay",
                "none",
                "--no-addrs",
            ],
            &[
                "hq",
                "bridge",
                "join",
                "hqpair://ticket/AAAA",
                "--bind",
                "127.0.0.1:0",
            ],
            &["hq", "bridge", "join", "-"],
            &["hq", "bridge", "join"],
            &["hq", "bridge", "serve"],
            &["hq", "bridge", "run"],
            &[
                "hq",
                "bridge",
                "run",
                "--relay",
                "none",
                "--name",
                "phone",
                "--no-addrs",
            ],
            &["hq", "bridge", "ping", "abcd", "--count", "3"],
            &["hq", "bridge", "peers", "--all", "--json"],
            &["hq", "bridge", "revoke", "abcd"],
        ] {
            assert!(Cli::try_parse_from(argv).is_ok(), "{argv:?}");
        }
        // The one-time code travels only inside the link, never as its own option, and pairing
        // cannot be answered from the command line: the digits are typed at the prompt.
        for argv in [
            &["hq", "bridge", "pair", "--code", "x"][..],
            &["hq", "bridge", "run", "--digits", "123456"],
            &["hq", "bridge", "pair", "--auto-accept"],
            &["hq", "bridge", "pair", "--ttl", "99999999999"],
            &["hq", "bridge", "pair", "--ttl", "1"],
            &["hq", "bridge", "join", "--digits", "123456", "x"],
        ] {
            assert!(Cli::try_parse_from(argv).is_err(), "{argv:?}");
        }
    }

    #[cfg(all(unix, feature = "tunnel"))]
    #[test]
    fn tunnel_commands_parse_and_nothing_takes_a_key_on_the_command_line() {
        let parse = |args: &[&str]| {
            let mut argv = vec!["hq", "net", "tunnel"];
            argv.extend_from_slice(args);
            Cli::try_parse_from(argv)
        };
        for ok in [
            &["keygen"][..],
            &["keygen", "--stdout"],
            &["import", "--confirm-hub", "k"],
            &["import", "join.txt"],
            &["check"],
            &["--tunnel-dir", "d", "run"],
            &["run", "--tunnel-dir", "d"],
            &["run", "--key-file", "k", "--no-control"],
            &["status", "--json"],
        ] {
            assert!(parse(ok).is_ok(), "{ok:?}");
        }
        for bad in [
            &["run", "--key", "x"][..],
            &["run", "--private-key", "x"],
            &["run", "--secret", "x"],
            &["import", "--private-key", "x"],
            &["keygen", "--private-key", "x"],
        ] {
            assert!(parse(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn peer_add_parses_a_keepalive_as_whole_seconds_and_leaves_the_range_to_add_peer() {
        let parse = |keepalive: &str| {
            let argv = [
                "hq",
                "net",
                "peer",
                "add",
                "pixel",
                "--pubkey",
                "k",
                "--keepalive",
                keepalive,
            ];
            Cli::try_parse_from(argv)
        };
        for ok in ["0", "25", "65535"] {
            assert!(parse(ok).is_ok(), "{ok}");
        }
        for bad in ["-1", "65536", "x", ""] {
            assert!(parse(bad).is_err(), "{bad}");
        }
    }
}
