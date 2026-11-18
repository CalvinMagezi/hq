use anyhow::Result;
use clap::Parser;
use cli::{Cli, Commands};
#[cfg(test)]
use dispatch::{LiteRule, lite_rule};
use dispatch::{dispatch, run_chat};
use hq_core::config::HqConfig;
use tracing_subscriber::{EnvFilter, fmt};

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod android_runtime;
mod cli;
mod commands;
mod dispatch;
mod render;

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
    // Every command decides from this one config which built-in skills it offers.
    let unknown = hq_tools::skills::builtin::configure(&config);
    if !unknown.is_empty() {
        tracing::warn!(names = ?unknown, "skills.builtin.off names skills HQ does not ship; check the spelling");
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
        || (args.get(1).is_some_and(|a| a == "web") && before_dashes.clone().any(|a| a == "link"))
        // Adoption reports are read over `adb` and piped; their logs stay out of the numbers.
        || args.get(1).is_some_and(|a| a == "adoption" || a == "offers");
    // iroh logs full endpoint ids at info; the edge keeps them out of its log unless asked.
    let is_edge = args
        .iter()
        .skip(1)
        .take_while(|a| *a != "--")
        .any(|a| a == "edge");
    // Reports print their own findings; startup and migration logs would bury them.
    let is_report = args
        .get(1)
        .is_some_and(|a| matches!(a.as_str(), "doctor" | "health" | "ps" | "status"))
        // Pairing walks a person through a code and six digits; a fresh database's migrations
        // would scroll the code off the screen.
        || (args.get(1).is_some_and(|a| a == "bridge")
            && args.get(2).is_some_and(|a| a == "pair" || a == "join"));
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
            &["suite", "measure", "x"],
            &["suite", "jobs"],
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
mod mcp_args_tests {
    use super::{Cli, Commands};
    use clap::Parser;

    #[test]
    fn bare_hq_mcp_only_reports_and_install_has_to_be_asked_for() {
        let sub_of = |argv: &[&str]| match Cli::try_parse_from(argv).unwrap().command {
            Some(Commands::Mcp { sub, .. }) => sub,
            _ => panic!("not the mcp command"),
        };
        assert_eq!(sub_of(&["hq", "mcp"]), "status");
        assert_eq!(sub_of(&["hq", "mcp", "install"]), "install");
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

    #[cfg(all(unix, feature = "bridge"))]
    #[test]
    fn claude_commands_take_an_optional_hub_name_and_no_key() {
        for ok in [
            &["hq", "claude", "add"][..],
            &["hq", "claude", "status", "phone"],
            &["hq", "claude", "remove", "phone"],
        ] {
            assert!(Cli::try_parse_from(ok).is_ok(), "{ok:?}");
        }
        for bad in [
            &["hq", "claude"][..],
            &["hq", "claude", "add", "--key", "x"],
            &["hq", "claude", "add", "a", "b"],
        ] {
            assert!(Cli::try_parse_from(bad).is_err(), "{bad:?}");
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
