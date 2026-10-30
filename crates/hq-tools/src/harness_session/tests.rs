use super::*;
use crate::agent_host::scripted::ScriptedHost;
use crate::registry::HqTool;
use hq_core::config::AgentHostConfig;
use std::os::unix::fs::PermissionsExt;

const CREATED: &str = r#"{"id":"x","result":{"type":"workspace_created","workspace":{"workspace_id":"w9"},"root_pane":{"pane_id":"w9:p1"}}}"#;
const OK: &str = r#"{"id":"x","result":{"type":"ok"}}"#;
const PENDING_AGENT: &str = r#"{"id":"x","result":{"type":"agent_info","agent":{"agent":"claude","agent_status":"unknown","cwd":"/t","name":"hs-t5","pane_id":"w9:p1","workspace_id":"w9","tab_id":"w9:t1","state_change_seq":0,"launch_pending":true}}}"#;
const NOT_READY: &str =
    r#"{"error":{"code":"agent_not_ready","message":"blocked during startup"},"id":"x"}"#;

fn agent_json(name: &str, status: &str) -> String {
    format!(
        r#"{{"id":"x","result":{{"type":"agent_info","agent":{{"agent":"claude","agent_status":"{status}","cwd":"/t","name":"{name}","pane_id":"w9:p1","workspace_id":"w9","tab_id":"w9:t1","state_change_seq":1}}}}}}"#
    )
}

/// Fake host: each `(needle, stdout, fail_stderr)` answers commands containing
/// the needle, first match wins; every call is appended to `calls.log`.
fn fake_host(replies: &[(&str, String, Option<&str>)]) -> (tempfile::TempDir, ScriptedHost) {
    let (dir, host) = fake_host_checking_binaries(replies);
    (dir, host.without_binary_preflight())
}

/// Like `fake_host`, but the host still checks that the harness binary exists.
fn fake_host_checking_binaries(
    replies: &[(&str, String, Option<&str>)],
) -> (tempfile::TempDir, ScriptedHost) {
    let dir = tempfile::tempdir().unwrap();
    let mut script =
        String::from("#!/bin/sh\necho \"$@\" >> \"$(dirname \"$0\")/calls.log\"\ncase \"$*\" in\n");
    for (needle, stdout, err) in replies {
        let (body, code) = match err {
            Some(e) => (format!("printf '%s' '{e}' >&2"), 1),
            None => (format!("printf '%s' '{stdout}'"), 0),
        };
        script.push_str(&format!("  *\"{needle}\"*) {body}; exit {code} ;;\n"));
    }
    script.push_str("  *) echo unexpected >&2; exit 2 ;;\nesac\n");
    let path = dir.path().join("scripted-host");
    std::fs::write(&path, script).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    (dir, ScriptedHost::new(path))
}

fn launch<'a>(host: ScriptedHost, id: &'a str, prompt: Option<&'a str>) -> Launch<'a> {
    Launch {
        host: std::sync::Arc::new(host),
        session_id: id,
        cwd: "/t",
        label: "demo",
        prompt,
        resume_token: None,
        resuming: false,
        mission_id: None,
        watch: None,
        parent: None,
        goal: GoalText::default(),
        purpose: Purpose::Code,
        observe: &[],
        one_shot: None,
        origin: None,
        policy: None,
    }
}

const GOAL: &str = "Add rate limiting to the login endpoint";
const DONE: &str = "Login returns 429 after 5 failed attempts and cargo test passes";

fn goal_text() -> GoalText<'static> {
    GoalText {
        goal: Some(GOAL),
        done_criteria: Some(DONE),
    }
}

fn harness(name: &str) -> Harness {
    resolve_in(&AgentHostConfig::default(), name).unwrap()
}

fn profile_harness(yaml: &str, name: &str) -> Harness {
    let cfg: AgentHostConfig = serde_yaml::from_str(&format!("harness_profiles:\n{yaml}")).unwrap();
    resolve_in(&cfg, name).unwrap()
}

fn calls(dir: &tempfile::TempDir) -> String {
    std::fs::read_to_string(dir.path().join("calls.log")).unwrap_or_default()
}

#[test]
fn build_args_substitutes_token_on_resume() {
    let spec = harness("cursor");
    let args = build_args(&spec, "hs-x", Some("abc123"), true);
    assert!(args.contains(&"--resume".to_string()));
    assert!(args.contains(&"abc123".to_string()));
}

#[test]
fn build_args_falls_back_fresh_without_token() {
    let spec = harness("cursor");
    let args = build_args(&spec, "hs-x", None, true);
    assert!(!args.iter().any(|a| a.contains("{token}")));
    assert!(!args.contains(&"--resume".to_string()));
}

#[test]
fn claude_resumes_its_own_conversation_when_the_id_is_known() {
    let h = harness("claude-code");
    let with = build_args(&h, "hs-x", Some("conv-1"), true);
    assert_eq!(with[with.len() - 2..], ["--resume", "conv-1"]);
    let without = build_args(&h, "hs-x", None, true);
    assert_eq!(without.last().map(String::as_str), Some("-c"));
    let fresh = build_args(&h, "hs-x", Some("conv-1"), false);
    assert!(!fresh.contains(&"--resume".to_string()));
}

#[test]
fn a_chosen_model_is_passed_on_fresh_starts_and_resumes() {
    let h = harness("claude-code").with_model(Some("sonnet")).unwrap();
    let fresh = build_args(&h, "hs-x", None, false);
    assert_eq!(fresh[fresh.len() - 2..], ["--model", "sonnet"]);
    let resumed = build_args(&h, "hs-x", Some("conv-1"), true);
    assert!(resumed.windows(2).any(|w| w == ["--resume", "conv-1"]));
    assert_eq!(resumed[resumed.len() - 2..], ["--model", "sonnet"]);
    let default = build_args(&harness("claude-code"), "hs-x", None, false);
    assert!(!default.contains(&"--model".to_string()));
}

#[test]
fn a_model_is_refused_for_an_agent_without_a_model_flag_or_when_it_reads_as_a_flag() {
    let no_flag = harness("hermes").with_model(Some("gpt-5.5")).unwrap_err();
    assert!(no_flag.to_string().contains("no model flag"), "{no_flag}");
    for bad in ["--dangerously-skip-permissions", "a b", "x;rm"] {
        assert!(harness("codex").with_model(Some(bad)).is_err(), "{bad}");
    }
    assert!(
        harness("hermes")
            .with_model(Some("  "))
            .unwrap()
            .model
            .is_none()
    );
}

#[test]
fn the_model_is_kept_on_the_session_row() {
    let db = Database::open_memory().unwrap();
    db.with_conn(|c| {
        registry::insert(
            c,
            &registry::NewSession {
                id: "hs-m",
                harness: "codex",
                label: "",
                cwd: "/t",
                mission_id: None,
                placement: registry::Placement {
                    host: "local",
                    agent_name: "hs-m",
                    workspace_id: "w",
                    pane_id: "p",
                },
                purpose: None,
            },
        )?;
        registry::set_model(c, "hs-m", "gpt-5.5")?;
        assert_eq!(
            registry::get(c, "hs-m")?.unwrap().model.as_deref(),
            Some("gpt-5.5")
        );
        Ok(())
    })
    .unwrap();
}

#[test]
fn a_reported_conversation_id_becomes_the_resume_token_once() {
    let db = Arc::new(Database::open_memory().unwrap());
    db.with_conn(|c| {
        registry::insert(
            c,
            &registry::NewSession {
                id: "hs-id",
                harness: "claude-code",
                label: "t",
                cwd: "/t",
                mission_id: None,
                placement: registry::Placement {
                    host: "native",
                    agent_name: "hs-id",
                    workspace_id: "hs-id",
                    pane_id: "hs-id",
                },
                purpose: None,
            },
        )
    })
    .unwrap();
    let Liveness::Alive(mut agent) = alive("hs-id") else {
        unreachable!()
    };

    let row = get_row(&db, "hs-id").unwrap();
    assert!(
        !record_agent_session_id(&db, &row, &agent).unwrap(),
        "no id reported yet"
    );

    agent.agent_session_id = Some("conv-9".into());
    assert!(record_agent_session_id(&db, &row, &agent).unwrap());
    let row = get_row(&db, "hs-id").unwrap();
    assert_eq!(row.resume_token.as_deref(), Some("conv-9"));
    assert!(
        !record_agent_session_id(&db, &row, &agent).unwrap(),
        "unchanged"
    );
}

#[test]
fn session_dir_harness_gets_dir_arg() {
    let spec = harness("pi");
    let args = build_args(&spec, "hs-pi-1", None, false);
    assert!(args.contains(&"--session-dir".to_string()));
    assert!(args.iter().any(|a| a.contains("hs-pi-1")));
    assert!(args.iter().any(|a| a.starts_with("{home}/")));
}

#[test]
fn antigravity_resume_uses_continue_flag() {
    let spec = harness("antigravity");
    let resumed = build_args(&spec, "hs-x", None, true);
    assert_eq!(
        resumed,
        vec![
            "--dangerously-skip-permissions".to_string(),
            "-c".to_string()
        ]
    );
    let fresh = build_args(&spec, "hs-x", None, false);
    assert_eq!(fresh, vec!["--dangerously-skip-permissions".to_string()]);
}

#[test]
fn session_ids_are_unique_and_valid_host_names() {
    let a = new_session_id("github-copilot");
    std::thread::sleep(Duration::from_millis(2));
    let b = new_session_id("github-copilot");
    assert_ne!(a, b);
    let valid = regex::Regex::new(r"^[a-z][a-z0-9_-]{0,31}$").unwrap();
    for spec in spec::SPECS {
        assert!(
            valid.is_match(&new_session_id(spec.harness)),
            "{}",
            spec.harness
        );
    }
    let long = new_session_id("My_Very_Long_Wrapper_Profile_Name");
    assert!(valid.is_match(&long), "{long}");
}

#[test]
fn a_profile_resumes_with_the_base_arguments() {
    let h = profile_harness(
        "  wrapped:\n    base: claude-code\n    args: [\"--model\", \"x\"]\n",
        "wrapped",
    );
    let fresh = build_args(&h, "hs-x", None, false);
    assert_eq!(fresh, vec!["--model".to_string(), "x".to_string()]);
    let resumed = build_args(&h, "hs-x", None, true);
    assert_eq!(
        resumed,
        vec![
            "--dangerously-skip-permissions".to_string(),
            "-c".to_string()
        ]
    );
}

#[tokio::test]
async fn a_wrapper_profile_launches_through_the_shell_and_types_the_prompt() {
    let (dir, host) = fake_host(&[
        ("workspace create", CREATED.into(), None),
        ("pane run", String::new(), None),
        ("agent rename", agent_json("hs-t4", "idle"), None),
        ("agent wait", agent_json("hs-t4", "idle"), None),
        ("agent get", agent_json("hs-t4", "idle"), None),
        ("agent prompt", OK.into(), None),
    ]);
    let db = Arc::new(Database::open_memory().unwrap());
    let vault = tempfile::tempdir().unwrap();
    let h = profile_harness(
        "  claude-alt:\n    base: claude-code\n    command: alt-wrapper\n    env:\n      ACCOUNT: work\n",
        "claude-alt",
    );

    let report = launch_session(vault.path(), &db, &h, launch(host, "hs-t4", Some("go")))
        .await
        .unwrap();

    assert_eq!(report["harness"], "claude-alt");
    assert_eq!(report["agent_status"], "idle");
    let log = calls(&dir);
    assert!(log.contains("--env ACCOUNT=work"), "{log}");
    assert!(
        log.contains("pane run w9:p1 alt-wrapper --dangerously-skip-permissions"),
        "{log}"
    );
    assert!(!log.contains("agent start"), "{log}");
    assert!(log.contains("agent rename w9:p1 hs-t4"), "{log}");
    assert!(log.contains("agent prompt hs-t4 go"), "{log}");
    assert_eq!(get_row(&db, "hs-t4").unwrap().harness, "claude-alt");
}

#[test]
fn every_spec_names_a_kind_the_host_supports() {
    const HOST_KINDS: &[&str] = &[
        "pi",
        "claude",
        "codex",
        "gemini",
        "cursor",
        "devin",
        "agy",
        "cline",
        "omp",
        "mastracode",
        "opencode",
        "copilot",
        "kimi",
        "kiro",
        "droid",
        "amp",
        "grok",
        "hermes",
        "openclaw",
        "kilo",
        "qodercli",
        "qwen",
        "letta",
        "muse",
    ];
    for spec in spec::SPECS {
        assert!(
            HOST_KINDS.contains(&spec.kind),
            "{} -> {}",
            spec.harness,
            spec.kind
        );
    }
}

#[test]
fn extract_resume_token_picks_the_last_match_not_the_first() {
    let pattern = spec_for("cursor").unwrap().token_pattern.unwrap();
    let content = "chat_id: 20260101_000000_aaaaaa\nsome output\nchat_id: 20260714_142148_ff76c3\n";
    let token = extract_resume_token(pattern, content).unwrap();
    assert_eq!(token.as_deref(), Some("20260714_142148_ff76c3"));
}

#[test]
fn extract_resume_token_returns_none_when_absent() {
    let pattern = spec_for("cursor").unwrap().token_pattern.unwrap();
    assert_eq!(
        extract_resume_token(pattern, "no token here").unwrap(),
        None
    );
}

#[tokio::test]
async fn launch_records_the_session_and_types_the_prompt() {
    let (dir, host) = fake_host(&[
        ("workspace create", CREATED.into(), None),
        ("agent start", OK.into(), None),
        ("agent get", agent_json("hs-t1", "idle"), None),
        ("agent prompt", OK.into(), None),
    ]);
    let db = Arc::new(Database::open_memory().unwrap());
    let vault = tempfile::tempdir().unwrap();
    let spec = harness("claude-code");

    let report = launch_session(
        vault.path(),
        &db,
        &spec,
        launch(host, "hs-t1", Some("fix the bug")),
    )
    .await
    .unwrap();

    assert_eq!(report["status"], "running");
    assert_eq!(report["agent_status"], "idle");
    assert!(report.get("blocked").is_none());
    let row = get_row(&db, "hs-t1").unwrap();
    assert_eq!(row.workspace_id.as_deref(), Some("w9"));
    assert_eq!(row.host, "local");
    let log = calls(&dir);
    assert!(log.contains("--kind claude"), "{log}");
    assert!(log.contains("agent prompt hs-t1 fix the bug"), "{log}");
}

#[tokio::test]
async fn copilot_gets_its_first_prompt_at_start_and_nothing_is_typed() {
    // No "agent prompt" reply: a typed prompt would hit the script's `unexpected` arm and fail.
    let (dir, host) = fake_host(&[
        ("workspace create", CREATED.into(), None),
        ("agent start", OK.into(), None),
        ("agent get", agent_json("hs-c1", "idle"), None),
    ]);
    let db = Arc::new(Database::open_memory().unwrap());
    let vault = tempfile::tempdir().unwrap();

    let report = launch_session(
        vault.path(),
        &db,
        &harness("github-copilot"),
        launch(host, "hs-c1", Some("-v fix the bug")),
    )
    .await
    .unwrap();

    assert_eq!(report["status"], "running");
    let log = calls(&dir);
    assert!(log.contains("--kind copilot"), "{log}");
    assert!(log.contains("--interactive=-v fix the bug"), "{log}");
    assert!(!log.contains("agent prompt"), "{log}");
}

#[tokio::test]
async fn a_resumed_copilot_session_has_its_prompt_typed_not_passed_at_start() {
    let (dir, host) = fake_host(&[
        ("workspace create", CREATED.into(), None),
        ("agent start", OK.into(), None),
        ("agent get", agent_json("hs-c2", "idle"), None),
        ("agent prompt", OK.into(), None),
    ]);
    let db = Arc::new(Database::open_memory().unwrap());
    let vault = tempfile::tempdir().unwrap();
    let mut l = launch(host, "hs-c2", Some("carry on"));
    l.resuming = true;

    launch_session(vault.path(), &db, &harness("github-copilot"), l)
        .await
        .unwrap();

    let log = calls(&dir);
    assert!(!log.contains("--interactive"), "{log}");
    assert!(log.contains("agent prompt hs-c2 carry on"), "{log}");
}

#[tokio::test]
async fn a_delegated_session_has_its_parent_from_the_moment_it_exists() {
    let (_dir, host) = fake_host(&[
        ("workspace create", CREATED.into(), None),
        ("agent start", OK.into(), None),
        ("agent get", agent_json("hs-kid", "idle"), None),
        ("agent prompt", OK.into(), None),
    ]);
    let db = Arc::new(Database::open_memory().unwrap());
    let vault = tempfile::tempdir().unwrap();
    let mut l = launch(host, "hs-kid", Some("work"));
    l.parent = Some(("hs-parent", 1));
    launch_session(vault.path(), &db, &harness("claude-code"), l)
        .await
        .unwrap();

    // Read straight after the launch call: there is no later write that sets it.
    let row = get_row(&db, "hs-kid").unwrap();
    assert_eq!(row.parent_session_id.as_deref(), Some("hs-parent"));
    assert_eq!(row.spawn_depth, 1);
    let children = db
        .with_conn(|c| registry::running_children(c, "hs-parent"))
        .unwrap();
    assert_eq!(children.len(), 1, "the limits count it at once");
}

#[tokio::test]
async fn a_launch_for_a_task_links_the_session_and_starts_the_task() {
    let (_dir, host) = fake_host(&[
        ("workspace create", CREATED.into(), None),
        ("agent start", OK.into(), None),
        ("agent get", agent_json("hs-t2", "idle"), None),
    ]);
    let db = Arc::new(Database::open_memory().unwrap());
    let task = db
        .with_conn(|c| {
            c.execute(
                "INSERT INTO initiatives (id, space_id, name, slug, id_prefix) VALUES ('in-1', 'personal', 'Work', 'work', 'FR')",
                [],
            )?;
            let new = hq_db::tasks::NewTask {
                title: "durable missions",
                created_by: "test",
                ..Default::default()
            };
            Ok(hq_db::tasks::create_task(c, "tk-1", "in-1", &new)?.id)
        })
        .unwrap();
    let vault = tempfile::tempdir().unwrap();
    let l = Launch {
        mission_id: Some(&task),
        ..launch(host, "hs-t2", None)
    };

    let report = launch_session(vault.path(), &db, &harness("claude-code"), l)
        .await
        .unwrap();

    assert_eq!(report["task"]["display_id"], "FR-001");
    assert_eq!(report["task"]["status"], "in_progress");
    assert_eq!(
        get_row(&db, "hs-t2").unwrap().mission_id.as_deref(),
        Some(task.as_str())
    );
    let linked = db
        .with_conn(|c| registry::list_for_mission(c, &task))
        .unwrap();
    assert_eq!(linked.len(), 1);
}

#[tokio::test]
async fn a_blocked_launch_reports_the_screen_and_types_nothing() {
    let dangerous_default = "Quick safety check\n No, exit\n Yes, I trust this folder";
    let (dir, host) = fake_host(&[
        ("workspace create", CREATED.into(), None),
        ("agent start", String::new(), Some(NOT_READY)),
        ("agent get", agent_json("hs-t2", "blocked"), None),
        ("agent read", dangerous_default.into(), None),
    ]);
    let db = Arc::new(Database::open_memory().unwrap());
    let vault = tempfile::tempdir().unwrap();
    let spec = harness("claude-code");

    let report = launch_session(vault.path(), &db, &spec, launch(host, "hs-t2", Some("go")))
        .await
        .unwrap();

    assert_eq!(report["agent_status"], "blocked");
    assert!(
        report["blocked"]["screen"]
            .as_str()
            .unwrap()
            .contains("No, exit")
    );
    let log = calls(&dir);
    assert!(
        !log.contains("agent prompt"),
        "must not type into a dialog: {log}"
    );
    assert!(
        !log.contains("send-keys"),
        "claude has no vouched trust default: {log}"
    );
    assert_eq!(
        get_row(&db, "hs-t2").unwrap().status,
        registry::STATUS_RUNNING
    );
}

