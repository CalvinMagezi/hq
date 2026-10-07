//! `hq host install` and `hq host authorize`: set the built-in host up as a
//! login-session service, and pin a remote machine's key to `hq host gate`.

use anyhow::{Context, Result, bail};
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;

const LAUNCHD_LABEL: &str = "com.hq.host";
const SYSTEMD_UNIT: &str = "hq-host.service";
const DIR_MODE: u32 = 0o700;
const FILE_MODE: u32 = 0o600;
/// Longest public key line accepted.
const MAX_KEY_BYTES: usize = 2048;
/// Folders macOS keeps launchd jobs from reading.
const PROTECTED_DIRS: &[&str] = &["Documents", "Desktop", "Downloads"];
const KEY_TYPES: &[&str] = &["ssh-ed25519", "ssh-rsa", "ecdsa-sha2-nistp256", "ecdsa-sha2-nistp384", "ecdsa-sha2-nistp521"];

fn home() -> Result<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|h| h.is_absolute())
        .context("HOME is not set")
}

/// The binary the service will run, refused where launchd cannot read it.
fn service_binary(home: &Path) -> Result<PathBuf> {
    let exe = std::env::current_exe().context("finding this binary")?;
    let exe = exe.canonicalize().unwrap_or(exe);
    if cfg!(target_os = "macos") {
        for dir in PROTECTED_DIRS {
            if exe.starts_with(home.join(dir)) {
                bail!(
                    "{} is under ~/{dir}, which launchd jobs may not read; copy hq to ~/.local/bin and run `hq host install` from there",
                    exe.display()
                );
            }
        }
    }
    Ok(exe)
}

fn path_env(home: &Path) -> String {
    format!(
        "{}/.local/bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin",
        home.display()
    )
}

fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

pub(crate) fn launchd_plist(exe: &Path, home: &Path) -> String {
    let log = home.join("Library/Logs/hq-host.log");
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>Label</key><string>{LAUNCHD_LABEL}</string>
  <key>ProgramArguments</key><array>
    <string>{exe}</string><string>host</string><string>serve</string>
  </array>
  <key>EnvironmentVariables</key><dict>
    <key>PATH</key><string>{path}</string>
  </dict>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>StandardOutPath</key><string>{log}</string>
  <key>StandardErrorPath</key><string>{log}</string>
</dict></plist>
"#,
        exe = xml_escape(&exe.to_string_lossy()),
        path = xml_escape(&path_env(home)),
        log = xml_escape(&log.to_string_lossy()),
    )
}

pub(crate) fn systemd_unit(exe: &Path, home: &Path) -> String {
    format!(
        "[Unit]\nDescription=Agent HQ built-in host\n\n[Service]\nExecStart={exe} host serve\nEnvironment=PATH={path}\nRestart=on-failure\n\n[Install]\nWantedBy=default.target\n",
        exe = exe.display(),
        path = path_env(home),
    )
}

fn write_private(path: &Path, text: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let _ = std::fs::remove_file(path);
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(FILE_MODE)
        .open(path)?;
    file.write_all(text.as_bytes())?;
    Ok(())
}

