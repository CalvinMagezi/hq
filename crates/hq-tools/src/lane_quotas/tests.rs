use super::*;
use crate::agent_host::scripted::ScriptedHost;
use chrono::TimeZone;
use hq_core::quota::WindowKind;
use std::os::unix::fs::PermissionsExt;
use std::sync::Arc;

/// A host whose answer is whatever `reply.json` holds, or a refusal while `down` exists. Every
/// request is appended to `calls.log`.
struct Fake {
    dir: tempfile::TempDir,
}

impl Fake {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let script = "#!/bin/sh\n\
            echo \"$@\" >> \"$(dirname \"$0\")/calls.log\"\n\
            d=\"$(dirname \"$0\")\"\n\
            if [ -e \"$d/down\" ]; then echo '{\"error\":{\"code\":\"gone\",\"message\":\"offline\"}}' >&2; exit 1; fi\n\
            cat \"$d/reply.json\"\n";
        let path = dir.path().join("host");
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        Self { dir }
    }
    fn host(&self, name: &str) -> Host {
        Arc::new(ScriptedHost::new(self.dir.path().join("host")).named(name))
    }
    fn say(&self, readings: &[LaneQuota]) {
        let body = json!({ "result": { "quotas": readings } });
        std::fs::write(self.dir.path().join("reply.json"), body.to_string()).unwrap();
    }
    fn set_down(&self, down: bool) {
        let flag = self.dir.path().join("down");
        if down {
            std::fs::write(flag, "").unwrap();
        } else {
            let _ = std::fs::remove_file(flag);
        }
    }
    fn calls(&self) -> String {
        std::fs::read_to_string(self.dir.path().join("calls.log")).unwrap_or_default()
    }
}

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 15, 12, 0, 0).unwrap()
}

fn reading(lane: &str, used: f64, observed: DateTime<Utc>) -> LaneQuota {
    LaneQuota {
        lane: lane.into(),
        account: Some("seat".into()),
        windows: vec![QuotaWindow {
            label: "week".into(),
            kind: WindowKind::Weekly,
            used_percent: Some(used),
            remaining: None,
            resets_at: Some(now() + Duration::days(3)),
        }],
        source: QuotaSource::Api,
        observed_at: observed,
        confidence: Confidence::Live,
        reason: None,
    }
}

fn cfg() -> HqConfig {
    HqConfig::default()
}

fn ask_now() -> Options {
    Options {
        refresh: true,
        ..Options::default()
    }
}

#[test]
fn a_live_reading_is_stored_and_reported_with_pace() {
    let db = Database::open_memory().unwrap();
    let fake = Fake::new();
    fake.say(&[reading("claude", 85.0, now())]);
    let r = read_hosts(&db, &cfg(), &[fake.host("laptop")], &ask_now(), now()).unwrap();
    assert!(r.hosts[0].reachable, "{:?}", r.hosts);
    let lane = &r.lanes[0];
    assert_eq!(
        (lane.host.as_str(), lane.lane.as_str()),
        ("laptop", "claude")
    );
    assert_eq!(lane.confidence, Confidence::Live);
    assert!(lane.windows[0].warn && !lane.windows[0].block);
    assert!(lane.windows[0].pace.is_some());
    let stored = db.with_conn(store::latest_per_lane).unwrap();
    assert_eq!(stored.len(), 1);
}

#[test]
fn an_unreachable_host_gives_the_last_reading_stale_never_zero() {
    let db = Database::open_memory().unwrap();
    let fake = Fake::new();
    let seen = now() - Duration::hours(2);
    fake.say(&[reading("claude", 42.0, seen)]);
    read_hosts(&db, &cfg(), &[fake.host("laptop")], &ask_now(), seen).unwrap();

    fake.set_down(true);
    let r = read_hosts(&db, &cfg(), &[fake.host("laptop")], &ask_now(), now()).unwrap();
    assert!(!r.hosts[0].reachable && r.hosts[0].error.is_some());
    let lane = &r.lanes[0];
    assert_eq!(lane.confidence, Confidence::Stale);
    assert_eq!(lane.age_seconds, 2 * 3600);
    assert_eq!(lane.windows[0].window.used_percent, Some(42.0));
    assert!(lane.reason.as_deref().unwrap().contains("offline"));
}

#[test]
fn an_unreachable_host_with_nothing_stored_reports_no_numbers() {
    let db = Database::open_memory().unwrap();
    let fake = Fake::new();
    fake.set_down(true);
    let r = read_hosts(&db, &cfg(), &[fake.host("laptop")], &ask_now(), now()).unwrap();
    assert!(r.lanes.is_empty() && !r.hosts[0].reachable);
    let named = Options {
        lane: Some("codex".into()),
        ..ask_now()
    };
    let r = read_hosts(&db, &cfg(), &[fake.host("laptop")], &named, now()).unwrap();
    assert_eq!(r.lanes[0].confidence, Confidence::Unknown);
    assert!(r.lanes[0].windows.is_empty());
}

