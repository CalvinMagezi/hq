use super::*;
use hq_core::config::HerdrHostConfig;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

const CREATED: &str = r#"{"id":"cli:workspace:create","result":{"type":"workspace_created","workspace":{"workspace_id":"w9","label":"t"},"root_pane":{"pane_id":"w9:p1","cwd":"/tmp"}}}"#;
const OK: &str = r#"{"id":"x","result":{"type":"ok"}}"#;
const IDLE_AGENT: &str = r#"{"id":"x","result":{"type":"agent_info","agent":{"agent":"claude","agent_status":"idle","cwd":"/tmp","name":"hs-a","pane_id":"w9:p1","workspace_id":"w9","tab_id":"w9:t1","state_change_seq":7}}}"#;
const BLOCKED_AGENT: &str = r#"{"id":"x","result":{"type":"agent_info","agent":{"agent":"claude","agent_status":"blocked","cwd":"/tmp","name":"hs-a","pane_id":"w9:p1","workspace_id":"w9","tab_id":"w9:t1","state_change_seq":3,"launch_pending":true}}}"#;
const NOT_FOUND: &str =
    r#"{"error":{"code":"agent_not_found","message":"agent target hs-a not found"},"id":"x"}"#;
const NOT_READY: &str =
    r#"{"error":{"code":"agent_not_ready","message":"blocked during startup"},"id":"x"}"#;
const STALLED: &str =
    r#"{"error":{"code":"agent_prompt_stalled","message":"no activity"},"id":"x"}"#;
const LIST: &str = r#"{"id":"x","result":{"type":"agent_list","agents":[{"agent":"claude","agent_status":"working","cwd":"/r","pane_id":"w2:p1","workspace_id":"w2","tab_id":"w2:t1","terminal_title_stripped":"Fixing bug","state_change_seq":5},{"agent":"codex","agent_status":"idle","cwd":"/r","name":"reviewer","pane_id":"w3:p1","workspace_id":"w3","tab_id":"w3:t1","state_change_seq":1}]}}"#;

/// A fake `herdr` that answers each subcommand from `replies` (prefix of the
/// argument string, then stdout, then stderr and exit code) and logs every call.
struct Fake {
    dir: tempfile::TempDir,
}

impl Fake {
    fn new(replies: &[(&str, &str, Option<&str>)]) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let mut script = String::from(
            "#!/bin/sh\necho \"$@\" >> \"$(dirname \"$0\")/calls.log\"\ncase \"$*\" in\n",
        );
        for (prefix, stdout, stderr) in replies {
            let (out, code) = match stderr {
                Some(err) => (format!("printf '%s' '{err}' >&2"), 1),
                None => (format!("printf '%s' '{stdout}'"), 0),
            };
            script.push_str(&format!("  *\"{prefix}\"*) {out}; exit {code} ;;\n"));
        }
        script.push_str("  *) echo unexpected >&2; exit 2 ;;\nesac\n");
        let path = dir.path().join("herdr");
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        Self { dir }
    }

    fn host(&self, session: Option<&str>) -> HerdrHost {
        let cfg = HerdrConfig {
            binary: self.binary().to_string_lossy().to_string(),
            session: session.map(str::to_string),
            ..HerdrConfig::default()
        };
        HerdrHost::from_config(&cfg, LOCAL_HOST).unwrap()
    }

    fn binary(&self) -> PathBuf {
        self.dir.path().join("herdr")
    }

    fn calls(&self) -> String {
        std::fs::read_to_string(self.dir.path().join("calls.log")).unwrap_or_default()
    }
}

fn request() -> LaunchRequest {
    LaunchRequest {
        name: "hs-a".into(),
        kind: "claude".into(),
        cwd: "/tmp".into(),
        label: "t".into(),
        env: vec![("K".into(), "V".into())],
        args: vec!["--dangerously-skip-permissions".into()],
        command: None,
        start_timeout: Duration::from_secs(30),
    }
}