fn run(program: &str, args: &[&str]) -> Result<()> {
    let out = Command::new(program).args(args).output().with_context(|| format!("running {program}"))?;
    if !out.status.success() {
        bail!("{program} {} failed: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(())
}

/// Replaces a loaded LaunchAgent with the one at `plist`. launchd finishes
/// unloading after `bootout` returns, and a `bootstrap` in that window fails
/// with an I/O error, so wait for the old job to go and retry the load.
fn reload_launch_agent(domain: &str, plist: &Path) -> Result<()> {
    const POLL: std::time::Duration = std::time::Duration::from_millis(200);
    const UNLOAD_WAIT: usize = 25;
    const LOAD_TRIES: usize = 5;
    let target = format!("{domain}/{LAUNCHD_LABEL}");
    let loaded = || Command::new("launchctl").args(["print", &target]).output().is_ok_and(|o| o.status.success());
    if loaded() {
        let _ = run("launchctl", &["bootout", &target]);
        for _ in 0..UNLOAD_WAIT {
            if !loaded() {
                break;
            }
            std::thread::sleep(POLL);
        }
    }
    let mut last = None;
    for _ in 0..LOAD_TRIES {
        match run("launchctl", &["bootstrap", domain, &plist.to_string_lossy()]) {
            Ok(()) => return Ok(()),
            Err(e) => last = Some(e),
        }
        std::thread::sleep(POLL * 5);
    }
    Err(last.unwrap_or_else(|| anyhow::anyhow!("launchctl bootstrap failed")))
}

pub fn install() -> Result<()> {
    let home = home()?;
    let exe = service_binary(&home)?;
    if cfg!(target_os = "macos") {
        let path = home.join("Library/LaunchAgents").join(format!("{LAUNCHD_LABEL}.plist"));
        write_private(&path, &launchd_plist(&exe, &home))?;
        // SAFETY of the id: `id -u` is the current user's.
        let uid = String::from_utf8(Command::new("id").arg("-u").output()?.stdout)?.trim().to_string();
        let domain = format!("gui/{uid}");
        reload_launch_agent(&domain, &path)?;
        println!("hq host installed as a LaunchAgent ({}); logs in ~/Library/Logs/hq-host.log", path.display());
    } else {
        let path = home.join(".config/systemd/user").join(SYSTEMD_UNIT);
        write_private(&path, &systemd_unit(&exe, &home))?;
        run("systemctl", &["--user", "daemon-reload"])?;
        run("systemctl", &["--user", "enable", "--now", SYSTEMD_UNIT])?;
        println!("hq host installed as a user service ({}); `loginctl enable-linger` keeps it running after logout", path.display());
    }
    Ok(())
}

fn valid_key(key: &str) -> Result<&str> {
    let key = key.trim();
    let mut parts = key.split_whitespace();
    let kind = parts.next().unwrap_or("");
    let blob = parts.next().unwrap_or("");
    if key.len() > MAX_KEY_BYTES
        || key.contains(['\n', '\r', '"'])
        || !KEY_TYPES.contains(&kind)
        || blob.is_empty()
        || !blob.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '='))
    {
        bail!("expected one ssh public key line such as `ssh-ed25519 AAAA... comment`");
    }
    Ok(key)
}

fn valid_from(from: &str) -> Result<&str> {
    let ok = !from.is_empty()
        && from.len() <= 255
        && from.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | ':' | '*' | ',' | '-' | '/' | '_'));
    if !ok {
        bail!("--from must be an address or pattern for ssh's `from=` option, for example a tailnet IP");
    }
    Ok(from)
}

/// The `authorized_keys` line that lets `key` call the host and nothing else.
pub(crate) fn authorized_line(exe: &Path, key: &str, from: &str) -> Result<String> {
    let key = valid_key(key)?;
    let from = valid_from(from)?;
    let exe = exe.to_string_lossy();
    if exe.contains(['"', '\n']) {
        bail!("the binary path cannot be used in an authorized_keys command");
    }
    Ok(format!("restrict,from=\"{from}\",command=\"{exe} host gate\" {key}"))
}

/// Adds `line` to `file` unless that key is already there. True when added.
pub(crate) fn append_authorized(file: &Path, line: &str) -> Result<bool> {
    let blob = line.split_whitespace().rev().nth(1).unwrap_or("");
    let existing = std::fs::read_to_string(file).unwrap_or_default();
    if !blob.is_empty() && existing.lines().any(|l| l.split_whitespace().any(|w| w == blob)) {
        return Ok(false);
    }
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)?;
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(DIR_MODE))?;
    }
    let mut out = std::fs::OpenOptions::new().append(true).create(true).mode(FILE_MODE).open(file)?;
    if !existing.is_empty() && !existing.ends_with('\n') {
        out.write_all(b"\n")?;
    }
    out.write_all(format!("{line}\n").as_bytes())?;
    Ok(true)
}

