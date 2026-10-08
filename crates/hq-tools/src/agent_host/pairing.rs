//! Pairing a machine with this HQ as a built-in host. The machine makes a join
//! code (who it is and where it is reachable, nothing secret); this HQ turns it
//! into a key pair and a config entry and answers with the one command the
//! machine runs to trust that key. No service listens for pairing: the code and
//! the command travel by whatever a person or an agent uses to talk to both.

use anyhow::{Context, Result, bail};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hq_core::config::{HqConfig, LOCAL_HOST, NATIVE_HOST};
use serde::{Deserialize, Serialize};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

const JOIN_PREFIX: &str = "hqjoin1.";
const MAX_JOIN_BYTES: usize = 1024;
const MAX_NAME_LEN: usize = 32;
const MAX_ADDR_LEN: usize = 255;
const MAX_USER_LEN: usize = 64;
const DIR_MODE: u32 = 0o700;
/// The config section that lists hosts, and the name configs used before it was renamed.
const SECTION: &str = "agent_host:";
const LEGACY_SECTION: &str = "herdr:";

/// Who a machine is and how to reach it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Join {
    /// What HQ calls the host in `host:` arguments and config.
    pub name: String,
    pub user: String,
    /// Tailnet address or name HQ's ssh connects to.
    pub addr: String,
    pub os: String,
}

pub fn valid_host_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '-'))
        && name.len() <= MAX_NAME_LEN
        && name != LOCAL_HOST
        && name != NATIVE_HOST
}

fn valid_user(user: &str) -> bool {
    !user.is_empty()
        && user.len() <= MAX_USER_LEN
        && !user.starts_with('-')
        && user.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// An address or name ssh can use, and one that cannot be read as an option or
/// carry a second `user@`.
fn valid_addr(addr: &str) -> bool {
    !addr.is_empty()
        && addr.len() <= MAX_ADDR_LEN
        && !addr.starts_with('-')
        && addr.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':'))
}

pub fn encode_join(join: &Join) -> String {
    let json = serde_json::to_vec(join).unwrap_or_default();
    format!("{JOIN_PREFIX}{}", URL_SAFE_NO_PAD.encode(json))
}

pub fn decode_join(code: &str) -> Result<Join> {
    let code = code.trim();
    if code.len() > MAX_JOIN_BYTES {
        bail!("that join code is too long");
    }
    let body = code.strip_prefix(JOIN_PREFIX).context("not a join code (it starts with hqjoin1.)")?;
    let bytes = URL_SAFE_NO_PAD.decode(body).context("the join code is damaged")?;
    let join: Join = serde_json::from_slice(&bytes).context("the join code is damaged")?;
    if !valid_host_name(&join.name) {
        bail!("host name {:?} is not allowed: use a lowercase name (letters, digits, - or _), not local or native", join.name);
    }
    if !valid_user(&join.user) {
        bail!("user name {:?} is not allowed", join.user);
    }
    if !valid_addr(&join.addr) {
        bail!("address {:?} is not allowed", join.addr);
    }
    Ok(join)
}

/// This machine's tailnet IPv4 address, when tailscale is installed and up.
pub fn tailscale_ip() -> Option<String> {
    const CANDIDATES: &[&str] = &["tailscale", "/Applications/Tailscale.app/Contents/MacOS/Tailscale"];
    CANDIDATES.iter().find_map(|bin| {
        let out = Command::new(bin).args(["ip", "-4"]).output().ok()?;
        let text = String::from_utf8(out.stdout).ok()?;
        let ip = text.lines().next()?.trim().to_string();
        (out.status.success() && valid_addr(&ip)).then_some(ip)
    })
}

/// What `add_host` did and what the machine must do next.
#[derive(Debug, Clone, Serialize)]
pub struct Added {
    pub name: String,
    pub ssh: String,
    pub identity_file: PathBuf,
    pub public_key: String,
    /// Run this on the machine to let this HQ in.
    pub authorize_command: String,
    /// False when the config already described this host.
    pub config_changed: bool,
}

