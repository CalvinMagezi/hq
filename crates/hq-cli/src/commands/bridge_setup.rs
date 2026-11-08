//! `hq bridge pair --setup` (and `join --setup`): everything a computer needs to run helpers for
//! a phone, in one command. Before pairing it lets the built-in host take calls from the bridge
//! and starts the host as a login service; after pairing it keeps `hq bridge serve` running as one
//! too. Without `--setup` these stay steps the owner takes (`docs/JOIN_A_MACHINE.md`).

use super::host_install::{self, LaunchAgent};
use anyhow::{Context, Result, bail};
use hq_core::config::{AgentHostConfig, HqConfig};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Where the host's gate listens when `--setup` turns it on: loopback, where bridge traffic arrives.
pub(crate) const GATE_LISTEN: &str = "127.0.0.1:7747";
const LOOPBACK: &str = "127.0.0.1";
/// The relay a computer set up this way uses unless `--relay` names another: the phone app's own
/// default, so a phone on mobile data can still find a computer behind a home router.
pub(crate) const SETUP_RELAY: &str = "default";
const SERVE_LABEL: &str = "com.hq.bridge";
const SERVE_UNIT: &str = "hq-bridge.service";
/// Coding agents a helper can be, by the command that starts them.
const AGENT_COMMANDS: &[(&str, &str)] = &[("claude", "Claude Code"), ("codex", "Codex")];
const NAME_MAX: usize = 63;

/// What `--setup` does with the host's gate.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum GatePlan {
    /// No gate yet: turn it on at [`GATE_LISTEN`].
    Write,
    /// Already on loopback and admitting loopback.
    Keep,
    /// Set by hand to something the bridge cannot use; left as it is.
    Conflict(String),
}

pub(crate) fn gate_plan(host: &AgentHostConfig) -> GatePlan {
    let Some(listen) = host.gate_listen.as_deref() else {
        return GatePlan::Write;
    };
    let loopback = listen
        .parse::<SocketAddr>()
        .is_ok_and(|a| a.ip().is_loopback());
    let admitted = host.gate_allowed_sources.iter().any(|s| s == LOOPBACK);
    if loopback && admitted {
        GatePlan::Keep
    } else {
        GatePlan::Conflict(listen.to_string())
    }
}

/// The change [`GatePlan::Write`] makes to the config file.
pub(crate) fn turn_gate_on(config: &mut HqConfig) {
    config.agent_host.gate_listen = Some(GATE_LISTEN.to_string());
    if !config
        .agent_host
        .gate_allowed_sources
        .iter()
        .any(|s| s == LOOPBACK)
    {
        config
            .agent_host
            .gate_allowed_sources
            .push(LOOPBACK.to_string());
    }
}

/// Before pairing: the gate on loopback, and the host running as a service so it reads it. A host
/// that cannot be installed is reported, not fatal: pairing still works and the host can be run
/// by hand.
pub(crate) fn prepare() -> Result<()> {
    let config = HqConfig::load().context("reading the HQ config")?;
    match gate_plan(&config.agent_host) {
        GatePlan::Write => {
            HqConfig::save_patch(turn_gate_on).context("saving the HQ config")?;
            println!(
                "Turned on the host's gate at {GATE_LISTEN}, so a paired phone can start helpers here."
            );
        }
        GatePlan::Keep => {}
        GatePlan::Conflict(listen) => println!(
            "note: agent_host.gate_listen is {listen}, set by hand, so it was left as it is. The phone reaches helpers only through a gate on 127.0.0.1 that admits 127.0.0.1."
        ),
    }
    match start_host() {
        Ok(()) => println!("The host runs as a service and starts again when you log in."),
        Err(e) => println!(
            "note: the host could not be started as a service ({e:#}). Run `hq host serve` in a terminal and keep it open."
        ),
    }
    Ok(())
}

/// Installs the host as a login service, and restarts it so a running one reads the new gate.
fn start_host() -> Result<()> {
    host_install::install()?;
    if !cfg!(target_os = "macos") {
        // `enable --now` leaves a running host as it was; the gate is read at start.
        host_install::run(
            "systemctl",
            &["--user", "restart", host_install::SYSTEMD_UNIT],
        )?;
    }
    Ok(())
}

/// The arguments the serve service runs `hq` with.
pub(crate) fn serve_args(relay: &str) -> Result<Vec<&str>> {
    if relay.is_empty() || relay.chars().any(|c| c.is_whitespace() || c.is_control()) {
        bail!("the relay cannot be written into a service");
    }
    Ok(vec!["bridge", "serve", "--relay", relay])
}

pub(crate) fn serve_unit(exe: &Path, home: &Path, relay: &str) -> Result<String> {
    let exe_text = exe.to_string_lossy();
    if exe_text
        .chars()
        .any(|c| c.is_whitespace() || c.is_control())
    {
        bail!(
            "{} has a space in its path, which a service line cannot hold; install hq in ~/.local/bin",
            exe.display()
        );
    }
    Ok(format!(
        "[Unit]\nDescription=Agent HQ bridge to paired devices\n\n[Service]\nExecStart={exe_text} {args}\nEnvironment=PATH={path}\nRestart=always\nRestartSec=10\n\n[Install]\nWantedBy=default.target\n",
        args = serve_args(relay)?.join(" "),
        path = host_install::path_env(home),
    ))
}

