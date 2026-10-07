//! Checks around a launch, so a host that cannot run the harness fails fast and
//! cleanly instead of leaving a pane that waits for an agent forever.

use super::Harness;
use crate::herdr::{AgentInfo, HostBackend, Launched};
use anyhow::{Result, bail};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// Directories under `$HOME` that a login shell or a version manager usually
/// puts on PATH. The pane runs the user's shell, so these count even when HQ's
/// own PATH lacks them.
const HOME_BIN_DIRS: [&str; 9] = [
    ".local/bin",
    ".bun/bin",
    ".cargo/bin",
    ".npm-global/bin",
    ".volta/bin",
    ".local/share/mise/shims",
    ".asdf/shims",
    ".local/share/pnpm",
    ".claude/local",
];

/// System directories a service started by systemd or launchd often omits.
const SYSTEM_BIN_DIRS: [&str; 2] = ["/usr/local/bin", "/opt/homebrew/bin"];

/// `~/.nvm/versions/node/<version>/bin`: one directory per installed Node.
const NVM_VERSIONS: &str = ".nvm/versions/node";

/// Pane lines quoted in the error when an agent never came up.
const STUCK_SCREEN_LINES: usize = 20;

/// Quoted lines and characters, so an error stays a sentence and not a dump of
/// pane text (which is untrusted).
const QUOTED_LINES: usize = 3;
const QUOTED_CHARS: usize = 240;

/// Anything here in a wrapper command means a shell will interpret it, so HQ
/// cannot tell which binary actually runs.
const SHELL_SYNTAX: [char; 10] = ['&', ';', '|', '<', '>', '`', '(', ')', '\n', '\\'];

/// Characters that make the first word something other than a literal name.
const NON_LITERAL_WORD: [char; 5] = ['=', '$', '~', '\'', '"'];

/// The one binary to look for, when it can be known. A wrapper `command` is
/// typed into a shell, so it is only checked when it is a plain program name or
/// absolute path followed by arguments: an env prefix (`A=b claude`), a `cd x
/// && claude`, a quoted path or a `$VAR` all leave the real binary to the shell
/// and yield `None`.
fn plain_binary(harness: &Harness) -> Option<String> {
    let Some(command) = harness.profile.as_ref().and_then(|p| p.command.as_deref()) else {
        return Some(harness.spec.binary().to_string());
    };
    let first = command.split_whitespace().next()?;
    let literal = !first.contains(NON_LITERAL_WORD) && !command.contains(SHELL_SYNTAX);
    let bare_or_absolute = !first.contains('/') || first.starts_with('/');
    (literal && bare_or_absolute).then(|| first.to_string())
}

/// Where the pane could find a binary: the profile's PATH entries (literal
/// `$PATH` and `~` entries skipped, since only a shell expands them), HQ's own
/// PATH, then the login-shell and version-manager directories under `home`.
fn search_roots(
    harness: &Harness,
    home: Option<&Path>,
    process_path: Option<OsString>,
) -> Vec<PathBuf> {
    let from_profile = harness.profile.as_ref().and_then(|p| p.env.get("PATH"));
    let mut roots: Vec<PathBuf> = from_profile
        .into_iter()
        .flat_map(|path| std::env::split_paths(path))
        .filter(|dir| {
            let text = dir.to_string_lossy();
            !text.contains('$') && !text.starts_with('~')
        })
        .collect();
    roots.extend(process_path.iter().flat_map(|p| std::env::split_paths(p)));
    if let Some(home) = home {
        roots.extend(HOME_BIN_DIRS.iter().map(|d| home.join(d)));
        roots.extend(nvm_bins(home));
    }
    roots.extend(SYSTEM_BIN_DIRS.iter().map(PathBuf::from));
    roots
}

fn nvm_bins(home: &Path) -> Vec<PathBuf> {
    let Ok(versions) = std::fs::read_dir(home.join(NVM_VERSIONS)) else {
        return Vec::new();
    };
    versions
        .filter_map(|v| v.ok())
        .map(|v| v.path().join("bin"))
        .collect()
}

