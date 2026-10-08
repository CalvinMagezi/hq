use super::*;
use crate::coding::BashTool;
use crate::tools::AgentTool;
use serde_json::json;

fn ctx(network: bool) -> SandboxContext {
    SandboxContext {
        cwd: PathBuf::from("/work/repo"),
        writable: vec![PathBuf::from("/home/u"), PathBuf::from("/work/repo")],
        masked_files: vec![PathBuf::from("/home/u/.ssh/id_ed25519")],
        readonly_files: vec![PathBuf::from("/home/u/.ssh/authorized_keys")],
        network,
    }
}

fn strings(args: &[OsString]) -> Vec<String> {
    args.iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect()
}

fn position(args: &[String], window: &[&str]) -> Option<usize> {
    args.windows(window.len())
        .position(|w| w.iter().zip(window).all(|(a, b)| a == b))
}

async fn run_bash(settings: BashSettings, command: &str) -> String {
    let result = BashTool::new(settings)
        .execute("t", json!({ "command": command }))
        .await
        .unwrap();
    result.content[0].text.clone()
}

fn unsandboxed(passthrough: &[&str]) -> BashSettings {
    BashSettings {
        env_passthrough: passthrough.iter().map(|s| s.to_string()).collect(),
        sandbox: BashSandboxMode::Off,
        network: true,
        writable_paths: Vec::new(),
        read_only: false,
    }
}

#[test]
fn bwrap_isolates_pid_namespace_and_masks_after_binds() {
    let args = strings(&bwrap_args(&ctx(true), "cargo test"));
    assert!(args.contains(&"--unshare-pid".to_string()));
    assert!(!args.contains(&"--unshare-net".to_string()));
    let bind = position(&args, &["--bind", "/work/repo", "/work/repo"]).unwrap();
    let mask = position(
        &args,
        &["--ro-bind", "/dev/null", "/home/u/.ssh/id_ed25519"],
    )
    .unwrap();
    assert!(bind < mask, "a later bind would unmask the key");
    assert_eq!(&args[args.len() - 3..], ["bash", "-c", "cargo test"]);
}

#[test]
fn bwrap_drops_network_when_denied() {
    let args = strings(&bwrap_args(&ctx(false), "true"));
    assert!(args.contains(&"--unshare-net".to_string()));
}

#[test]
fn seatbelt_denies_masked_reads_and_network() {
    let profile = seatbelt_profile(&ctx(false));
    assert!(
        profile.contains("(deny file-read* file-write* (literal \"/home/u/.ssh/id_ed25519\"))")
    );
    assert!(profile.contains("(deny network-outbound (remote ip))"));
    assert!(profile.contains("(subpath \"/work/repo\")"));
    assert!(!seatbelt_profile(&ctx(true)).contains("network-outbound"));
}

#[test]
fn sandbox_off_launches_directly() {
    assert!(matches!(
        plan_launch(&unsandboxed(&[]), "true"),
        Launch::Direct
    ));
}

#[test]
fn ssh_masks_cover_private_keys_but_not_public_files() {
    let dir = tempfile::tempdir().unwrap();
    let ssh = dir.path().join(".ssh");
    std::fs::create_dir(&ssh).unwrap();
    for name in [
        "id_ed25519",
        "id_ed25519.pub",
        "known_hosts",
        "config",
        "deploy_key",
    ] {
        std::fs::write(ssh.join(name), "x").unwrap();
    }
    let keys: Vec<String> = ssh_private_keys(&ssh)
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert!(keys.contains(&"id_ed25519".to_string()));
    assert!(keys.contains(&"deploy_key".to_string()));
    assert!(
        !keys
            .iter()
            .any(|k| k.ends_with(".pub") || k == "known_hosts" || k == "config")
    );
}

// The env tests set uniquely named vars, so they cannot race other tests.
#[tokio::test]
async fn secret_env_vars_never_reach_the_child() {
    // SAFETY: the name is unique to this test; nothing else reads it.
    unsafe { std::env::set_var("HQ_SEC_TEST_FAKE_API_KEY", "sk-must-not-leak") };
    let out = run_bash(
        unsandboxed(&[]),
        "echo \"[${HQ_SEC_TEST_FAKE_API_KEY:-absent}]\"",
    )
    .await;
    assert!(out.contains("[absent]"), "child saw the secret: {out}");
    assert!(!out.contains("sk-must-not-leak"));
}