#[tokio::test]
async fn a_dialog_that_appears_after_ready_is_a_blocked_launch_not_an_error() {
    // Codex reads as idle, then shows its folder trust question before the prompt lands.
    let refused = r#"{"error":{"code":"agent_blocked","message":"waiting at a dialog"},"id":"x"}"#;
    let (_dir, host) = fake_host(&[
        ("workspace create", CREATED.into(), None),
        ("agent start", OK.into(), None),
        ("agent get", agent_json("hs-t4", "idle"), None),
        ("agent prompt", String::new(), Some(refused)),
        (
            "agent read",
            "Trust this folder?\n› 1. Trust and continue".into(),
            None,
        ),
    ]);
    let db = Arc::new(Database::open_memory().unwrap());
    let vault = tempfile::tempdir().unwrap();

    let report = launch_session(
        vault.path(),
        &db,
        &harness("claude-code"),
        launch(host, "hs-t4", Some("go")),
    )
    .await
    .expect("the session exists, so the spawn reports it");

    assert_eq!(report["agent_status"], "blocked", "{report}");
    assert!(
        report["blocked"]["screen"]
            .as_str()
            .unwrap()
            .contains("Trust this folder?"),
        "{report}"
    );
    assert_eq!(
        get_row(&db, "hs-t4").unwrap().status,
        registry::STATUS_RUNNING
    );
}

#[tokio::test]
async fn an_agent_still_loading_is_not_reported_as_blocked() {
    let (dir, host) = fake_host(&[
        ("workspace create", CREATED.into(), None),
        ("agent start", String::new(), Some(NOT_READY)),
        ("agent get", agent_json("hs-t2", "working"), None),
    ]);
    let db = Arc::new(Database::open_memory().unwrap());
    let vault = tempfile::tempdir().unwrap();
    let spec = harness("claude-code");

    let report = launch_session(vault.path(), &db, &spec, launch(host, "hs-t2", Some("go")))
        .await
        .unwrap();

    assert_eq!(report["agent_status"], "working");
    assert!(report.get("blocked").is_none(), "{report}");
    assert!(
        report["note"]
            .as_str()
            .unwrap()
            .contains("prompt was not typed"),
        "{report}"
    );
    let log = calls(&dir);
    assert!(
        !log.contains("agent prompt"),
        "must not type into a loading agent: {log}"
    );
}

#[tokio::test]
async fn a_vouched_trust_dialog_is_accepted_then_the_prompt_is_typed() {
    let (dir, host) = fake_host(&[
        ("workspace create", CREATED.into(), None),
        ("agent start", String::new(), Some(NOT_READY)),
        ("agent get", agent_json("hs-t3", "blocked"), None),
        (
            "agent read",
            "Do you trust the contents of this project?".into(),
            None,
        ),
        ("agent send-keys", OK.into(), None),
        ("agent wait", agent_json("hs-t3", "idle"), None),
        ("agent prompt", OK.into(), None),
    ]);
    let db = Arc::new(Database::open_memory().unwrap());
    let vault = tempfile::tempdir().unwrap();
    let spec = harness("antigravity");

    let report = launch_session(
        vault.path(),
        &db,
        &spec,
        launch(host, "hs-t3", Some("hello")),
    )
    .await
    .unwrap();

    assert_eq!(report["agent_status"], "idle");
    let log = calls(&dir);
    assert!(log.contains("agent send-keys hs-t3 enter"), "{log}");
    assert!(log.contains("agent prompt hs-t3 hello"), "{log}");
}

fn row(host: &str, name: &str) -> HarnessSessionRow {
    HarnessSessionRow {
        id: name.into(),
        harness: "pi".into(),
        label: String::new(),
        host: host.into(),
        agent_name: name.into(),
        workspace_id: None,
        pane_id: None,
        cwd: "/t".into(),
        status: "running".into(),
        resume_token: None,
        mission_id: None,
        created_at: String::new(),
        updated_at: String::new(),
        owner_thread: None,
        drive: false,
        pm_wake: None,
        last_driven_at: None,
        last_agent_status: None,
        last_seen_at: None,
        goal: None,
        done_criteria: None,
        keys_sent: 0,
        origin: "user".into(),
        dismissals: 0,
        last_dismiss_tail: None,
        nudges_sent: 0,
        last_wake_nudges: None,
        no_progress_streak: 0,
        progress_mark: None,
        drive_off_reason: None,
        parent_session_id: None,
        spawn_depth: 0,
        purpose: "code".into(),
        model: None,
    }
}

#[test]
fn liveness_distinguishes_gone_from_unreachable() {
    let list = r#"{"id":"x","result":{"agents":[{"agent":"pi","agent_status":"working","cwd":"/t","name":"hs-live","pane_id":"w1:p1","workspace_id":"w1"}]}}"#;
    let (_dir, host) = fake_host(&[("agent list", list.into(), None)]);
    let rows = [
        row("local", "hs-live"),
        row("local", "hs-dead"),
        row("laptop", "hs-away"),
    ];

    let polled = poll_hosts_with(&rows, |name| match name {
        "local" => Ok(Arc::new(host.clone())),
        other => Err(anyhow::anyhow!("host '{other}' unreachable: no route")),
    });

    assert!(
        matches!(liveness(&polled, &rows[0]), Liveness::Alive(a) if a.status == AgentStatus::Working)
    );
    assert_eq!(liveness(&polled, &rows[1]), Liveness::Gone);
    assert!(
        matches!(liveness(&polled, &rows[2]), Liveness::HostUnreachable(d) if d.contains("no route"))
    );
}

#[test]
fn only_a_clean_submit_goes_without_a_note() {
    assert!(prompt_note(PromptOutcome::Submitted).is_none());
    assert!(prompt_note(PromptOutcome::Resubmitted).is_some());
    let timed_out = prompt_note(PromptOutcome::TimedOut("10s".into()));
    assert!(timed_out.unwrap().contains("timed out"));
}

fn chat(drive_new: bool) -> WatchingChat {
    WatchingChat {
        thread: "th-1".into(),
        drive_new,
        driver_turn: false,
        from_ask: false,
    }
}

#[tokio::test]
async fn a_session_launched_from_a_chat_is_watched_and_driven_by_default() {
    let (_dir, host) = fake_host(&[
        ("workspace create", CREATED.into(), None),
        ("agent start", OK.into(), None),
        ("agent get", agent_json("hs-t3", "idle"), None),
    ]);
    let db = Arc::new(Database::open_memory().unwrap());
    let vault = tempfile::tempdir().unwrap();
    let web = chat(true);
    let l = Launch {
        watch: Some(web.new_watch(&json!({}))),
        goal: goal_text(),
        ..launch(host, "hs-t3", None)
    };

    launch_session(vault.path(), &db, &harness("claude-code"), l)
        .await
        .unwrap();

    assert!(with_watch_state(&db, "hs-t3", json!({}))["drive"] == true);
    let row = get_row(&db, "hs-t3").unwrap();
    assert_eq!(row.owner_thread.as_deref(), Some("th-1"));
    assert!(row.drive);
    assert_eq!(row.goal.as_deref(), Some(GOAL));
    assert_eq!(row.done_criteria.as_deref(), Some(DONE));
}

#[tokio::test]
async fn a_session_launched_without_a_usable_goal_is_watched_but_not_driven() {
    let (_dir, host) = fake_host(&[
        ("workspace create", CREATED.into(), None),
        ("agent start", OK.into(), None),
        ("agent get", agent_json("hs-t5", "idle"), None),
    ]);
    let db = Arc::new(Database::open_memory().unwrap());
    let vault = tempfile::tempdir().unwrap();
    let web = chat(true);
    let vague = GoalText {
        goal: Some(GOAL),
        done_criteria: Some("when done"),
    };
    let l = Launch {
        watch: Some(web.new_watch(&json!({}))),
        goal: vague,
        ..launch(host, "hs-t5", None)
    };

    launch_session(vault.path(), &db, &harness("claude-code"), l)
        .await
        .unwrap();

    let state = with_watch_state(&db, "hs-t5", json!({}));
    assert_eq!(state["drive"], false);
    assert_eq!(state["mode"], "observe");
    assert!(
        state["drive_blocked_by"][0]
            .as_str()
            .unwrap()
            .contains("definition of done"),
        "{state}"
    );
    assert_eq!(
        get_row(&db, "hs-t5").unwrap().owner_thread.as_deref(),
        Some("th-1"),
        "still watched"
    );
}

/// A watched, running session row on the fake local host.
fn seed_watched(db: &Arc<Database>, id: &str, goal: Option<GoalText<'_>>) {
    db.with_conn(|c| {
        registry::insert(
            c,
            &registry::NewSession {
                id,
                harness: "pi",
                label: "",
                cwd: "/t",
                mission_id: None,
                placement: Placement {
                    host: "local",
                    agent_name: id,
                    workspace_id: "w1",
                    pane_id: "w1:p1",
                },
                purpose: None,
            },
        )?;
        registry::watch_from_chat(c, id, "th-1", false)?;
        if let Some(g) = goal {
            registry::set_goal(c, id, g.goal, g.done_criteria, registry::ACTOR_USER)?;
        }
        Ok(())
    })
    .unwrap();
}

fn mode_req(drive: bool) -> ModeRequest<'static> {
    ModeRequest {
        thread: "th-1",
        drive,
        untrusted: false,
        actor: registry::ACTOR_HQ,
    }
}

fn alive(name: &str) -> Liveness {
    Liveness::Alive(Box::new(AgentInfo {
        name: Some(name.into()),
        kind: "pi".into(),
        status: AgentStatus::Working,
        pane_id: "w1:p1".into(),
        workspace_id: "w1".into(),
        cwd: "/t".into(),
        title: None,
        state_change_seq: 1,
        launch_pending: false,
        agent_session_id: None,
        exited: false,
        exit_code: None,
        rule: None,
        pending: None,
        pending_seq: None,
    }))
}

#[test]
fn drive_toggles_mid_chat_and_never_touches_the_agent() {
    let db = Arc::new(Database::open_memory().unwrap());
    seed_watched(&db, "hs-m", None);
    let row = get_row(&db, "hs-m").unwrap();

    let refused = set_mode_with(&db, &row, &alive("hs-m"), mode_req(true)).unwrap();
    assert_eq!(refused["mode"], "observe");
    assert!(
        refused["drive_blocked_by"][0]
            .as_str()
            .unwrap()
            .contains("goal is missing"),
        "{refused}"
    );
    assert!(
        !get_row(&db, "hs-m").unwrap().drive,
        "the gate failed, so HQ stays observing"
    );

    set_goal(&db, "hs-m", goal_text(), registry::ACTOR_HQ).unwrap();
    let driving = set_mode_with(
        &db,
        &get_row(&db, "hs-m").unwrap(),
        &alive("hs-m"),
        mode_req(true),
    )
    .unwrap();
    assert_eq!(driving["mode"], "drive");
    assert!(get_row(&db, "hs-m").unwrap().drive);

    let observing = set_mode_with(
        &db,
        &get_row(&db, "hs-m").unwrap(),
        &alive("hs-m"),
        mode_req(false),
    )
    .unwrap();
    assert_eq!(observing["mode"], "observe");
    assert!(observing["agent_untouched"].is_string());
    assert!(
        !get_row(&db, "hs-m").unwrap().drive,
        "drive off takes effect at once"
    );
    assert_eq!(
        get_row(&db, "hs-m").unwrap().status,
        registry::STATUS_RUNNING,
        "the agent's session is untouched"
    );

    let events = db
        .with_conn(|c| registry::list_events(c, "hs-m", 20))
        .unwrap();
    let kinds: Vec<&str> = events.iter().map(|e| e.kind.as_str()).collect();
    assert_eq!(
        kinds,
        ["drive_refused", "goal_set", "drive_on", "drive_off"]
    );
    assert_eq!(
        events[2].goal.as_deref(),
        Some(GOAL),
        "the audit record carries the goal"
    );
}

#[test]
fn drive_is_refused_for_a_gone_session_an_unreachable_host_and_an_untrusted_turn() {
    let db = Arc::new(Database::open_memory().unwrap());
    seed_watched(&db, "hs-x", Some(goal_text()));
    let row = get_row(&db, "hs-x").unwrap();

    let gone = set_mode_with(&db, &row, &Liveness::Gone, mode_req(true)).unwrap_err();
    assert!(gone.to_string().contains("has ended"), "{gone}");
    let away = set_mode_with(
        &db,
        &row,
        &Liveness::HostUnreachable("no route".into()),
        mode_req(true),
    )
    .unwrap_err();
    assert!(
        away.to_string().contains("unreachable") && away.to_string().contains("not enabled"),
        "{away}"
    );
    let tainted = ModeRequest {
        untrusted: true,
        ..mode_req(true)
    };
    assert!(set_mode_with(&db, &row, &alive("hs-x"), tainted).is_err());
    assert!(
        !get_row(&db, "hs-x").unwrap().drive,
        "nothing pretended success"
    );

    let elsewhere = ModeRequest {
        thread: "th-2",
        ..mode_req(false)
    };
    let other_chat = set_mode_with(&db, &row, &alive("hs-x"), elsewhere).unwrap_err();
    assert!(
        other_chat.to_string().contains("not watched by this chat"),
        "{other_chat}"
    );
}

#[test]
fn observe_still_works_when_the_host_cannot_be_reached() {
    let db = Arc::new(Database::open_memory().unwrap());
    seed_watched(&db, "hs-o", Some(goal_text()));
    set_mode_with(
        &db,
        &get_row(&db, "hs-o").unwrap(),
        &alive("hs-o"),
        mode_req(true),
    )
    .unwrap();

    let report = set_mode_with(
        &db,
        &get_row(&db, "hs-o").unwrap(),
        &Liveness::HostUnreachable("no route".into()),
        mode_req(false),
    )
    .unwrap();

    assert_eq!(report["mode"], "observe");
    assert_eq!(
        report["agent"]["state"], "unknown",
        "it does not claim to know the agent's state"
    );
    assert!(!get_row(&db, "hs-o").unwrap().drive);
}

#[test]
fn an_ended_session_drops_out_of_drive_and_a_resume_keeps_its_goal() {
    let db = Arc::new(Database::open_memory().unwrap());
    seed_watched(&db, "hs-r", Some(goal_text()));
    db.with_conn(|c| registry::set_status(c, "hs-r", registry::STATUS_EXITED))
        .unwrap();
    let ended = set_mode_with(
        &db,
        &get_row(&db, "hs-r").unwrap(),
        &alive("hs-r"),
        mode_req(true),
    )
    .unwrap();
    assert_eq!(
        ended["mode"], "observe",
        "a stale row is not driven even if a same-named agent shows up"
    );

    db.with_conn(|c| {
        registry::relaunch(
            c,
            "hs-r",
            &Placement {
                host: "local",
                agent_name: "hs-r",
                workspace_id: "w2",
                pane_id: "w2:p1",
            },
        )
    })
    .unwrap();
    let back = set_mode_with(
        &db,
        &get_row(&db, "hs-r").unwrap(),
        &alive("hs-r"),
        mode_req(true),
    )
    .unwrap();
    assert_eq!(back["mode"], "drive");
    assert_eq!(get_row(&db, "hs-r").unwrap().goal.as_deref(), Some(GOAL));
}

#[test]
fn changing_a_goal_to_something_vague_switches_a_driven_session_to_observing() {
    let db = Arc::new(Database::open_memory().unwrap());
    seed_watched(&db, "hs-v", Some(goal_text()));
    set_mode_with(
        &db,
        &get_row(&db, "hs-v").unwrap(),
        &alive("hs-v"),
        mode_req(true),
    )
    .unwrap();

    let report = set_goal(
        &db,
        "hs-v",
        GoalText {
            goal: None,
            done_criteria: Some("TBD"),
        },
        registry::ACTOR_USER,
    )
    .unwrap();

    assert_eq!(report["mode"], "observe");
    assert!(
        report["drive_stopped"].is_string() && report["drive_blocked_by"].is_array(),
        "{report}"
    );
    assert!(
        set_goal(&db, "hs-v", GoalText::default(), registry::ACTOR_USER).is_err(),
        "nothing to set"
    );
    assert!(set_goal(&db, "hs-none", goal_text(), registry::ACTOR_USER).is_err());
}

const HAND_STARTED: &str = r#"{"id":"x","result":{"type":"agent_info","agent":{"agent":"claude","agent_status":"idle","cwd":"/work/app","name":"by-hand","pane_id":"w3:p1","workspace_id":"w3","tab_id":"w3:t1","state_change_seq":4}}}"#;
const NOT_FOUND: &str =
    r#"{"error":{"code":"agent_not_found","message":"agent target nope not found"},"id":"x"}"#;

#[test]
fn attach_tracks_a_hand_started_agent_observation_only_and_is_idempotent() {
    let (_dir, host) = fake_host(&[("agent get", HAND_STARTED.into(), None)]);
    let db = Arc::new(Database::open_memory().unwrap());

    let first = attach(&db, &host, "by-hand", "th-1").unwrap();
    let id = first["session_id"].as_str().unwrap().to_string();
    assert_eq!(first["mode"], "observe");
    assert_eq!(first["already_tracked"], false);
    let row = get_row(&db, &id).unwrap();
    assert_eq!(
        (
            row.harness.as_str(),
            row.agent_name.as_str(),
            row.cwd.as_str()
        ),
        ("claude-code", "by-hand", "/work/app")
    );
    assert_eq!(row.owner_thread.as_deref(), Some("th-1"));
    assert!(!row.drive);

    let again = attach(&db, &host, "by-hand", "th-2").unwrap();
    assert_eq!(again["session_id"], id.as_str());
    assert_eq!(again["already_tracked"], true);
    assert_eq!(
        get_row(&db, &id).unwrap().owner_thread.as_deref(),
        Some("th-2"),
        "another chat can take it over"
    );
    let kinds: Vec<String> = db
        .with_conn(|c| registry::list_events(c, &id, 10))
        .unwrap()
        .into_iter()
        .map(|e| e.kind)
        .collect();
    assert_eq!(kinds, ["attached", "attached"]);
}

#[test]
fn attach_reports_a_missing_agent_and_an_unreachable_host_instead_of_pretending() {
    let db = Arc::new(Database::open_memory().unwrap());
    let (_dir, gone) = fake_host(&[("agent get", String::new(), Some(NOT_FOUND))]);
    let err = attach(&db, &gone, "nope", "th-1").unwrap_err();
    assert!(err.to_string().contains("no agent 'nope'"), "{err}");

    let (_dir, away) = fake_host(&[(
        "agent get",
        String::new(),
        Some("ssh: connect to host laptop: connection refused"),
    )]);
    let err = attach(&db, &away, "by-hand", "th-1").unwrap_err();
    assert!(err.to_string().contains("cannot reach host"), "{err}");
    assert!(
        db.with_conn(|c| registry::list(c, None, 10))
            .unwrap()
            .is_empty(),
        "nothing was recorded"
    );
}

#[test]
fn a_new_watch_drives_unless_opted_out_untrusted_or_started_by_the_driver() {
    let on = chat(true);
    assert!(on.new_watch(&json!({})).drive);
    assert!(!on.new_watch(&json!({"drive": false})).drive);
    assert!(on.new_watch(&json!({"drive": false})).opted_out);
    let untrusted = on.new_watch(&json!({ UNTRUSTED_TURN_ARG: true }));
    assert!(
        !untrusted.drive && !untrusted.opted_out,
        "an untrusted turn loses the default but stops nothing"
    );
    assert!(
        !chat(false).new_watch(&json!({})).drive,
        "driver turns and drive_new_watches: false"
    );
}

