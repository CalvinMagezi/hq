//! GitHub Copilot: the premium allowance of each account `gh` is logged in to.

use chrono::{DateTime, NaiveDate, Utc};
use hq_core::quota::{Confidence, LaneQuota, QuotaSource, QuotaWindow, WindowKind};
use serde_json::Value;

use super::{HTTP_TIMEOUT, LOCAL_CMD_TIMEOUT, OUTPUT_CAP_BYTES, clamp_percent, run_capture};

const LANE: &str = "copilot";
const USER_URL: &str = "https://api.github.com/copilot_internal/user";
// The endpoint answers editor clients only.
const EDITOR_VERSION: &str = "vscode/1.99.0";
const USER_AGENT: &str = "GitHubCopilotChat/0.26.7";
const PERCENT_SCALE: f64 = 100.0;

/// One account's premium snapshot.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Seat {
    pub entitlement: f64,
    pub remaining: f64,
    pub percent_remaining: f64,
    pub has_quota: bool,
    pub unlimited: bool,
    pub resets_at: Option<DateTime<Utc>>,
}

impl Seat {
    pub fn is_metered(&self) -> bool {
        self.has_quota && !self.unlimited && self.entitlement > 0.0
    }
}

fn num(v: &Value, key: &str) -> Option<f64> {
    v.get(key).and_then(Value::as_f64)
}

fn flag(v: &Value, key: &str) -> bool {
    v.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn reset(body: &Value) -> Option<DateTime<Utc>> {
    if let Some(s) = body.get("quota_reset_date_utc").and_then(Value::as_str)
        && let Ok(d) = DateTime::parse_from_rfc3339(s)
    {
        return Some(d.with_timezone(&Utc));
    }
    let day = body.get("quota_reset_date").and_then(Value::as_str)?;
    let date = NaiveDate::parse_from_str(day, "%Y-%m-%d").ok()?;
    Some(date.and_hms_opt(0, 0, 0)?.and_utc())
}

/// The `premium_interactions` snapshot of a `copilot_internal/user` body.
pub(crate) fn parse_seat(body: &Value) -> Option<Seat> {
    let snap = body.pointer("/quota_snapshots/premium_interactions")?;
    let entitlement = num(snap, "entitlement").unwrap_or(0.0);
    let remaining = num(snap, "quota_remaining")
        .or_else(|| num(snap, "remaining"))
        .unwrap_or(0.0);
    let percent_remaining = num(snap, "percent_remaining")
        .or_else(|| (entitlement > 0.0).then(|| remaining / entitlement * PERCENT_SCALE))?;
    Some(Seat {
        entitlement,
        remaining,
        percent_remaining,
        has_quota: flag(snap, "has_quota"),
        unlimited: flag(snap, "unlimited"),
        resets_at: reset(body),
    })
}

fn credits_window(seat: &Seat) -> QuotaWindow {
    let used = clamp_percent(PERCENT_SCALE - seat.percent_remaining);
    QuotaWindow {
        label: "premium requests".into(),
        kind: WindowKind::Credits,
        used_percent: used,
        remaining: Some(format!(
            "{} of {} left",
            seat.remaining.round(),
            seat.entitlement.round()
        )),
        resets_at: seat.resets_at,
    }
}

/// Logins from `gh auth status` text (`Logged in to github.com account <login> (...)`).
pub(crate) fn parse_gh_logins(text: &str) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for line in text.lines().filter(|l| l.contains("Logged in to")) {
        let mut words = line.split_whitespace();
        while let Some(w) = words.next() {
            if w == "account"
                && let Some(login) = words.next()
                && !found.iter().any(|f| f == login)
            {
                found.push(login.to_string());
            }
        }
    }
    found
}

fn gh(args: &[&str]) -> Result<super::run::Captured, String> {
    let mut argv = vec!["gh".to_string()];
    argv.extend(args.iter().map(|a| a.to_string()));
    run_capture(&argv, None, LOCAL_CMD_TIMEOUT, OUTPUT_CAP_BYTES)
}