#[test]
fn unknown_stays_unknown_and_never_overwrites_a_good_reading() {
    let db = Database::open_memory().unwrap();
    let fake = Fake::new();
    let unknown = LaneQuota::unknown("codex", QuotaSource::LocalLog, "no log found");
    fake.say(std::slice::from_ref(&unknown));
    let r = read_hosts(&db, &cfg(), &[fake.host("h")], &ask_now(), now()).unwrap();
    assert_eq!(r.lanes[0].confidence, Confidence::Unknown);
    assert!(r.lanes[0].windows.is_empty());
    assert!(db.with_conn(store::latest_per_lane).unwrap().is_empty());
    assert_eq!(unknown_lanes(&r)["h/codex"], "no log found");

    fake.say(&[reading("codex", 10.0, now() - Duration::hours(1))]);
    read_hosts(&db, &cfg(), &[fake.host("h")], &ask_now(), now()).unwrap();
    fake.say(&[unknown]);
    let r = read_hosts(&db, &cfg(), &[fake.host("h")], &ask_now(), now()).unwrap();
    assert_eq!(r.lanes[0].confidence, Confidence::Stale);
    assert_eq!(r.lanes[0].windows[0].window.used_percent, Some(10.0));
    assert!(
        r.lanes[0]
            .reason
            .as_deref()
            .unwrap()
            .contains("no log found")
    );
}

#[test]
fn without_refresh_a_recent_stored_reading_spares_the_host() {
    let db = Database::open_memory().unwrap();
    let fake = Fake::new();
    fake.say(&[reading("claude", 10.0, now() - Duration::minutes(1))]);
    read_hosts(&db, &cfg(), &[fake.host("h")], &ask_now(), now()).unwrap();
    let asked = fake.calls().lines().count();
    let plain = Options::default();
    let r = read_hosts(&db, &cfg(), &[fake.host("h")], &plain, now()).unwrap();
    assert_eq!(fake.calls().lines().count(), asked);
    assert_eq!(r.lanes[0].confidence, Confidence::Live);
    // Past the interval it asks again.
    let later = now() + Duration::minutes(30);
    read_hosts(&db, &cfg(), &[fake.host("h")], &plain, later).unwrap();
    assert_eq!(fake.calls().lines().count(), asked + 1);
}

#[test]
fn profile_readers_and_disabled_lanes_go_to_the_host_as_the_host_expects() {
    let db = Database::open_memory().unwrap();
    let fake = Fake::new();
    fake.say(&[]);
    let mut c = cfg();
    c.quota.lanes.insert("codex".into(), false);
    c.agent_host.harness_profiles = serde_yaml::from_str(
        "gateway:\n  base: claude-code\n  quota:\n    command: [/bin/echo, hi]\n    timeout_secs: 7\nplain:\n  base: claude-code\n",
    )
    .unwrap();
    let opts = Options {
        lane: Some("gateway".into()),
        ..ask_now()
    };
    read_hosts(&db, &c, &[fake.host("h")], &opts, now()).unwrap();
    let sent: Value = serde_json::from_str(
        fake.calls()
            .lines()
            .next()
            .unwrap()
            .trim_start_matches("quota "),
    )
    .unwrap();
    assert_eq!(sent["lane"], "gateway");
    assert_eq!(sent["disabled"], json!(["codex"]));
    assert_eq!(
        sent["custom"],
        json!([{ "lane": "gateway", "command": ["/bin/echo", "hi"], "timeout_secs": 7 }])
    );
}

#[test]
fn a_window_that_reset_after_it_was_read_is_marked_expired_and_flags_nothing() {
    let mut q = reading("claude", 99.0, now() - Duration::days(5));
    q.windows[0].resets_at = Some(now() - Duration::days(1));
    let v = LaneView::from_reading("h", as_stale(q, "x"), now(), &cfg().quota);
    let w = &v.windows[0];
    assert!(w.expired && !w.warn && !w.block && w.pace.is_none());
}

#[test]
fn off_means_nothing_is_asked_and_an_unknown_host_is_an_error() {
    let db = Database::open_memory().unwrap();
    let fake = Fake::new();
    fake.say(&[]);
    let mut c = cfg();
    c.quota.enabled = false;
    let r = read_hosts(&db, &c, &[fake.host("h")], &ask_now(), now()).unwrap();
    assert!(!r.enabled && r.lanes.is_empty() && fake.calls().is_empty());
    let opts = Options {
        host: Some("nope".into()),
        ..ask_now()
    };
    let err = read_hosts(&db, &cfg(), &[fake.host("h")], &opts, now()).unwrap_err();
    assert!(err.to_string().contains("known: h"));
}

#[test]
fn the_tool_is_read_only_and_the_report_has_a_stable_shape() {
    let tool = LaneQuotasTool::new(Arc::new(Database::open_memory().unwrap()));
    assert_eq!(tool.name(), "lane_quotas");
    assert!(tool.is_read_only());
    let props = tool.parameters()["properties"].clone();
    for key in ["host", "lane", "refresh"] {
        assert!(props.get(key).is_some(), "{key}");
    }

    let db = Database::open_memory().unwrap();
    let fake = Fake::new();
    fake.say(&[reading("claude", 50.0, now())]);
    let r = read_hosts(&db, &cfg(), &[fake.host("h")], &ask_now(), now()).unwrap();
    let v = serde_json::to_value(&r).unwrap();
    let lane = &v["lanes"][0];
    assert_eq!(lane["confidence"], "live");
    assert_eq!(lane["windows"][0]["kind"], "weekly");
    assert!(lane["windows"][0]["pace"]["status"].is_string());
    assert_eq!(v["hosts"][0]["reachable"], true);
    assert_eq!(v["warn_percent"], 80.0);
}
