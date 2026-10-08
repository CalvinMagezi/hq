use anyhow::Result;
use clap::{Parser, Subcommand};
use hq_core::config::HqConfig;
use std::path::PathBuf;
use tracing_subscriber::{EnvFilter, fmt};

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod commands;
mod render;
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
    },

    /// Manage long-lived harness sessions (claude-code, cursor, opencode, antigravity, pi...)
    Sessions {
        /// Subcommand: list (default), spawn <harness>, status <id>, logs <id>, send <id>, stop <id>, resume <id>
        #[arg(default_value = "list")]
        sub: String,
        /// Harness name (spawn) or session id (status/logs/send/stop/resume)
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
        /// Log lines to tail (logs)
        #[arg(long, default_value_t = 40)]
        lines: usize,
    },

    /// Run or inspect the built-in agent host (long-lived coding agents in pseudo-terminals)
    Host {
        /// serve (run the host in this terminal), status, stop, install (run it as a login service), join (print this machine's join code), add <code> (pair a machine, on the HQ side), check <host>, authorize (pin a remote key to the gate), report (used by agent hooks), or gate (the ssh forced command for remote access)
        #[arg(default_value = "status")]
        sub: String,
        /// add: the join code; check: the host name; join: the name for this machine
        arg: Option<String>,
        /// Directory holding host.sock and operator.token (default: ~/.hq/run/host)
        #[arg(long)]
        dir: Option<std::path::PathBuf>,
        /// serve only: also start agents that are not under the process sandbox. Without it the host refuses them, whoever asks.
        #[arg(long)]
        allow_unsandboxed: bool,
        /// authorize only: the ssh public key to pin to `hq host gate`
        #[arg(long)]
        key: Option<String>,
        /// authorize only: the address the key may connect from (ssh `from=`)
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
    Doctor,

    /// Set up API keys interactively
    Env,

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
    /// Vault operations (list, read, write, export-pdf, stats, context)
    Vault {
        /// Subcommand: list, tree, read, write, export-pdf, stats, context
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
    McpServe,

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
        /// Target: all, agent, relay, daemon
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
    // Load .env files BEFORE spawning any tokio threads, so set_var is safe.
    // `update` and `update-db` run as root from a timer: a .env in the current
    // directory must not be able to redirect them.
    let privileged_command = matches!(
        std::env::args().nth(1).as_deref(),
        Some("update" | "update-db")
    );
    // An agent's hook runs inside its sandbox, where the user's env files and
    // config are unreadable on purpose; it needs neither.
    let args: Vec<String> = std::env::args().skip(1).take(2).collect();
    let hook_command = matches!(args.as_slice(), [host, sub] if host == "host" && (sub == "report" || sub == "gate"));
    if !privileged_command && !hook_command {
        if let Ok(home) = std::env::var("HOME") {
            load_env_file(&std::path::PathBuf::from(home).join(".env.local"));
        }
        load_env_file(&std::path::PathBuf::from(".env.local"));
        load_env_file(&std::path::PathBuf::from(".env"));
    }
    apply_config_flag()?;

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(async_main())
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
        _ => {}
    }

    // The host's hook reporter and ssh gate need no config, and the reporter
    // runs where the config is unreadable.
    if let Some(Commands::Host { sub, arg, dir, allow_unsandboxed, key, from, port, unix, rest, addr }) = &cli.command
        && matches!(sub.as_str(), "report" | "gate" | "relay" | "sandbox-init")
    {
        return commands::host::run(commands::host::HostArgs {
            sub: sub.clone(),
            arg: arg.clone(),
            addr: addr.clone(),
            dir: dir.clone(),
            allow_unsandboxed: *allow_unsandboxed,
            key: key.clone(),
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

/// MCP stdio uses stdout for JSON-RPC, so `mcp-serve` logs to stderr; chat
/// (bare `hq` or `hq chat`) silences library logs. Both are read from the raw
/// args because tracing must be set up before clap parses.
fn init_tracing() {
    let args: Vec<String> = std::env::args().collect();
    let is_mcp = args.iter().any(|a| a == "mcp-serve");
    let is_chat = args.len() <= 1 || args.iter().any(|a| a == "chat");
    let default_level = if is_chat { "off" } else { "info" };
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_level));
    if is_mcp {
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
    let preset = permission_preset.as_deref().map(parse_permission_preset).transpose()?;
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
async fn dispatch(command: Commands, config: &HqConfig) -> Result<()> {
    match command {
        // Getting Started
        Commands::Install {
            vault_path,
            non_interactive,
            upgrade,
            minimal,
        } => commands::install::run(vault_path, non_interactive, upgrade, minimal).await,
        Commands::Onboard { step, reset } => commands::onboard::run(config, step, reset).await,
        Commands::Sessions {
            sub,
            arg,
            prompt,
            cwd,
            label,
            host,
            lines,
        } => commands::sessions::run(config, &sub, arg, prompt, cwd, label, host, lines).await,
        Commands::Host { sub, arg, dir, allow_unsandboxed, key, from, port, unix, rest, addr } => commands::host::run(commands::host::HostArgs { sub, arg, dir, allow_unsandboxed, key, from, port, unix, rest, addr }).await,
        Commands::SelfApply { run_id } => commands::self_apply::run(config, run_id).await,
        Commands::NotifyRestart { phase, reason, sha } => {
            commands::notify_restart::run(config, &phase, &reason, sha.as_deref()).await
        }
        Commands::Update(_) | Commands::UpdateDb { .. } => {
            unreachable!("handled before config load")
        }
        Commands::Health => commands::health::run(config).await,
        Commands::Doctor => commands::doctor::run(config).await,
        Commands::Pair { platform } => commands::pair::run(config, &platform),
        Commands::Env => commands::env::run(config).await,

        // Chat
        Commands::Chat {
            model,
            tui,
            permission_preset,
            server,
            local,
            prompt,
        } => {
            let flag =
                commands::chat::ServerFlag::from_cli(local, server, commands::chat::auto_from_env());
            let preset = permission_preset.as_deref().map(parse_permission_preset).transpose()?;
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
        Commands::Search { query, limit } => {
            commands::search::run(config, &query.join(" "), limit).await
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
        } => {
            let path = path.map(PathBuf::from);
            let scope = commands::mcp::Scope { target, global, path };
            commands::mcp::run(config, &sub, scope).await
        }
        Commands::McpServe => commands::mcp_serve::run(config).await,
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
        Commands::Decisions { days, site } => commands::decisions::run(config, days, site.as_deref()),

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
