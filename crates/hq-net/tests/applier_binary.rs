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