fn ensure_key(path: &Path, name: &str) -> Result<String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(DIR_MODE))?;
    }
    if !path.exists() {
        let out = Command::new("ssh-keygen")
            .args(["-q", "-t", "ed25519", "-N", "", "-C", &format!("hq-gate-{name}"), "-f"])
            .arg(path)
            .output()
            .context("running ssh-keygen (is OpenSSH installed?)")?;
        if !out.status.success() {
            bail!("ssh-keygen failed: {}", String::from_utf8_lossy(&out.stderr).trim());
        }
    }
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    let public = std::fs::read_to_string(path.with_extension("pub"))
        .or_else(|_| std::fs::read_to_string(format!("{}.pub", path.display())))
        .context("reading the public key")?;
    Ok(public.trim().to_string())
}

/// Records the machine as a built-in host in the config at `config_path`, with a
/// key pair under `ssh_dir`, and returns the command for the machine to run.
pub fn add_host_at(config_path: &Path, join_code: &str, gateway_addr: &str, ssh_dir: &Path) -> Result<Added> {
    let join = decode_join(join_code)?;
    if !valid_addr(gateway_addr) {
        bail!("this machine's address {gateway_addr:?} is not usable; pass the tailnet address the host should accept connections from");
    }
    let identity = ssh_dir.join(format!("hq_gate_{}", join.name));
    let public_key = ensure_key(&identity, &join.name)?;
    let ssh = format!("{}@{}", join.user, join.addr);
    let existing = std::fs::read_to_string(config_path).unwrap_or_default();
    let identity_text = identity.display().to_string();
    let changed = match insert_host(&existing, &join.name, &ssh, &identity_text)? {
        Some(updated) => {
            write_config(config_path, &existing, &updated)?;
            true
        }
        None => false,
    };
    Ok(Added {
        authorize_command: format!("hq host authorize --key '{public_key}' --from {gateway_addr}"),
        name: join.name,
        ssh,
        identity_file: identity,
        public_key,
        config_changed: changed,
    })
}

/// `add_host_at` for the config and ssh directory this process uses.
pub fn add_host(join_code: &str, gateway_addr: Option<&str>) -> Result<Added> {
    let gateway = match gateway_addr {
        Some(addr) => addr.to_string(),
        None => tailscale_ip().context("could not find this machine's tailnet address; pass it explicitly")?,
    };
    let home = std::env::var_os("HOME").map(PathBuf::from).context("HOME is not set")?;
    add_host_at(&HqConfig::config_file_path(), join_code, &gateway, &home.join(".ssh"))
}

/// Asks the host `name` for its status and turns a failure into the likely cause.
pub fn check_host(name: &str) -> serde_json::Value {
    let host = match super::host(Some(name)) {
        Ok(host) => host,
        Err(e) => {
            return serde_json::json!({ "host": name, "reachable": false, "error": e.to_string(),
                "hint": "no such host: add it with host_add and a join code from `hq host join`" });
        }
    };
    match host.version() {
        Ok(version) => serde_json::json!({ "host": name, "reachable": true, "host_version": version }),
        Err(e) => serde_json::json!({
            "host": name,
            "reachable": false,
            "error": e.to_string(),
            "hint": "check that `hq host install` ran on the machine, that its `hq host authorize` command was run, that the machine is on the same tailnet, and that its sshd is running",
        }),
    }
}

/// The leading spaces of `line`.
fn indent_of(line: &str) -> usize {
    line.len() - line.trim_start_matches(' ').len()
}

fn is_blank_or_comment(line: &str) -> bool {
    let t = line.trim();
    t.is_empty() || t.starts_with('#')
}