#[tokio::test]
async fn the_watch_tool_defaults_drive_on_and_never_turns_it_back_on() {
    let db = Arc::new(Database::open_memory().unwrap());
    db.with_conn(|c| {
        for id in ["hs-a", "hs-b"] {
            registry::insert(
                c,
                &registry::NewSession {
                    id,
                    harness: "pi",
                    label: "",
                    cwd: "/t",
                    mission_id: None,
                    placement: Placement {
                        host: "local",
                        agent_name: id,
                        workspace_id: "w1",
                        pane_id: "w1:p1",
                    },
                    purpose: None,
                },
            )?;
            registry::set_goal(c, id, Some(GOAL), Some(DONE), registry::ACTOR_USER)?;
        }
        Ok(())
    })
    .unwrap();
    let tools = tools::create_harness_session_tools(
        std::path::PathBuf::from("/tmp"),
        db.clone(),
        Some(chat(true)),
        None,
    );
    let watch_tool = tools
        .iter()
        .find(|t| t.name() == "harness_session_watch")
        .unwrap();
    let drive = |id: &str| get_row(&db, id).unwrap().drive;

    watch_tool
        .execute(json!({"session_id": "hs-a"}))
        .await
        .unwrap();
    assert!(drive("hs-a"));
    watch_tool
        .execute(json!({"session_id": "hs-a", "drive": false}))
        .await
        .unwrap();
    assert!(!drive("hs-a"), "drive=false stops driving");
    watch_tool
        .execute(json!({"session_id": "hs-a"}))
        .await
        .unwrap();
    assert!(!drive("hs-a"), "watching again keeps the switch off");
    let refused = watch_tool
        .execute(json!({"session_id": "hs-a", "drive": true}))
        .await;
    assert!(refused.is_err());
    assert!(!drive("hs-a"));

    watch_tool
        .execute(json!({"session_id": "hs-b", UNTRUSTED_TURN_ARG: true}))
        .await
        .unwrap();
    assert!(
        !drive("hs-b"),
        "an untrusted turn's watch starts with Drive off"
    );
}

#[test]
fn linking_from_a_chat_reports_whether_it_drives() {
    let db = Arc::new(Database::open_memory().unwrap());
    let task = db
        .with_conn(|c| {
            c.execute(
                "INSERT INTO initiatives (id, space_id, name, slug, id_prefix) VALUES ('in-1', 'personal', 'Work', 'work', 'FR')",
                [],
            )?;
            let new = hq_db::tasks::NewTask { title: "t", created_by: "test", ..Default::default() };
            let id = hq_db::tasks::create_task(c, "tk-1", "in-1", &new)?.id;
            registry::insert(
                c,
                &registry::NewSession {
                    id: "hs-l",
                    harness: "pi",
                    label: "",
                    cwd: "/t",
                    mission_id: None,
                    placement: Placement { host: "local", agent_name: "hs-l", workspace_id: "w1", pane_id: "w1:p1" },
                    purpose: None,
                },
            )?;
            Ok(id)
        })
        .unwrap();
    let web = chat(true);
    let linked = link(
        &db,
        "hs-l",
        &task,
        Some(web.new_watch(&json!({ UNTRUSTED_TURN_ARG: true }))),
    )
    .unwrap();
    assert_eq!(linked["watched_by_thread"], "th-1");
    assert_eq!(
        linked["drive"], false,
        "the result says Drive is off, so the model cannot claim otherwise"
    );
}

#[test]
fn a_launch_needs_a_project_directory_not_home_or_root() {
    assert!(require_cwd(None).is_err());
    assert!(require_cwd(Some("  ")).is_err());
    assert!(require_cwd(Some("/")).is_err());
    let home = dirs::home_dir().expect("home dir");
    assert!(require_cwd(home.to_str()).is_err());
    assert_eq!(
        require_cwd(Some("/work/project")).unwrap(),
        std::path::PathBuf::from("/work/project")
    );
}

fn deny(entries: &[&str]) -> Vec<String> {
    entries.iter().map(|e| e.to_string()).collect()
}

#[test]
fn an_empty_deny_list_refuses_nothing() {
    assert!(check_cwd_allowed("/work/anything", &[]).is_ok());
    assert!(
        check_cwd_allowed("/work/anything", &deny(&["", "  "])).is_ok(),
        "blank entries match nothing"
    );
}

#[test]
fn a_denied_substring_is_refused_with_a_clear_error() {
    let list = deny(&["/clients/acme"]);
    let err = check_cwd_allowed("/home/me/clients/acme/app", &list)
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("agent_host.spawn_cwd_deny") && err.contains("/clients/acme"),
        "{err}"
    );
    assert!(err.contains("no session was started"), "{err}");
    assert!(check_cwd_allowed("/home/me/clients/other", &list).is_ok());
}

#[test]
fn the_deny_list_sees_through_case_and_dot_dot() {
    let list = deny(&["/Clients/Acme"]);
    assert!(check_cwd_allowed("/srv/clients/acme", &list).is_err());
    assert!(check_cwd_allowed("/srv/ok/../clients/acme/x", &list).is_err());
    assert!(check_cwd_allowed("/srv/./clients/./acme", &list).is_err());
    assert!(
        check_cwd_allowed("/srv/clients/acme/../other", &list).is_ok(),
        "the path resolves outside it"
    );
}

#[test]
fn a_cwd_must_be_a_literal_absolute_path() {
    for bad in [
        "relative/dir",
        "./here",
        "~/project",
        "/work/$HOME/x",
        "/work/`id`",
        "/work/a\0b",
        "/safe/link/../x",
        "/work/..",
    ] {
        assert!(require_cwd(Some(bad)).is_err(), "{bad:?} should be refused");
    }
    assert!(require_cwd(Some("/work/a.b/c-d")).is_ok());
}

#[test]
fn the_deny_list_ignores_trailing_and_doubled_slashes_in_entries_and_paths() {
    let list = deny(&["/clients/acme/", "//Private//"]);
    assert!(check_cwd_allowed("/srv/clients/acme", &list).is_err());
    assert!(check_cwd_allowed("/srv/clients/acme/app", &list).is_err());
    assert!(check_cwd_allowed("/srv//clients///acme", &list).is_err());
    assert!(check_cwd_allowed("/srv/private/x", &list).is_err());
    assert!(check_cwd_allowed("/srv/clients/other", &list).is_ok());
    assert!(
        check_cwd_allowed("/srv/x", &deny(&["/", "//"])).is_ok(),
        "an entry that normalizes to nothing matches nothing"
    );
}

#[test]
fn a_connector_may_use_the_hosts_hq_folder_without_any_list() {
    let owner = hq_core::config::AgentHostConfig::default();
    let bounded = connector_folders(&owner, Some("/home/user/Documents/HQ"));
    let ok = |p: &str| require_cwd_in(Some(p), &bounded, CwdBound::Connector).is_ok();
    assert!(ok("/home/user/Documents/HQ/app"));
    assert!(
        !ok("/home/user/Documents/HQ-other"),
        "a sibling with the same prefix is not inside"
    );
    assert!(
        !ok("/home/user/Documents/HQ/../../.ssh"),
        "dot-dot does not climb out"
    );
    assert!(!ok("/etc"));
    assert!(
        owner.handoff_cwd_allow.is_empty(),
        "the owner's config is not changed"
    );
    let none = connector_folders(&owner, None);
    assert!(require_cwd_in(Some("/srv/app"), &none, CwdBound::Connector).is_err());
    let blank = connector_folders(&owner, Some("  "));
    assert!(require_cwd_in(Some("/srv/app"), &blank, CwdBound::Connector).is_err());
}

#[test]
fn a_connector_starts_sessions_only_in_listed_folders_and_none_while_the_list_is_empty() {
    let mut host_cfg = hq_core::config::AgentHostConfig::default();
    let refused = require_cwd_in(Some("/srv/work/app"), &host_cfg, CwdBound::Connector)
        .unwrap_err()
        .to_string();
    assert!(refused.contains("handoff_cwd_allow"), "{refused}");
    host_cfg.handoff_cwd_allow = deny(&["/srv/work"]);
    assert!(require_cwd_in(Some("/srv/work/app"), &host_cfg, CwdBound::Connector).is_ok());
    assert!(require_cwd_in(Some("/etc"), &host_cfg, CwdBound::Connector).is_err());
    let marked = json!({ CONNECTOR_ARG: "Claude", HANDOFF_SCOPE_ARG: true });
    assert_eq!(
        CwdBound::of(&marked),
        CwdBound::Connector,
        "the stricter bound wins"
    );
    assert_eq!(
        CwdBound::of(&json!({ HANDOFF_SCOPE_ARG: true })),
        CwdBound::Handoff
    );
    assert_eq!(CwdBound::of(&json!({})), CwdBound::Any);
}

#[test]
fn the_handoff_allow_list_binds_only_the_handoff_scope() {
    let mut host_cfg = hq_core::config::AgentHostConfig {
        handoff_cwd_allow: deny(&["/srv/work"]),
        ..Default::default()
    };
    let inside = Some("/srv/work/app");
    assert!(require_cwd_in(inside, &host_cfg, CwdBound::Handoff).is_ok());
    assert!(require_cwd_in(Some("/srv/work"), &host_cfg, CwdBound::Handoff).is_ok());
    assert!(
        require_cwd_in(Some("/srv/worker"), &host_cfg, CwdBound::Handoff).is_err(),
        "a sibling that shares a prefix is outside"
    );
    assert!(require_cwd_in(Some("/etc"), &host_cfg, CwdBound::Handoff).is_err());
    assert!(
        require_cwd_in(Some("/etc"), &host_cfg, CwdBound::Any).is_ok(),
        "other keys are not bound by it"
    );
    host_cfg.handoff_cwd_allow.clear();
    assert!(
        require_cwd_in(Some("/etc"), &host_cfg, CwdBound::Handoff).is_ok(),
        "empty means unrestricted"
    );
    host_cfg.spawn_cwd_deny = deny(&["/etc"]);
    assert!(
        require_cwd_in(Some("/etc"), &host_cfg, CwdBound::Handoff).is_err(),
        "the deny list still applies"
    );
}

#[test]
fn only_logical_key_names_reach_the_host() {
    let ok = |k: &[&str]| {
        crate::agent_host::validate_keys(&k.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    };
    assert!(ok(&["enter", "ctrl+c", "down", "f5", "shift+tab", "y"]).is_ok());
    for bad in [
        "-rf",
        "--help",
        "",
        " enter",
        "a b",
        "ctrl+c;ls",
        "$(id)",
        &"k".repeat(33),
    ] {
        assert!(ok(&[bad]).is_err(), "{bad:?} should be refused");
    }
}

fn handoff_req(external_id: &str) -> handoff::HandoffRequest {
    handoff::HandoffRequest {
        title: "Add rate limiting".into(),
        description: "Limit login attempts".into(),
        acceptance: DONE.into(),
        harness: "claude-code".into(),
        cwd: "/t".into(),
        external_id: external_id.into(),
        drive_new: true,
        ..Default::default()
    }
}

/// A task to launch work on, in a fresh database.
fn a_task(db: &Database) -> String {
    db.with_conn(|c| {
        c.execute(
            "INSERT OR IGNORE INTO initiatives (id, space_id, name, slug, id_prefix) VALUES ('in-1', 'personal', 'Work', 'work', 'FR')",
            [],
        )?;
        let new = hq_db::tasks::NewTask { title: "watch the refactor", created_by: "test", ..Default::default() };
        Ok(hq_db::tasks::create_task(c, "tk-watch", "in-1", &new)?.id)
    })
    .unwrap()
}

fn spawn_req<'a>(
    harness: &'a str,
    purpose: Option<Purpose>,
    task: Option<&'a str>,
    observe: &'a [String],
) -> SpawnRequest<'a> {
    SpawnRequest {
        host: None,
        harness,
        prompt: None,
        cwd: std::path::Path::new("/t"),
        label: "",
        mission_id: task,
        watch: None,
        parent: None,
        goal: GoalText::default(),
        purpose,
        observe,
        model: None,
        worktree: None,
        owns: &[],
        policy: None,
    }
}

