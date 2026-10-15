//! What the skill invocation records say, with the sample size beside every claim.
//!
//! This reads `skill_invocations` (and nothing else) and reports, per skill and per version of
//! it, how often it ran, in how many sessions, how the work turned out, and which patterns
//! repeat. It never reads a prompt or a document, because none is stored, and it makes no claim
//! about cause: a pattern is "this keeps happening", not "this is why".
//!
//! A pattern only counts as evidence once it shows up in at least [`MIN_INVOCATIONS`] loads
//! across at least [`MIN_SESSIONS`] sessions inside the window ([`WINDOW_DAYS`] by default).
//! Anything smaller is still listed, labelled as too small, so a reader sees the numbers but
//! nothing is treated as a finding. The same thresholds decide whether a proposal is made.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use anyhow::Result;
use rusqlite::{Connection, params};
use serde::Serialize;

/// Loads showing the same pattern needed before it is evidence (decision D5).
pub const MIN_INVOCATIONS: u32 = 5;
/// Distinct sessions those loads must span.
pub const MIN_SESSIONS: u32 = 3;
/// How far back evidence is gathered unless asked otherwise.
pub const WINDOW_DAYS: u32 = 30;
/// Example invocation ids kept per pattern. Ids only: no text exists to quote.
const MAX_EXAMPLES: usize = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum PatternKind {
    /// The work the skill was used on keeps being reopened or rejected.
    ReworkedOutcomes,
    /// The post-session review keeps judging the skill harmful.
    JudgedHarmful,
    /// The skill is loaded and then not applied.
    LoadedButUnused,
    /// Work of one kind keeps coming back after the skill was used on it.
    ReworkedForPurpose,
    /// The skill is loaded by hand although its hints should load it.
    HintsDidNotActivate,
}

impl PatternKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ReworkedOutcomes => "reworked_outcomes",
            Self::JudgedHarmful => "judged_harmful",
            Self::LoadedButUnused => "loaded_but_unused",
            Self::ReworkedForPurpose => "reworked_for_purpose",
            Self::HintsDidNotActivate => "hints_did_not_activate",
        }
    }
}

