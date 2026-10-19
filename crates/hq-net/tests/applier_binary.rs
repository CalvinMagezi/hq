#![cfg(target_os = "linux")]
//! The built `hq-net-apply` program, run as a child process: its exit codes and what it prints.
//! It is never given a real `wg` or root, so these cover argument handling and refusal paths.

use std::process::{Command, Stdio};

fn run(args: &[&str]) -> (Option<i32>, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_hq-net-apply"))
        .args(args)
        .env_clear()
        .stdin(Stdio::null())
        .output()
        .unwrap();
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn no_arguments_is_a_usage_error() {
    let (code, stderr) = run(&[]);
    assert_eq!(code, Some(2));
    assert!(stderr.contains("usage:") && stderr.contains("--interface"));
}

#[test]
fn unknown_or_repeated_arguments_are_usage_errors() {
    assert_eq!(run(&["--force"]).0, Some(2));
    assert_eq!(
        run(&["--interface", "hq0", "--interface", "hq1"]).0,
        Some(2)
    );
    assert_eq!(run(&["--interface=hq0"]).0, Some(2));
}

#[test]
fn a_public_subnet_is_refused_before_anything_else_happens() {
    let args = [
        "--interface",
        "hq0",
        "--subnet",
        "8.8.8.0/24",
        "--hub",
        "8.8.8.1",
        "--owner-uid",
        "1000",
        "--input",
        "/var/lib/hq-net/hq0.conf",
    ];
    let (code, stderr) = run(&args);
    assert_eq!(code, Some(2));
    assert!(stderr.contains("--subnet and --hub"), "{stderr}");
}

#[test]
fn a_missing_or_untrusted_wg_stops_the_run_with_exit_1() {
    let args = [
        "--interface",
        "hq0",
        "--subnet",
        "10.77.0.0/24",
        "--hub",
        "10.77.0.1",
        "--owner-uid",
        "1000",
        "--input",
        "/var/lib/hq-net/hq0.conf",
        "--wg",
        "/nonexistent/wg",
    ];
    let (code, stderr) = run(&args);
    assert_eq!(code, Some(1));
    assert!(stderr.contains("/nonexistent/wg"), "{stderr}");
}

fn me() -> u32 {
    // SAFETY: geteuid has no preconditions and cannot fail.
    unsafe { libc::geteuid() }
}

/// A root-owned program that does nothing and succeeds, so `wg` can be located without root. The
/// tests below are refused before `wg` would be asked anything.
const ROOT_OWNED_NOOP: &str = "/usr/bin/true";

fn run_with_input(path: &std::path::Path) -> (Option<i32>, String) {
    let uid = me().to_string();
    let input = path.to_str().unwrap();
    run(&[
        "--interface",
        "hq0",
        "--subnet",
        "10.77.0.0/24",
        "--hub",
        "10.77.0.1",
        "--owner-uid",
        &uid,
        "--input",
        input,
        "--wg",
        ROOT_OWNED_NOOP,
    ])
}

fn private_file(dir: &std::path::Path, name: &str, text: &str) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join(name);
    std::fs::write(&path, text).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    path
}

#[test]
fn a_hostile_peers_file_is_refused_with_a_line_and_a_rule_and_nothing_else() {
    if !std::path::Path::new(ROOT_OWNED_NOOP).exists() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let text =
        "# hq-net peers v1\n[Interface]\nPostUp = touch /tmp/pwned-by-the-test\n# hq-net end 0\n";
    let (code, stderr) = run_with_input(&private_file(dir.path(), "peers.conf", text));
    assert_eq!(code, Some(1));
    assert!(
        stderr.contains("nothing applied: refused at line 2"),
        "{stderr}"
    );
    assert!(
        !stderr.contains("PostUp") && !stderr.contains("pwned"),
        "the line must not be echoed: {stderr}"
    );
}

#[test]
fn a_linked_or_missing_input_is_refused_by_the_real_program() {
    if !std::path::Path::new(ROOT_OWNED_NOOP).exists() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let real = private_file(
        dir.path(),
        "real.conf",
        "# hq-net peers v1\n# hq-net end 0\n",
    );
    let link = dir.path().join("link.conf");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let (code, stderr) = run_with_input(&link);
    assert_eq!(code, Some(1));
    assert!(stderr.contains("link"), "{stderr}");
    let (code, stderr) = run_with_input(&dir.path().join("absent.conf"));
    assert_eq!(code, Some(1));
    assert!(stderr.contains("nothing applied"), "{stderr}");
}

#[test]
fn a_valid_file_reaches_wg_and_a_wg_that_says_nothing_fails_the_run() {
    if !std::path::Path::new(ROOT_OWNED_NOOP).exists() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let path = private_file(
        dir.path(),
        "peers.conf",
        "# hq-net peers v1\n# hq-net end 0\n",
    );
    let (code, stderr) = run_with_input(&path);
    assert_eq!(code, Some(1));
    assert!(
        stderr.contains("failed:") && stderr.contains("[Interface]"),
        "{stderr}"
    );
}