#[tokio::test]
async fn a_general_purpose_agent_is_never_started_for_code_work() {
    let (dir, host) = startable_host();
    let db = Arc::new(Database::open_memory().unwrap());
    let vault = tempfile::tempdir().unwrap();
    let err = spawn_on(
        vault.path(),
        &db,
        Arc::new(host),
        spawn_req("hermes", Some(Purpose::Code), None, &[]),
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(err.contains("'hermes' is a general-purpose agent"), "{err}");
    assert!(
        err.contains("claude-code"),
        "it says where code goes: {err}"
    );
    assert!(
        !calls(&dir).contains("workspace create"),
        "refused before anything started"
    );
    assert_eq!(count(&db, "SELECT COUNT(*) FROM harness_sessions"), 0);
}

#[tokio::test]
async fn a_general_purpose_agent_defaults_to_general_and_a_coding_agent_to_code() {
    let (dir, host) = startable_host();
    let db = Arc::new(Database::open_memory().unwrap());
    let vault = tempfile::tempdir().unwrap();
    let report = spawn_on(
        vault.path(),
        &db,
        Arc::new(host),
        spawn_req("hermes", None, None, &[]),
    )
    .await
    .unwrap();
    assert_eq!(report["category"], "general_purpose");
    assert_eq!(report["purpose"], "general");
    let id = report["session_id"].as_str().unwrap();
    assert_eq!(get_row(&db, id).unwrap().purpose, "general");
    assert!(report.get("observing").is_none());
    let log = calls(&dir);
    assert!(
        log.contains("agent start") && log.contains("hermes") && log.contains("chat"),
        "launched as `hermes chat`: {log}"
    );

    let (_dir, host) = startable_host();
    let report = spawn_on(
        vault.path(),
        &db,
        Arc::new(host),
        spawn_req("claude-code", None, None, &[]),
    )
    .await
    .unwrap();
    assert_eq!(report["category"], "coding");
    assert_eq!(report["purpose"], "code");
}

#[tokio::test]
async fn only_a_monitor_or_reviewer_on_a_task_can_be_given_sessions_to_observe() {
    let db = Arc::new(Database::open_memory().unwrap());
    let vault = tempfile::tempdir().unwrap();
    let task = a_task(&db);
    let watched = vec!["hs-coder".to_string()];

    let (dir, host) = startable_host();
    let err = spawn_on(
        vault.path(),
        &db,
        Arc::new(host),
        spawn_req("hermes", Some(Purpose::Research), Some(&task), &watched),
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(
        err.contains("only a session with purpose monitor or review"),
        "{err}"
    );
    assert!(!calls(&dir).contains("workspace create"));

    let (dir, host) = startable_host();
    let err = spawn_on(
        vault.path(),
        &db,
        Arc::new(host),
        spawn_req("hermes", Some(Purpose::Monitor), None, &watched),
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(err.contains("needs a task_id"), "{err}");
    assert!(!calls(&dir).contains("workspace create"));

    let too_many: Vec<String> = (0..11).map(|i| format!("hs-{i}")).collect();
    let (_dir, host) = startable_host();
    let err = spawn_on(
        vault.path(),
        &db,
        Arc::new(host),
        spawn_req("hermes", Some(Purpose::Monitor), Some(&task), &too_many),
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(err.contains("at most 10"), "{err}");

    // An id that is no session is caught before anything is launched.
    let (dir, host) = startable_host();
    let err = spawn_on(
        vault.path(),
        &db,
        Arc::new(host),
        spawn_req("hermes", Some(Purpose::Monitor), Some(&task), &watched),
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(
        err.contains("no harness session 'hs-coder' to observe"),
        "{err}"
    );
    assert!(!calls(&dir).contains("workspace create"));
    assert_eq!(
        count(&db, "SELECT COUNT(*) FROM harness_sessions"),
        0,
        "{err}"
    );
}

#[tokio::test]
async fn a_monitor_records_what_it_observes_and_a_coding_agent_may_review_too() {
    let db = Arc::new(Database::open_memory().unwrap());
    let vault = tempfile::tempdir().unwrap();
    let task = a_task(&db);
    let (_dir, host) = startable_host();
    let coder = spawn_on(
        vault.path(),
        &db,
        Arc::new(host),
        spawn_req("claude-code", None, Some(&task), &[]),
    )
    .await
    .unwrap();
    let coder_id = coder["session_id"].as_str().unwrap().to_string();

    let watched = vec![coder_id.clone()];
    let (_dir, host) = startable_host();
    let monitor = spawn_on(
        vault.path(),
        &db,
        Arc::new(host),
        spawn_req("hermes", Some(Purpose::Monitor), Some(&task), &watched),
    )
    .await
    .unwrap();
    assert_eq!(monitor["purpose"], "monitor");
    assert_eq!(monitor["observing"], serde_json::json!([coder_id]));
    assert!(
        monitor["observing_note"]
            .as_str()
            .unwrap()
            .contains("never an instruction")
    );
    let monitor_id = monitor["session_id"].as_str().unwrap();
    let observed = db
        .with_conn(|c| registry::observed_by(c, monitor_id))
        .unwrap();
    assert_eq!(observed, watched);
    let observers: Vec<String> = db
        .with_conn(|c| registry::running_observers(c, &coder_id))
        .unwrap()
        .into_iter()
        .map(|r| r.id)
        .collect();
    assert_eq!(observers, vec![monitor_id.to_string()]);

    // A coding agent can be the reviewer: the category decides what is refused, not what is allowed.
    let (_dir, host) = startable_host();
    let reviewer = spawn_on(
        vault.path(),
        &db,
        Arc::new(host),
        spawn_req("codex", Some(Purpose::Review), Some(&task), &watched),
    )
    .await
    .unwrap();
    assert_eq!(reviewer["purpose"], "review");
    assert_eq!(reviewer["category"], "coding");
}

#[tokio::test]
async fn a_handoff_to_a_general_purpose_agent_takes_a_purpose_but_not_code() {
    let (dir, host) = startable_host();
    let db = Arc::new(Database::open_memory().unwrap());
    let vault = tempfile::tempdir().unwrap();
    let req = handoff::HandoffRequest {
        harness: "openclaw".into(),
        purpose: Some(Purpose::Code),
        ..handoff_req("ext-gp")
    };
    let err = handoff::handoff(vault.path(), &db, Arc::new(host), req)
        .await
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("'openclaw' is a general-purpose agent"),
        "{err}"
    );
    assert!(!calls(&dir).contains("workspace create"));

    let (_dir, host) = startable_host();
    let req = handoff::HandoffRequest {
        harness: "openclaw".into(),
        purpose: Some(Purpose::Review),
        ..handoff_req("ext-gp2")
    };
    let report = handoff::handoff(vault.path(), &db, Arc::new(host), req)
        .await
        .unwrap();
    assert_eq!(report["handoff"], "started");
    let row = get_row(&db, report["session_id"].as_str().unwrap()).unwrap();
    assert_eq!(row.purpose, "review");
    // A reviewer never moves the task: the handoff's own launch left it where it was.
    assert_eq!(report["task"]["status"], "to_do", "{report}");
}

fn startable_host() -> (tempfile::TempDir, ScriptedHost) {
    fake_host(&[
        ("workspace create", CREATED.into(), None),
        ("agent start", OK.into(), None),
        ("agent get", agent_json("hs-h", "idle"), None),
        ("agent prompt", OK.into(), None),
    ])
}

fn live_list(session_id: &str) -> String {
    format!(
        r#"{{"id":"x","result":{{"agents":[{{"agent":"claude","agent_status":"idle","cwd":"/t","name":"{session_id}","pane_id":"w9:p1","workspace_id":"w9"}}]}}}}"#
    )
}

fn count(db: &Database, sql: &str) -> i64 {
    db.with_conn(|c| Ok(c.query_row(sql, [], |r| r.get(0))?))
        .unwrap()
}

#[tokio::test]
async fn a_handoff_files_a_task_starts_a_session_and_gives_a_thread_that_owns_it() {
    let (dir, host) = startable_host();
    let db = Arc::new(Database::open_memory().unwrap());
    let vault = tempfile::tempdir().unwrap();

    let report = handoff::handoff(vault.path(), &db, Arc::new(host), handoff_req("ext-42"))
        .await
        .unwrap();

    assert_eq!(report["handoff"], "started");
    assert_eq!(report["task"]["created"], true);
    assert_eq!(report["task"]["status"], "in_progress");
    let task_id = report["task"]["id"].as_str().unwrap();
    let session_id = report["session_id"].as_str().unwrap();
    let thread = report["thread_id"].as_str().unwrap();
    assert_eq!(report["links"]["chat"], format!("/chat?thread={thread}"));
    assert_eq!(report["links"]["task"], format!("/tasks?task={task_id}"));

    let row = get_row(&db, session_id).unwrap();
    assert_eq!(row.mission_id.as_deref(), Some(task_id));
    assert_eq!(
        row.owner_thread.as_deref(),
        Some(thread),
        "the thread owns the session, so Drive and notifications route to it"
    );
    assert_eq!(row.done_criteria.as_deref(), Some(DONE));
    assert!(row.goal.as_deref().unwrap().contains("Add rate limiting"));
    assert!(
        row.drive,
        "goal and acceptance pass the drive gate, so HQ drives"
    );
    let task = db
        .with_conn(|c| hq_db::tasks::get_task(c, task_id))
        .unwrap()
        .unwrap();
    assert_eq!(task.external_id.as_deref(), Some("ext-42"));
    assert!(
        task.description.contains(DONE),
        "the acceptance criteria are kept on the task"
    );
    let log = calls(&dir);
    assert!(
        log.contains("agent prompt") && log.contains("Add rate limiting"),
        "{log}"
    );
}

#[tokio::test]
async fn repeating_a_handoff_returns_the_same_task_and_session_without_a_second_launch() {
    let (_dir, host) = startable_host();
    let db = Arc::new(Database::open_memory().unwrap());
    let vault = tempfile::tempdir().unwrap();
    let first = handoff::handoff(vault.path(), &db, Arc::new(host), handoff_req("ext-7"))
        .await
        .unwrap();
    let session_id = first["session_id"].as_str().unwrap().to_string();

    let (dir, host) = fake_host(&[("agent list", live_list(&session_id), None)]);
    let again = handoff::handoff(vault.path(), &db, Arc::new(host), handoff_req("ext-7"))
        .await
        .unwrap();

    assert_eq!(again["handoff"], "existing_session");
    assert_eq!(again["task"]["id"], first["task"]["id"]);
    assert_eq!(again["task"]["deduplicated"], true);
    assert_eq!(again["session_id"], first["session_id"]);
    assert_eq!(again["thread_id"], first["thread_id"]);
    assert!(
        !calls(&dir).contains("workspace create"),
        "no second launch"
    );
    assert_eq!(count(&db, "SELECT COUNT(*) FROM harness_sessions"), 1);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM tasks"), 1);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM chat_threads"), 1);
}

#[tokio::test]
async fn a_session_marked_running_that_its_host_no_longer_has_does_not_block_a_new_one() {
    let (_dir, host) = startable_host();
    let db = Arc::new(Database::open_memory().unwrap());
    let vault = tempfile::tempdir().unwrap();
    handoff::handoff(vault.path(), &db, Arc::new(host), handoff_req("ext-8"))
        .await
        .unwrap();

    let empty = r#"{"id":"x","result":{"agents":[]}}"#.to_string();
    let (_dir, host) = fake_host(&[
        ("agent list", empty, None),
        ("workspace create", CREATED.into(), None),
        ("agent start", OK.into(), None),
        ("agent get", agent_json("hs-h", "idle"), None),
        ("agent prompt", OK.into(), None),
    ]);
    let again = handoff::handoff(vault.path(), &db, Arc::new(host), handoff_req("ext-8"))
        .await
        .unwrap();

    assert_eq!(again["handoff"], "started");
    assert_eq!(
        again["task"]["deduplicated"], true,
        "same task, new session"
    );
    assert_eq!(count(&db, "SELECT COUNT(*) FROM harness_sessions"), 2);
}

#[tokio::test]
async fn an_unreachable_host_is_reported_and_leaves_no_session_or_dangling_thread() {
    let (_dir, host) = fake_host(&[(
        "workspace create",
        String::new(),
        Some("ssh: connect timed out"),
    )]);
    let db = Arc::new(Database::open_memory().unwrap());
    let vault = tempfile::tempdir().unwrap();

    let err = handoff::handoff(vault.path(), &db, Arc::new(host), handoff_req("ext-9"))
        .await
        .unwrap_err()
        .to_string();

    assert!(
        err.contains("no session was started") && err.contains("timed out"),
        "{err}"
    );
    assert_eq!(count(&db, "SELECT COUNT(*) FROM harness_sessions"), 0);
    assert_eq!(
        count(
            &db,
            "SELECT COUNT(*) FROM chat_threads WHERE status = 'active'"
        ),
        0
    );
    assert_eq!(
        count(&db, "SELECT COUNT(*) FROM tasks"),
        1,
        "the task stays so a retry reuses it"
    );
    let status: String = db
        .with_conn(|c| Ok(c.query_row("SELECT status FROM tasks", [], |r| r.get(0))?))
        .unwrap();
    assert_eq!(status, "to_do", "a task nothing worked on is not started");
}

#[tokio::test]
async fn an_agent_stuck_at_a_dialog_is_reported_as_blocked_with_no_prompt_typed() {
    let (dir, host) = fake_host(&[
        ("workspace create", CREATED.into(), None),
        ("agent start", String::new(), Some(NOT_READY)),
        ("agent get", agent_json("hs-h", "blocked"), None),
        (
            "agent read",
            "Quick safety check\n No, exit\n Yes, I trust this folder".into(),
            None,
        ),
    ]);
    let db = Arc::new(Database::open_memory().unwrap());
    let vault = tempfile::tempdir().unwrap();

    let report = handoff::handoff(vault.path(), &db, Arc::new(host), handoff_req("ext-10"))
        .await
        .unwrap();

    assert_eq!(report["handoff"], "blocked_at_dialog");
    assert!(report["warning"].as_str().unwrap().contains("NOT typed"));
    assert!(
        report["session"]["blocked"]["screen"]
            .as_str()
            .unwrap()
            .contains("No, exit")
    );
    assert!(!calls(&dir).contains("agent prompt"));

    assert_eq!(
        report["task"]["status"], "blocked",
        "the board does not claim work is running"
    );
    let task_id = report["task"]["id"].as_str().unwrap();
    let comments: Vec<String> = db
        .with_conn(|c| hq_db::tasks::list_comments(c, task_id))
        .unwrap()
        .into_iter()
        .map(|c| c.body)
        .collect();
    assert!(
        comments
            .iter()
            .any(|c| c.contains("stopped at a startup dialog") && c.contains("not typed")),
        "{comments:?}"
    );
    let row = get_row(&db, report["session_id"].as_str().unwrap()).unwrap();
    assert_eq!(
        row.status,
        registry::STATUS_RUNNING,
        "the session stays registered so the dialog can be answered"
    );
    assert_eq!(
        row.owner_thread.as_deref(),
        report["thread_id"].as_str(),
        "the thread keeps watching"
    );
}

#[tokio::test]
async fn a_handoff_can_work_an_existing_task_and_refuses_ambiguous_or_empty_requests() {
    let (_dir, host) = startable_host();
    let db = Arc::new(Database::open_memory().unwrap());
    let vault = tempfile::tempdir().unwrap();
    let task = db
        .with_conn(|c| {
            c.execute(
                "INSERT INTO initiatives (id, space_id, name, slug, id_prefix) VALUES ('in-1', 'personal', 'Work', 'work', 'FR')",
                [],
            )?;
            let new = hq_db::tasks::NewTask { title: "existing", created_by: "test", ..Default::default() };
            Ok(hq_db::tasks::create_task(c, "tk-1", "in-1", &new)?.id)
        })
        .unwrap();

    let both = handoff::HandoffRequest {
        task_id: task.clone(),
        ..handoff_req("ext-11")
    };
    let err = handoff::handoff(vault.path(), &db, Arc::new(host.clone()), both)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("not both"), "{err}");
    let untitled = handoff::HandoffRequest {
        title: " ".into(),
        ..handoff_req("")
    };
    assert!(
        handoff::handoff(vault.path(), &db, Arc::new(host.clone()), untitled)
            .await
            .is_err()
    );
    assert_eq!(
        count(&db, "SELECT COUNT(*) FROM chat_threads"),
        0,
        "refused before anything is created"
    );

    let existing = handoff::HandoffRequest {
        task_id: "FR-001".into(),
        title: String::new(),
        ..handoff_req("")
    };
    let report = handoff::handoff(vault.path(), &db, Arc::new(host), existing)
        .await
        .unwrap();
    assert_eq!(report["task"]["id"], task);
    assert_eq!(report["task"]["created"], false);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM tasks"), 1);
}

const SHORT_BOUND: Duration = Duration::from_millis(1200);

/// A fake whose agent never leaves `launch_pending`, as when the harness binary
/// is missing on the machine the host runs on.
fn stuck_host() -> (tempfile::TempDir, ScriptedHost) {
    let (dir, host) = fake_host(&[
        ("workspace create", CREATED.into(), None),
        ("agent start", String::new(), Some(NOT_READY)),
        ("agent get", PENDING_AGENT.into(), None),
        ("agent read", "bash: claude: command not found".into(), None),
        ("workspace close", OK.into(), None),
    ]);
    (dir, host.with_launch_bound(SHORT_BOUND))
}

#[test]
fn a_cwd_shaped_like_any_users_home_is_refused_on_every_host() {
    for refused in [
        "/",
        "//",
        "/Users/alice",
        "/Users/alice/",
        "/Users//alice//",
        "/users/alice",
        "/home/alice",
        "/home/alice/",
        "/root",
        "/root/",
        "/var/root",
        "/var/root/",
        "/opt/hq",
        "/opt/hq/",
        "/Users",
        "/home",
    ] {
        let err = require_cwd(Some(refused)).unwrap_err().to_string();
        assert!(err.contains("home or root directory"), "{refused}: {err}");
    }
    for fine in [
        "/Users/alice/Documents/GitHub/agent-hq",
        "/home/alice/work",
        "/root/work",
        "/opt/hq/work",
        "/opt/other",
        "/var/lib/app",
        "/srv/home/alice",
    ] {
        assert!(require_cwd(Some(fine)).is_ok(), "{fine} should be allowed");
    }
}

#[test]
fn a_missing_binary_on_a_local_host_is_named_before_anything_launches() {
    let (dir, host) = fake_host_checking_binaries(&[]);
    let h = profile_harness(
        "  ghost:\n    base: claude-code\n    command: definitely-not-installed-xyz\n",
        "ghost",
    );
    let err = preflight::require_binary(&host, &h)
        .unwrap_err()
        .to_string();
    assert!(
        err.starts_with("harness 'ghost' is not installed on host 'local' (binary 'definitely-not-installed-xyz' not found on PATH; searched "),
        "{err}"
    );
    assert!(
        err.contains("/usr/local/bin"),
        "the search roots are named: {err}"
    );
    assert!(calls(&dir).is_empty(), "the host was never asked");

    let found = profile_harness(
        "  sh-alias:\n    base: claude-code\n    command: /bin/sh\n",
        "sh-alias",
    );
    assert!(preflight::require_binary(&host, &found).is_ok());
    let on_path = profile_harness(
        "  sh-path:\n    base: claude-code\n    command: sh -x\n",
        "sh-path",
    );
    assert!(
        preflight::require_binary(&host, &on_path).is_ok(),
        "only the first word is the binary"
    );
    let remote_like = fake_host(&[]).1;
    assert!(
        preflight::require_binary(&remote_like, &h).is_ok(),
        "a host that opts out is not checked"
    );
}

#[test]
fn a_profile_path_decides_where_the_binary_is_looked_for() {
    let (_dir, host) = fake_host_checking_binaries(&[]);
    let bin = tempfile::tempdir().unwrap();
    let tool = bin.path().join("my-wrapper");
    std::fs::write(&tool, "#!/bin/sh\n").unwrap();
    std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
    let yaml = format!(
        "  w:\n    base: claude-code\n    command: my-wrapper\n    env:\n      PATH: {}\n",
        bin.path().display()
    );
    assert!(preflight::require_binary(&host, &profile_harness(&yaml, "w")).is_ok());
}

#[tokio::test]
async fn a_launch_for_a_missing_binary_fails_fast_without_touching_the_host() {
    let (dir, host) = fake_host_checking_binaries(&[("workspace create", CREATED.into(), None)]);
    let db = Arc::new(Database::open_memory().unwrap());
    let vault = tempfile::tempdir().unwrap();
    let h = profile_harness(
        "  ghost:\n    base: claude-code\n    command: definitely-not-installed-xyz\n",
        "ghost",
    );

    let err = launch_session(vault.path(), &db, &h, launch(host, "hs-t6", Some("go")))
        .await
        .unwrap_err()
        .to_string();

    assert!(err.contains("not installed on host 'local'"), "{err}");
    assert!(!calls(&dir).contains("workspace create"), "{}", calls(&dir));
    assert!(get_row(&db, "hs-t6").is_err());
}

#[tokio::test]
async fn an_agent_that_never_leaves_launch_pending_is_cleaned_up_and_reported() {
    let (dir, host) = stuck_host();
    let db = Arc::new(Database::open_memory().unwrap());
    let vault = tempfile::tempdir().unwrap();
    let started = std::time::Instant::now();

    let err = launch_session(
        vault.path(),
        &db,
        &harness("claude-code"),
        launch(host, "hs-t5", Some("go")),
    )
    .await
    .unwrap_err()
    .to_string();

    assert!(
        started.elapsed() < Duration::from_secs(10),
        "bounded, not the 120s host start wait"
    );
    assert!(
        err.contains("harness 'claude-code' did not start on host 'local'"),
        "{err}"
    );
    assert!(err.contains("launch still pending"), "{err}");
    assert!(err.contains("workspace was closed"), "{err}");
    assert!(
        err.contains("command not found"),
        "the pane's last lines are quoted: {err}"
    );
    let log = calls(&dir);
    assert!(
        log.contains("workspace close w9"),
        "no orphan pane is left: {log}"
    );
    assert!(!log.contains("agent prompt"), "{log}");
    assert!(
        get_row(&db, "hs-t5").is_err(),
        "no row claims a session that never ran"
    );
    assert_eq!(count(&db, "SELECT COUNT(*) FROM harness_sessions"), 0);
}

#[tokio::test]
async fn a_failed_cleanup_is_reported_instead_of_claimed() {
    let (_dir, host) = fake_host(&[
        ("workspace create", CREATED.into(), None),
        ("agent start", String::new(), Some(NOT_READY)),
        ("agent get", PENDING_AGENT.into(), None),
        ("agent read", String::new(), None),
        ("workspace close", String::new(), Some("boom")),
    ]);
    let db = Arc::new(Database::open_memory().unwrap());
    let vault = tempfile::tempdir().unwrap();

    let err = launch_session(
        vault.path(),
        &db,
        &harness("claude-code"),
        launch(host.with_launch_bound(SHORT_BOUND), "hs-t5", None),
    )
    .await
    .unwrap_err()
    .to_string();

    assert!(
        err.contains("Closing workspace w9 failed") && err.contains("close it by hand"),
        "{err}"
    );
}

/// Rewrites the fake host so the matching subcommand takes a second to answer.
fn slow_down(dir: &tempfile::TempDir, needle: &str) {
    let path = dir.path().join("scripted-host");
    let script = std::fs::read_to_string(&path).unwrap();
    let marker = format!("*\"{needle}\"*) ");
    assert!(script.contains(&marker));
    std::fs::write(
        &path,
        script.replace(&marker, &format!("{marker}sleep 1; ")),
    )
    .unwrap();
}

async fn eventually(what: &str, mut ready: impl FnMut() -> bool) {
    for _ in 0..100 {
        if ready() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("timed out waiting for {what}");
}

#[tokio::test]
async fn a_caller_that_disconnects_mid_launch_still_gets_the_orphan_closed() {
    let (dir, host) = stuck_host();
    slow_down(&dir, "agent start");
    let db = Arc::new(Database::open_memory().unwrap());
    let vault = tempfile::tempdir().unwrap();

    let cancelled = tokio::time::timeout(
        Duration::from_millis(200),
        launch_session(
            vault.path(),
            &db,
            &harness("claude-code"),
            launch(host, "hs-t5", None),
        ),
    )
    .await;
    assert!(
        cancelled.is_err(),
        "the caller gave up while the launch was running"
    );

    eventually("the orphan workspace to be closed", || {
        calls(&dir).contains("workspace close w9")
    })
    .await;
    assert_eq!(count(&db, "SELECT COUNT(*) FROM harness_sessions"), 0);
}

#[tokio::test]
async fn a_caller_that_disconnects_mid_launch_still_gets_the_session_recorded() {
    let (dir, host) = startable_host();
    slow_down(&dir, "agent start");
    let db = Arc::new(Database::open_memory().unwrap());
    let vault = tempfile::tempdir().unwrap();

    let cancelled = tokio::time::timeout(
        Duration::from_millis(200),
        handoff::handoff(vault.path(), &db, Arc::new(host), handoff_req("ext-cancel")),
    )
    .await;
    assert!(cancelled.is_err());

    eventually("the session to be recorded", || {
        count(
            &db,
            "SELECT COUNT(*) FROM harness_sessions WHERE owner_thread IS NOT NULL",
        ) == 1
    })
    .await;
    assert_eq!(
        count(
            &db,
            "SELECT COUNT(*) FROM chat_threads WHERE status = 'active'"
        ),
        1
    );
}

#[tokio::test]
async fn a_cancelled_handoff_for_a_stuck_harness_still_settles_the_thread_and_task() {
    let (dir, host) = stuck_host();
    slow_down(&dir, "agent start");
    let db = Arc::new(Database::open_memory().unwrap());
    let vault = tempfile::tempdir().unwrap();

    let cancelled = tokio::time::timeout(
        Duration::from_millis(200),
        handoff::handoff(
            vault.path(),
            &db,
            Arc::new(host),
            handoff_req("ext-cancel-stuck"),
        ),
    )
    .await;
    assert!(cancelled.is_err());

    eventually("the thread to be archived", || {
        calls(&dir).contains("workspace close w9")
            && count(
                &db,
                "SELECT COUNT(*) FROM chat_threads WHERE status = 'active'",
            ) == 0
    })
    .await;
    assert_eq!(count(&db, "SELECT COUNT(*) FROM harness_sessions"), 0);
}

#[tokio::test]
async fn a_handoff_to_a_host_whose_harness_never_starts_leaves_nothing_behind() {
    let (dir, host) = stuck_host();
    let db = Arc::new(Database::open_memory().unwrap());
    let vault = tempfile::tempdir().unwrap();

    let err = handoff::handoff(vault.path(), &db, Arc::new(host), handoff_req("ext-stuck"))
        .await
        .unwrap_err()
        .to_string();

    assert!(
        err.contains("no session was started") && err.contains("did not start on host 'local'"),
        "{err}"
    );
    assert!(calls(&dir).contains("workspace close w9"));
    assert_eq!(count(&db, "SELECT COUNT(*) FROM harness_sessions"), 0);
    assert_eq!(
        count(
            &db,
            "SELECT COUNT(*) FROM chat_threads WHERE status = 'active'"
        ),
        0
    );
    let status: String = db
        .with_conn(|c| Ok(c.query_row("SELECT status FROM tasks", [], |r| r.get(0))?))
        .unwrap();
    assert_eq!(status, "to_do", "the task is not left looking started");

    let (_dir, host) = startable_host();
    let again = handoff::handoff(vault.path(), &db, Arc::new(host), handoff_req("ext-stuck"))
        .await
        .unwrap();
    assert_eq!(
        again["task"]["deduplicated"], true,
        "a retry reuses the task"
    );
    assert_eq!(again["handoff"], "started");
}

#[tokio::test]
async fn a_failure_after_the_session_was_recorded_says_so_and_keeps_the_thread() {
    let boom = r#"{"error":{"code":"boom","message":"host fell over"},"id":"x"}"#;
    let (_dir, host) = fake_host(&[
        ("workspace create", CREATED.into(), None),
        ("agent start", OK.into(), None),
        ("agent get", agent_json("hs-h", "idle"), None),
        ("agent prompt", String::new(), Some(boom)),
    ]);
    let db = Arc::new(Database::open_memory().unwrap());
    let vault = tempfile::tempdir().unwrap();

    let err = handoff::handoff(vault.path(), &db, Arc::new(host), handoff_req("ext-late"))
        .await
        .unwrap_err()
        .to_string();

    assert!(
        err.contains("was started for task") && err.contains("is tracked"),
        "{err}"
    );
    assert!(!err.contains("nothing has run"), "{err}");
    assert_eq!(count(&db, "SELECT COUNT(*) FROM harness_sessions"), 1);
    assert_eq!(
        count(
            &db,
            "SELECT COUNT(*) FROM chat_threads WHERE status = 'active'"
        ),
        1
    );
}

#[tokio::test]
async fn an_agent_lookup_that_fails_after_the_start_still_closes_the_workspace() {
    let boom = r#"{"error":{"code":"boom","message":"lookup died"},"id":"x"}"#;
    let (dir, host) = fake_host(&[
        ("workspace create", CREATED.into(), None),
        ("agent start", OK.into(), None),
        ("agent get", String::new(), Some(boom)),
        ("workspace close", OK.into(), None),
    ]);
    let db = Arc::new(Database::open_memory().unwrap());
    let vault = tempfile::tempdir().unwrap();

    let err = launch_session(
        vault.path(),
        &db,
        &harness("claude-code"),
        launch(host, "hs-t7", None),
    )
    .await
    .unwrap_err()
    .to_string();

    assert!(err.contains("lookup died"), "{err}");
    assert!(
        calls(&dir).contains("workspace close w9"),
        "{}",
        calls(&dir)
    );
    assert!(get_row(&db, "hs-t7").is_err());
}

#[tokio::test]
async fn a_close_that_fails_during_cleanup_is_named_in_the_error() {
    let boom = r#"{"error":{"code":"boom","message":"lookup died"},"id":"x"}"#;
    let (_dir, host) = fake_host(&[
        ("workspace create", CREATED.into(), None),
        ("agent start", OK.into(), None),
        ("agent get", String::new(), Some(boom)),
        ("workspace close", String::new(), Some("close refused")),
    ]);
    let db = Arc::new(Database::open_memory().unwrap());
    let vault = tempfile::tempdir().unwrap();

    let err = launch_session(
        vault.path(),
        &db,
        &harness("claude-code"),
        launch(host, "hs-t7", None),
    )
    .await
    .unwrap_err()
    .to_string();

    assert!(
        err.contains("workspace w9 could not be closed") && err.contains("close it by hand"),
        "{err}"
    );
}

#[tokio::test]
async fn a_row_that_cannot_be_recorded_closes_the_workspace_that_was_just_launched() {
    let (dir, host) = fake_host(&[
        ("workspace create", CREATED.into(), None),
        ("agent start", OK.into(), None),
        ("agent get", agent_json("hs-t8", "idle"), None),
        ("workspace close", OK.into(), None),
    ]);
    let db = Arc::new(Database::open_memory().unwrap());
    let vault = tempfile::tempdir().unwrap();
    db.with_conn(|c| {
        registry::insert(
            c,
            &registry::NewSession {
                id: "hs-t8",
                harness: "claude-code",
                label: "",
                cwd: "/t",
                mission_id: None,
                placement: Placement {
                    host: "local",
                    agent_name: "hs-t8",
                    workspace_id: "w0",
                    pane_id: "w0:p1",
                },
                purpose: None,
            },
        )
    })
    .unwrap();

    let err = launch_session(
        vault.path(),
        &db,
        &harness("claude-code"),
        launch(host, "hs-t8", None),
    )
    .await
    .unwrap_err()
    .to_string();

    assert!(err.contains("workspace was closed"), "{err}");
    assert!(
        calls(&dir).contains("workspace close w9"),
        "{}",
        calls(&dir)
    );
}

#[tokio::test]
async fn an_agent_whose_last_poll_shows_launch_not_pending_is_never_closed() {
    let unclassified = r#"{"id":"x","result":{"type":"agent_info","agent":{"agent":"claude","agent_status":"unknown","cwd":"/t","name":"hs-t9","pane_id":"w9:p1","workspace_id":"w9","state_change_seq":0,"launch_pending":false}}}"#;
    let (dir, host) = fake_host(&[
        ("workspace create", CREATED.into(), None),
        ("agent start", String::new(), Some(NOT_READY)),
        ("agent get", unclassified.into(), None),
        ("agent read", "some banner".into(), None),
    ]);
    let db = Arc::new(Database::open_memory().unwrap());
    let vault = tempfile::tempdir().unwrap();

    let report = launch_session(
        vault.path(),
        &db,
        &harness("claude-code"),
        launch(host.with_launch_bound(SHORT_BOUND), "hs-t9", None),
    )
    .await
    .unwrap();

    assert_eq!(report["status"], "running");
    assert!(!calls(&dir).contains("workspace close"), "{}", calls(&dir));
}

#[tokio::test]
async fn a_second_launch_of_the_same_agent_is_refused_while_the_first_is_still_running() {
    let (dir, host) = startable_host();
    slow_down(&dir, "agent start");
    let db = Arc::new(Database::open_memory().unwrap());
    let vault = tempfile::tempdir().unwrap();
    let first = tokio::time::timeout(
        Duration::from_millis(200),
        launch_session(
            vault.path(),
            &db,
            &harness("claude-code"),
            launch(host.clone(), "hs-t10", None),
        ),
    )
    .await;
    assert!(first.is_err(), "the caller gave up, the launch carries on");

    let err = launch_session(
        vault.path(),
        &db,
        &harness("claude-code"),
        launch(host.clone(), "hs-t10", None),
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(err.contains("already in progress"), "{err}");

    eventually("the first launch to be recorded", || {
        get_row(&db, "hs-t10").is_ok()
    })
    .await;
    eventually("the claim to be released", || {
        InFlight::claim(&db, "local", "hs-t10").is_ok()
    })
    .await;
}

fn tool(
    db: &Arc<Database>,
    chat: Option<WatchingChat>,
    name: &str,
) -> Box<dyn crate::registry::HqTool> {
    tools::create_harness_session_tools(PathBuf::from("/tmp"), db.clone(), chat, None)
        .into_iter()
        .find(|t| t.name() == name)
        .unwrap()
}

fn driver_chat() -> WatchingChat {
    WatchingChat {
        driver_turn: true,
        ..chat(false)
    }
}

fn drive_on(db: &Arc<Database>, id: &str) {
    seed_watched(db, id, Some(goal_text()));
    db.with_conn(|c| registry::request_drive(c, id, true, registry::ACTOR_USER))
        .unwrap();
}

#[tokio::test]
async fn every_launched_pane_carries_its_session_id_for_the_mcp_client() {
    let (dir, host) = fake_host(&[
        ("workspace create", CREATED.into(), None),
        ("agent start", OK.into(), None),
        ("agent get", agent_json("hs-env", "idle"), None),
    ]);
    let db = Arc::new(Database::open_memory().unwrap());
    let vault = tempfile::tempdir().unwrap();

    launch_session(
        vault.path(),
        &db,
        &harness("claude-code"),
        launch(host, "hs-env", None),
    )
    .await
    .unwrap();

    assert!(
        calls(&dir).contains("--env HQ_SESSION_ID=hs-env"),
        "{}",
        calls(&dir)
    );
}

fn sent(db: &Arc<Database>, id: &str, chat: Option<&WatchingChat>, kind: SendKind) -> Result<()> {
    metered(db, id, chat, kind, || Ok(()))
}

#[test]
fn a_driver_turn_sends_only_to_the_session_its_chat_drives_and_within_its_budget() {
    let db = Arc::new(Database::open_memory().unwrap());
    drive_on(&db, "hs-b");
    let driver = driver_chat();
    let budget = agent_host_config().nudge_budget();
    for _ in 0..budget {
        sent(&db, "hs-b", Some(&driver), SendKind::Text).unwrap();
    }
    let err = sent(&db, "hs-b", Some(&driver), SendKind::Text)
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("used its") && err.contains("instructions"),
        "{err}"
    );
    assert_eq!(
        get_row(&db, "hs-b").unwrap().nudges_sent,
        budget,
        "a refused send is not counted"
    );
    sent(&db, "hs-b", Some(&chat(false)), SendKind::Text).unwrap();
    sent(&db, "hs-b", None, SendKind::Text).unwrap();

    db.with_conn(|c| registry::request_drive(c, "hs-b", true, registry::ACTOR_USER))
        .unwrap();
    sent(&db, "hs-b", Some(&driver), SendKind::Text).unwrap();

    db.with_conn(|c| registry::request_drive(c, "hs-b", false, registry::ACTOR_USER))
        .unwrap();
    let off = sent(&db, "hs-b", Some(&driver), SendKind::Text)
        .unwrap_err()
        .to_string();
    assert!(off.contains("Drive is off"), "{off}");
}

#[test]
fn key_presses_have_their_own_larger_allowance() {
    let db = Arc::new(Database::open_memory().unwrap());
    drive_on(&db, "hs-k");
    let driver = driver_chat();
    let budget = agent_host_config().nudge_budget();
    for _ in 0..budget {
        sent(&db, "hs-k", Some(&driver), SendKind::Text).unwrap();
    }
    sent(&db, "hs-k", Some(&driver), SendKind::Keys).unwrap();
    let allowance = agent_host_config().key_allowance();
    assert!(allowance > budget);
    for _ in 1..allowance {
        sent(&db, "hs-k", Some(&driver), SendKind::Keys).unwrap();
    }
    let err = sent(&db, "hs-k", Some(&driver), SendKind::Keys)
        .unwrap_err()
        .to_string();
    assert!(err.contains("key presses"), "{err}");
}

#[test]
fn a_failed_send_gives_its_slot_back_and_parallel_sends_cannot_overshoot() {
    let db = Arc::new(Database::open_memory().unwrap());
    drive_on(&db, "hs-p");
    let driver = driver_chat();
    let failed: Result<()> = metered(&db, "hs-p", Some(&driver), SendKind::Text, || {
        bail!("host down")
    });
    assert!(failed.is_err());
    assert_eq!(get_row(&db, "hs-p").unwrap().nudges_sent, 0);

    let budget = agent_host_config().nudge_budget() as usize;
    let handles: Vec<_> = (0..budget * 3)
        .map(|_| {
            let (db, driver) = (db.clone(), driver.clone());
            std::thread::spawn(move || sent(&db, "hs-p", Some(&driver), SendKind::Text).is_ok())
        })
        .collect();
    let ok = handles
        .into_iter()
        .filter_map(|h| h.join().ok())
        .filter(|ok| *ok)
        .count();
    assert_eq!(ok, budget, "exactly the budget got through");
    assert_eq!(get_row(&db, "hs-p").unwrap().nudges_sent as usize, budget);
}

#[test]
fn a_driver_turn_cannot_send_to_a_session_another_chat_watches() {
    let db = Arc::new(Database::open_memory().unwrap());
    drive_on(&db, "hs-o");
    db.with_conn(|c| registry::set_owner(c, "hs-o", Some("th-other")))
        .unwrap();
    assert!(sent(&db, "hs-o", Some(&driver_chat()), SendKind::Text).is_err());
}

#[test]
fn a_send_from_anyone_but_the_driver_rearms_the_one_wake_per_nudge_rule() {
    let db = Arc::new(Database::open_memory().unwrap());
    drive_on(&db, "hs-r");
    db.with_conn(|c| registry::set_last_wake_nudges(c, "hs-r", 0))
        .unwrap();
    sent(&db, "hs-r", Some(&driver_chat()), SendKind::Text).unwrap();
    let row = get_row(&db, "hs-r").unwrap();
    assert_eq!((row.nudges_sent, row.last_wake_nudges), (1, Some(0)));
    sent(&db, "hs-r", Some(&chat(false)), SendKind::Text).unwrap();
    let row = get_row(&db, "hs-r").unwrap();
    assert_eq!((row.nudges_sent, row.last_wake_nudges), (1, None));
}

#[test]
fn a_session_that_exits_or_is_stopped_stops_being_driven_with_a_reason_and_an_event() {
    for end in ["exit", "stop"] {
        let db = Arc::new(Database::open_memory().unwrap());
        drive_on(&db, "hs-e");
        db.with_conn(|c| match end {
            "exit" => registry::set_status_exited_if_running(c, "hs-e").map(|_| ()),
            _ => registry::set_status(c, "hs-e", registry::STATUS_STOPPED),
        })
        .unwrap();
        let row = get_row(&db, "hs-e").unwrap();
        assert!(!row.drive, "{end}");
        assert_eq!(
            row.drive_off_reason.as_deref(),
            Some(registry::SESSION_ENDED),
            "{end}"
        );
        let events = db
            .with_conn(|c| registry::list_events(c, "hs-e", 10))
            .unwrap();
        let last = events.last().unwrap();
        assert_eq!(
            (last.kind.as_str(), last.actor.as_str()),
            (registry::EVENT_DRIVE_OFF, registry::ACTOR_GUARD),
            "{end}"
        );
    }
}

#[test]
fn a_default_on_watch_past_the_driven_session_cap_starts_observing_and_says_why() {
    let db = Arc::new(Database::open_memory().unwrap());
    let cap = 1;
    for id in ["hs-c1", "hs-c2"] {
        seed_watched(&db, id, Some(goal_text()));
        db.with_conn(|c| registry::set_owner(c, id, None)).unwrap();
        let watch = NewWatch {
            thread: "th-1",
            drive: true,
            opted_out: false,
        };
        db.with_conn(|c| start_watch_capped(c, id, watch, cap))
            .unwrap();
    }
    let (first, second) = (
        get_row(&db, "hs-c1").unwrap(),
        get_row(&db, "hs-c2").unwrap(),
    );
    assert!(first.drive);
    assert!(!second.drive);
    assert!(
        second
            .drive_off_reason
            .unwrap()
            .contains("agent_host.max_driven_sessions")
    );
    assert_eq!(with_watch_state(&db, "hs-c2", json!({}))["mode"], "observe");
}

#[tokio::test]
async fn the_mode_tool_lets_a_driver_turn_observe_but_never_drive() {
    let db = Arc::new(Database::open_memory().unwrap());
    seed_watched(&db, "hs-m2", Some(goal_text()));
    let err = tool(&db, Some(driver_chat()), "harness_session_mode")
        .execute(json!({"session_id": "hs-m2", "mode": "drive"}))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("only the user can"), "{err}");
}

#[tokio::test]
async fn a_spawn_from_inside_a_session_is_a_delegation_and_a_handoff_is_refused() {
    let db = Arc::new(Database::open_memory().unwrap());
    let marked = json!({"harness": "claude-code", "cwd": "/srv/x", "prompt": "fix it", SPAWNED_SESSION_ARG: "hs-claude-code-1"});
    let named =
        json!({"harness": "claude-code", "prompt": "fix it", "from_session": "hs-claude-code-1"});
    let spawn = tool(&db, None, "harness_session_spawn");
    // The delegator is not a running session here, so the delegation path refuses it
    // before anything starts; a plain launch would have failed differently.
    for args in [marked.clone(), named] {
        let err = spawn.execute(args).await.unwrap_err().to_string();
        assert!(err.contains("delegating session is not running"), "{err}");
    }
    let handoff = tool(&db, None, "harness_session_handoff");
    let err = handoff.execute(marked).await.unwrap_err();
    assert_eq!(err.to_string(), SPAWNED_REFUSAL);

    // A chat cannot claim to be a session to slip past its own caps.
    let from_chat = tool(&db, Some(driver_chat()), "harness_session_spawn");
    let err = from_chat
        .execute(
            json!({"harness": "claude-code", "prompt": "x", "from_session": "hs-claude-code-1"}),
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("a chat starts sessions without it"), "{err}");
}

#[tokio::test]
async fn a_connector_claiming_a_session_still_meets_its_own_folder_fence() {
    let db = Arc::new(Database::open_memory().unwrap());
    db.with_conn(|c| {
        registry::insert(
            c,
            &registry::NewSession {
                id: "hs-lead",
                harness: "claude-code",
                label: "",
                cwd: "/srv/private-repo",
                mission_id: Some("tk-x"),
                placement: Placement {
                    host: "local",
                    agent_name: "hs-lead",
                    workspace_id: "w1",
                    pane_id: "w1:p1",
                },
                purpose: None,
            },
        )
    })
    .unwrap();
    let spawn = tool(&db, None, "harness_session_spawn");
    let err = spawn
        .execute(json!({
            "harness": "claude-code",
            "prompt": "do the thing",
            "from_session": "hs-lead",
            CONNECTOR_ARG: "Claude",
        }))
        .await
        .unwrap_err()
        .to_string();
    assert!(
        !err.contains("delegating session is not running") && !err.contains("sub-task"),
        "the claim must stop at the caller's own fence, not reach the delegation: {err}"
    );
    assert!(err.contains("/srv/private-repo"), "{err}");

    let err = spawn
        .execute(json!({
            "harness": "claude-code",
            "prompt": "do the thing",
            "from_session": "hs-lead",
            HANDOFF_SCOPE_ARG: true,
        }))
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("handoff key cannot start a worker"), "{err}");
}