fn seat_for(login: &str) -> Result<Seat, String> {
    let token = gh(&["auth", "token", "--user", login])?.stdout;
    let token = token.trim();
    if token.is_empty() {
        return Err("gh gave no token".into());
    }
    let headers = [
        ("Authorization", format!("token {token}")),
        ("Editor-Version", EDITOR_VERSION.to_string()),
        ("User-Agent", USER_AGENT.to_string()),
    ];
    let body = super::http_get_json(USER_URL, &headers, HTTP_TIMEOUT)?;
    parse_seat(&body).ok_or_else(|| "no premium snapshot in the answer".to_string())
}

/// Picks the reading out of every account's result: a metered seat wins; otherwise the lane is
/// unknown with the reason, never an empty answer.
pub(crate) fn choose(seats: Vec<(String, Result<Seat, String>)>) -> LaneQuota {
    let total = seats.len();
    let mut failures = Vec::new();
    let mut unmetered = 0usize;
    for (login, seat) in seats {
        match seat {
            Ok(seat) if seat.is_metered() => {
                return LaneQuota {
                    lane: LANE.into(),
                    account: Some(login),
                    windows: vec![credits_window(&seat)],
                    source: QuotaSource::Api,
                    observed_at: Utc::now(),
                    confidence: Confidence::Live,
                    reason: None,
                };
            }
            Ok(_) => unmetered += 1,
            Err(e) => failures.push(e),
        }
    }
    let reason = if unmetered == total {
        format!("{total} gh account(s), none with a metered Copilot seat (unmetered or free plan)")
    } else {
        format!(
            "no metered seat; {unmetered} unmetered, {} failed ({})",
            failures.len(),
            failures.first().map(String::as_str).unwrap_or("")
        )
    };
    LaneQuota::unknown(LANE, QuotaSource::Api, reason)
}

pub(crate) fn read() -> LaneQuota {
    let status = match gh(&["auth", "status"]) {
        // Older gh prints this to stderr, newer to stdout.
        Ok(c) => format!("{}\n{}", c.stdout, c.stderr),
        Err(e) => return LaneQuota::unknown(LANE, QuotaSource::Api, format!("gh: {e}")),
    };
    let logins = parse_gh_logins(&status);
    if logins.is_empty() {
        return LaneQuota::unknown(LANE, QuotaSource::Api, "gh has no logged-in account");
    }
    let seats = logins
        .into_iter()
        .map(|login| {
            let seat = seat_for(&login);
            (login, seat)
        })
        .collect();
    choose(seats)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> Value {
        let path = format!("{}/tests/fixtures/quota/{name}", env!("CARGO_MANIFEST_DIR"));
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    #[test]
    fn a_metered_seat_reports_used_percent_and_reset() {
        let seat = parse_seat(&fixture("copilot_metered.json")).unwrap();
        assert!(seat.is_metered());
        assert_eq!(
            seat.resets_at.unwrap().to_rfc3339(),
            "2026-12-01T00:00:00+00:00"
        );
        let q = choose(vec![("seat-user".into(), Ok(seat))]);
        assert_eq!(q.confidence, Confidence::Live);
        assert_eq!(q.account.as_deref(), Some("seat-user"));
        assert_eq!(q.windows[0].used_percent, Some(75.0));
        assert_eq!(q.windows[0].remaining.as_deref(), Some("75 of 300 left"));
    }

    #[test]
    fn an_unmetered_account_is_unknown_with_a_reason() {
        let seat = parse_seat(&fixture("copilot_unmetered.json")).unwrap();
        assert!(!seat.is_metered());
        let q = choose(vec![("free-user".into(), Ok(seat))]);
        assert_eq!(q.confidence, Confidence::Unknown);
        assert!(q.windows.is_empty());
        assert!(q.reason.unwrap().contains("none with a metered"));
    }

    #[test]
    fn the_metered_account_wins_over_an_unmetered_one() {
        let free = parse_seat(&fixture("copilot_unmetered.json")).unwrap();
        let paid = parse_seat(&fixture("copilot_metered.json")).unwrap();
        let q = choose(vec![
            ("free-user".into(), Ok(free)),
            ("bad".into(), Err("HTTP 401".into())),
            ("seat-user".into(), Ok(paid)),
        ]);
        assert_eq!(q.account.as_deref(), Some("seat-user"));
    }

    #[test]
    fn logins_come_from_gh_status_text() {
        let text = std::fs::read_to_string(format!(
            "{}/tests/fixtures/quota/gh_auth_status.txt",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap();
        assert_eq!(parse_gh_logins(&text), ["seat-user", "free-user"]);
    }
}