/// One load behind a pattern, by reference. Nothing here is text anyone wrote.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Example {
    /// The invocation id.
    pub id: String,
    pub loaded_at: String,
    /// The task the session was on, when known.
    pub task_id: Option<String>,
    /// The record that settled the outcome (`task:<id>`, `run:<id>`, `note:<id>`), when one did.
    pub outcome_ref: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Pattern {
    pub kind: PatternKind,
    /// For `ReworkedForPurpose`, the purpose; otherwise empty.
    pub qualifier: String,
    /// The skill version the loads ran against; `None` when it was not recorded.
    pub skill_hash: Option<String>,
    pub invocations: u32,
    pub sessions: u32,
    pub first: String,
    pub last: String,
    /// Newest first.
    pub examples: Vec<Example>,
    /// Whether this reaches the evidence threshold.
    pub enough: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct VersionStats {
    pub skill_hash: Option<String>,
    pub invocations: u32,
    pub sessions: u32,
    pub first: String,
    pub last: String,
    pub scored: u32,
    pub mean_score: Option<f64>,
    pub accepted: u32,
    pub reworked: u32,
    pub rejected: u32,
    /// Loads with no acceptance signal yet (or `unknown`).
    pub no_signal: u32,
    pub helped: u32,
    pub hurt: u32,
    pub unused: u32,
}

/// How outcomes differ between a version and the one before it. Correlation only.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct VersionComparison {
    pub before: Option<String>,
    pub after: Option<String>,
    pub before_invocations: u32,
    pub after_invocations: u32,
    pub before_mean: Option<f64>,
    pub after_mean: Option<f64>,
    /// Either side is under the evidence threshold, so the difference is anecdotal.
    pub small_sample: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SkillEvidence {
    pub skill: String,
    pub window_days: u32,
    pub invocations: u32,
    pub sessions: u32,
    pub versions: Vec<VersionStats>,
    pub comparisons: Vec<VersionComparison>,
    pub patterns: Vec<Pattern>,
}

impl SkillEvidence {
    /// Patterns that reach the threshold.
    pub fn findings(&self) -> impl Iterator<Item = &Pattern> {
        self.patterns.iter().filter(|p| p.enough)
    }

    /// Whether there is anything to propose.
    pub fn sufficient(&self) -> bool {
        self.findings().next().is_some()
    }
}

fn enough(invocations: u32, sessions: u32) -> bool {
    invocations >= MIN_INVOCATIONS && sessions >= MIN_SESSIONS
}

struct Row {
    id: String,
    skill: String,
    session: String,
    trigger: String,
    loaded_at: String,
    hash: Option<String>,
    purpose: Option<String>,
    used: Option<String>,
    score: Option<f64>,
    status: Option<String>,
    task_id: Option<String>,
    outcome_ref: Option<String>,
}

fn load_rows(conn: &Connection, skill: Option<&str>, days: u32) -> Result<Vec<Row>> {
    let mut stmt = conn.prepare(
        "SELECT id, skill_name, session_id, trigger, loaded_at, skill_hash, purpose, used,
                outcome_score, outcome_status, task_id, outcome_ref
           FROM skill_invocations
          WHERE loaded_at >= datetime('now', ?1) AND (?2 IS NULL OR skill_name = ?2)
          ORDER BY loaded_at DESC, rowid DESC",
    )?;
    let rows = stmt.query_map(params![format!("-{days} days"), skill], |r| {
        Ok(Row {
            id: r.get(0)?,
            skill: r.get(1)?,
            session: r.get(2)?,
            trigger: r.get(3)?,
            loaded_at: r.get(4)?,
            hash: r.get(5)?,
            purpose: r.get(6)?,
            used: r.get(7)?,
            score: r.get(8)?,
            status: r.get(9)?,
            task_id: r.get(10)?,
            outcome_ref: r.get(11)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

fn sessions_of<'a>(rows: impl Iterator<Item = &'a Row>) -> u32 {
    rows.map(|r| r.session.as_str())
        .collect::<HashSet<_>>()
        .len() as u32
}

fn pattern(
    kind: PatternKind,
    qualifier: &str,
    hash: &Option<String>,
    matching: Vec<&Row>,
) -> Option<Pattern> {
    if matching.is_empty() {
        return None;
    }
    let invocations = matching.len() as u32;
    let sessions = sessions_of(matching.iter().copied());
    // `matching` is newest first.
    Some(Pattern {
        kind,
        qualifier: qualifier.to_string(),
        skill_hash: hash.clone(),
        invocations,
        sessions,
        first: matching
            .last()
            .map(|r| r.loaded_at.clone())
            .unwrap_or_default(),
        last: matching[0].loaded_at.clone(),
        examples: matching
            .iter()
            .take(MAX_EXAMPLES)
            .map(|r| Example {
                id: r.id.clone(),
                loaded_at: r.loaded_at.clone(),
                task_id: r.task_id.clone(),
                outcome_ref: r.outcome_ref.clone(),
            })
            .collect(),
        enough: enough(invocations, sessions),
    })
}

fn version_stats(hash: &Option<String>, rows: &[&Row]) -> VersionStats {
    let count = |f: &dyn Fn(&Row) -> bool| rows.iter().filter(|r| f(r)).count() as u32;
    let scores: Vec<f64> = rows.iter().filter_map(|r| r.score).collect();
    VersionStats {
        skill_hash: hash.clone(),
        invocations: rows.len() as u32,
        sessions: sessions_of(rows.iter().copied()),
        first: rows.last().map(|r| r.loaded_at.clone()).unwrap_or_default(),
        last: rows
            .first()
            .map(|r| r.loaded_at.clone())
            .unwrap_or_default(),
        scored: scores.len() as u32,
        mean_score: (!scores.is_empty()).then(|| scores.iter().sum::<f64>() / scores.len() as f64),
        accepted: count(&|r| r.status.as_deref() == Some("accepted")),
        reworked: count(&|r| r.status.as_deref() == Some("reworked")),
        rejected: count(&|r| r.status.as_deref() == Some("rejected")),
        no_signal: count(&|r| matches!(r.status.as_deref(), None | Some("unknown"))),
        helped: count(&|r| r.used.as_deref() == Some("helped")),
        hurt: count(&|r| r.used.as_deref() == Some("hurt")),
        unused: count(&|r| r.used.as_deref() == Some("unused")),
    }
}

/// The evidence for `skill`, or for every skill with invocations in the window. `auto_loading`
/// names the skills whose hints are meant to load them without being asked; only for those is
/// a run of manual loads a sign that the hints failed.
pub fn evidence(
    conn: &Connection,
    skill: Option<&str>,
    days: u32,
    auto_loading: &BTreeSet<String>,
) -> Result<Vec<SkillEvidence>> {
    let rows = load_rows(conn, skill, days)?;
    let mut by_skill: BTreeMap<&str, Vec<&Row>> = BTreeMap::new();
    for row in &rows {
        by_skill.entry(row.skill.as_str()).or_default().push(row);
    }

    let mut out = Vec::new();
    for (name, skill_rows) in by_skill {
        let mut by_version: BTreeMap<Option<String>, Vec<&Row>> = BTreeMap::new();
        for row in &skill_rows {
            by_version.entry(row.hash.clone()).or_default().push(row);
        }

        let mut versions: Vec<VersionStats> = by_version
            .iter()
            .map(|(hash, rows)| version_stats(hash, rows))
            .collect();
        // Oldest version first, so a comparison reads "before" then "after".
        versions.sort_by(|a, b| a.first.cmp(&b.first));

        let comparisons = versions
            .windows(2)
            .map(|pair| VersionComparison {
                before: pair[0].skill_hash.clone(),
                after: pair[1].skill_hash.clone(),
                before_invocations: pair[0].invocations,
                after_invocations: pair[1].invocations,
                before_mean: pair[0].mean_score,
                after_mean: pair[1].mean_score,
                small_sample: pair[0].invocations < MIN_INVOCATIONS
                    || pair[1].invocations < MIN_INVOCATIONS,
            })
            .collect();

        let mut patterns = Vec::new();
        for (hash, rows) in &by_version {
            let pick = |f: &dyn Fn(&Row) -> bool| -> Vec<&Row> {
                rows.iter().copied().filter(|r| f(r)).collect()
            };
            let reworked = |r: &Row| matches!(r.status.as_deref(), Some("reworked" | "rejected"));
            patterns.extend(pattern(
                PatternKind::ReworkedOutcomes,
                "",
                hash,
                pick(&reworked),
            ));
            patterns.extend(pattern(
                PatternKind::JudgedHarmful,
                "",
                hash,
                pick(&|r| r.used.as_deref() == Some("hurt")),
            ));
            patterns.extend(pattern(
                PatternKind::LoadedButUnused,
                "",
                hash,
                pick(&|r| r.used.as_deref() == Some("unused")),
            ));
            let purposes: BTreeSet<&str> =
                rows.iter().filter_map(|r| r.purpose.as_deref()).collect();
            for purpose in purposes {
                patterns.extend(pattern(
                    PatternKind::ReworkedForPurpose,
                    purpose,
                    hash,
                    pick(&|r| reworked(r) && r.purpose.as_deref() == Some(purpose)),
                ));
            }
            if auto_loading.contains(name) {
                patterns.extend(pattern(
                    PatternKind::HintsDidNotActivate,
                    "",
                    hash,
                    pick(&|r| r.trigger == "load_skill"),
                ));
            }
        }
        patterns.sort_by(|a, b| {
            b.enough
                .cmp(&a.enough)
                .then(b.invocations.cmp(&a.invocations))
                .then(a.kind.cmp(&b.kind))
        });

        out.push(SkillEvidence {
            skill: name.to_string(),
            window_days: days,
            invocations: skill_rows.len() as u32,
            sessions: sessions_of(skill_rows.iter().copied()),
            versions,
            comparisons,
            patterns,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Database;
    use crate::skill_invocations::{
        Context, InvocationTrigger, Purpose, Used, log_invocation_with,
    };

    struct Load<'a> {
        skill: &'a str,
        session: &'a str,
        hash: &'a str,
        trigger: InvocationTrigger,
        purpose: Option<Purpose>,
        used: Used,
        status: Option<&'a str>,
        score: Option<f64>,
        days_ago: i64,
    }

    fn load(c: &Connection, l: Load<'_>) -> String {
        let id = log_invocation_with(
            c,
            l.skill,
            l.session,
            l.trigger,
            &Context {
                skill_hash: Some(l.hash),
                task_id: None,
                purpose: l.purpose,
                used: Some(l.used),
            },
        )
        .unwrap();
        c.execute(
            "UPDATE skill_invocations
                SET loaded_at = datetime('now', ?2), outcome_status = ?3, outcome_score = ?4
              WHERE id = ?1",
            params![id, format!("-{} days", l.days_ago), l.status, l.score],
        )
        .unwrap();
        id
    }

    fn reworked(skill: &'static str, session: &'static str, hash: &'static str) -> Load<'static> {
        Load {
            skill,
            session,
            hash,
            trigger: InvocationTrigger::LoadSkill,
            purpose: Some(Purpose::Code),
            used: Used::Loaded,
            status: Some("reworked"),
            score: Some(0.5),
            days_ago: 1,
        }
    }

    fn report(db: &Database, skill: &str) -> SkillEvidence {
        db.with_conn(|c| evidence(c, Some(skill), WINDOW_DAYS, &BTreeSet::new()))
            .unwrap()
            .remove(0)
    }

    #[test]
    fn five_reworked_loads_across_three_sessions_is_a_finding() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            for session in ["s1", "s1", "s2", "s3", "s3"] {
                load(c, reworked("sheets", session, "v1"));
            }
            Ok(())
        })
        .unwrap();

        let e = report(&db, "sheets");

        assert!(e.sufficient());
        let p = e
            .findings()
            .find(|p| p.kind == PatternKind::ReworkedOutcomes)
            .unwrap();
        assert_eq!((p.invocations, p.sessions), (5, 3));
        assert_eq!(p.examples.len(), 5);
        assert!(
            p.examples.iter().all(|e| e.id.starts_with("inv_")),
            "ids only"
        );
        assert_eq!(p.skill_hash.as_deref(), Some("v1"));
    }

    #[test]
    fn a_small_sample_is_shown_with_its_size_and_never_called_a_finding() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            for session in ["s1", "s2", "s3", "s4"] {
                load(c, reworked("sheets", session, "v1"));
            }
            Ok(())
        })
        .unwrap();
        let e = report(&db, "sheets");
        assert!(!e.sufficient(), "four loads is under the threshold of five");
        let p = &e.patterns[0];
        assert_eq!((p.invocations, p.sessions, p.enough), (4, 4, false));
    }

    #[test]
    fn many_loads_in_one_session_is_not_enough_either() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            for _ in 0..8 {
                load(c, reworked("sheets", "one-session", "v1"));
            }
            Ok(())
        })
        .unwrap();
        let e = report(&db, "sheets");
        assert!(!e.sufficient());
        assert_eq!((e.patterns[0].invocations, e.patterns[0].sessions), (8, 1));
    }

    #[test]
    fn evidence_from_outside_the_window_does_not_count() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            for session in ["s1", "s2", "s3", "s4", "s5"] {
                load(
                    c,
                    Load {
                        days_ago: 45,
                        ..reworked("sheets", session, "v1")
                    },
                );
            }
            Ok(())
        })
        .unwrap();
        let all = db
            .with_conn(|c| evidence(c, Some("sheets"), WINDOW_DAYS, &BTreeSet::new()))
            .unwrap();
        assert!(all.is_empty(), "nothing in the last 30 days");
        let wider = db
            .with_conn(|c| evidence(c, Some("sheets"), 60, &BTreeSet::new()))
            .unwrap();
        assert!(wider[0].sufficient());
    }

    #[test]
    fn patterns_are_counted_per_skill_version() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            // Three on the old version, three on the new: neither has five.
            for session in ["s1", "s2", "s3"] {
                load(
                    c,
                    Load {
                        days_ago: 10,
                        ..reworked("sheets", session, "v1")
                    },
                );
                load(c, reworked("sheets", session, "v2"));
            }
            Ok(())
        })
        .unwrap();
        let e = report(&db, "sheets");
        assert!(!e.sufficient());
        assert_eq!(e.versions.len(), 2);
        assert_eq!(
            e.versions[0].skill_hash.as_deref(),
            Some("v1"),
            "oldest first"
        );
    }

    #[test]
    fn versions_are_compared_with_their_sample_sizes_and_flagged_when_small() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            for session in ["a", "b", "c", "d", "e"] {
                load(
                    c,
                    Load {
                        days_ago: 20,
                        status: Some("reworked"),
                        score: Some(0.5),
                        ..reworked("sheets", session, "v1")
                    },
                );
            }
            for session in ["f", "g"] {
                load(
                    c,
                    Load {
                        status: Some("accepted"),
                        score: Some(1.0),
                        ..reworked("sheets", session, "v2")
                    },
                );
            }
            Ok(())
        })
        .unwrap();
        let e = report(&db, "sheets");
        assert_eq!(e.comparisons.len(), 1);
        let c = &e.comparisons[0];
        assert_eq!(
            (c.before.as_deref(), c.after.as_deref()),
            (Some("v1"), Some("v2"))
        );
        assert_eq!((c.before_invocations, c.after_invocations), (5, 2));
        assert_eq!((c.before_mean, c.after_mean), (Some(0.5), Some(1.0)));
        assert!(c.small_sample, "two loads on the new version is anecdote");
    }

    #[test]
    fn unused_and_harmful_verdicts_are_their_own_patterns() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            for session in ["s1", "s2", "s3", "s4", "s5"] {
                load(
                    c,
                    Load {
                        used: Used::Unused,
                        status: None,
                        score: None,
                        ..reworked("docs", session, "v1")
                    },
                );
                load(
                    c,
                    Load {
                        used: Used::Hurt,
                        status: None,
                        score: Some(0.0),
                        ..reworked("docs", session, "v1")
                    },
                );
            }
            Ok(())
        })
        .unwrap();
        let e = report(&db, "docs");
        let kinds: Vec<_> = e.findings().map(|p| p.kind).collect();
        assert!(kinds.contains(&PatternKind::LoadedButUnused));
        assert!(kinds.contains(&PatternKind::JudgedHarmful));
        assert!(!kinds.contains(&PatternKind::ReworkedOutcomes));
    }

    #[test]
    fn rework_is_broken_out_by_the_kind_of_work() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            for session in ["s1", "s2", "s3", "s4", "s5"] {
                load(
                    c,
                    Load {
                        purpose: Some(Purpose::Research),
                        ..reworked("docs", session, "v1")
                    },
                );
            }
            for session in ["s1", "s2"] {
                load(
                    c,
                    Load {
                        purpose: Some(Purpose::Code),
                        ..reworked("docs", session, "v1")
                    },
                );
            }
            Ok(())
        })
        .unwrap();
        let e = report(&db, "docs");
        let research = e
            .patterns
            .iter()
            .find(|p| p.kind == PatternKind::ReworkedForPurpose && p.qualifier == "research")
            .unwrap();
        assert!(research.enough);
        let code = e
            .patterns
            .iter()
            .find(|p| p.kind == PatternKind::ReworkedForPurpose && p.qualifier == "code")
            .unwrap();
        assert!(!code.enough);
    }

    #[test]
    fn manual_loads_signal_failing_hints_only_for_skills_meant_to_load_themselves() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            for session in ["s1", "s2", "s3", "s4", "s5"] {
                load(
                    c,
                    Load {
                        status: None,
                        score: None,
                        ..reworked("docs", session, "v1")
                    },
                );
            }
            Ok(())
        })
        .unwrap();
        let without = report(&db, "docs");
        assert!(
            !without
                .patterns
                .iter()
                .any(|p| p.kind == PatternKind::HintsDidNotActivate)
        );

        let auto: BTreeSet<String> = ["docs".to_string()].into();
        let with = db
            .with_conn(|c| evidence(c, Some("docs"), WINDOW_DAYS, &auto))
            .unwrap()
            .remove(0);
        assert!(
            with.findings()
                .any(|p| p.kind == PatternKind::HintsDidNotActivate)
        );
    }

    #[test]
    fn a_skill_with_loads_but_no_pattern_reports_none() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            load(
                c,
                Load {
                    status: Some("accepted"),
                    score: Some(1.0),
                    ..reworked("docs", "s1", "v1")
                },
            );
            Ok(())
        })
        .unwrap();
        let e = report(&db, "docs");
        assert!(e.patterns.is_empty());
        assert_eq!(e.invocations, 1);
        assert!(!e.sufficient());
    }

    #[test]
    fn the_report_carries_no_text_beyond_ids_hashes_and_labels() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            for session in ["s1", "s2", "s3", "s4", "s5"] {
                load(c, reworked("sheets", session, "v1"));
            }
            Ok(())
        })
        .unwrap();
        let json = serde_json::to_string(&report(&db, "sheets")).unwrap();
        // Everything in it is one of: an id, a hash, a label from a fixed list, a number or a time.
        for forbidden in ["prompt", "title", "description", "content", "text"] {
            assert!(!json.contains(forbidden), "{forbidden} in {json}");
        }
    }
}