#[tokio::test]
async fn chats_an_ask_started_may_own_only_so_many_running_sessions() {
    let db = Arc::new(Database::open_memory().unwrap());
    let cap = agent_host_config().ask_spawned_session_cap();
    for n in 0..cap {
        let id = format!("hs-ask{n}");
        seed_watched(&db, &id, None);
        tag_origin(&db, &json!({"session_id": id}), registry::ORIGIN_ASK);
    }
    let from_ask = WatchingChat {
        from_ask: true,
        ..chat(false)
    };
    let err = tool(&db, Some(from_ask), "harness_session_spawn")
        .execute(json!({"harness": "claude-code", "cwd": "/srv/x"}))
        .await
        .unwrap_err();
    assert!(
        err.to_string()
            .contains("agent_host.max_ask_spawned_sessions"),
        "{err}"
    );
}

#[tokio::test]
async fn a_driver_turn_cannot_start_attach_or_link_sessions_and_an_ask_reply_cannot_attach_or_link()
{
    let db = Arc::new(Database::open_memory().unwrap());
    let launch_args = json!({"harness": "claude-code", "cwd": "/srv/x"});
    for name in ["harness_session_spawn", "harness_session_handoff"] {
        let err = tool(&db, Some(driver_chat()), name)
            .execute(launch_args.clone())
            .await
            .unwrap_err();
        assert!(
            err.to_string()
                .contains("driver turn cannot start sessions"),
            "{name}: {err}"
        );
    }
    let from_ask = WatchingChat {
        from_ask: true,
        ..chat(false)
    };
    for chat in [driver_chat(), from_ask] {
        for (name, args) in [
            ("harness_session_attach", json!({"agent": "x"})),
            (
                "harness_session_link",
                json!({"session_id": "hs-x", "task_id": "FR-1"}),
            ),
        ] {
            let err = tool(&db, Some(chat.clone()), name)
                .execute(args)
                .await
                .unwrap_err();
            assert!(
                err.to_string().contains("cannot attach or link"),
                "{name}: {err}"
            );
        }
    }
}

