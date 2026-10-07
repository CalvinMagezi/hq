//! Pairing a machine with this HQ as a built-in host. The machine makes a join
//! code (who it is and where it is reachable, nothing secret); this HQ turns it
//! into a key pair and a config entry and answers with the one command the
//! machine runs to trust that key. No service listens for pairing: the code and
//! the command travel by whatever a person or an agent uses to talk to both.

use anyhow::{Context, Result, bail};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hq_core::config::{HerdrHostConfig, HqConfig, HostKind, LOCAL_HOST, NATIVE_HOST};
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
    let entry: HerdrHostConfig = serde_yaml::from_str(&format!(
        "kind: native\nssh: \"{ssh}\"\nidentity_file: \"{}\"\n",
        identity.display()
    ))?;
    let mut changed = false;
    HqConfig::save_patch_to_path(config_path, |c| {
        let same = c.herdr.hosts.get(&join.name).is_some_and(|h| {
            h.kind == HostKind::Native && h.ssh == entry.ssh && h.identity_file == entry.identity_file
        });
        if !same {
            c.herdr.hosts.insert(join.name.clone(), entry.clone());
            changed = true;
        }
    })?;
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

#[cfg(test)]
mod tests;