/// `text` with host `name` added under `agent_host.hosts`, everything else left as
/// written (comments, order, unrelated keys, defaults not pinned). None when the
/// config already holds exactly this entry. An entry for the same name with a
/// different address or key is replaced.
pub(crate) fn insert_host(text: &str, name: &str, ssh: &str, identity: &str) -> Result<Option<String>> {
    let lines: Vec<&str> = text.lines().collect();
    let entry = |child: usize| -> Vec<String> {
        let pad = " ".repeat(child);
        let inner = " ".repeat(child + 2);
        vec![format!("{pad}{name}:"), format!("{inner}ssh: \"{ssh}\""), format!("{inner}identity_file: \"{identity}\"")]
    };
    // A config written before the section was renamed gets its heading migrated.
    let top = lines
        .iter()
        .position(|l| l.starts_with(SECTION) || l.starts_with(LEGACY_SECTION));
    let mut out: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
    if let Some(h) = top
        && lines[h].starts_with(LEGACY_SECTION)
    {
        out[h] = lines[h].replacen(LEGACY_SECTION, SECTION, 1);
    }
    match top {
        None => {
            if !out.is_empty() && !out.last().is_some_and(|l| l.is_empty()) {
                out.push(String::new());
            }
            out.push(SECTION.into());
            out.push("  hosts:".into());
            out.extend(entry(4));
        }
        Some(h) => {
            let rest = out[h].trim_start_matches(SECTION).trim();
            if rest.chars().next().is_some_and(|c| c != '#') {
                bail!("`{SECTION}` in the config is written inline; add the host by hand");
            }
            let end = (h + 1..lines.len()).find(|&i| !lines[i].trim().is_empty() && indent_of(lines[i]) == 0).unwrap_or(lines.len());
            let hosts = (h + 1..end).find(|&i| lines[i].trim_start().starts_with("hosts:") && !lines[i].trim_start().starts_with('#'));
            match hosts {
                None => {
                    let mut block = vec!["  hosts:".to_string()];
                    block.extend(entry(4));
                    out.splice(h + 1..h + 1, block);
                }
                Some(hi) => {
                    let value = lines[hi].trim_start().trim_start_matches("hosts:").trim();
                    if value == "{}" {
                        out[hi] = format!("{}hosts:", " ".repeat(indent_of(lines[hi])));
                    } else if !value.is_empty() && !value.starts_with('#') {
                        bail!("`hosts:` in the config is written inline; add the host by hand");
                    }
                    let base = indent_of(lines[hi]);
                    let block_end = (hi + 1..end).find(|&i| !is_blank_or_comment(lines[i]) && indent_of(lines[i]) <= base).unwrap_or(end);
                    let child = (hi + 1..block_end)
                        .find(|&i| !is_blank_or_comment(lines[i]))
                        .map_or(base + 2, |i| indent_of(lines[i]));
                    let own = (hi + 1..block_end).find(|&i| indent_of(lines[i]) == child && lines[i].trim() == format!("{name}:"));
                    if let Some(oi) = own {
                        let own_end = (oi + 1..block_end).find(|&i| !is_blank_or_comment(lines[i]) && indent_of(lines[i]) <= child).unwrap_or(block_end);
                        let body = lines[oi + 1..own_end].join("\n");
                        if body.contains(&format!("ssh: \"{ssh}\"")) && body.contains(&format!("identity_file: \"{identity}\"")) {
                            return Ok(None);
                        }
                        out.splice(oi..own_end, entry(child));
                    } else {
                        let last = (hi + 1..block_end).rev().find(|&i| !lines[i].trim().is_empty()).unwrap_or(hi);
                        out.splice(last + 1..last + 1, entry(child));
                    }
                }
            }
        }
    }
    let mut updated = out.join("\n");
    updated.push('\n');
    let parsed: serde_yaml::Value = serde_yaml::from_str(&updated).context("the edited config is not valid YAML")?;
    let got = parsed["agent_host"]["hosts"][name]["ssh"].as_str();
    if got != Some(ssh) {
        bail!("could not add the host to the config cleanly; add it by hand");
    }
    Ok(Some(updated))
}

/// Keeps a copy of the config as it was, then replaces it, keeping it private.
fn write_config(path: &Path, before: &str, after: &str) -> Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    if !before.is_empty() {
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let backup = path.with_file_name(format!("{}.bak-{stamp}", path.file_name().map_or("config".into(), |n| n.to_string_lossy())));
        std::fs::write(&backup, before)?;
        std::fs::set_permissions(&backup, std::fs::Permissions::from_mode(0o600))?;
    }
    let tmp = path.with_extension("hq-tmp");
    let _ = std::fs::remove_file(&tmp);
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&tmp)?;
    file.write_all(after.as_bytes())?;
    file.sync_all()?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests;