#[tokio::test]
async fn an_ask_reply_cannot_turn_drive_on() {
    let db = Arc::new(Database::open_memory().unwrap());
    seed_watched(&db, "hs-ask-m", Some(goal_text()));
    let from_ask = WatchingChat {
        from_ask: true,
        ..chat(false)
    };
    let err = tool(&db, Some(from_ask), "harness_session_mode")
        .execute(json!({"session_id": "hs-ask-m", "mode": "drive"}))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("only the user can"), "{err}");
}

#[test]
fn hq_driving_past_the_cap_through_the_mode_tool_is_refused_while_the_user_is_not() {
    let db = Arc::new(Database::open_memory().unwrap());
    let cap = agent_host_config().driven_session_cap();
    for n in 0..=cap {
        seed_watched(&db, &format!("hs-cap{n}"), Some(goal_text()));
    }
    for n in 0..cap {
        let row = get_row(&db, &format!("hs-cap{n}")).unwrap();
        set_mode_with(&db, &row, &alive(&row.id), mode_req(true)).unwrap();
    }
    let last = get_row(&db, &format!("hs-cap{cap}")).unwrap();
    let refused = set_mode_with(&db, &last, &alive(&last.id), mode_req(true)).unwrap();
    assert_eq!(refused["mode"], "observe");
    assert!(
        refused["drive_blocked_by"][0]
            .as_str()
            .unwrap()
            .contains("max_driven_sessions"),
        "{refused}"
    );
    let user = ModeRequest {
        actor: registry::ACTOR_USER,
        ..mode_req(true)
    };
    assert_eq!(
        set_mode_with(&db, &last, &alive(&last.id), user).unwrap()["mode"],
        "drive"
    );
}

#[test]
fn a_watch_the_user_already_turned_on_survives_the_cap() {
    let db = Arc::new(Database::open_memory().unwrap());
    for id in ["hs-u1", "hs-u2"] {
        drive_on(&db, id);
    }
    let watch = NewWatch {
        thread: "th-1",
        drive: true,
        opted_out: false,
    };
    db.with_conn(|c| start_watch_capped(c, "hs-u2", watch, 1))
        .unwrap();
    assert!(
        get_row(&db, "hs-u2").unwrap().drive,
        "re-watching does not switch off what the user turned on"
    );
}

#[test]
fn sessions_started_without_a_chat_or_from_an_ask_are_capped_by_origin() {
    let db = Arc::new(Database::open_memory().unwrap());
    assert_eq!(start_origin(None), registry::ORIGIN_MCP);
    assert_eq!(start_origin(Some(&chat(true))), registry::ORIGIN_USER);
    assert_eq!(
        start_origin(Some(&WatchingChat {
            from_ask: true,
            ..chat(false)
        })),
        registry::ORIGIN_ASK
    );
    check_origin_cap(&db, registry::ORIGIN_USER).unwrap();
    let cap = agent_host_config().mcp_started_session_cap();
    for n in 0..cap {
        let id = format!("hs-mcp{n}");
        seed_watched(&db, &id, None);
        tag_origin(&db, &json!({"session_id": id}), registry::ORIGIN_MCP);
    }
    let err = check_origin_cap(&db, registry::ORIGIN_MCP)
        .unwrap_err()
        .to_string();
    assert!(err.contains("agent_host.max_mcp_started_sessions"), "{err}");
    check_origin_cap(&db, registry::ORIGIN_ASK).unwrap();
}

#[tokio::test]
async fn a_marked_caller_cannot_resume_and_may_type_only_into_its_own_session() {
    let db = Arc::new(Database::open_memory().unwrap());
    let err = tool(&db, None, "harness_session_resume")
        .execute(json!({"session_id": "hs-x", SPAWNED_SESSION_ARG: "hs-own"}))
        .await
        .unwrap_err();
    assert_eq!(err.to_string(), SPAWNED_REFUSAL);
    let other = json!({"session_id": "hs-other", "text": "hi", SPAWNED_SESSION_ARG: "hs-own"});
    let err = tool(&db, None, "harness_session_send")
        .execute(other)
        .await
        .unwrap_err();
    assert_eq!(err.to_string(), SPAWNED_REFUSAL);
    assert!(spawned_may_target(&json!({SPAWNED_SESSION_ARG: "hs-own"}), "hs-own").is_ok());
    assert!(spawned_may_target(&json!({}), "anything").is_ok());
}

#[test]
fn a_handoff_over_the_handoff_key_or_from_an_ask_never_starts_driven() {
    let scoped = json!({HANDOFF_SCOPE_ARG: true});
    let from_ask = WatchingChat {
        from_ask: true,
        ..chat(true)
    };
    assert!(tools::handoff_drives_new(true, &json!({}), None));
    assert!(!tools::handoff_drives_new(true, &scoped, None));
    assert!(!tools::handoff_drives_new(
        true,
        &json!({}),
        Some(&from_ask)
    ));
    assert!(!tools::handoff_drives_new(false, &json!({}), None));
}

#[tokio::test]
async fn goal_changes_over_mcp_are_audited_as_mcp_not_as_the_user() {
    let db = Arc::new(Database::open_memory().unwrap());
    seed_watched(&db, "hs-goal", None);
    tool(&db, None, "harness_session_goal")
        .execute(json!({"session_id": "hs-goal", "goal": GOAL, "done_criteria": DONE}))
        .await
        .unwrap();
    let events = db
        .with_conn(|c| registry::list_events(c, "hs-goal", 5))
        .unwrap();
    assert_eq!(events.last().unwrap().actor, registry::ACTOR_MCP);
}

#[tokio::test]
async fn a_marked_caller_cannot_host_send_to_another_pane() {
    let args =
        json!({"host": "local", "target": "hs-other", "text": "x", SPAWNED_SESSION_ARG: "hs-own"});
    let err = crate::agent_host::tools::HostSendTool
        .execute(args)
        .await
        .unwrap_err();
    assert_eq!(err.to_string(), SPAWNED_REFUSAL);
}

fn row_with_snapshot(id: &str, status: &str) -> Arc<Database> {
    let db = Arc::new(Database::open_memory().unwrap());
    let (id, status) = (id.to_string(), status.to_string());
    db.with_conn(move |c| {
        registry::insert(
            c,
            &registry::NewSession {
                id: &id,
                harness: "claude-code",
                label: "",
                cwd: "/t",
                mission_id: None,
                placement: Placement {
                    host: "local",
                    agent_name: "hs-tl",
                    workspace_id: "w1",
                    pane_id: "w1:p1",
                },
                purpose: None,
            },
        )?;
        registry::set_last_snapshot(c, &id, "stored line")?;
        registry::set_status(c, &id, &status)
    })
    .unwrap();
    db
}

#[test]
fn a_session_the_registry_says_is_over_is_never_read_live() {
    for status in [
        registry::STATUS_EXITED,
        registry::STATUS_STOPPED,
        registry::STATUS_ORPHANED,
    ] {
        let db = row_with_snapshot("hs-tl", status);
        let reads = std::cell::Cell::new(0);
        let screen = tail_log_with(&db, "hs-tl", 10, |_| {
            reads.set(reads.get() + 1);
            Some(("shell prompt $".into(), "recent-unwrapped"))
        })
        .unwrap();
        assert_eq!(reads.get(), 0, "{status}");
        assert_eq!(screen["source"], "snapshot");
        assert_eq!(screen["lines"][0], "stored line");
    }
}

#[test]
fn a_running_session_is_read_live_once_and_falls_back_when_the_read_fails() {
    let db = row_with_snapshot("hs-tl", registry::STATUS_RUNNING);
    let reads = std::cell::Cell::new(0);
    let live = tail_log_with(&db, "hs-tl", 10, |_| {
        reads.set(reads.get() + 1);
        Some(("a\nb".into(), "visible"))
    })
    .unwrap();
    assert_eq!((reads.get(), live["source"].as_str()), (1, Some("live")));
    assert_eq!(live["host_source"], "visible");
    let down = tail_log_with(&db, "hs-tl", 10, |_| None).unwrap();
    assert_eq!(down["source"], "snapshot");
    assert!(down["host_source"].is_null());
}

// ---- The Agentic Stack -------------------------------------------------------

fn stack_config(yaml: &str, vault: &std::path::Path) -> hq_core::config::HqConfig {
    hq_core::config::HqConfig {
        vault_path: vault.to_path_buf(),
        stack: serde_yaml::from_str(yaml).unwrap(),
        ..Default::default()
    }
}

/// A task assigned to `assignee` in a fresh initiative.
fn assigned_task(db: &Database, id: &str, title: &str, assignee: &str, tags: &[&str]) -> String {
    let tags: Vec<String> = tags.iter().map(|t| t.to_string()).collect();
    let assignees = vec![assignee.to_string()];
    db.with_conn(|c| {
        let _ = c.execute(
            "INSERT OR IGNORE INTO initiatives (id, space_id, name, slug, id_prefix) VALUES ('in-1', 'personal', 'Market research', 'mr', 'MR')",
            [],
        );
        let new = hq_db::tasks::NewTask {
            title,
            description: "Find three suppliers of solar pumps in Uganda.\nDone when each has a price and a contact.",
            created_by: "test",
            tags: &tags,
            assignees: &assignees,
            ..Default::default()
        };
        Ok(hq_db::tasks::create_task(c, id, "in-1", &new)?.id)
    })
    .unwrap()
}

fn stack_resolver(host: ScriptedHost) -> impl Fn(Option<&str>) -> anyhow::Result<Host> + Sync {
    move |_| Ok(Arc::new(host.clone()) as Host)
}

#[tokio::test]
async fn a_specialist_runs_the_task_assigned_to_it_once_and_as_one_shot() {
    let (dir, host) = startable_host();
    let db = Arc::new(Database::open_memory().unwrap());
    let vault = tempfile::tempdir().unwrap();
    let cfg = stack_config(
        "specialists:\n  - name: hermes\n    harness: hermes\n    purposes: [research, general]\n    cwd: /t\n",
        vault.path(),
    );
    let task = assigned_task(&db, "tk-1", "Solar pump suppliers", "hermes", &[]);
    assigned_task(&db, "tk-other", "Someone else's", "codex", &[]);

    let resolve = stack_resolver(host);
    let report = stack::dispatch(vault.path(), &db, &cfg, &resolve)
        .await
        .unwrap();
    assert_eq!(report.started.len(), 1, "{report:?}");
    assert!(report.skipped.is_empty(), "{report:?}");
    let run = &report.started[0];
    assert_eq!(run["specialist"], "hermes");
    assert_eq!(
        run["purpose"], "research",
        "the lane's first purpose when the task hints at none"
    );
    let id = run["session_id"].as_str().unwrap();
    let row = get_row(&db, id).unwrap();
    assert_eq!(
        (
            row.origin.as_str(),
            row.label.as_str(),
            row.purpose.as_str()
        ),
        ("stack", "stack/hermes", "research")
    );
    assert_eq!(row.mission_id.as_deref(), Some(task.as_str()));
    let log = calls(&dir);
    assert!(
        log.contains("agent start")
            && log.contains("-q")
            && log.contains("Task MR-001: Solar pump suppliers"),
        "one-shot with the brief: {log}"
    );
    assert!(
        log.contains("solar pumps in Uganda"),
        "the description is the brief: {log}"
    );
    assert!(
        !log.contains("agent prompt"),
        "nothing is typed into a one-shot run: {log}"
    );
    let status: String = db
        .with_conn(|c| Ok(hq_db::tasks::get_task(c, "tk-1")?.unwrap().status))
        .unwrap();
    assert_eq!(status, "in_progress");

    // Nothing more starts while the run is up, and the other assignee's task is never touched.
    let (_dir2, host2) = startable_host();
    let resolve2 = stack_resolver(host2);
    let again = stack::dispatch(vault.path(), &db, &cfg, &resolve2)
        .await
        .unwrap();
    assert!(again.started.is_empty(), "{again:?}");
    assert_eq!(count(&db, "SELECT COUNT(*) FROM harness_sessions"), 1);
}

#[tokio::test]
async fn a_specialist_is_never_offered_code_work_and_a_tag_picks_its_purpose() {
    let (_dir, host) = startable_host();
    let db = Arc::new(Database::open_memory().unwrap());
    let vault = tempfile::tempdir().unwrap();
    let cfg = stack_config(
        "specialists:\n  - name: hermes\n    harness: hermes\n    purposes: [research, review]\n    cwd: /t\n",
        vault.path(),
    );
    assigned_task(&db, "tk-code", "Fix the login bug", "hermes", &["code"]);
    let reviewed = assigned_task(&db, "tk-review", "Read the draft", "hermes", &["review"]);

    let resolve = stack_resolver(host);
    let report = stack::dispatch(vault.path(), &db, &cfg, &resolve)
        .await
        .unwrap();
    assert_eq!(report.started.len(), 1, "{report:?}");
    let run = &report.started[0];
    assert_eq!(run["purpose"], "review");
    assert_eq!(
        get_row(&db, run["session_id"].as_str().unwrap())
            .unwrap()
            .mission_id
            .as_deref(),
        Some(reviewed.as_str())
    );
    let code_status: String = db
        .with_conn(|c| Ok(hq_db::tasks::get_task(c, "tk-code")?.unwrap().status))
        .unwrap();
    assert_eq!(
        code_status, "to_do",
        "the code task was left for a coding agent"
    );

    let err = stack::run_for(vault.path(), &db, &cfg, "hermes", "tk-code", &resolve)
        .await
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("already running") || err.contains("code work"),
        "{err}"
    );
}

#[tokio::test]
async fn a_run_that_cannot_start_leaves_a_comment_and_backs_off() {
    let (_dir, host) = fake_host(&[("workspace create", String::new(), Some("no such host"))]);
    let db = Arc::new(Database::open_memory().unwrap());
    let vault = tempfile::tempdir().unwrap();
    let cfg = stack_config(
        "specialists:\n  - name: hermes\n    harness: hermes\n    cwd: /t\n",
        vault.path(),
    );
    let task = assigned_task(&db, "tk-fail", "Will not start", "hermes", &[]);

    let resolve = stack_resolver(host);
    let report = stack::dispatch(vault.path(), &db, &cfg, &resolve)
        .await
        .unwrap();
    assert!(report.started.is_empty());
    assert_eq!(report.skipped.len(), 1, "{report:?}");
    assert!(report.skipped[0].starts_with("hermes:"), "{report:?}");
    let notes: Vec<String> = db
        .with_conn(|c| {
            Ok(hq_db::tasks::list_comments(c, &task)?
                .into_iter()
                .map(|c| c.body)
                .collect())
        })
        .unwrap();
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(
        notes[0].contains("could not start") && notes[0].contains("30 minutes"),
        "{notes:?}"
    );

    let (_dir2, host2) = startable_host();
    let resolve2 = stack_resolver(host2);
    let again = stack::dispatch(vault.path(), &db, &cfg, &resolve2)
        .await
        .unwrap();
    assert!(
        again.started.is_empty() && again.skipped.is_empty(),
        "backed off, not retried at once: {again:?}"
    );
}

#[tokio::test]
async fn a_backed_off_task_does_not_hold_up_the_rest_of_the_queue() {
    let (_dir, host) = startable_host();
    let db = Arc::new(Database::open_memory().unwrap());
    let vault = tempfile::tempdir().unwrap();
    let cfg = stack_config(
        "specialists:\n  - name: hermes\n    harness: hermes\n    cwd: /t\n",
        vault.path(),
    );
    let first = assigned_task(&db, "tk-stuck", "Could not start last time", "hermes", &[]);
    let second = assigned_task(&db, "tk-next", "Ready to go", "hermes", &[]);
    stack::back_off(&first);

    let resolve = stack_resolver(host);
    let report = stack::dispatch(vault.path(), &db, &cfg, &resolve)
        .await
        .unwrap();
    assert_eq!(report.started.len(), 1, "{report:?}");
    let row = get_row(&db, report.started[0]["session_id"].as_str().unwrap()).unwrap();
    assert_eq!(
        row.mission_id.as_deref(),
        Some(second.as_str()),
        "the queue's next task runs instead"
    );
    assert_eq!(row.origin, "stack", "the origin is written with the row");
}

#[tokio::test]
async fn a_spent_budget_is_nothing_to_do_not_a_problem() {
    let (_dir, host) = startable_host();
    let db = Arc::new(Database::open_memory().unwrap());
    let vault = tempfile::tempdir().unwrap();
    let cfg = stack_config(
        "specialists:\n  - name: hermes\n    harness: hermes\n    cwd: /t\n    runs_per_day: 1\n",
        vault.path(),
    );
    assigned_task(&db, "tk-one", "First", "hermes", &[]);
    assigned_task(&db, "tk-two", "Second", "hermes", &[]);

    let resolve = stack_resolver(host);
    let report = stack::dispatch(vault.path(), &db, &cfg, &resolve)
        .await
        .unwrap();
    assert_eq!(report.started.len(), 1, "{report:?}");
    let id = report.started[0]["session_id"]
        .as_str()
        .unwrap()
        .to_string();
    db.with_conn(|c| registry::set_status(c, &id, registry::STATUS_EXITED))
        .unwrap();

    let (_dir2, host2) = startable_host();
    let resolve2 = stack_resolver(host2);
    let again = stack::dispatch(vault.path(), &db, &cfg, &resolve2)
        .await
        .unwrap();
    assert!(
        again.started.is_empty() && again.skipped.is_empty(),
        "a spent budget is quiet: {again:?}"
    );
    assert_eq!(stack::status(&db, &cfg).unwrap()[0]["runs_last_24h"], 1);
}

#[tokio::test]
async fn a_stack_run_is_never_resumed() {
    let db = Arc::new(Database::open_memory().unwrap());
    let vault = tempfile::tempdir().unwrap();
    db.with_conn(|c| {
        registry::insert(
            c,
            &registry::NewSession {
                id: "hs-run",
                harness: "hermes",
                label: "stack/hermes",
                cwd: "/t",
                mission_id: None,
                placement: registry::Placement {
                    host: "local",
                    agent_name: "hs-run",
                    workspace_id: "w1",
                    pane_id: "w1:p1",
                },
                purpose: Some("research"),
            },
        )?;
        registry::set_origin(c, "hs-run", registry::ORIGIN_STACK)?;
        registry::set_status(c, "hs-run", registry::STATUS_EXITED)
    })
    .unwrap();
    let err = resume(vault.path(), &db, "hs-run", None)
        .await
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("one-shot run") && err.contains("hq stack run"),
        "{err}"
    );
}

#[test]
fn a_lane_refuses_coding_agents_and_the_code_purpose() {
    let vault = tempfile::tempdir().unwrap();
    let cfg = stack_config(
        "specialists:\n  - name: coder\n    harness: claude-code\n  - name: hermes\n    harness: hermes\n    purposes: [code]\n  - name: ok\n    harness: openclaw\n    purposes: [general, research, general]\n",
        vault.path(),
    );
    let err = stack::lane_for(&cfg, cfg.stack.get("coder").unwrap())
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("a coding agent") && err.contains("hermes"),
        "{err}"
    );
    let err = stack::lane_for(&cfg, cfg.stack.get("hermes").unwrap())
        .unwrap_err()
        .to_string();
    assert!(err.contains("general-purpose agent"), "{err}");
    let lane = stack::lane_for(&cfg, cfg.stack.get("ok").unwrap()).unwrap();
    assert_eq!(lane.purposes, vec![Purpose::General, Purpose::Research]);
    let status = stack::status(&Arc::new(Database::open_memory().unwrap()), &cfg).unwrap();
    assert_eq!(status.len(), 3);
    assert!(
        status[0]["problem"]
            .as_str()
            .unwrap()
            .contains("coding agent")
    );
    assert_eq!(
        status[2]["purposes"],
        serde_json::json!(["general", "research"])
    );
}

