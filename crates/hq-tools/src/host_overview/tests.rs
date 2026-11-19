use super::*;
use crate::agent_host::scripted::ScriptedHost;
use chrono::TimeZone;
use hq_core::quota::{LaneQuota, QuotaWindow, WindowKind};
use std::os::unix::fs::PermissionsExt;
use std::sync::Arc;

/// A host script that answers `status`, `agent list` and `quota` from files beside it. A `down`
/// file makes every call fail, `slow` makes `status` hang, `quota_fail` fails only `quota`.
struct Fake {
    dir: tempfile::TempDir,
}

impl Fake {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let script = r#"#!/bin/sh
d="$(dirname "$0")"
if [ -e "$d/down" ]; then echo '{"error":{"code":"gone","message":"offline"}}' >&2; exit 1; fi
case "$1" in
  status) [ -e "$d/slow" ] && sleep 20; echo "version: 9.9.9" ;;
  agent) cat "$d/agents.json" ;;
  quota)
    if [ -e "$d/quota_fail" ]; then echo '{"error":{"code":"boom","message":"quota broke"}}' >&2; exit 1; fi
    cat "$d/quota.json" ;;
esac
"#;
        let path = dir.path().join("host");
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        let fake = Self { dir };
        fake.write(
            "agents.json",
            json!({ "result": { "agents": [{
                "name": "w1", "agent": "claude", "agent_status": "idle",
                "pane_id": "p1", "workspace_id": "w1", "cwd": "/work"
            }] } }),
        );
        fake.say(&[reading("claude", 40.0, Utc::now())]);
        fake
    }
    fn write(&self, file: &str, body: Value) {
        std::fs::write(self.dir.path().join(file), body.to_string()).unwrap();
    }
    fn say(&self, readings: &[LaneQuota]) {
        self.write("quota.json", json!({ "result": { "quotas": readings } }));
    }
    fn flag(&self, name: &str) {
        std::fs::write(self.dir.path().join(name), "").unwrap();
    }
    fn host(&self, name: &str) -> Host {
        Arc::new(
            ScriptedHost::new(self.dir.path().join("host"))
                .named(name)
                .with_command_timeout(Duration::from_secs(30)),
        )
    }
}

fn reading(lane: &str, used: f64, observed: DateTime<Utc>) -> LaneQuota {
    LaneQuota {
        lane: lane.into(),
        account: None,
        windows: vec![QuotaWindow {
            label: "week".into(),
            kind: WindowKind::Weekly,
            used_percent: Some(used),
            remaining: None,
            resets_at: Some(Utc::now() + chrono::Duration::days(3)),
        }],
        source: QuotaSource::Api,
        observed_at: observed,
        confidence: Confidence::Live,
        reason: None,
    }
}

fn unknown(lane: &str) -> LaneQuota {
    LaneQuota::unknown(lane, QuotaSource::Cli, "no login on this machine")
}

fn inputs(db: &Arc<Database>, refresh: bool, per_host: Duration) -> Inputs {
    Inputs {
        db: db.clone(),
        cfg: HqConfig::default(),
        refresh,
        bounds: Bounds {
            per_host,
            overall: OVERALL_BOUND,
        },
    }
}

fn row<'a>(out: &'a Value, host: &str) -> &'a Value {
    out["hosts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["host"] == host)
        .unwrap()
}

