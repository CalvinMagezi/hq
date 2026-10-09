//! Commands for running HQ Lite on a machine you do not fully control: `hq autostart`,
//! `hq uninstall lite`, `hq lite export|import`, and `hq doctor --windows`.
//!
//! Nothing here needs administrator rights. Autostart is one value under the current user's
//! `Run` key; uninstall removes that and stops the server, and leaves the program folder and your
//! notes for you to delete, because a running program cannot delete itself.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use hq_core::config::HqConfig;

const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_VALUE: &str = "HQLite";

/// The command line the `Run` value holds: start the web app in the background, no browser.
fn autostart_command(exe: &Path) -> String {
    format!("\"{}\" web --detach --no-open", exe.display())
}

/// `reg.exe` arguments to set the autostart value.
fn reg_add_args(exe: &Path) -> Vec<String> {
    ["add", RUN_KEY, "/v", RUN_VALUE, "/t", "REG_SZ", "/d", &autostart_command(exe), "/f"]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

/// A Windows system program by full path: a bare name is also looked up in the folder `hq.exe`
/// itself sits in, which a user can write to.
fn system_tool(rel: &str) -> PathBuf {
    let root = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    root.join("System32").join(rel)
}

fn reg(args: &[String]) -> Result<std::process::Output> {
    std::process::Command::new(system_tool("reg.exe"))
        .args(args)
        .output()
        .context("could not run reg.exe")
}

fn autostart_is_on() -> bool {
    reg(&["query".into(), RUN_KEY.into(), "/v".into(), RUN_VALUE.into()])
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// `hq autostart on|off|status`.
pub fn autostart(sub: &str) -> Result<()> {
    if !cfg!(windows) {
        bail!(
            "`hq autostart` sets a Windows per-user startup entry. On Linux and macOS use `hq service install` (systemd or launchd)."
        );
    }
    match sub {
        "status" | "" => {
            println!("autostart: {}", if autostart_is_on() { "on" } else { "off" });
            Ok(())
        }
        "on" => {
            let exe = std::env::current_exe().context("cannot find the hq program")?;
            if exe.to_string_lossy().contains('%') {
                bail!("the program path contains a %, which Windows would expand at sign-in; move HQ to a folder without one");
            }
            let out = reg(&reg_add_args(&exe))?;
            if !out.status.success() {
                bail!("could not set the startup entry: {}", String::from_utf8_lossy(&out.stderr).trim());
            }
            println!("HQ will start the web app in the background when you sign in.");
            println!("It is one value under {RUN_KEY} for your user only. Turn it off with `hq autostart off`.");
            println!("Security software sometimes flags startup entries; that is why this is opt-in.");
            Ok(())
        }
        "off" => {
            if autostart_is_on() {
                let out = reg(&["delete".into(), RUN_KEY.into(), "/v".into(), RUN_VALUE.into(), "/f".into()])?;
                if !out.status.success() {
                    bail!("could not remove the startup entry: {}", String::from_utf8_lossy(&out.stderr).trim());
                }
            }
            println!("autostart: off");
            Ok(())
        }
        other => bail!("unknown option `{other}`; use `hq autostart on|off|status`"),
    }
}

/// `hq uninstall lite`: stop the server and remove the startup entry.
pub fn uninstall_lite(config: &HqConfig) -> Result<()> {
    let state = HqConfig::hq_dir().join("web.json");
    if let Ok(text) = std::fs::read_to_string(&state)
        && let Some(pid) = serde_json::from_str::<serde_json::Value>(&text).ok().and_then(|v| v["pid"].as_u64())
    {
        // The pid file can outlive a crashed server, and Windows reuses pids: only stop it if it
        // is still one of ours.
        if super::web::is_our_server(pid as u32) {
            super::stop::kill_tree(pid as u32);
            println!("Stopped the background web server.");
        }
        let _ = std::fs::remove_file(&state);
    }
    if cfg!(windows) && autostart_is_on() {
        autostart("off")?;
    }
    let exe = std::env::current_exe().ok();
    println!("\nWhat is left is yours to delete, because a running program cannot remove itself:");
    if let Some(dir) = exe.as_deref().and_then(Path::parent) {
        println!("  the program folder:   {}", dir.display());
    }
    println!("  your notes and tasks: {} (and {})", config.vault_path.display(), HqConfig::hq_dir().display());
    println!("If you added the program folder to your PATH, remove it in Settings > System > About > Advanced system settings > Environment Variables.");
    println!("Take your notes with you first if you want them: `hq lite export <folder>`.");
    Ok(())
}

/// Whether `p` sits in a folder OneDrive syncs. A SQLite database in WAL mode must not.
pub fn in_synced_folder(p: &Path) -> bool {
    p.components().any(|c| {
        let s = c.as_os_str().to_string_lossy().to_ascii_lowercase();
        s == "onedrive" || s.starts_with("onedrive - ") || s == "dropbox" || s == "google drive"
    })
}

/// The path with symlinks, junctions, `..` and spelling differences resolved, even when the
/// last parts do not exist yet.
fn resolve(p: &Path) -> PathBuf {
    let abs = if p.is_absolute() { p.to_path_buf() } else { std::env::current_dir().unwrap_or_default().join(p) };
    let mut existing = abs.as_path();
    let mut tail = Vec::new();
    while !existing.exists() {
        match (existing.file_name(), existing.parent()) {
            (Some(n), Some(parent)) => {
                tail.push(n.to_os_string());
                existing = parent;
            }
            _ => break,
        }
    }
    let mut out = std::fs::canonicalize(existing).unwrap_or_else(|_| existing.to_path_buf());
    out.extend(tail.iter().rev());
    out
}

/// Copy every note outside HQ's own folders (`_system`, `_data`, dot folders at the top) from
/// `src` to `dst`: never following links (in either folder), never overwriting, never copying
/// into `dst` itself. Returns (copied, skipped existing).
fn copy_visible(src: &Path, dst: &Path) -> Result<(usize, usize)> {
    use std::io::Write;
    fn is_link(p: &Path) -> bool {
        std::fs::symlink_metadata(p).is_ok_and(|m| m.file_type().is_symlink())
    }
    fn walk(from: &Path, to: &Path, top: bool, skip: &Path, counts: &mut (usize, usize)) -> Result<()> {
        if is_link(to) {
            bail!("{} is a link; refusing to write through it", to.display());
        }
        std::fs::create_dir_all(to)?;
        for entry in std::fs::read_dir(from)? {
            let entry = entry?;
            let name = entry.file_name();
            let lossy = name.to_string_lossy();
            if top && (lossy.starts_with('_') || lossy.starts_with('.')) {
                continue;
            }
            let ft = entry.file_type()?;
            if ft.is_symlink() || resolve(&entry.path()) == skip {
                continue;
            }
            let target = to.join(&name);
            if ft.is_dir() {
                if lossy.starts_with('.') {
                    continue;
                }
                walk(&entry.path(), &target, false, skip, counts)?;
            } else if ft.is_file() {
                if std::fs::symlink_metadata(&target).is_ok() {
                    counts.1 += 1;
                    continue;
                }
                let mut out = match std::fs::OpenOptions::new().write(true).create_new(true).open(&target) {
                    Ok(f) => f,
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                        counts.1 += 1;
                        continue;
                    }
                    Err(e) => return Err(e.into()),
                };
                out.write_all(&std::fs::read(entry.path())?)?;
                counts.0 += 1;
            }
        }
        Ok(())
    }
    let mut counts = (0, 0);
    let skip = resolve(dst);
    walk(src, dst, true, &skip, &mut counts)?;
    Ok(counts)
}

/// `hq lite export <folder>` and `hq lite import <folder>`: the vault is plain markdown, so moving
/// between Lite and Full HQ is a folder copy. Only notes move; HQ's own folders stay behind.
pub fn lite(config: &HqConfig, sub: &str, folder: Option<&Path>) -> Result<()> {
    let folder = folder.ok_or_else(|| anyhow::anyhow!("usage: hq lite export <folder> | hq lite import <folder>"))?;
    let vault = &config.vault_path;
    match sub {
        "export" => {
            let (v, f) = (resolve(vault), resolve(folder));
            if f.starts_with(&v) || v.starts_with(&f) {
                bail!("choose a folder that is neither inside the vault nor contains it");
            }
            if folder.exists() && std::fs::read_dir(folder)?.next().is_some() {
                bail!("{} is not empty; choose a new or empty folder", folder.display());
            }
            let (copied, _) = copy_visible(vault, folder)?;
            println!("Exported {copied} file(s) to {}", folder.display());
            println!("To use them in Full HQ, copy that folder's contents into its vault (or run `hq lite import` there).");
            Ok(())
        }
        "import" => {
            if !folder.is_dir() {
                bail!("{} is not a folder", folder.display());
            }
            let (v, f) = (resolve(vault), resolve(folder));
            if f.starts_with(&v) || v.starts_with(&f) {
                bail!("choose a folder that is neither inside the vault nor contains it");
            }
            let (copied, skipped) = copy_visible(folder, vault)?;
            println!("Imported {copied} file(s) into {}; {skipped} already existed and were left as they were.", vault.display());
            println!("Run `hq reindex` so search finds them.");
            Ok(())
        }
        other => bail!("unknown option `{other}`; use `hq lite export <folder>` or `hq lite import <folder>`"),
    }
}

fn on_path(program: &str) -> Option<PathBuf> {
    let names: Vec<String> = if cfg!(windows) {
        vec![format!("{program}.exe"), format!("{program}.cmd")]
    } else {
        vec![program.to_string()]
    };
    std::env::split_paths(&std::env::var_os("PATH")?)
        .flat_map(|d| names.iter().map(move |n| d.join(n)))
        .find(|p| p.is_file())
}

/// `hq doctor --windows`: read-only checks for a managed Windows computer. It changes nothing.
pub fn doctor_windows(config: &HqConfig) -> Result<()> {
    println!("HQ on this computer (read-only; nothing is changed)\n");
    let line = |ok: Option<bool>, what: &str, detail: String| {
        let mark = match ok {
            Some(true) => "ok  ",
            Some(false) => "WARN",
            None => "info",
        };
        println!("  [{mark}] {what}: {detail}");
    };
    line(None, "profile", (if config.profile.is_lite() { "lite" } else { "full" }).to_string());
    line(None, "program", std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_else(|_| "unknown".into()));
    line(None, "config file", HqConfig::config_read_path().display().to_string());
    let vault = &config.vault_path;
    line(Some(!in_synced_folder(vault)), "notes folder", format!(
        "{}{}", vault.display(),
        if in_synced_folder(vault) { " (inside a synced folder: a SQLite database there can be corrupted by sync; set vault_path to a local folder)" } else { "" }
    ));
    let db = config.db_path();
    line(Some(!in_synced_folder(&db)), "database", db.display().to_string());
    line(Some(db.as_os_str().len() < 200), "path length", format!("database path is {} characters (Windows limits paths near 260 unless long paths are enabled)", db.as_os_str().len()));
    for (tool, why) in [("gh", "needed for `hq copilot link`"), ("code", "VS Code command line"), ("git", "for projects")] {
        match on_path(tool) {
            Some(p) => line(Some(true), tool, format!("{} ({why})", p.display())),
            None => line(None, tool, format!("not on PATH ({why})")),
        }
    }
    let proxy = ["HTTPS_PROXY", "https_proxy", "HTTP_PROXY"].iter().find_map(|k| std::env::var(k).ok());
    line(None, "proxy", proxy.unwrap_or_else(|| "none set in the environment (the Windows system proxy is used by the installer)".into()));
    let port = config.ws_port;
    let free = std::net::TcpListener::bind(("127.0.0.1", port)).is_ok();
    line(None, &format!("port {port}"), if free { "free".into() } else { "in use (an HQ may already be running, or another program)".into() });
    if cfg!(windows) {
        line(None, "autostart", if autostart_is_on() { "on".into() } else { "off".into() });
    }
    println!("\nWhether AppLocker, WDAC or Smart App Control allow programs from your profile is decided when a program starts; the installer tests that once. See docs/CORPORATE_WORKSTATION.md.");
    println!("Run `hq doctor --egress` to see what, if anything, could leave this computer.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_startup_value_quotes_the_program_and_starts_quietly() {
        let cmd = autostart_command(Path::new(r"D:\apps\Some One\hq-lite\hq.exe"));
        assert_eq!(cmd, "\"D:\\apps\\Some One\\hq-lite\\hq.exe\" web --detach --no-open");
        let args = reg_add_args(Path::new("hq.exe"));
        assert_eq!(&args[..4], ["add", RUN_KEY, "/v", RUN_VALUE]);
        assert!(args.contains(&"/f".to_string()));
    }

    #[test]
    fn synced_folders_are_recognised() {
        assert!(in_synced_folder(Path::new("D:/data/OneDrive/Documents/vault")));
        assert!(in_synced_folder(Path::new("D:/data/OneDrive - Example/vault")));
        assert!(in_synced_folder(Path::new("D:/data/Dropbox/vault")));
        assert!(!in_synced_folder(Path::new("D:/data/.hq/vault")));
    }

    #[test]
    fn export_and_import_move_notes_but_not_hq_folders_and_never_overwrite() {
        let src = tempfile::tempdir().unwrap();
        let s = src.path();
        std::fs::create_dir_all(s.join("Notebooks/sub")).unwrap();
        std::fs::create_dir_all(s.join("_system")).unwrap();
        std::fs::create_dir_all(s.join(".git")).unwrap();
        std::fs::write(s.join("Notebooks/a.md"), "a").unwrap();
        std::fs::write(s.join("Notebooks/sub/b.md"), "b").unwrap();
        std::fs::write(s.join("_system/SOUL.md"), "private").unwrap();
        std::fs::write(s.join(".git/config"), "x").unwrap();
        let dst = tempfile::tempdir().unwrap();
        let out = dst.path().join("out");
        assert_eq!(copy_visible(s, &out).unwrap(), (2, 0));
        assert!(out.join("Notebooks/sub/b.md").exists());
        assert!(!out.join("_system").exists() && !out.join(".git").exists());
        // Importing again changes nothing and says so.
        std::fs::write(out.join("Notebooks/a.md"), "edited").unwrap();
        let back = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(back.path().join("Notebooks")).unwrap();
        std::fs::write(back.path().join("Notebooks/a.md"), "mine").unwrap();
        assert_eq!(copy_visible(&out, back.path()).unwrap(), (1, 1));
        // Copying a folder into one of its own subfolders does not recurse into itself.
        let inner = s.join("Notebooks/copy");
        assert!(copy_visible(s, &inner).is_ok());
        assert!(!inner.join("Notebooks/copy").exists());
        assert_eq!(std::fs::read_to_string(back.path().join("Notebooks/a.md")).unwrap(), "mine");
    }

    #[cfg(unix)]
    #[test]
    fn links_are_not_followed() {
        let src = tempfile::tempdir().unwrap();
        let secret = tempfile::tempdir().unwrap();
        std::fs::write(secret.path().join("s.md"), "secret").unwrap();
        std::os::unix::fs::symlink(secret.path(), src.path().join("link")).unwrap();
        let dst = tempfile::tempdir().unwrap();
        assert_eq!(copy_visible(src.path(), &dst.path().join("o")).unwrap(), (0, 0));
    }
}