#[test]
fn the_brief_carries_the_task_its_resume_point_and_the_rules() {
    let db = Database::open_memory().unwrap();
    assigned_task(&db, "tk-b", "Price the pumps", "hermes", &[]);
    let task = db
        .with_conn(|c| Ok(hq_db::tasks::get_task(c, "tk-b")?.unwrap()))
        .unwrap();
    let cp = hq_db::tasks::TaskCheckpoint {
        id: 1,
        task_id: task.id.clone(),
        work_session_id: None,
        actor: "hermes".into(),
        summary: "Two suppliers found".into(),
        next_step: "Call the third".into(),
        open_questions: "Import duty?".into(),
        files: vec![],
        created_at: String::new(),
    };
    let brief = stack::brief_for(
        "hermes",
        Purpose::Research,
        &task,
        Some("Market research"),
        Some(&cp),
    );
    for needle in [
        "'hermes'",
        "Purpose: research",
        "Task MR-001: Price the pumps",
        "Initiative: Market research",
        "Done when each has a price",
        "Two suppliers found",
        "Call the third",
        "Import duty?",
        "REPORT",
        "change no repository",
    ] {
        assert!(brief.contains(needle), "missing {needle:?} in:\n{brief}");
    }
    assert!(brief.contains("Where the last run left off"));
    let fresh = stack::brief_for("hermes", Purpose::General, &task, None, None);
    assert!(!fresh.contains("Where the last run left off") && !fresh.contains("Initiative:"));
}

#[test]
fn an_exited_pane_counts_as_gone_but_can_still_be_read() {
    let row = db_row_for_liveness("hs-ran");
    let mut polled = HostPoll::new();
    let mut agent = match alive("hs-ran") {
        Liveness::Alive(a) => *a,
        _ => unreachable!(),
    };
    agent.exited = true;
    polled.insert("local".into(), Ok(vec![agent]));
    assert_eq!(liveness(&polled, &row), Liveness::Gone);
    assert!(exited_pane(&polled, &row).is_some());
    let mut live = HostPoll::new();
    live.insert(
        "local".into(),
        Ok(vec![match alive("hs-ran") {
            Liveness::Alive(a) => *a,
            _ => unreachable!(),
        }]),
    );
    assert!(matches!(liveness(&live, &row), Liveness::Alive(_)));
    assert!(exited_pane(&live, &row).is_none());
}

fn db_row_for_liveness(id: &str) -> HarnessSessionRow {
    let db = Database::open_memory().unwrap();
    db.with_conn(|c| {
        registry::insert(
            c,
            &registry::NewSession {
                id,
                harness: "hermes",
                label: "stack/hermes",
                cwd: "/t",
                mission_id: None,
                placement: Placement {
                    host: "local",
                    agent_name: id,
                    workspace_id: "w1",
                    pane_id: "w1:p1",
                },
                purpose: None,
            },
        )?;
        Ok(registry::get(c, id)?.unwrap())
    })
    .unwrap()
}

#[test]
fn a_connection_drives_only_sessions_a_connection_started() {
    let db = Arc::new(Database::open_memory().unwrap());
    for (id, origin) in [
        ("hs-owner", registry::ORIGIN_USER),
        ("hs-claude", registry::ORIGIN_CONNECTOR),
        ("hs-worker", registry::ORIGIN_USER),
        ("hs-owners-worker", registry::ORIGIN_USER),
    ] {
        db.with_conn(|c| {
            registry::insert(
                c,
                &registry::NewSession {
                    id,
                    harness: "claude-code",
                    label: "t",
                    cwd: "/srv/work",
                    mission_id: None,
                    placement: registry::Placement {
                        host: "native",
                        agent_name: id,
                        workspace_id: id,
                        pane_id: id,
                    },
                    purpose: None,
                },
            )?;
            registry::set_origin(c, id, origin)
        })
        .unwrap();
    }
    let connector = json!({ CONNECTOR_ARG: "Claude" });
    db.with_conn(|c| {
        registry::set_parent(c, "hs-worker", "hs-claude", 1)?;
        registry::set_parent(c, "hs-owners-worker", "hs-owner", 1)
    })
    .unwrap();
    assert!(connector_may_drive(&db, &connector, "hs-claude").is_ok());
    assert!(
        connector_may_drive(&db, &connector, "hs-worker").is_ok(),
        "a worker its lead brought in"
    );
    assert!(connector_may_drive(&db, &connector, "hs-owner").is_err());
    assert!(connector_may_drive(&db, &connector, "hs-owners-worker").is_err());
    assert!(connector_may_drive(&db, &connector, "hs-missing").is_err());
    assert!(
        connector_may_drive(&db, &json!({}), "hs-owner").is_ok(),
        "other callers are unaffected"
    );
    assert_eq!(call_origin(None, &connector), registry::ORIGIN_CONNECTOR);
    assert_eq!(call_origin(None, &json!({})), registry::ORIGIN_MCP);
}

/// The host's `agent.pending` reply for a Copilot path dialog numbered `seq`.
fn pending_json(seq: u64) -> String {
    format!(
        r#"{{"id":"x","result":{{"state":"blocked","rule":"approval_dialog","pending_seq":{seq},"pending":{{"kind":"path","question":"Do you want to allow this?","command":"/tmp -> /private/tmp","options":[{{"id":1,"label":"Yes","remember":false,"typed":false,"selected":false}},{{"id":2,"label":"Yes, and remember this path for this session","remember":true,"typed":false,"selected":true}},{{"id":3,"label":"No (Esc)","remember":false,"typed":false,"selected":false}}],"truncated":false,"text":"Allow path access"}}}}}}"#
    )
}

fn answer_on(
    host: ScriptedHost,
    seq: u64,
    choice: answer::Choice,
    who: answer::Answerer,
) -> Result<Value> {
    let db = Arc::new(Database::open_memory().unwrap());
    seed_watched(&db, "hs-a", None);
    let row = control::get_row(&db, "hs-a").unwrap();
    let host: crate::agent_host::Host = Arc::new(host);
    answer::answer_with(&db, &host, &row, seq, choice, who, None)
}

#[test]
fn an_answer_names_the_dialog_it_read_and_the_host_presses_the_keys() {
    let (dir, host) = fake_host(&[
        ("agent pending", pending_json(7), None),
        (
            "agent answer",
            r#"{"id":"x","result":{"registered":true}}"#.to_string(),
            None,
        ),
    ]);
    let report = answer_on(
        host,
        7,
        answer::Choice::Option(1),
        answer::Answerer::Automated("a Drive turn"),
    )
    .unwrap();
    assert_eq!(report["registered"], true, "{report}");
    assert_eq!(report["answered"]["label"], "Yes");
    assert!(
        calls(&dir).contains("agent answer hs-a --seq 7 --option 1"),
        "{}",
        calls(&dir)
    );
}

#[test]
fn a_stale_number_or_a_remember_option_from_an_automated_caller_presses_nothing() {
    let replies = [
        ("agent pending", pending_json(8), None),
        (
            "agent answer",
            r#"{"id":"x","result":{"registered":true}}"#.to_string(),
            None,
        ),
    ];
    let (dir, host) = fake_host(&replies);
    let err = answer_on(host, 7, answer::Choice::Option(1), answer::Answerer::Owner)
        .unwrap_err()
        .to_string();
    assert!(err.contains("different dialog now (number 8)"), "{err}");
    assert!(!calls(&dir).contains("agent answer"));

    let (dir, host) = fake_host(&replies);
    let err = answer_on(
        host,
        8,
        answer::Choice::Option(2),
        answer::Answerer::Automated("a claude.ai connection"),
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("only the owner"), "{err}");
    assert!(!calls(&dir).contains("agent answer"));

    // The owner may widen the agent's permissions.
    let (dir, host) = fake_host(&replies);
    answer_on(host, 8, answer::Choice::Option(2), answer::Answerer::Owner).unwrap();
    assert!(calls(&dir).contains("agent answer hs-a --seq 8 --option 2"));
}

#[test]
fn an_answer_the_dialog_did_not_take_says_so() {
    let (_dir, host) = fake_host(&[
        ("agent pending", pending_json(9), None),
        (
            "agent answer",
            r#"{"id":"x","result":{"registered":false}}"#.to_string(),
            None,
        ),
    ]);
    let report = answer_on(
        host,
        9,
        answer::Choice::Decline(None),
        answer::Answerer::Owner,
    )
    .unwrap();
    assert_eq!(report["not_registered"], true, "{report}");
    assert_eq!(report["answered"], "declined");
}

fn writer<'a>(owns: &'a [String]) -> SpawnRequest<'a> {
    SpawnRequest {
        owns,
        ..spawn_req("claude-code", None, None, &[])
    }
}

#[tokio::test]
async fn a_second_writer_in_a_folder_is_refused_unless_their_paths_do_not_meet() {
    let db = Arc::new(Database::open_memory().unwrap());
    let vault = tempfile::tempdir().unwrap();
    let start = |req: SpawnRequest<'static>| {
        let (db, vault) = (db.clone(), vault.path().to_path_buf());
        async move {
            let (_dir, host) = startable_host();
            spawn_on(&vault, &db, Arc::new(host), req).await
        }
    };
    let (api, ui, api_handlers) = (
        Box::leak(Box::new(vec!["src/api".to_string()])),
        Box::leak(Box::new(vec!["web/".to_string()])),
        Box::leak(Box::new(vec!["src/api/handlers".to_string()])),
    );
    let first = start(writer(api)).await.unwrap();
    assert_eq!(first["owns"], serde_json::json!(["src/api"]));
    start(writer(ui))
        .await
        .expect("paths that do not meet share the folder");

    let err = start(writer(api_handlers)).await.unwrap_err().to_string();
    assert!(
        err.contains("already writes in /t") && err.contains("src/api"),
        "{err}"
    );
    assert!(
        err.contains("worktree"),
        "the refusal names the way out: {err}"
    );
    let err = start(writer(&[])).await.unwrap_err().to_string();
    assert!(
        err.contains("already writes in /t"),
        "no owns is the whole folder: {err}"
    );

    // A reviewer writes nothing, so it may sit in the folder.
    start(spawn_req("claude-code", Some(Purpose::Review), None, &[]))
        .await
        .expect("a reviewer is not a writer");
}

const WORKTREE: &str = r#"{"id":"x","result":{"path":"/h/HQ/worktrees/t/hs-1","repo":"/t","branch":"feature","head":"abc123","created":true}}"#;

#[tokio::test]
async fn a_writer_with_a_worktree_starts_there_and_leaves_the_folder_free() {
    let db = Arc::new(Database::open_memory().unwrap());
    let vault = tempfile::tempdir().unwrap();
    let (dir, host) = fake_host(&[
        ("worktree add", WORKTREE.into(), None),
        ("workspace create", CREATED.into(), None),
        ("agent start", OK.into(), None),
        ("agent get", agent_json("hs-h", "idle"), None),
        ("agent prompt", OK.into(), None),
    ]);
    let req = SpawnRequest {
        worktree: Some(crate::agent_host::WorktreeBase::Branch("feature".into())),
        ..spawn_req("claude-code", None, None, &[])
    };
    let report = spawn_on(vault.path(), &db, Arc::new(host), req)
        .await
        .unwrap();
    assert_eq!(report["cwd"], "/h/HQ/worktrees/t/hs-1");
    assert_eq!(report["worktree"]["branch"], "feature");
    assert_eq!(report["worktree"]["repo"], "/t");
    let calls = std::fs::read_to_string(dir.path().join("calls.log")).unwrap();
    assert!(
        calls.contains("worktree add --repo /t --session hs-claude-code-")
            && calls.contains("--branch feature"),
        "{calls}"
    );
    let id = report["session_id"].as_str().unwrap().to_string();
    let checkout = db
        .with_conn(|c| hq_db::session_checkouts::get(c, &id))
        .unwrap()
        .unwrap();
    assert_eq!(checkout.folder, "/t");
    assert_eq!(checkout.worktree.as_deref(), Some("/h/HQ/worktrees/t/hs-1"));
    assert_eq!(checkout.last_head.as_deref(), Some("abc123"));

    // The folder itself has no writer, so a plain session may start there.
    let (_dir, host) = startable_host();
    spawn_on(
        vault.path(),
        &db,
        Arc::new(host),
        spawn_req("claude-code", None, None, &[]),
    )
    .await
    .expect("the worktree writer does not hold the folder");
}

#[tokio::test]
async fn a_launch_that_fails_takes_the_new_worktree_with_it() {
    let db = Arc::new(Database::open_memory().unwrap());
    let vault = tempfile::tempdir().unwrap();
    let (dir, host) = fake_host(&[
        ("worktree add", WORKTREE.into(), None),
        (
            "worktree remove",
            r#"{"removed":true,"dirty":false}"#.into(),
            None,
        ),
        ("workspace create", String::new(), Some("no room")),
    ]);
    let req = SpawnRequest {
        worktree: Some(crate::agent_host::WorktreeBase::Branch("feature".into())),
        ..spawn_req("claude-code", None, None, &[])
    };
    assert!(
        spawn_on(vault.path(), &db, Arc::new(host), req)
            .await
            .is_err()
    );
    let calls = std::fs::read_to_string(dir.path().join("calls.log")).unwrap();
    assert!(
        calls.contains("worktree remove /h/HQ/worktrees/t/hs-1"),
        "{calls}"
    );
}

#[test]
fn a_stopped_sessions_worktree_goes_unless_something_is_uncommitted() {
    let db = Arc::new(Database::open_memory().unwrap());
    let record = |id: &str| {
        db.with_conn(|c| {
            hq_db::session_checkouts::record(
                c,
                &hq_db::session_checkouts::NewCheckout {
                    session_id: id,
                    folder: "/t",
                    worktree: Some("/h/HQ/worktrees/t/x"),
                    repo: Some("/t"),
                    branch: Some("feature"),
                    detached: None,
                    owns: &[],
                    head: Some("abc"),
                },
            )
        })
        .unwrap();
    };
    record("hs-clean");
    let (_d, clean) = fake_host(&[(
        "worktree remove",
        r#"{"removed":true,"dirty":false}"#.into(),
        None,
    )]);
    let clean: crate::agent_host::Host = Arc::new(clean);
    let said = checkout::release(&db, &clean, "hs-clean").unwrap();
    assert_eq!(said["removed"], "/h/HQ/worktrees/t/x");
    assert!(
        checkout::release(&db, &clean, "hs-clean").is_none(),
        "removed once"
    );

    record("hs-dirty");
    let (_d, dirty) = fake_host(&[(
        "worktree remove",
        r#"{"removed":false,"dirty":true}"#.into(),
        None,
    )]);
    let said = checkout::release(
        &db,
        &(Arc::new(dirty) as crate::agent_host::Host),
        "hs-dirty",
    )
    .unwrap();
    assert_eq!(said["kept"], "/h/HQ/worktrees/t/x");
    assert!(checkout::release(&db, &clean, "nobody").is_none());
}

#[test]
fn a_commit_is_news_once_and_the_starting_point_never() {
    let db = Arc::new(Database::open_memory().unwrap());
    db.with_conn(|c| {
        registry::insert(
            c,
            &registry::NewSession {
                id: "hs-w",
                harness: "claude-code",
                label: "writer",
                cwd: "/t",
                mission_id: None,
                placement: registry::Placement {
                    host: "local",
                    agent_name: "hs-w",
                    workspace_id: "w1",
                    pane_id: "w1:p1",
                },
                purpose: None,
            },
        )?;
        hq_db::session_checkouts::record(
            c,
            &hq_db::session_checkouts::NewCheckout {
                session_id: "hs-w",
                folder: "/t",
                worktree: None,
                repo: None,
                branch: None,
                detached: None,
                owns: &[],
                head: None,
            },
        )
    })
    .unwrap();
    let head = |sha: &str| crate::agent_host::GitHead {
        sha: sha.into(),
        subject: "add the parser".into(),
        branch: Some("feature".into()),
    };
    assert!(
        !checkout::note_commit(&db, "hs-w", &head("aaa")),
        "where it started"
    );
    assert!(checkout::note_commit(&db, "hs-w", &head("bbb")));
    assert!(
        !checkout::note_commit(&db, "hs-w", &head("bbb")),
        "said once"
    );
    let events = db
        .with_conn(|c| registry::list_events(c, "hs-w", 10))
        .unwrap();
    assert_eq!(events.iter().filter(|e| e.kind == "commit").count(), 1);
    assert_eq!(
        checkout::commit_line(&head("0123456789abcdef")),
        "committed 0123456789ab \"add the parser\" on branch feature"
    );
}

#[test]
fn worktree_and_owns_arguments_are_checked() {
    use crate::agent_host::WorktreeBase;
    assert_eq!(checkout::parse_worktree(&json!({})).unwrap(), None);
    assert_eq!(
        checkout::parse_worktree(&json!({ "worktree": { "branch": "feat" } })).unwrap(),
        Some(WorktreeBase::Branch("feat".into()))
    );
    assert_eq!(
        checkout::parse_worktree(&json!({ "worktree": { "detached": "abc" } })).unwrap(),
        Some(WorktreeBase::Detached("abc".into()))
    );
    for bad in [
        json!({ "worktree": {} }),
        json!({ "worktree": { "branch": "a", "detached": "b" } }),
        json!({ "worktree": { "branch": " " } }),
    ] {
        assert!(checkout::parse_worktree(&bad).is_err(), "{bad}");
    }
    assert_eq!(
        checkout::parse_owns(&json!({ "owns": ["./src/", "src", "docs/a.md"] })).unwrap(),
        ["src", "docs/a.md"]
    );
    for bad in [
        json!(["/etc"]),
        json!(["../x"]),
        json!(["."]),
        json!(["a/../b"]),
    ] {
        assert!(
            checkout::parse_owns(&json!({ "owns": bad })).is_err(),
            "{bad}"
        );
    }
    assert!(checkout::overlap(&[], &["a".into()]));
    assert!(checkout::overlap(&["src".into()], &["src/api".into()]));
    assert!(!checkout::overlap(
        &["src/api".into()],
        &["src/apiv2".into()]
    ));
}

#[tokio::test]
async fn a_session_starts_under_its_purposes_policy_unless_it_names_another() {
    let db = Arc::new(Database::open_memory().unwrap());
    let vault = tempfile::tempdir().unwrap();
    let start = |req: SpawnRequest<'static>| {
        let (db, vault) = (db.clone(), vault.path().to_path_buf());
        async move {
            let (_dir, host) = startable_host();
            spawn_on(&vault, &db, Arc::new(host), req).await
        }
    };
    let coder = start(spawn_req("claude-code", None, None, &[]))
        .await
        .unwrap();
    assert_eq!(coder["policy"], "builder");
    let id = coder["session_id"].as_str().unwrap();
    assert_eq!(policy::of(&db, id).unwrap().name, "builder");
    assert!(
        db.with_conn(|c| registry::list_events(c, id, 10))
            .unwrap()
            .iter()
            .any(|e| e.kind == "policy")
    );

    let reviewer = start(spawn_req("claude-code", Some(Purpose::Review), None, &[]))
        .await
        .unwrap();
    assert_eq!(reviewer["policy"], "reviewer");
    let none = start(SpawnRequest {
        policy: Some("none"),
        ..spawn_req("codex", Some(Purpose::Review), None, &[])
    })
    .await
    .unwrap();
    assert!(none.get("policy").is_none());
    let err = start(SpawnRequest {
        policy: Some("yolo"),
        ..spawn_req("codex", Some(Purpose::Review), None, &[])
    })
    .await
    .unwrap_err()
    .to_string();
    assert!(err.contains("no command policy 'yolo'"), "{err}");
}

