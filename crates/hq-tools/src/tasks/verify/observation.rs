//! The facts a verification rests on. HQ writes them; a model only ever cites them by id.

use chrono::Utc;
use serde::Serialize;
use serde_json::{Value, json};

/// Longest fact sentence kept.
const MAX_FACT_CHARS: usize = 300;

/// One fact HQ recorded.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Observation {
    /// `o1`, `o2`, ...: what a criterion cites.
    pub id: String,
    /// Where it came from: `github`, `vault`, `http`, `diff_scan`, `hq`.
    pub source: String,
    /// What it is a fact about: `pr_state`, `check_runs`, `required_checks`, `note_hash`,
    /// `url_probe`, `diff_scan`, `changed_paths`.
    pub kind: String,
    /// One sentence a person can read.
    pub fact: String,
    /// `Some(true)` supports completion, `Some(false)` contradicts it, `None` is neither: it is
    /// informational, could not be read, or is flagged for scrutiny.
    pub ok: Option<bool>,
    /// The source could not be read. Never counts as support.
    pub unavailable: bool,
    pub data: Value,
    /// UTC `YYYY-MM-DD HH:MM:SS`.
    pub observed_at: String,
}

/// The observations of one verification, in the order they were recorded. There is no way to
/// change or remove one once recorded.
#[derive(Debug, Default, Clone)]
pub struct ObservationLog {
    items: Vec<Observation>,
}

impl ObservationLog {
    pub fn new() -> Self {
        Self::default()
    }

    fn push(&mut self, mut o: Observation) -> String {
        o.id = format!("o{}", self.items.len() + 1);
        o.fact = o.fact.chars().take(MAX_FACT_CHARS).collect();
        let id = o.id.clone();
        self.items.push(o);
        id
    }

    /// Records a fact and returns its id.
    pub fn record(
        &mut self,
        source: &str,
        kind: &str,
        fact: impl Into<String>,
        ok: Option<bool>,
        data: Value,
    ) -> String {
        self.push(Observation {
            id: String::new(),
            source: source.to_string(),
            kind: kind.to_string(),
            fact: fact.into(),
            ok,
            unavailable: false,
            data,
            observed_at: now(),
        })
    }

    /// Records that a source could not be read, and why. This is a fact too, and it never
    /// supports anything.
    pub fn unavailable(&mut self, source: &str, kind: &str, reason: impl Into<String>) -> String {
        let reason = reason.into();
        self.push(Observation {
            id: String::new(),
            source: source.to_string(),
            kind: kind.to_string(),
            fact: format!("could not read {kind}: {reason}"),
            ok: None,
            unavailable: true,
            data: json!({ "reason": reason }),
            observed_at: now(),
        })
    }

    pub fn items(&self) -> &[Observation] {
        &self.items
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn of_kind<'a>(&'a self, kind: &'a str) -> impl Iterator<Item = &'a Observation> + 'a {
        self.items.iter().filter(move |o| o.kind == kind)
    }

    /// Whether some observation of `kind` supports completion. An unavailable one never does.
    pub fn supports(&self, kind: &str) -> bool {
        self.of_kind(kind)
            .any(|o| o.ok == Some(true) && !o.unavailable)
    }

    /// Whether some observation of `kind` contradicts completion.
    pub fn contradicts(&self, kind: &str) -> bool {
        self.of_kind(kind).any(|o| o.ok == Some(false))
    }

    /// The ids, for checking that a criterion cites facts that exist.
    pub fn ids(&self) -> Vec<&str> {
        self.items.iter().map(|o| o.id.as_str()).collect()
    }

    /// The list as stored on a verdict.
    pub fn to_json(&self) -> Value {
        serde_json::to_value(&self.items).unwrap_or_else(|_| json!([]))
    }
}

fn now() -> String {
    Utc::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_sequential_and_facts_are_capped() {
        let mut log = ObservationLog::new();
        let a = log.record(
            "github",
            "pr_state",
            "x".repeat(1000),
            Some(true),
            json!({}),
        );
        let b = log.record("github", "check_runs", "ok", None, json!({}));
        assert_eq!((a.as_str(), b.as_str()), ("o1", "o2"));
        assert_eq!(log.items()[0].fact.chars().count(), 300);
        assert_eq!(log.ids(), ["o1", "o2"]);
    }

    #[test]
    fn an_unavailable_source_never_supports_anything() {
        let mut log = ObservationLog::new();
        log.unavailable("github", "check_runs", "gh is not installed");
        assert!(!log.supports("check_runs"));
        assert!(!log.contradicts("check_runs"));
        let o = &log.items()[0];
        assert!(o.unavailable && o.ok.is_none());
        assert!(o.fact.contains("gh is not installed"));
    }

    #[test]
    fn support_and_contradiction_follow_the_flag_per_kind() {
        let mut log = ObservationLog::new();
        log.record("github", "pr_state", "merged", Some(true), json!({}));
        log.record(
            "github",
            "check_runs",
            "a check failed",
            Some(false),
            json!({}),
        );
        assert!(log.supports("pr_state") && !log.supports("check_runs"));
        assert!(log.contradicts("check_runs") && !log.contradicts("pr_state"));
        assert!(
            !log.supports("note_hash"),
            "nothing recorded, nothing supported"
        );
    }

    #[test]
    fn the_json_form_is_what_a_verdict_stores() {
        let mut log = ObservationLog::new();
        log.record(
            "hq",
            "note_hash",
            "the note exists",
            Some(true),
            json!({"sha256": "ab"}),
        );
        let v = log.to_json();
        assert_eq!(v[0]["id"], "o1");
        assert_eq!(v[0]["data"]["sha256"], "ab");
        assert_eq!(v[0]["unavailable"], false);
    }
}