pub fn authorize(key: Option<&str>, from: Option<&str>) -> Result<()> {
    let (Some(key), Some(from)) = (key, from) else {
        bail!("usage: hq host authorize --key '<ssh public key>' --from <address of the machine that will connect>");
    };
    let home = home()?;
    let exe = std::env::current_exe().context("finding this binary")?;
    let line = authorized_line(&exe.canonicalize().unwrap_or(exe), key, from)?;
    let file = home.join(".ssh/authorized_keys");
    if append_authorized(&file, &line)? {
        println!("key added to {}: it can call the host through `hq host gate`, from {from} only", file.display());
    } else {
        println!("that key is already in {}", file.display());
    }
    println!("The key can start any command as you; treat it like an ssh login.");
    Ok(())
}

/// A host name for this machine: its hostname as HQ's name rules allow it.
pub(crate) fn default_host_name(hostname: &str) -> String {
    let mut name: String = hostname
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '-' })
        .collect();
    name = name.trim_matches('-').to_string();
    if !name.chars().next().is_some_and(|c| c.is_ascii_lowercase()) {
        name.insert_str(0, "host-");
    }
    name.truncate(32);
    if hq_tools::agent_host::pairing::valid_host_name(&name) {
        name
    } else {
        "machine".to_string()
    }
}

/// Runs on the machine that will host agents: installs the service and prints
/// the join code to give to the HQ.
pub fn join(name: Option<&str>, addr: Option<&str>) -> Result<()> {
    use hq_tools::agent_host::pairing::{Join, encode_join, valid_host_name};
    let name = match name {
        Some(n) if valid_host_name(n) => n.to_string(),
        Some(n) => bail!("{n:?} is not a usable host name: lowercase letters, digits, - and _, starting with a letter"),
        None => {
            let out = Command::new("hostname").output().context("running hostname")?;
            default_host_name(String::from_utf8_lossy(&out.stdout).trim())
        }
    };
    let addr = match addr {
        Some(a) => a.to_string(),
        None => hq_tools::agent_host::pairing::tailscale_ip()
            .context("could not ask tailscale for this machine's address; is it installed and up? Pass --addr <tailnet address> otherwise")?,
    };
    let user = std::env::var("USER").or_else(|_| std::env::var("LOGNAME")).context("USER is not set")?;
    install()?;
    let code = encode_join(&Join { name: name.clone(), user, addr, os: std::env::consts::OS.to_string() });
    println!("\nThis machine is ready to host agents as '{name}'. Give your HQ this join code:\n\n  {code}\n");
    println!("On the HQ run `hq host add {code}` (or have its agent call the host_add tool with the code).");
    println!("It answers with one command, `hq host authorize ...`; run that here, then `hq host check {name}` on the HQ.");
    Ok(())
}

/// Runs on the HQ side: pairs the machine the join code names.
pub fn add(code: Option<&str>, gateway: Option<&str>) -> Result<()> {
    let code = code.context("usage: hq host add <join code from `hq host join`>")?;
    let added = hq_tools::agent_host::pairing::add_host(code, gateway)?;
    println!("Host '{}' ({}) {} in the config.", added.name, added.ssh, if added.config_changed { "added" } else { "was already" });
    println!("\nRun this on that machine, so it accepts this HQ:\n\n  {}\n", added.authorize_command);
    println!("Then check it with `hq host check {}`.", added.name);
    Ok(())
}

/// Runs on the HQ side: says whether a host answers.
pub fn check(name: Option<&str>) -> Result<()> {
    let name = name.context("usage: hq host check <host name>")?;
    let result = hq_tools::agent_host::pairing::check_host(name);
    println!("{}", serde_json::to_string_pretty(&result)?);
    if result["reachable"] != true {
        std::process::exit(1);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