#[test]
fn a_started_session_may_not_widen_its_starters_policy() {
    let db = Arc::new(Database::open_memory().unwrap());
    let builder = hq_core::command_policy::resolve("builder", &Default::default())
        .unwrap()
        .unwrap();
    db.with_conn(|c| hq_db::session_policies::record(c, "hs-lead", &builder))
        .unwrap();
    assert_eq!(
        policy::choose(&db, None, Purpose::Code, Some("hs-lead"))
            .unwrap()
            .unwrap()
            .name,
        "builder"
    );
    let err = policy::choose(&db, Some("reviewer"), Purpose::Code, Some("hs-lead"))
        .unwrap_err()
        .to_string();
    assert!(err.contains("lets through more than 'builder'"), "{err}");
    let err = policy::choose(&db, Some("none"), Purpose::Code, Some("hs-lead"))
        .unwrap_err()
        .to_string();
    assert!(err.contains("needs one too"), "{err}");
    // A starter without a policy constrains nothing.
    assert!(policy::choose(&db, Some("reviewer"), Purpose::Code, Some("hs-other")).is_ok());
}

fn command_dialog(kind: &str, command: &str, truncated: bool) -> crate::agent_host::PendingDialog {
    let option = |id, label: &str, remember| crate::agent_host::PendingOption {
        id,
        label: label.into(),
        remember,
        typed: false,
        selected: id == 1,
    };
    crate::agent_host::PendingDialog {
        kind: kind.into(),
        question: Some("Run this?".into()),
        command: Some(command.into()),
        options: vec![
            option(1, "Yes", false),
            option(2, "Yes, always", true),
            option(3, "No", false),
        ],
        truncated,
        text: None,
    }
}

#[test]
fn a_policy_judges_commands_and_paths_and_leaves_the_rest_to_the_owner() {
    let builder = hq_core::command_policy::resolve("builder", &Default::default())
        .unwrap()
        .unwrap();
    let judge = |d| policy::judge(&builder, &d, "/w/app").map(|d| d.as_str());
    assert_eq!(
        judge(command_dialog("command", "git status", false)),
        Some("allow")
    );
    assert_eq!(
        judge(command_dialog("command", "git push", false)),
        Some("deny")
    );
    assert_eq!(
        judge(command_dialog("path", "/w/app/src", false)),
        Some("allow")
    );
    assert_eq!(
        judge(command_dialog("edit", "/etc/hosts", false)),
        Some("escalate")
    );
    assert_eq!(
        judge(command_dialog("command", "git status", true)),
        Some("escalate"),
        "a cut dialog may hide more"
    );
    assert_eq!(
        judge(command_dialog("command", "git push", true)),
        Some("deny")
    );
    assert_eq!(judge(command_dialog("question", "x", false)), None);
    assert_eq!(judge(command_dialog("trust", "x", false)), None);
    assert_eq!(
        policy::yes_option(&command_dialog("command", "ls", false)),
        Some(1),
        "never the remember option"
    );
}

#[test]
fn copilots_denies_come_from_the_policy() {
    let builder = hq_core::command_policy::resolve("builder", &Default::default())
        .unwrap()
        .unwrap();
    let reviewer = hq_core::command_policy::resolve("reviewer", &Default::default())
        .unwrap()
        .unwrap();
    let copilot: Vec<String> = spec::COPILOT_ARGS.iter().map(|a| a.to_string()).collect();
    let b = with_policy_args(copilot.clone(), Some(&builder));
    assert_eq!(b[0], "--allow-all-tools");
    assert!(b.contains(&"--deny-tool=shell(git push)".to_string()));
    assert!(b.contains(&"--deny-tool=shell(git restore)".to_string()));
    let r = with_policy_args(copilot.clone(), Some(&reviewer));
    assert!(
        !r.contains(&"--deny-tool=shell(git push)".to_string()),
        "a reviewer may push: {r:?}"
    );
    assert_eq!(with_policy_args(copilot.clone(), None), copilot);
    let custom = vec!["--model".to_string(), "x".to_string()];
    assert_eq!(with_policy_args(custom.clone(), Some(&builder)), custom);
}

#[test]
fn a_drive_turn_does_not_answer_a_command_dialog_a_policy_decides() {
    let db = Arc::new(Database::open_memory().unwrap());
    let d = command_dialog("command", "curl https://example.com", false);
    assert!(
        policy::refuse_drive(&db, "hs-1", &d).is_none(),
        "no policy: as before"
    );
    let builder = hq_core::command_policy::resolve("builder", &Default::default())
        .unwrap()
        .unwrap();
    db.with_conn(|c| hq_db::session_policies::record(c, "hs-1", &builder))
        .unwrap();
    assert!(
        policy::refuse_drive(&db, "hs-1", &d)
            .unwrap()
            .contains("waits for the owner")
    );
    assert!(policy::refuse_drive(&db, "hs-1", &command_dialog("question", "q", false)).is_none());
}

/// Sessions `lead` (started by a claude.ai connection, on task `task`) with a worker under it,
/// and an unrelated `other`.
fn event_world(db: &Database) -> String {
    let task = a_task(db);
    db.with_conn(|c| {
        for id in ["hs-lead", "hs-worker", "hs-other"] {
            registry::insert(
                c,
                &registry::NewSession {
                    id,
                    harness: "claude-code",
                    label: id,
                    cwd: "/w",
                    mission_id: None,
                    placement: registry::Placement {
                        host: "local",
                        agent_name: id,
                        workspace_id: "w1",
                        pane_id: "w1:p1",
                    },
                    purpose: None,
                },
            )?;
        }
        registry::set_parent(c, "hs-worker", "hs-lead", 1)?;
        registry::set_origin(c, "hs-lead", registry::ORIGIN_CONNECTOR)?;
        registry::set_mission(c, "hs-lead", &task)?;
        Ok(())
    })
    .unwrap();
    task
}

fn kinds(v: &Value) -> Vec<String> {
    v["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            format!(
                "{}:{}",
                e["session_id"].as_str().unwrap(),
                e["kind"].as_str().unwrap()
            )
        })
        .collect()
}

#[test]
fn the_event_stream_reads_each_event_once_from_a_cursor() {
    use events::{Reader, Scope, read};
    let db = Arc::new(Database::open_memory().unwrap());
    event_world(&db);
    let owner = Reader::default();
    let start = read(&db, &Scope::All, &owner, None, 10, Duration::ZERO).unwrap();
    let cursor = start["next"].as_i64().unwrap();
    db.with_conn(|c| {
        registry::record_event(
            c,
            "hs-worker",
            "blocked",
            "supervisor",
            Some(r#"{"seq":3}"#),
        )?;
        registry::record_event(c, "hs-other", "finished", "supervisor", None)
    })
    .unwrap();
    let team = Scope::Team("hs-lead".into());
    let got = read(&db, &team, &owner, Some(cursor), 10, Duration::ZERO).unwrap();
    assert_eq!(kinds(&got), ["hs-worker:blocked"]);
    assert_eq!(
        got["events"][0]["detail"]["seq"], 3,
        "detail comes back as data"
    );
    let next = got["next"].as_i64().unwrap();
    let again = read(&db, &team, &owner, Some(next), 10, Duration::ZERO).unwrap();
    assert!(kinds(&again).is_empty());
    assert_eq!(again["next"], next, "an empty read keeps the cursor");

    let everything = read(&db, &Scope::All, &owner, Some(cursor), 1, Duration::ZERO).unwrap();
    assert_eq!(everything["more"], true, "one at a time");
    let started = std::time::Instant::now();
    read(&db, &team, &owner, Some(next), 10, Duration::from_secs(1)).unwrap();
    assert!(
        started.elapsed() >= Duration::from_millis(900),
        "it waited for an event"
    );
}

#[test]
fn a_task_reads_its_sessions_with_their_teams() {
    use events::{Reader, Scope, read};
    let db = Arc::new(Database::open_memory().unwrap());
    let task = event_world(&db);
    db.with_conn(|c| {
        registry::record_event(c, "hs-lead", "finished", "supervisor", None)?;
        registry::record_event(c, "hs-worker", "commit", "supervisor", None)?;
        registry::record_event(c, "hs-other", "exited", "supervisor", None)
    })
    .unwrap();
    let got = read(
        &db,
        &Scope::Task(task),
        &Reader::default(),
        Some(0),
        10,
        Duration::ZERO,
    )
    .unwrap();
    assert_eq!(kinds(&got), ["hs-lead:finished", "hs-worker:commit"]);
}

#[test]
fn a_reader_sees_only_what_it_may() {
    use events::{Reader, Scope, read};
    let db = Arc::new(Database::open_memory().unwrap());
    event_world(&db);
    db.with_conn(|c| {
        registry::record_event(c, "hs-worker", "blocked", "supervisor", None)?;
        registry::record_event(c, "hs-other", "blocked", "supervisor", None)
    })
    .unwrap();
    let worker = Reader {
        caller: Some("hs-worker".into()),
        connector: false,
    };
    let own = read(&db, &Scope::All, &worker, Some(0), 10, Duration::ZERO).unwrap();
    assert_eq!(kinds(&own), ["hs-worker:blocked"], "its team only");
    let err = read(
        &db,
        &Scope::Sessions(vec!["hs-other".into()]),
        &worker,
        Some(0),
        10,
        Duration::ZERO,
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("its own team"), "{err}");

    let connector = Reader {
        caller: None,
        connector: true,
    };
    let theirs = read(&db, &Scope::All, &connector, Some(0), 10, Duration::ZERO).unwrap();
    assert_eq!(
        kinds(&theirs),
        ["hs-worker:blocked"],
        "only teams it started"
    );
    assert!(
        read(
            &db,
            &Scope::Sessions(vec!["hs-other".into()]),
            &connector,
            Some(0),
            10,
            Duration::ZERO
        )
        .is_err()
    );
}

#[test]
fn an_events_read_names_one_scope() {
    use events::Scope;
    assert_eq!(Scope::from_args(&json!({})).unwrap(), Scope::All);
    assert_eq!(
        Scope::from_args(&json!({ "team": "hs-1" })).unwrap(),
        Scope::Team("hs-1".into())
    );
    assert!(Scope::from_args(&json!({ "team": "hs-1", "task_id": "T-1" })).is_err());
}

#[test]
fn a_remote_client_adds_sessions_to_a_team_it_may_reach() {
    let db = Arc::new(Database::open_memory().unwrap());
    event_world(&db);
    let (root, depth) = team::join(&db, &json!({}), "hs-lead").unwrap();
    assert_eq!((root.as_str(), depth), ("hs-lead", 1));
    let connector = json!({ CONNECTOR_ARG: "Claude" });
    assert!(
        team::join(&db, &connector, "hs-lead").is_ok(),
        "a team it started"
    );
    db.with_conn(|c| registry::set_origin(c, "hs-other", registry::ORIGIN_USER))
        .unwrap();
    assert!(
        team::join(&db, &connector, "hs-other").is_err(),
        "not one it started"
    );
    let err = team::join(&db, &json!({}), "hs-worker")
        .unwrap_err()
        .to_string();
    assert!(err.contains("name the team's root, hs-lead"), "{err}");
    let err = team::join(&db, &json!({ HANDOFF_SCOPE_ARG: true }), "hs-lead")
        .unwrap_err()
        .to_string();
    assert!(err.contains("handoff key"), "{err}");
    let err = team::join(&db, &json!({ CALLER_SESSION_ARG: "hs-worker" }), "hs-lead")
        .unwrap_err()
        .to_string();
    assert!(err.contains("agent_delegate"), "{err}");
    db.with_conn(|c| registry::set_status(c, "hs-lead", registry::STATUS_STOPPED))
        .unwrap();
    assert!(
        team::join(&db, &json!({}), "hs-lead").is_err(),
        "a stopped root"
    );
    assert_eq!(team::writers(&db, "hs-lead"), ["hs-worker"]);
}

#[test]
fn moving_a_session_to_the_next_task_moves_its_goal_and_names_the_task_only() {
    let db = Arc::new(Database::open_memory().unwrap());
    let first = a_task(&db);
    let next = db
        .with_conn(|c| {
            let new = hq_db::tasks::NewTask {
                title: "step two: the parser",
                description: "Long brief nobody should paste.\n\nAcceptance criteria:\nthe parser tests pass",
                created_by: "test",
                ..Default::default()
            };
            Ok(hq_db::tasks::create_task(c, "tk-next", "in-1", &new)?.id)
        })
        .unwrap();
    seed_session(&db, "hs-move");
    link(&db, "hs-move", &first, None).unwrap();
    let moved = link_with(&db, "hs-move", &next, None, false, None).unwrap();
    assert_eq!(moved["goal_moved"], true);
    let message = moved["next_message"].as_str().unwrap();
    assert!(
        message.starts_with("Continue with ") && message.contains("step two: the parser"),
        "{message}"
    );
    assert!(!message.contains("Long brief"), "never the description");
    let row = get_row(&db, "hs-move").unwrap();
    assert!(
        row.goal
            .as_deref()
            .unwrap()
            .starts_with("step two: the parser")
    );
    assert_eq!(row.done_criteria.as_deref(), Some("the parser tests pass"));
    let same = link_with(&db, "hs-move", &next, None, false, None).unwrap();
    assert!(
        same.get("goal_moved").is_none(),
        "linking to the same task moves nothing"
    );
}

#[test]
fn a_task_lists_the_dialogs_its_agents_wait_on_you_for() {
    let db = Arc::new(Database::open_memory().unwrap());
    let task = event_world(&db);
    let item = hq_core::types::ValueItem::new(
        "session-supervisor",
        hq_core::types::ValueKind::ActionNeeded,
        "Harness session 'hs-worker' is waiting for you",
        "x",
    );
    hq_db::value_items::insert(&db, &item).unwrap();
    let options = vec![
        hq_db::dialog_alerts::DialogOption {
            id: 1,
            label: "Yes".into(),
            remember: false,
            typed: false,
        },
        hq_db::dialog_alerts::DialogOption {
            id: 2,
            label: "No".into(),
            remember: false,
            typed: false,
        },
    ];
    hq_db::dialog_alerts::record(
        &db,
        &hq_db::dialog_alerts::NewDialogAlert {
            item_id: &item.id,
            session_id: "hs-worker",
            pending_seq: 9,
            kind: "command",
            question: Some("Run this?"),
            command: Some("curl https://example.com"),
            options: &options,
        },
    )
    .unwrap();
    let got = decisions::for_task(&db, &task, &events::Reader::default()).unwrap();
    let list = got["decisions"].as_array().unwrap();
    assert_eq!(list.len(), 1, "the worker is in the task's team");
    assert_eq!(list[0]["seq"], 9);
    assert_eq!(
        list[0]["notification_id"],
        format!("val_{}", item.id),
        "what the answer route takes"
    );
    assert_eq!(list[0]["approve"]["label"], "Yes");
    hq_db::dialog_alerts::claim(&db, &item.id, "approve").unwrap();
    let got = decisions::for_task(&db, &task, &events::Reader::default()).unwrap();
    assert!(got["decisions"].as_array().unwrap().is_empty(), "answered");
}

fn seed_session(db: &Database, id: &str) {
    db.with_conn(|c| {
        registry::insert(
            c,
            &registry::NewSession {
                id,
                harness: "claude-code",
                label: id,
                cwd: "/w",
                mission_id: None,
                placement: registry::Placement {
                    host: "local",
                    agent_name: id,
                    workspace_id: "w1",
                    pane_id: "w1:p1",
                },
                purpose: None,
            },
        )
    })
    .unwrap();
}

#[test]
fn an_answer_records_who_gave_it() {
    let (_dir, host) = fake_host(&[
        ("agent pending", pending_json(4), None),
        (
            "agent answer",
            r#"{"id":"x","result":{"registered":true}}"#.to_string(),
            None,
        ),
    ]);
    let db = Arc::new(Database::open_memory().unwrap());
    seed_watched(&db, "hs-a", None);
    let row = control::get_row(&db, "hs-a").unwrap();
    let host: crate::agent_host::Host = Arc::new(host);
    answer::answer_with(
        &db,
        &host,
        &row,
        4,
        answer::Choice::Option(1),
        answer::Answerer::Owner,
        None,
    )
    .unwrap();
    let events = db
        .with_conn(|c| registry::list_events(c, "hs-a", 10))
        .unwrap();
    let answered = events.iter().find(|e| e.kind == "answered").unwrap();
    assert_eq!(answered.actor, "owner");
    let d: Value = serde_json::from_str(answered.detail.as_deref().unwrap()).unwrap();
    assert_eq!(d["choice"]["option"], 1);
    assert_eq!(d["registered"], true);
}

fn event(db: &Database, id: &str, kind: &str, detail: Value) {
    db.with_conn(|c| registry::record_event(c, id, kind, "supervisor", Some(&detail.to_string())))
        .unwrap();
}

#[test]
fn a_teams_numbers_come_from_its_event_log() {
    let db = Arc::new(Database::open_memory().unwrap());
    event_world(&db);
    for (id, kind, d) in [
        ("hs-lead", "blocked", json!({ "seq": 1 })),
        ("hs-lead", "unblocked", json!({ "blocked_ms": 60_000 })),
        ("hs-worker", "question", json!({ "seq": 2 })),
        ("hs-worker", "unblocked", json!({ "blocked_ms": 600_000 })),
        (
            "hs-worker",
            "policy_decision",
            json!({ "outcome": "allowed" }),
        ),
        (
            "hs-worker",
            "policy_decision",
            json!({ "outcome": "denied" }),
        ),
        (
            "hs-worker",
            "policy_decision",
            json!({ "outcome": "escalated" }),
        ),
        ("hs-lead", "finished", json!({})),
        ("hs-worker", "commit", json!({ "sha": "abc" })),
        ("hs-other", "blocked", json!({ "seq": 9 })),
    ] {
        event(&db, id, kind, d);
    }
    db.with_conn(|c| {
        for (id, by) in [("hs-lead", "owner"), ("hs-worker", "the command policy")] {
            registry::record_event(
                c,
                id,
                "answered",
                by,
                Some(&json!({ "by": by, "registered": true }).to_string()),
            )?;
        }
        Ok(())
    })
    .unwrap();
    let got = stats::report(
        &db,
        &events::Scope::Team("hs-lead".into()),
        &events::Reader::default(),
    )
    .unwrap();
    assert_eq!(got["sessions"], 2, "hs-other is not in the team");
    assert_eq!(got["dialogs"]["blocked"], 2);
    assert_eq!(got["dialogs"]["questions"], 1);
    assert_eq!(got["dialogs"]["answered_by_owner"], 1);
    assert_eq!(
        got["dialogs"]["answered_automatically"]["the command policy"],
        1
    );
    assert_eq!(got["waits"]["count"], 2);
    assert_eq!(got["waits"]["max_ms"], 600_000);
    assert_eq!(got["waits"]["total_minutes"], 11);
    assert_eq!(got["policy"]["denied"], 1);
    assert_eq!(got["work"]["finished_turns"], 1);
    assert_eq!(got["work"]["commits"], 1);
}

#[test]
fn a_policy_summary_counts_since_the_last_one_and_quotes_only_rules() {
    let db = Arc::new(Database::open_memory().unwrap());
    seed_session(&db, "hs-p");
    assert!(
        policy::summary_since_last(&db, "hs-p").is_none(),
        "nothing decided"
    );
    for (outcome, rule) in [
        ("allowed", "cargo test"),
        ("denied", "git push"),
        ("denied", "git push"),
        ("escalated", "the policy does not name curl"),
    ] {
        event(
            &db,
            "hs-p",
            "policy_decision",
            json!({ "policy": "builder", "outcome": outcome, "rule": rule, "command": "curl https://example.com" }),
        );
    }
    let line = policy::summary_since_last(&db, "hs-p").unwrap();
    assert_eq!(
        line,
        "Command policy 'builder' on session `hs-p`: 1 allowed, 2 declined (`git push` x2), 1 left to the owner."
    );
    assert!(!line.contains("example.com"), "no command the agent wrote");
    assert!(
        policy::summary_since_last(&db, "hs-p").is_none(),
        "said once"
    );
    event(
        &db,
        "hs-p",
        "policy_decision",
        json!({ "policy": "builder", "outcome": "allowed", "rule": "ls" }),
    );
    assert!(
        policy::summary_since_last(&db, "hs-p")
            .unwrap()
            .contains("1 allowed, 0 declined")
    );
}