/// After pairing: `hq bridge serve` as a login service, so the phone can reach this computer
/// whenever it is on. Returns where the service is defined.
pub(crate) fn install_serve(relay: &str) -> Result<PathBuf> {
    let home = host_install::home()?;
    if cfg!(target_os = "macos") {
        let args = serve_args(relay)?;
        let log = home.join("Library/Logs/hq-bridge.log");
        return LaunchAgent {
            label: SERVE_LABEL,
            args: &args,
            stdout: log.clone(),
            stderr: log,
        }
        .install();
    }
    let exe = host_install::service_binary(&home)?;
    let path = home.join(".config/systemd/user").join(SERVE_UNIT);
    host_install::write_private(&path, &serve_unit(&exe, &home, relay)?)?;
    host_install::run("systemctl", &["--user", "daemon-reload"])?;
    host_install::run("systemctl", &["--user", "enable", SERVE_UNIT])?;
    host_install::run("systemctl", &["--user", "restart", SERVE_UNIT])?;
    Ok(path)
}

/// After pairing, the last words: the service, and what a helper still needs on this computer.
pub(crate) fn finish(relay: &str) {
    match install_serve(relay) {
        Ok(path) => {
            println!(
                "This computer stays reachable by the phone while you are logged in ({}).",
                path.display()
            );
            if !cfg!(target_os = "macos") {
                println!(
                    "To keep it reachable after you log out: loginctl enable-linger \"$USER\""
                );
            }
        }
        Err(e) => println!(
            "note: could not keep the bridge running as a service ({e:#}). Run `hq bridge serve --relay {relay}` and keep it open."
        ),
    }
    let found: Vec<&str> = AGENT_COMMANDS
        .iter()
        .filter(|(cmd, _)| on_path(cmd))
        .map(|(_, name)| *name)
        .collect();
    if found.is_empty() {
        println!(
            "Last step: install Claude Code or Codex on this computer and sign in once. Then start a helper from the phone."
        );
    } else {
        println!(
            "Done. Open Helpers on the phone and start one: it runs here with {}.",
            found.join(" or ")
        );
    }
}

fn on_path(cmd: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|paths| {
        std::env::split_paths(&paths).any(|dir| {
            let file = dir.join(cmd);
            std::fs::metadata(&file).is_ok_and(|m| m.is_file())
        })
    })
}

/// The name the phone lists this computer under: the Mac's own name, else the host name.
pub(crate) fn computer_name() -> Option<String> {
    let asked = if cfg!(target_os = "macos") {
        command_line("scutil", &["--get", "ComputerName"])
    } else {
        None
    };
    asked
        .or_else(|| command_line("hostname", &[]))
        .map(|n| clean_name(&n))
        .filter(|n| !n.is_empty())
}

fn command_line(program: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(program).args(args).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// One printable line, at most [`NAME_MAX`] characters.
pub(crate) fn clean_name(name: &str) -> String {
    name.chars()
        .filter(|c| !c.is_control())
        .take(NAME_MAX)
        .collect::<String>()
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host(listen: Option<&str>, allowed: &[&str]) -> AgentHostConfig {
        AgentHostConfig {
            gate_listen: listen.map(str::to_string),
            gate_allowed_sources: allowed.iter().map(|s| s.to_string()).collect(),
            ..AgentHostConfig::default()
        }
    }

    #[test]
    fn the_gate_is_turned_on_only_when_nobody_set_it() {
        assert_eq!(gate_plan(&host(None, &[])), GatePlan::Write);
        assert_eq!(
            gate_plan(&host(Some("127.0.0.1:7747"), &["127.0.0.1"])),
            GatePlan::Keep
        );
        assert_eq!(
            gate_plan(&host(Some("100.64.0.2:7747"), &["100.64.0.1"])),
            GatePlan::Conflict("100.64.0.2:7747".into())
        );
        assert_eq!(
            gate_plan(&host(Some("127.0.0.1:7747"), &["100.64.0.1"])),
            GatePlan::Conflict("127.0.0.1:7747".into())
        );
    }

    #[test]
    fn turning_the_gate_on_keeps_other_sources_and_adds_loopback_once() {
        let mut c = HqConfig::default();
        c.agent_host.gate_allowed_sources = vec!["100.64.0.1".into()];
        turn_gate_on(&mut c);
        turn_gate_on(&mut c);
        assert_eq!(c.agent_host.gate_listen.as_deref(), Some(GATE_LISTEN));
        assert_eq!(
            c.agent_host.gate_allowed_sources,
            vec!["100.64.0.1".to_string(), LOOPBACK.to_string()]
        );
        assert_eq!(gate_plan(&c.agent_host), GatePlan::Keep);
    }

    #[test]
    fn the_serve_service_runs_the_bridge_with_its_relay() {
        let unit = serve_unit(
            Path::new("/home/example/.local/bin/hq"),
            Path::new("/home/example"),
            "https://relay.example.com/",
        )
        .unwrap();
        assert!(unit.contains(
            "ExecStart=/home/example/.local/bin/hq bridge serve --relay https://relay.example.com/\n"
        ));
        assert!(unit.contains("Restart=always"));
        assert!(serve_args("a b").is_err());
        assert!(serve_args("").is_err());
        assert!(
            serve_unit(
                Path::new("/home/example/My Apps/hq"),
                Path::new("/home/example"),
                "default"
            )
            .is_err()
        );
    }

    #[test]
    fn a_computer_name_is_one_short_printable_line() {
        assert_eq!(clean_name("  Example's MacBook\n"), "Example's MacBook");
        assert_eq!(clean_name(&"x".repeat(100)).len(), NAME_MAX);
    }
}