#[test]
fn launch_reports_a_ready_agent() {
    let fake = Fake::new(&[
        ("workspace create", CREATED, None),
        ("agent start", OK, None),
        ("agent get", IDLE_AGENT, None),
    ]);
    let launched = fake.host(None).launch(&request()).unwrap();
    assert!(launched.ready);
    assert_eq!(launched.workspace_id, "w9");
    assert_eq!(launched.pane_id, "w9:p1");
    assert_eq!(launched.agent.unwrap().status, AgentStatus::Idle);
    let calls = fake.calls();
    assert!(calls.contains("--env K=V"), "{calls}");
    assert!(
        calls.contains("-- --dangerously-skip-permissions"),
        "{calls}"
    );
}

#[test]
fn launch_blocked_on_a_dialog_is_not_an_error() {
    let fake = Fake::new(&[
        ("workspace create", CREATED, None),
        ("agent start", "", Some(NOT_READY)),
        ("agent get", BLOCKED_AGENT, None),
    ]);
    let launched = fake.host(None).launch(&request()).unwrap();
    assert!(!launched.ready);
    let agent = launched.agent.unwrap();
    assert_eq!(agent.status, AgentStatus::Blocked);
    assert!(agent.launch_pending);
}

#[test]
fn a_failed_start_closes_the_workspace_it_opened() {
    let bad_kind = r#"{"error":{"code":"invalid_params","message":"unknown kind"}}"#;
    let fake = Fake::new(&[
        ("workspace create", CREATED, None),
        ("agent start", "", Some(bad_kind)),
        ("workspace close", OK, None),
    ]);
    let err = fake.host(None).launch(&request()).unwrap_err();
    assert_eq!(err.code(), Some("invalid_params"));
    assert!(
        fake.calls().contains("workspace close w9"),
        "{}",
        fake.calls()
    );
}

#[test]
fn a_missing_agent_is_none_not_an_error() {
    let fake = Fake::new(&[("agent get", "", Some(NOT_FOUND))]);
    assert!(fake.host(None).agent("hs-a").unwrap().is_none());
}

#[test]
fn an_unreachable_host_is_not_a_missing_agent() {
    let cfg = HerdrConfig {
        binary: "/nonexistent/herdr".into(),
        ..HerdrConfig::default()
    };
    let host = HerdrHost::from_config(&cfg, LOCAL_HOST).unwrap();
    assert!(host.agent("hs-a").unwrap_err().is_unreachable());
}

#[test]
fn a_stalled_prompt_is_reported_as_probably_delivered() {
    let fake = Fake::new(&[("agent prompt", "", Some(STALLED))]);
    let outcome = fake
        .host(None)
        .prompt("hs-a", "hi", Some(Duration::from_secs(10)))
        .unwrap();
    assert!(matches!(outcome, PromptOutcome::Stalled(_)));
}

#[test]
fn prompts_that_look_like_options_get_a_leading_space() {
    let fake = Fake::new(&[("agent prompt", OK, None)]);
    fake.host(None).prompt("hs-a", "-x go", None).unwrap();
    assert!(
        fake.calls().contains("agent prompt hs-a  -x go"),
        "{}",
        fake.calls()
    );
}

#[test]
fn agents_parse_named_and_unnamed_entries() {
    let fake = Fake::new(&[("agent list", LIST, None)]);
    let agents = fake.host(None).agents().unwrap();
    assert_eq!(agents.len(), 2);
    assert_eq!(agents[0].name, None);
    assert_eq!(agents[0].title.as_deref(), Some("Fixing bug"));
    assert_eq!(agents[1].name.as_deref(), Some("reviewer"));
}

#[test]
fn the_configured_session_leads_every_call() {
    let fake = Fake::new(&[("agent list", LIST, None)]);
    fake.host(Some("hq")).agents().unwrap();
    assert!(
        fake.calls().starts_with("--session hq agent list"),
        "{}",
        fake.calls()
    );
}