#[tokio::test]
async fn passthrough_vars_reach_the_child() {
    // SAFETY: the name is unique to this test; nothing else reads it.
    unsafe { std::env::set_var("HQ_SEC_TEST_GRANTED_TOKEN", "granted-value") };
    let settings = unsandboxed(&["HQ_SEC_TEST_GRANTED_TOKEN"]);
    let out = run_bash(settings, "echo \"[$HQ_SEC_TEST_GRANTED_TOKEN]\"").await;
    assert!(
        out.contains("[granted-value]"),
        "passthrough missing: {out}"
    );
}

#[tokio::test]
async fn child_env_is_exactly_the_allowlist() {
    let out = run_bash(unsandboxed(&[]), "env | cut -d= -f1").await;
    for line in out.lines() {
        let key = line.trim();
        let allowed = key.is_empty()
            || key.starts_with("LC_")
            || ["PWD", "SHLVL", "_", "OLDPWD"].contains(&key)
            || crate::bash_policy::build_child_env([(key.to_string(), String::new())], &[]).len()
                == 1;
        assert!(allowed, "unexpected var in child env: {key}");
    }
}

/// Run `command` wrapped by `backend` with `dir` as the only writable root
/// and `secret` masked; returns (success, stdout).
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn run_wrapped(backend: &Backend, dir: &Path, secret: &Path, command: &str) -> (bool, String) {
    let ctx = SandboxContext {
        cwd: dir.to_path_buf(),
        writable: vec![dir.to_path_buf()],
        masked_files: vec![secret.to_path_buf()],
        readonly_files: Vec::new(),
        network: false,
    };
    let Launch::Wrapped { program, args } = wrap(backend, &ctx, command) else {
        panic!("expected a wrapped launch");
    };
    // BashTool never sets a cwd, so the context's cwd is always the process
    // cwd; match that here since seatbelt has no chdir of its own.
    let out = std::process::Command::new(program)
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    )
}

/// A temp dir holding a secret file, both canonical (seatbelt matches
/// resolved paths).
fn secret_fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let secret = root.join("secret.txt");
    std::fs::write(&secret, "top-secret").unwrap();
    (dir, root, secret)
}

const INSIDE_WRITE: &str = "echo ok > inside.txt && cat inside.txt";