/// Fails with a plain message when `harness` cannot start on `host` because its
/// binary is missing. Only a local host is checked (a remote host is left to the
/// bounded wait in `ensure_started`), and only when the binary to look for is
/// known (see `plain_binary`); otherwise the launch goes ahead and the bounded
/// wait is the safety net.
pub fn require_binary(host: &dyn HostBackend, harness: &Harness) -> Result<()> {
    if !host.checks_binaries() {
        return Ok(());
    }
    let Some(wanted) = plain_binary(harness) else {
        tracing::debug!(harness = %harness.name, "launch preflight skipped: the command is shell syntax");
        return Ok(());
    };
    let roots = search_roots(
        harness,
        dirs::home_dir().as_deref(),
        std::env::var_os("PATH"),
    );
    require_found(&wanted, &roots, &harness.name, host.name())
}

fn require_found(wanted: &str, roots: &[PathBuf], harness: &str, host: &str) -> Result<()> {
    if binary_found(wanted, roots) {
        return Ok(());
    }
    let searched: Vec<String> = roots.iter().map(|r| r.display().to_string()).collect();
    bail!(
        "harness '{harness}' is not installed on host '{host}' (binary '{wanted}' not found on PATH; searched {})",
        searched.join(":")
    )
}

fn binary_found(wanted: &str, roots: &[PathBuf]) -> bool {
    if wanted.starts_with('/') {
        return which::which(wanted).is_ok();
    }
    let Ok(path) = std::env::join_paths(roots) else {
        return false;
    };
    which::which_in(wanted, Some(path), Path::new("/")).is_ok()
}

/// The agent Herdr reports for a launch, once it is past `launch_pending`.
/// When it never gets there within the host's launch bound the workspace is
/// closed and the call fails, so nothing is left running and nothing is
/// recorded. `began` is when the launch started: the time its own start wait
/// already used counts against the bound.
pub fn ensure_started(
    host: &dyn HostBackend,
    harness: &Harness,
    name: &str,
    launched: Launched,
    began: Instant,
) -> Result<Launched> {
    if launched.ready || launched.agent.as_ref().is_some_and(AgentInfo::is_started) {
        return Ok(launched);
    }
    let left = host.launch_bound().saturating_sub(began.elapsed());
    let observed = host.await_started(name, left);
    if let Ok(agent) = &observed
        && agent.as_ref().is_some_and(AgentInfo::is_started)
    {
        return Ok(Launched {
            agent: agent.clone(),
            ..launched
        });
    }
    let saw = match &observed {
        Ok(Some(agent)) => format!(
            "agent status {}, launch still pending",
            agent.status.as_str()
        ),
        Ok(None) => "no agent registered for the pane".to_string(),
        Err(e) => format!("the host stopped answering: {e}"),
    };
    let screen = host
        .read(&launched.pane_id, STUCK_SCREEN_LINES)
        .unwrap_or_default();
    let outcome = close_note(host, &launched.workspace_id);
    bail!(
        "harness '{}' did not start on host '{}' within {}s ({saw}). {outcome}{}",
        harness.name,
        host.name(),
        host.launch_bound().as_secs().max(1),
        quoted_screen(&screen)
    )
}

/// Closes a workspace a failed launch left behind and says what happened, for
/// the error the caller sees. No session was recorded either way.
pub fn close_note(host: &dyn HostBackend, workspace_id: &str) -> String {
    match host.close_workspace(workspace_id) {
        Ok(()) => "The workspace was closed and no session was recorded.".to_string(),
        Err(e) => format!(
            "Closing workspace {workspace_id} failed ({e}); close it by hand. No session was recorded."
        ),
    }
}