#[test]
fn unknown_host_names_the_known_ones() {
    let mut cfg = HerdrConfig::default();
    cfg.hosts.insert(
        "laptop".into(),
        HerdrHostConfig {
            ssh: "me@100.64.0.1".into(),
            identity_file: None,
            gate_command: "hq-herdr-gate".into(),
            session: None,
        },
    );
    let err = HerdrHost::from_config(&cfg, "nope")
        .unwrap_err()
        .to_string();
    assert!(err.contains("laptop"), "{err}");
    assert!(HerdrHost::from_config(&cfg, "laptop").is_ok());
}

/// Diagnostic against a real remote host behind `hq-herdr-gate`. It is not a
/// regression gate: it needs an ssh target and a key. Opt in with
///
/// `HQ_TEST_HERDR_SSH=user@host HQ_TEST_HERDR_KEY=/path/key \
///   cargo test -p hq-tools herdr::tests::real_ssh -- --ignored`
#[test]
#[ignore = "needs an ssh host running Herdr behind hq-herdr-gate"]
fn real_ssh_host_answers_through_the_gate() {
    let target = std::env::var("HQ_TEST_HERDR_SSH").expect("HQ_TEST_HERDR_SSH");
    let mut cfg = HerdrConfig::default();
    cfg.hosts.insert(
        "remote".into(),
        HerdrHostConfig {
            ssh: target,
            identity_file: std::env::var("HQ_TEST_HERDR_KEY").ok(),
            gate_command: "hq-herdr-gate".into(),
            session: None,
        },
    );
    let host = HerdrHost::from_config(&cfg, "remote").unwrap();

    let version = host.version().expect("version through the gate");
    let agents = host.agents().expect("agent list through the gate");
    eprintln!("herdr {version} answered with {} agents", agents.len());

    // A prompt that would be dangerous if any remote shell parsed it.
    let hostile = "$(touch /tmp/hq-gate-pwned); `id`; \"quoted\"";
    let err = host
        .prompt("no-such-agent-hq-test", hostile, None)
        .unwrap_err();
    assert_eq!(err.code(), Some("agent_not_found"), "{err}");
    assert!(!std::path::Path::new("/tmp/hq-gate-pwned").exists());
}

#[test]
fn a_wrapper_that_never_becomes_an_agent_is_an_error_and_closes_the_workspace() {
    let fake = Fake::new(&[
        ("workspace create", CREATED, None),
        ("pane run", "", None),
        ("agent get", "", Some(NOT_FOUND)),
        ("agent read", "zsh: command not found: my-wrapper", None),
        ("workspace close", OK, None),
    ]);
    let req = LaunchRequest {
        command: Some("my-wrapper".into()),
        start_timeout: Duration::from_millis(600),
        ..request()
    };
    let err = fake.host(None).launch(&req).unwrap_err();
    assert_eq!(err.code(), Some("agent_not_detected"));
    assert!(err.to_string().contains("command not found"), "{err}");
    assert!(
        fake.calls().contains("workspace close w9"),
        "{}",
        fake.calls()
    );
}

#[test]
fn a_wrapper_launch_quotes_arguments_and_names_the_agent() {
    let fake = Fake::new(&[
        ("workspace create", CREATED, None),
        ("pane run", "", None),
        ("agent rename", IDLE_AGENT, None),
        ("agent wait", IDLE_AGENT, None),
        ("agent get", IDLE_AGENT, None),
    ]);
    let req = LaunchRequest {
        command: Some("~/bin/claude-alt".into()),
        args: vec!["--model".into(), "a b".into(), "it's".into()],
        ..request()
    };
    let launched = fake.host(None).launch(&req).unwrap();
    assert!(launched.ready);
    let calls = fake.calls();
    assert!(
        calls.contains(r"pane run w9:p1 ~/bin/claude-alt --model 'a b' 'it'\''s'"),
        "{calls}"
    );
    assert!(calls.contains("agent rename w9:p1 hs-a"), "{calls}");
}