#[cfg(target_os = "macos")]
#[tokio::test]
async fn seatbelt_allows_work_but_blocks_masked_reads() {
    let backend = available_backend().expect("sandbox-exec ships with macOS");
    let (_guard, root, secret) = secret_fixture();

    // Positive control: a profile sandbox-exec rejects would also make the
    // denial below pass.
    let (ok, out) = run_wrapped(backend, &root, &secret, INSIDE_WRITE);
    assert!(ok && out.contains("ok"), "sandbox broke normal work: {out}");

    let read = format!("cat '{}'", secret.display());
    let (ok, out) = run_wrapped(backend, &root, &secret, &read);
    assert!(!ok && !out.contains("top-secret"));
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn required_sandbox_runs_real_commands_with_spaced_paths() {
    let (_guard, root, _) = secret_fixture();
    let spaced = root.join("Application Support");
    std::fs::create_dir(&spaced).unwrap();
    let settings = BashSettings {
        env_passthrough: Vec::new(),
        sandbox: BashSandboxMode::Required,
        network: true,
        writable_paths: vec![spaced.clone()],
        read_only: false,
    };
    let target = spaced.join("note.txt");
    let command = format!("echo hi > '{0}' && cat '{0}'", target.display());
    let out = run_bash(settings.clone(), &command).await;
    assert!(out.contains("hi"), "generated profile broke a write: {out}");

    let stray = format!("/Users/Shared/hq-sbx-{}", std::process::id());
    let out = run_bash(settings, &format!("touch {stray}")).await;
    let _ = std::fs::remove_file(&stray);
    assert!(
        out.contains("exit code"),
        "write outside the roots succeeded: {out}"
    );
}

#[cfg(target_os = "linux")]
#[tokio::test]
#[ignore = "needs bubblewrap with unprivileged user namespaces; run with `cargo test -p hq-agent bwrap -- --ignored`"]
async fn bwrap_allows_work_but_blocks_masked_reads_environ_and_network() {
    let backend = available_backend().expect("bwrap installed and working");
    let (_guard, root, secret) = secret_fixture();

    let (ok, out) = run_wrapped(backend, &root, &secret, INSIDE_WRITE);
    assert!(ok && out.contains("ok"), "sandbox broke normal work: {out}");

    let command = format!(
        "cat '{}'; cat /proc/{}/environ; curl -s -m 3 https://example.com",
        secret.display(),
        std::process::id()
    );
    let (_, out) = run_wrapped(backend, &root, &secret, &command);
    assert!(!out.contains("top-secret"));
    assert!(!out.contains("PATH="), "parent environ visible");
    assert!(!out.contains("Example Domain"), "network reachable");
}

// Runs in CI (Linux, non-root). Quote splitting dodges the policy regex, so
// only the non-dumpable flag keeps the child out of the parent's environment.
#[cfg(target_os = "linux")]
#[tokio::test]
async fn bash_children_cannot_read_the_daemon_environment() {
    // SAFETY: geteuid has no preconditions.
    if unsafe { libc::geteuid() } == 0 {
        eprintln!("skipped: root bypasses the dumpable check");
        return;
    }
    let pid = std::process::id();
    let command = format!(
        "cat /proc/{pid}/env''iron | tr '\\0' '\\n' | grep -c PATH=; ps eww -p {pid} | grep -c PATH="
    );
    let out = run_bash(unsandboxed(&[]), &command).await;
    let counts: Vec<&str> = out.lines().take(2).map(str::trim).collect();
    assert_eq!(counts, ["0", "0"], "daemon env readable: {out}");
}

fn settings_with(mode: BashSandboxMode) -> BashSettings {
    BashSettings {
        sandbox: mode,
        ..unsandboxed(&[])
    }
}

fn fake_bwrap() -> Backend {
    Backend::Bwrap(PathBuf::from("/usr/bin/bwrap"))
}

#[test]
fn default_policy_refuses_bash_when_no_backend_exists() {
    let settings = BashSettings::default();
    assert_eq!(settings.sandbox, BashSandboxMode::Required);
    let Launch::Refused(reason) = plan_launch_with(&settings, "echo hi", None) else {
        panic!("default settings must refuse without a backend");
    };
    assert!(
        reason.contains("best_effort"),
        "refusal must name the opt-out: {reason}"
    );
}

#[test]
fn default_policy_wraps_when_a_backend_exists() {
    let launch = plan_launch_with(&BashSettings::default(), "echo hi", Some(&fake_bwrap()));
    assert!(matches!(launch, Launch::Wrapped { .. }));
}

#[test]
fn explicit_best_effort_and_off_run_direct_without_a_backend() {
    for mode in [BashSandboxMode::BestEffort, BashSandboxMode::Off] {
        let launch = plan_launch_with(&settings_with(mode), "echo hi", None);
        assert!(matches!(launch, Launch::Direct), "{mode:?}");
    }
}

#[test]
fn off_never_wraps_even_with_a_backend() {
    let launch = plan_launch_with(
        &settings_with(BashSandboxMode::Off),
        "x",
        Some(&fake_bwrap()),
    );
    assert!(matches!(launch, Launch::Direct));
}

#[test]
fn status_reports_each_state_for_doctor() {
    let b = fake_bwrap();
    let status = |m, backend| sandbox_status(&settings_with(m), backend);
    assert_eq!(
        status(BashSandboxMode::Required, Some(&b)),
        SandboxStatus::Active("bubblewrap".into())
    );
    assert_eq!(
        status(BashSandboxMode::Required, None),
        SandboxStatus::Refusing
    );
    assert_eq!(
        status(BashSandboxMode::BestEffort, None),
        SandboxStatus::Unwrapped
    );
    assert_eq!(
        status(BashSandboxMode::Off, Some(&b)),
        SandboxStatus::Disabled
    );
    assert_eq!(SandboxStatus::Refusing.describe().0, "FAIL");
    assert_eq!(SandboxStatus::Unwrapped.describe().0, "warn");
    assert_eq!(SandboxStatus::Active("x".into()).describe().0, "ok");
}

#[test]
fn bwrap_probe_requests_unshare_net_exactly_when_the_real_argv_does() {
    assert!(!bwrap_probe_args(true).contains(&"--unshare-net"));
    assert!(bwrap_probe_args(false).contains(&"--unshare-net"));
    assert!(strings(&bwrap_args(&ctx(false), "true")).contains(&"--unshare-net".to_string()));
}

#[test]
fn authorized_keys_is_read_only_and_control_sockets_are_masked() {
    let args = strings(&bwrap_args(&ctx(true), "true"));
    let ro = position(&args, &["--ro-bind", "/home/u/.ssh/authorized_keys", "/home/u/.ssh/authorized_keys"]).unwrap();
    let bind = position(&args, &["--bind", "/home/u", "/home/u"]).unwrap();
    assert!(bind < ro, "the read-only bind must come after the writable one");
    assert!(seatbelt_profile(&ctx(true)).contains("(deny file-write* (literal \"/home/u/.ssh/authorized_keys\"))"));
    assert!(control_sockets().iter().any(|p| p.ends_with("docker.sock")));
}

#[test]
fn refusing_status_files_one_notification_per_boot() {
    let db = hq_db::Database::open_memory().unwrap();
    let settings = BashSettings::default();
    assert!(report_refusal(&settings, None, &db, "boot1").unwrap());
    assert!(report_refusal(&settings, None, &db, "boot1").unwrap());
    let count = |db: &hq_db::Database| {
        hq_db::value_items::list_filtered(db, None, None, 50).unwrap().len()
    };
    assert_eq!(count(&db), 1, "same boot collapses");
    report_refusal(&settings, None, &db, "boot2").unwrap();
    assert_eq!(count(&db), 2, "a new boot notifies again");
}

#[test]
fn working_or_opted_out_sandbox_files_nothing() {
    let db = hq_db::Database::open_memory().unwrap();
    let b = fake_bwrap();
    assert!(!report_refusal(&BashSettings::default(), Some(&b), &db, "b").unwrap());
    assert!(!report_refusal(&settings_with(BashSandboxMode::BestEffort), None, &db, "b").unwrap());
    assert!(!report_refusal(&settings_with(BashSandboxMode::Off), None, &db, "b").unwrap());
    assert!(hq_db::value_items::list_filtered(&db, None, None, 5).unwrap().is_empty());
}

#[test]
fn read_only_context_leaves_only_scratch_writable() {
    let mut settings = BashSettings::read_only(&BashConfig::default());
    settings.writable_paths = vec![PathBuf::from("/should/not/appear")];
    let ctx = SandboxContext::for_process(&settings);
    let home = dirs::home_dir().and_then(|h| h.canonicalize().ok());
    let cwd = std::env::current_dir().unwrap().canonicalize().unwrap();
    assert!(!ctx.writable.iter().any(|p| Some(p) == home.as_ref() || *p == cwd));
    assert!(!ctx.network);
}

#[test]
fn read_only_settings_refuse_without_a_backend() {
    let settings = BashSettings::read_only(&BashConfig::default());
    assert!(matches!(
        plan_launch_with(&settings, "ls", None),
        Launch::Refused(_)
    ));
}

#[tokio::test]
async fn read_only_bash_cannot_write_to_cwd_or_home() {
    if available_backend().is_none() {
        eprintln!("skipped: no sandbox backend on this host");
        return;
    }
    let probe = format!("hq-ro-probe-{}", std::process::id());
    let in_home = dirs::home_dir().unwrap().join(&probe);
    let command = format!("touch ./{probe}; touch '{}'; ls", in_home.display());
    let out = run_bash(BashSettings::read_only(&BashConfig::default()), &command).await;
    let cwd_file = std::env::current_dir().unwrap().join(&probe);
    let leaked = cwd_file.exists() || in_home.exists();
    let _ = std::fs::remove_file(&cwd_file);
    let _ = std::fs::remove_file(&in_home);
    assert!(!leaked, "read-only bash wrote to disk: {out}");
}
