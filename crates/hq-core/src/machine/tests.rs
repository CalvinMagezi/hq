use super::probe::{host_port_from_url, probe_web_search, resolve_and_connect};
use super::*;
use std::time::Duration;

#[test]
fn host_port_from_url_defaults_the_port_by_scheme() {
    // Regression: a searxng_url with no explicit port (a perfectly
    // normal config, e.g. behind a reverse proxy on 443/80) used to
    // return None entirely, making a genuinely reachable backend probe
    // as unavailable.
    assert_eq!(
        host_port_from_url("https://searx.example.org"),
        Some(("searx.example.org".to_string(), 443))
    );
    assert_eq!(
        host_port_from_url("http://searx.example.org"),
        Some(("searx.example.org".to_string(), 80))
    );
    assert_eq!(
        host_port_from_url("http://localhost:8080"),
        Some(("localhost".to_string(), 8080))
    );
}

#[test]
fn resolve_and_connect_does_not_hang_past_its_deadline() {
    // A non-routable TEST-NET-1 address (RFC 5737) that will neither
    // resolve as a hostname nor accept a connection — this is
    // specifically testing that a slow/failing attempt is bounded, not
    // that it succeeds.
    let start = std::time::Instant::now();
    let ok = resolve_and_connect("192.0.2.1".to_string(), 65535, Duration::from_millis(500));
    assert!(!ok);
    assert!(
        start.elapsed() < Duration::from_secs(2),
        "took {:?}, deadline should have bounded it",
        start.elapsed()
    );
}

fn fixture() -> MachineProfile {
    MachineProfile {
        generated_at: chrono::Utc::now(),
        os: "macos".into(),
        arch: "aarch64".into(),
        hostname: "test-host".into(),
        in_container: false,
        cpu_cores: 12,
        memory_gb: 36,
        home: PathBuf::from("/Users/test"),
        vault_path: Some(PathBuf::from("/Users/test/.vault")),
        binaries: PROBED_BINARIES
            .iter()
            .take(20)
            .map(|n| BinaryStatus {
                name: (*n).to_string(),
                path: Some(PathBuf::from(format!("/usr/bin/{n}"))),
                version: Some("1.2.3".into()),
            })
            .collect(),
        missing: vec!["gcloud".into(), "aws".into(), "vercel".into()],
        gh_auth: Some("alex (repo, workflow)".into()),
        deep_probed: true,
        docker_running: true,
        git_user: Some("Alice Example <alice@example.com>".into()),
        can_build_self: false,
        web_search_backend: Some("searxng (http://localhost:8080)".into()),
        web_search: Vec::new(),
    }
}

/// `sh` exists on every platform this runs on; asserting on `gh` or
/// `docker` would be flaky in CI.
#[test]
fn which_binary_finds_sh() {
    assert!(which_binary("sh").is_some());
}

#[test]
fn which_binary_returns_none_for_nonexistent() {
    assert!(which_binary("hq_definitely_not_a_real_binary_xyz").is_none());
}

#[test]
fn render_markdown_is_compact() {
    let md = render_markdown(&fixture());
    assert!(
        md.len() < 1500,
        "machine block grew to {} bytes; it ships in every prompt",
        md.len()
    );
}

/// The "never claim a Missing capability" contract depends on the block
/// actually naming what is absent.
#[test]
fn render_markdown_lists_missing_binaries() {
    let md = render_markdown(&fixture());
    assert!(md.contains("**Missing**"), "{md}");
    assert!(md.contains("gcloud"), "{md}");
    assert!(md.contains("Never claim a capability listed"), "{md}");
}

#[test]
fn render_markdown_reports_gh_auth_state() {
    let mut p = fixture();
    assert!(render_markdown(&p).contains("authenticated as alex"));

    p.gh_auth = None;
    let md = render_markdown(&p);
    assert!(md.contains("NOT authenticated"), "{md}");
    assert!(md.contains("gh auth login"), "{md}");
}

#[test]
fn web_search_unavailable_message_names_every_way_to_enable_a_backend() {
    // The message must work on a checkout-less VPS too, so it never points at
    // scripts/setup-searxng.sh (FR-005) and names config keys instead.
    for can_build_self in [false, true] {
        let mut p = fixture();
        p.can_build_self = can_build_self;
        p.web_search_backend = None;
        p.web_search = Vec::new();
        let md = render_markdown(&p);
        assert!(md.contains("UNAVAILABLE"), "{md}");
        assert!(!md.contains("scripts/setup-searxng.sh"), "{md}");
        assert!(md.contains("web_search_native"), "{md}");
    }
}