#[test]
fn a_wrapper_blocked_on_a_dialog_is_not_ready() {
    let fake = Fake::new(&[
        ("workspace create", CREATED, None),
        ("pane run", "", None),
        ("agent rename", BLOCKED_AGENT, None),
        ("agent wait", BLOCKED_AGENT, None),
        ("agent get", BLOCKED_AGENT, None),
    ]);
    let req = LaunchRequest {
        command: Some("w".into()),
        ..request()
    };
    assert!(!fake.host(None).launch(&req).unwrap().ready);
}

#[test]
fn a_submit_that_starts_a_turn_presses_nothing_more() {
    let fake = Fake::new(&[("agent prompt", IDLE_AGENT, None)]);
    let outcome = fake.host(None).submit("hs-a", "go").unwrap();
    assert!(matches!(outcome, PromptOutcome::Submitted));
    assert!(!fake.calls().contains("send-keys"), "{}", fake.calls());
}

#[test]
fn a_stalled_submit_presses_enter_once() {
    let fake = Fake::new(&[
        ("agent prompt", "", Some(STALLED)),
        ("agent send-keys", OK, None),
    ]);
    let outcome = fake.host(None).submit("hs-a", "go").unwrap();
    assert!(matches!(outcome, PromptOutcome::Resubmitted));
    assert!(
        fake.calls().contains("agent send-keys hs-a enter"),
        "{}",
        fake.calls()
    );
}

#[test]
fn shell_arguments_are_quoted_only_when_needed() {
    assert_eq!(shell_quote("--flag=1"), "--flag=1");
    assert_eq!(shell_quote(""), "''");
    assert_eq!(shell_quote("a b"), "'a b'");
    assert_eq!(shell_quote("$(x)"), "'$(x)'");
}

fn keys(k: &[&str]) -> Vec<String> {
    k.iter().map(|s| s.to_string()).collect()
}

#[test]
fn an_unreachable_host_sends_nothing() {
    let cfg = HerdrConfig {
        binary: "/nonexistent/herdr".into(),
        ..HerdrConfig::default()
    };
    let h = HerdrHost::from_config(&cfg, LOCAL_HOST).unwrap();
    let err = tools::send_to(&h, "w37:p4", "hi", &[])
        .unwrap_err()
        .to_string();
    assert!(err.contains("unreachable, nothing sent"), "{err}");
}

#[test]
fn a_stale_target_sends_nothing() {
    let fake = Fake::new(&[("agent get", "", Some(NOT_FOUND))]);
    let err = tools::send_to(&fake.host(None), "w37:p4", "hi", &[])
        .unwrap_err()
        .to_string();
    assert!(err.contains("unknown or stale target"), "{err}");
    assert!(!fake.calls().contains("agent prompt"), "{}", fake.calls());
}

#[test]
fn text_to_a_blocked_agent_is_refused_with_the_screen() {
    let fake = Fake::new(&[
        ("agent get", BLOCKED_AGENT, None),
        ("agent read", "Do you trust this folder?", None),
    ]);
    let err = tools::send_to(&fake.host(None), "w9:p1", "hi", &[])
        .unwrap_err()
        .to_string();
    assert!(err.contains("Do you trust this folder?"), "{err}");
    assert!(!fake.calls().contains("agent prompt"), "{}", fake.calls());
}

#[test]
fn keys_answer_a_blocked_agent() {
    let fake = Fake::new(&[
        ("agent get", BLOCKED_AGENT, None),
        ("agent send-keys", OK, None),
    ]);
    let out = tools::send_to(&fake.host(None), "w9:p1", "", &keys(&["down", "enter"])).unwrap();
    assert_eq!(out["status_before"], "blocked");
    assert!(
        fake.calls().contains("agent send-keys w9:p1 down enter"),
        "{}",
        fake.calls()
    );
}

#[test]
fn text_to_an_idle_agent_is_submitted() {
    let fake = Fake::new(&[
        ("agent get", IDLE_AGENT, None),
        ("agent prompt", IDLE_AGENT, None),
    ]);
    let out = tools::send_to(&fake.host(None), "w9:p1", "run the tests", &[]).unwrap();
    assert!(
        fake.calls().contains("agent prompt w9:p1 run the tests"),
        "{}",
        fake.calls()
    );
    assert_eq!(out["pane_id"], "w9:p1");
    assert_eq!(out["workspace_id"], "w9");
    assert_eq!(out["status_before"], "idle");
    assert!(out["note"].is_null());
}