fn statuses(row: &Value) -> Vec<(String, String)> {
    row["quota"]["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            (
                r["lane"].as_str().unwrap().to_string(),
                r["status"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

#[test]
fn a_reachable_host_reports_version_agents_and_live_quota() {
    let db = Arc::new(Database::open_memory().unwrap());
    let fake = Fake::new();
    let out = overview(vec![fake.host("laptop")], &inputs(&db, true, HOST_TIMEOUT));
    let r = row(&out, "laptop");
    assert_eq!(r["reachability"], "reachable");
    assert_eq!(r["version"], "9.9.9");
    assert!(r["checked_at"].is_string());
    assert_eq!(r["agents"][0]["kind"], "claude");
    assert_eq!(statuses(r), [("claude".into(), "available".into())]);
    let q = &r["quota"]["rows"][0];
    assert_eq!(q["units"], "percent_used");
    assert_eq!(q["source"], "api");
    assert!(q["observed_at"].is_string());
    // The scripted host cannot say what is installed, so it never claims anything.
    assert!(
        r["harnesses"]
            .as_array()
            .unwrap()
            .iter()
            .all(|h| h["installed"] == "unknown")
    );
}

#[test]
fn an_unreachable_host_is_named_and_keeps_its_stored_quota_stale() {
    let db = Arc::new(Database::open_memory().unwrap());
    let (up, down) = (Fake::new(), Fake::new());
    overview(vec![down.host("laptop")], &inputs(&db, true, HOST_TIMEOUT));
    let gone: Host = Arc::new(ScriptedHost::new("/nonexistent/hq-host").named("laptop"));
    let started = Instant::now();
    let out = overview(vec![up.host("vps"), gone], &inputs(&db, true, HOST_TIMEOUT));
    assert!(started.elapsed() < Duration::from_secs(10));
    let laptop = row(&out, "laptop");
    assert_eq!(laptop["reachability"], "unreachable");
    assert!(laptop["reason"].as_str().unwrap().len() < 300);
    assert!(laptop["agents"].is_null());
    assert_eq!(statuses(laptop), [("claude".into(), "stale".into())]);
    let vps = row(&out, "vps");
    assert_eq!(vps["reachability"], "reachable");
    assert_eq!(statuses(vps), [("claude".into(), "available".into())]);
}

#[test]
fn a_host_that_hangs_is_unknown_within_the_bound_and_does_not_hide_the_rest() {
    let db = Arc::new(Database::open_memory().unwrap());
    let (ok, hung) = (Fake::new(), Fake::new());
    hung.flag("slow");
    let started = Instant::now();
    let out = overview(
        vec![hung.host("laptop"), ok.host("vps")],
        &inputs(&db, true, Duration::from_secs(4)),
    );
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "{:?}",
        started.elapsed()
    );
    let laptop = row(&out, "laptop");
    assert_eq!(laptop["reachability"], "unknown");
    assert_eq!(laptop["reason"], TIMED_OUT);
    assert!(laptop["agents"].is_null());
    assert_eq!(row(&out, "vps")["reachability"], "reachable");
}

#[test]
fn the_overall_bound_caps_a_longer_per_host_limit() {
    let bounds = Bounds {
        per_host: Duration::from_secs(60),
        overall: Duration::from_secs(5),
    };
    assert_eq!(bounds.effective(), Duration::from_secs(5));
}

#[test]
fn one_lane_that_cannot_be_read_does_not_drop_the_others() {
    let db = Arc::new(Database::open_memory().unwrap());
    let fake = Fake::new();
    fake.say(&[reading("claude", 40.0, Utc::now()), unknown("codex")]);
    let out = overview(vec![fake.host("laptop")], &inputs(&db, true, HOST_TIMEOUT));
    let r = row(&out, "laptop");
    assert_eq!(
        statuses(r),
        [
            ("claude".into(), "available".into()),
            ("codex".into(), "check_failed".into())
        ]
    );
    let failed = &r["quota"]["rows"][1];
    assert!(failed["used"].is_null() && failed["reason"].is_string());
}

#[test]
fn a_quota_call_that_fails_keeps_agents_and_marks_the_last_reading_stale() {
    let db = Arc::new(Database::open_memory().unwrap());
    let fake = Fake::new();
    overview(vec![fake.host("laptop")], &inputs(&db, true, HOST_TIMEOUT));
    fake.flag("quota_fail");
    let out = overview(vec![fake.host("laptop")], &inputs(&db, true, HOST_TIMEOUT));
    let r = row(&out, "laptop");
    assert_eq!(r["reachability"], "reachable");
    assert_eq!(r["agents"].as_array().unwrap().len(), 1);
    assert_eq!(statuses(r), [("claude".into(), "stale".into())]);
    assert!(r["quota"]["host_error"].is_string());
}

#[test]
fn a_ledger_lane_is_not_exposed_rather_than_failed() {
    let db = Arc::new(Database::open_memory().unwrap());
    let fake = Fake::new();
    fake.say(&[LaneQuota::unknown(
        "cursor",
        QuotaSource::Ledger,
        "no balance API",
    )]);
    let out = overview(vec![fake.host("laptop")], &inputs(&db, true, HOST_TIMEOUT));
    assert_eq!(
        statuses(row(&out, "laptop")),
        [("cursor".into(), "not_exposed".into())]
    );
}

#[test]
fn an_old_reading_is_never_available() {
    let now = Utc.with_ymd_and_hms(2026, 11, 15, 12, 0, 0).unwrap();
    let cfg = HqConfig::default();
    let q = reading("claude", 10.0, now - chrono::Duration::hours(6));
    let view = LaneView::from_reading("laptop", q, now, &cfg.quota);
    let max_age = chrono::Duration::minutes(cfg.quota.interval_minutes as i64);
    assert_eq!(quota_status(&view, false, max_age), QuotaStatus::Stale);
}