#[test]
fn probe_machine_completes_under_timeout() {
    let start = std::time::Instant::now();
    let profile = probe_machine(None);
    assert!(
        start.elapsed() < Duration::from_secs(10),
        "probe took {:?}",
        start.elapsed()
    );
    // `sh` implies a real PATH, so at least one probe must have resolved.
    assert!(!profile.binaries.is_empty() || !profile.missing.is_empty());
}

#[test]
fn refresh_writes_markdown_and_json() {
    let dir = tempfile::tempdir().unwrap();
    let profile = refresh(dir.path()).unwrap();
    let md = std::fs::read_to_string(dir.path().join("_system/MACHINE.md")).unwrap();
    assert!(md.starts_with("# Machine Profile"));
    let json = std::fs::read_to_string(dir.path().join("_system/machine.json")).unwrap();
    let parsed: MachineProfile = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.hostname, profile.hostname);
}

#[test]
fn load_cached_flags_stale_profiles() {
    let dir = tempfile::tempdir().unwrap();
    refresh(dir.path()).unwrap();
    let fresh = load_cached(dir.path(), Duration::from_secs(3600)).unwrap();
    assert!(!fresh.contains("profile is stale"));
    let stale = load_cached(dir.path(), Duration::from_nanos(1)).unwrap();
    assert!(stale.contains("profile is stale"), "{stale}");
}

#[test]
fn load_cached_returns_none_without_a_profile() {
    let dir = tempfile::tempdir().unwrap();
    assert!(load_cached(dir.path(), Duration::from_secs(3600)).is_none());
}

/// Port 0 is never bindable as a listener, so connecting to it always fails. A
/// bind-then-drop port is not safe: a parallel test binding `:0` can be handed
/// the same ephemeral port and make the "closed" port accept connections.
const UNCONNECTABLE_PORT: u16 = 0;

#[test]
fn web_search_probe_reports_each_backend_separately() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let statuses = probe_web_search(Some(&url), Some("secret-brave-key"), false);

    let searxng = &statuses[0];
    assert_eq!(searxng.provider, "searxng");
    assert!(searxng.configured);
    assert_eq!(searxng.reachable, Some(true));
    assert_eq!(searxng.answered, None, "the passive probe sends no query");

    let native = &statuses[1];
    assert_eq!(native.provider, "native");
    assert!(!native.configured, "disabled when the flag is off");

    // Regression: SearxNG being up used to hide Brave entirely, and a bare key read as healthy.
    let brave = &statuses[2];
    assert_eq!(brave.provider, "brave");
    assert!(brave.configured);
    assert_eq!(brave.reachable, None);
    assert_eq!(brave.answered, None);
    assert!(brave.detail.contains("not verified"), "{}", brave.detail);
    assert!(
        statuses
            .iter()
            .all(|s| !s.detail.contains("secret-brave-key"))
    );
}

#[test]
fn web_search_probe_marks_an_unreachable_searxng_unusable() {
    let url = format!("http://127.0.0.1:{}", UNCONNECTABLE_PORT);
    let statuses = probe_web_search(Some(&url), None, false);
    assert_eq!(statuses[0].reachable, Some(false));
    assert!(!statuses[0].usable());
    assert!(!statuses[1].configured);
    assert!(!statuses[2].configured);

    let mut p = fixture();
    p.web_search_backend = None;
    p.web_search = statuses;
    assert!(render_markdown(&p).contains("UNAVAILABLE"));
}

#[test]
fn web_search_render_never_calls_a_bare_brave_key_available() {
    let mut p = fixture();
    p.web_search = probe_web_search(None, Some("k"), false);
    let md = render_markdown(&p);
    assert!(
        md.contains("brave API key configured, not verified"),
        "{md}"
    );
    assert!(!md.contains("available via"), "{md}");
    assert!(md.contains("backend that answered"), "{md}");
}

#[test]
fn the_built_in_engines_count_as_a_usable_backend_when_enabled() {
    let statuses = probe_web_search(None, None, true);
    let native = &statuses[1];
    assert!(native.configured && native.usable());
    assert_eq!(native.reachable, None);

    let mut p = fixture();
    p.web_search_backend = None;
    p.web_search = statuses;
    let md = render_markdown(&p);
    assert!(md.contains("built-in engines enabled"), "{md}");
    assert!(!md.contains("UNAVAILABLE"), "{md}");
}