#[test]
fn a_stalled_prompt_presses_enter_again_and_says_so() {
    let fake = Fake::new(&[
        ("agent get", IDLE_AGENT, None),
        ("agent prompt", "", Some(STALLED)),
        ("agent send-keys", OK, None),
    ]);
    let out = tools::send_to(&fake.host(None), "w9:p1", "go", &[]).unwrap();
    assert!(
        fake.calls().contains("agent send-keys w9:p1 enter"),
        "{}",
        fake.calls()
    );
    assert!(
        out["note"]
            .as_str()
            .unwrap()
            .contains("Enter was pressed once more")
    );
}

#[test]
fn herdr_send_needs_a_live_user_turn() {
    use crate::registry::HqTool;
    assert!(tools::HerdrSendTool.requires_live_user_turn());
}

const NOT_IDLE: &str = r#"{"error":{"code":"agent_not_idle","message":"cannot read 40 lines while hs-a is working"},"id":"x"}"#;

#[test]
fn a_deep_read_of_a_working_agent_falls_back_to_visible_once() {
    let fake = Fake::new(&[
        ("--source recent-unwrapped", "", Some(NOT_IDLE)),
        ("--source visible --lines 40", "live screen", None),
    ]);
    let host = fake.host(None);
    let (text, source) = host.read_sourced("hs-a", 40).unwrap();
    assert_eq!((text.as_str(), source), ("live screen", "visible"));
    assert_eq!(host.read("hs-a", 40).unwrap(), "live screen");
    let calls = fake.calls();
    let recent = "--source recent-unwrapped --lines 40";
    assert_eq!(calls.matches(recent).count(), 2);
    assert_eq!(calls.matches("--source visible --lines 40").count(), 2);
}

#[test]
fn an_idle_agent_is_read_without_the_visible_fallback() {
    let fake = Fake::new(&[("--source recent-unwrapped", "history", None)]);
    let (text, source) = fake.host(None).read_sourced("hs-a", 200).unwrap();
    assert_eq!((text.as_str(), source), ("history", "recent-unwrapped"));
    assert!(!fake.calls().contains("visible"), "{}", fake.calls());
}

#[test]
fn only_agent_not_idle_triggers_the_visible_fallback() {
    let fake = Fake::new(&[("agent read", "", Some(NOT_FOUND))]);
    let err = fake.host(None).read_sourced("hs-a", 40).unwrap_err();
    assert_eq!(err.code(), Some("agent_not_found"));
    assert_eq!(fake.calls().lines().count(), 1, "{}", fake.calls());

    let gone = HerdrHost::from_config(
        &HerdrConfig {
            binary: "/nonexistent/herdr".into(),
            ..HerdrConfig::default()
        },
        LOCAL_HOST,
    )
    .unwrap();
    assert!(gone.read_sourced("hs-a", 40).unwrap_err().is_unreachable());
}

#[test]
fn a_failed_visible_retry_is_the_error_and_is_not_retried_again() {
    let fake = Fake::new(&[("agent read", "", Some(NOT_IDLE))]);
    let err = fake.host(None).read_sourced("hs-a", 40).unwrap_err();
    assert_eq!(err.code(), Some("agent_not_idle"));
    assert_eq!(fake.calls().lines().count(), 2, "{}", fake.calls());
}

#[test]
fn writes_never_take_the_read_fallback() {
    let fake = Fake::new(&[("agent send-keys", "", Some(NOT_IDLE))]);
    let keys = vec!["enter".to_string()];
    assert!(fake.host(None).send_keys("hs-a", &keys).is_err());
    assert_eq!(fake.calls().lines().count(), 1, "{}", fake.calls());
    assert!(!fake.calls().contains("visible"));
}
