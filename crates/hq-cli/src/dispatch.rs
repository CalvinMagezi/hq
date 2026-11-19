//! Where a parsed command goes: the Lite gate and the routing table into `commands::*`.

use anyhow::Result;
use hq_core::config::HqConfig;
use std::path::PathBuf;

use crate::cli::{Commands, CursorAction, EnvAction};
use crate::commands;

pub(crate) fn project_scope(path: Option<String>) -> commands::mcp::Scope {
    commands::mcp::Scope {
        target: Some("project".into()),
        path: path.map(PathBuf::from),
        ..Default::default()
    }
}

pub(crate) async fn run_chat(
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

pub(crate) fn parse_permission_preset(name: &str) -> Result<hq_core::types::PermissionPreset> {
    use hq_core::types::PermissionPreset;
    PermissionPreset::from_name(name).ok_or_else(|| {
        let valid: Vec<&str> = PermissionPreset::all().iter().map(|p| p.name()).collect();
        anyhow::anyhow!(
            "Unknown permission preset '{name}'. Valid presets: {}",
            valid.join(", ")
        )
    })
}

pub(crate) async fn run_cursor(config: &HqConfig, action: CursorAction) -> Result<()> {
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
pub(crate) enum LiteRule {
    /// Local only: always runs (diagnostics, configuration, local reads and process control).
    Open,
    /// Not part of Lite: refused under it.
    NotInLite,
    /// Runs under Lite only when nothing configured sends data to another service.
    Checked,
}

pub(crate) fn lite_rule(command: &Commands) -> LiteRule {
    match command {
        Commands::Chat { .. }
        | Commands::Sessions { .. }
        | Commands::Stack { .. }
        | Commands::Host { .. }
        | Commands::Quota { .. }
        | Commands::SelfApply { .. }
        | Commands::Agents { .. }
        | Commands::Models { .. }
        | Commands::Github(_) => LiteRule::NotInLite,
        #[cfg(all(unix, feature = "bridge"))]
        Commands::Bridge(_) | Commands::Claude(_) => LiteRule::NotInLite,
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
        // The suite reads and writes only the vault and its own folders; it calls out to nothing.
        | Commands::Suite { .. }
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

pub(crate) async fn dispatch(command: Commands, config: &HqConfig) -> Result<()> {
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
            from,
            owns,
            policy,
            lines,
            watch,
        } => {
            if matches!(sub.as_str(), "watch" | "channel") {
                return commands::sessions_watch::run(config, &sub, task.as_deref(), *watch).await;
            }
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
                    from,
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
        Commands::Suite { command } => commands::suite::run(config, command).await,
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
        Commands::Quota {
            json,
            refresh,
            lane,
            host,
        } => commands::quota::run(config, json, refresh, lane, host).await,
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
        Commands::Github(action) => commands::github::run(action).await,
        #[cfg(all(unix, feature = "bridge"))]
        Commands::Bridge(action) => commands::bridge::run(config, action).await,
        #[cfg(all(unix, feature = "bridge"))]
        Commands::Claude(action) => commands::claude::run(config, action),
        Commands::Focus(action) => commands::focus::run(config, action),
        Commands::Inbox(action) => commands::inbox::run(config, action),
        Commands::Trust(action) => commands::trust::run(config, action),
        Commands::Adoption { json } => commands::adoption::run_adoption(config, json),
        Commands::Offers(action) => commands::adoption::run_offers(config, action),
        Commands::Decisions { days, site } => {
            commands::decisions::run(config, days, site.as_deref())
        }

        Commands::Version => {
            println!("hq {} (rust)", commands::update::VERSION);
            Ok(())
        }
    }
}