fn quoted_screen(screen: &str) -> String {
    let lines: Vec<&str> = screen
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    let tail = &lines[lines.len().saturating_sub(QUOTED_LINES)..];
    let text: String = tail
        .join(" | ")
        .chars()
        .filter(|c| !c.is_control())
        .collect();
    if text.is_empty() {
        return String::new();
    }
    let clipped: String = text.chars().take(QUOTED_CHARS).collect();
    format!(" Last screen lines: {clipped}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::herdr::scripted::ScriptedHost as HerdrHost;
    use hq_core::config::HerdrConfig;
    use std::os::unix::fs::PermissionsExt;

    fn with_command(command: &str, env: &str) -> Harness {
        let yaml = format!(
            "harness_profiles:\n  p:\n    base: claude-code\n    command: {command}\n    env:\n      {env}\n"
        );
        let cfg: HerdrConfig = serde_yaml::from_str(&yaml).unwrap();
        super::super::resolve_in(&cfg, "p").unwrap()
    }

    fn plain(command: &str) -> Option<String> {
        plain_binary(&with_command(
            &format!("'{}'", command.replace('\'', "''")),
            "X: y",
        ))
    }

    fn executable(dir: &Path, name: &str) -> PathBuf {
        std::fs::create_dir_all(dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[test]
    fn only_a_plain_program_or_absolute_path_is_checked() {
        assert_eq!(plain("claude").as_deref(), Some("claude"));
        assert_eq!(plain("claude --model x").as_deref(), Some("claude"));
        assert_eq!(plain("/opt/bin/claude").as_deref(), Some("/opt/bin/claude"));
        for skipped in [
            "CLAUDE_CONFIG_DIR=/x claude",
            "cd /x && claude",
            "cd /x; claude",
            "env A=b claude | tee",
            "\"/opt/my tools/claude\"",
            "'/opt/my tools/claude'",
            "$HOME/bin/claude",
            "~/bin/claude",
            "./claude",
            "bin/claude",
            "claude > /dev/null",
            "",
        ] {
            assert_eq!(
                plain(skipped),
                None,
                "{skipped:?} must be left to the shell"
            );
        }
    }

    #[test]
    fn the_built_in_binary_is_checked_when_the_profile_has_no_command() {
        let cfg = HerdrConfig::default();
        let h = super::super::resolve_in(&cfg, "agy").unwrap();
        assert_eq!(plain_binary(&h).as_deref(), Some("agy"));
    }

    #[test]
    fn shell_syntax_in_a_command_is_never_a_hard_failure() {
        let host = HerdrHost::new("/nonexistent/herdr");
        assert!(host.checks_binaries());
        for command in [
            "CLAUDE_CONFIG_DIR=/nowhere definitely-missing-xyz",
            "cd /nowhere && definitely-missing-xyz",
            "\"/no where/definitely-missing-xyz\"",
        ] {
            let h = with_command(&format!("'{}'", command.replace('\'', "''")), "X: y");
            assert!(require_binary(&host, &h).is_ok(), "{command}");
        }
        let missing = with_command("definitely-missing-xyz", "X: y");
        assert!(require_binary(&host, &missing).is_err());
    }

    #[test]
    fn a_profile_path_adds_to_hqs_path_and_literal_shell_entries_are_skipped() {
        let h = with_command("claude", "PATH: '$PATH:/profile/bin:~/mine'");
        let roots = search_roots(&h, None, Some(OsString::from("/hq/bin")));
        let roots: Vec<String> = roots.iter().map(|r| r.display().to_string()).collect();
        assert!(roots.contains(&"/profile/bin".to_string()), "{roots:?}");
        assert!(
            roots.contains(&"/hq/bin".to_string()),
            "HQ's PATH is kept: {roots:?}"
        );
        assert!(
            !roots.iter().any(|r| r.contains('$') || r.starts_with('~')),
            "{roots:?}"
        );
        assert!(roots.contains(&"/opt/homebrew/bin".to_string()));
    }

    #[test]
    fn a_binary_installed_by_a_version_manager_is_found() {
        let home = tempfile::tempdir().unwrap();
        executable(
            &home.path().join(".nvm/versions/node/v20.1.0/bin"),
            "claude",
        );
        executable(&home.path().join(".volta/bin"), "agy");
        let h = with_command("claude", "X: y");
        let roots = search_roots(&h, Some(home.path()), None);
        assert!(require_found("claude", &roots, "p", "local").is_ok(), "nvm");
        assert!(require_found("agy", &roots, "p", "local").is_ok(), "volta");
        let err = require_found("definitely-missing-xyz", &roots, "p", "local")
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("searched") && err.contains(".nvm/versions/node/v20.1.0/bin"),
            "{err}"
        );
    }

    #[test]
    fn an_absolute_path_is_checked_as_written() {
        let dir = tempfile::tempdir().unwrap();
        let tool = executable(dir.path(), "claude");
        assert!(require_found(&tool.to_string_lossy(), &[], "p", "local").is_ok());
        let missing = dir.path().join("absent");
        assert!(require_found(&missing.to_string_lossy(), &[], "p", "local").is_err());
    }
}
