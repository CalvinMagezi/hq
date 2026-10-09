//! Reads what the coding agents HQ's host runs have written about their own token use and records it
//! in the spend ledger, once per call. Only files and databases the agents already keep are read, and
//! only their numbers; see `harness_usage` for what is parsed.

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use anyhow::Result;
use chrono::NaiveDateTime;
use hq_db::Database;
use hq_db::harness_sessions_registry::{HarnessSessionRow, list};
use hq_db::task_outcomes::{TaskOutcome, insert_if_new};
use hq_llm::cost::{ProviderClass, price_call};
use rusqlite::{Connection, OpenFlags, params};

use crate::harness_usage::{
    CodexSession, HarnessUsageEvent, PiSession, parse_claude_line, parse_codex_line, parse_kimi_line,
    parse_pi_line, read_copilot, read_opencode,
};

/// A session's calls can land slightly before it was registered or after its last update.
const MATCH_SLACK_BEFORE_SECS: i64 = 120;
const MATCH_SLACK_AFTER_SECS: i64 = 600;
const SESSION_LIST_LIMIT: usize = 500;
/// How deep to look under a harness's session directory.
const MAX_DEPTH: usize = 5;
/// Harnesses that bill through a subscription, so a dollar figure per call is not meaningful.
const FLAT_HARNESSES: &[&str] = &["kimi", "github-copilot"];
const ORIGIN_HARNESS: &str = "harness";

#[derive(Debug, Default, PartialEq, Eq)]
pub struct ScanReport {
    pub sources_read: usize,
    pub calls_recorded: usize,
}

/// The one HQ session a call belongs to: same harness, same working directory, and the call falls in
/// the session's lifetime. More than one candidate is ambiguous and matches none.
pub fn match_session(sessions: &[HarnessSessionRow], event: &HarnessUsageEvent) -> Option<String> {
    let cwd = event.cwd.as_deref().filter(|c| !c.is_empty())?;
    let mut hits = sessions.iter().filter(|s| {
        s.harness == event.harness
            && s.cwd == cwd
            && parse_registry_ts(&s.created_at).is_some_and(|c| event.ts >= c - MATCH_SLACK_BEFORE_SECS)
            && parse_registry_ts(&s.updated_at).is_some_and(|u| event.ts <= u + MATCH_SLACK_AFTER_SECS)
    });
    match (hits.next(), hits.next()) {
        (Some(only), None) => Some(only.id.clone()),
        _ => None,
    }
}

fn parse_registry_ts(s: &str) -> Option<i64> {
    NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S")
        .ok()
        .map(|t| t.and_utc().timestamp())
}

/// Turn one event into a ledger row. Pi and OpenCode record dollars themselves, subscription
/// harnesses are flat, and the rest are priced at list price, which is what the call would have
/// cost on the API and not necessarily what was billed.
fn outcome_for(event: &HarnessUsageEvent, sessions: &[HarnessSessionRow]) -> TaskOutcome {
    let class = if FLAT_HARNESSES.contains(&event.harness) {
        ProviderClass::Flat
    } else {
        ProviderClass::Metered
    };
    let priced = price_call(class, &event.model, &event.usage);
    let session_id = match_session(sessions, event)
        .unwrap_or_else(|| format!("external:{}:{}", event.harness, event.session_ref));
    let mut o = TaskOutcome::now(session_id, 0, event.model.clone(), event.harness, "harness");
    o.recorded_at = event.ts;
    o.input_tokens = Some(i64::from(event.usage.input));
    o.output_tokens = Some(i64::from(event.usage.output));
    o.cache_read_tokens = i64::from(event.usage.cache_read);
    o.cache_write_tokens = i64::from(event.usage.cache_write);
    o.reasoning_tokens = i64::from(event.usage.reasoning);
    o.cost_usd = priced.usd;
    o.provider_cost_usd = event.usage.billed_usd;
    o.cost_source = priced.source.as_str().to_string();
    o.origin = ORIGIN_HARNESS.to_string();
    o.external_id = Some(format!("{}:{}", event.harness, event.source_id));
    o
}

fn record(db: &Database, events: &[HarnessUsageEvent], sessions: &[HarnessSessionRow]) -> Result<usize> {
    db.with_conn(|conn| {
        let tx = conn.unchecked_transaction()?;
        let mut new = 0;
        for e in events {
            if insert_if_new(&tx, &outcome_for(e, sessions))? {
                new += 1;
            }
        }
        tx.commit()?;
        Ok(new)
    })
}

fn state(db: &Database, path: &Path) -> Result<Option<(i64, i64)>> {
    let key = path.to_string_lossy().to_string();
    db.with_conn(|conn| {
        Ok(conn
            .query_row(
                "SELECT size, mtime FROM harness_usage_files WHERE path = ?1",
                params![key],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .ok())
    })
}

fn save_state(db: &Database, path: &Path, size: i64, mtime: i64) -> Result<()> {
    let key = path.to_string_lossy().to_string();
    db.with_conn(|conn| {
        conn.execute(
            "INSERT INTO harness_usage_files (path, size, mtime) VALUES (?1, ?2, ?3)
             ON CONFLICT(path) DO UPDATE SET size = excluded.size, mtime = excluded.mtime",
            params![key, size, mtime],
        )?;
        Ok(())
    })
}

fn file_signature(path: &Path) -> Option<(i64, i64)> {
    let meta = std::fs::metadata(path).ok()?;
    let mtime = meta
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs() as i64;
    Some((meta.len() as i64, mtime))
}

/// Every file named `*.jsonl` (or exactly `name`) under `root`, to a bounded depth.
fn find_files(root: &Path, want: &dyn Fn(&Path) -> bool) -> Vec<PathBuf> {
    fn walk(dir: &Path, depth: usize, want: &dyn Fn(&Path) -> bool, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if depth < MAX_DEPTH {
                    walk(&path, depth + 1, want, out);
                }
            } else if want(&path) {
                out.push(path);
            }
        }
    }
    let mut out = Vec::new();
    walk(root, 0, want, &mut out);
    out
}

fn is_jsonl(p: &Path) -> bool {
    p.extension().is_some_and(|e| e == "jsonl")
}

/// Read each unchanged-since-last-time JSONL file once, line by line, handing every line to `parse`.
fn scan_jsonl(
    db: &Database,
    files: Vec<PathBuf>,
    sessions: &[HarnessSessionRow],
    report: &mut ScanReport,
    mut parse: impl FnMut(&str, usize, &mut dyn FnMut(HarnessUsageEvent), &Path),
) -> Result<()> {
    for path in files {
        let Some((size, mtime)) = file_signature(&path) else {
            continue;
        };
        if state(db, &path)? == Some((size, mtime)) {
            continue;
        }
        let Ok(file) = File::open(&path) else {
            continue;
        };
        let mut events = Vec::new();
        for (i, line) in BufReader::new(file).lines().map_while(|l| l.ok()).enumerate() {
            parse(&line, i, &mut |e| events.push(e), &path);
        }
        report.calls_recorded += record(db, &events, sessions)?;
        report.sources_read += 1;
        save_state(db, &path, size, mtime)?;
    }
    Ok(())
}

fn open_readonly(path: &Path) -> Option<Connection> {
    Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).ok()
}

/// Scan every harness's usage records under `home` and record the calls not seen before.
pub fn scan(db: &Database, home: &Path) -> Result<ScanReport> {
    let sessions = db.with_conn(|c| list(c, None, SESSION_LIST_LIMIT))?;
    let mut report = ScanReport::default();

    // Claude Code: one config directory per account (`~/.claude`, `~/.claude-<name>`).
    let claude_dirs: Vec<PathBuf> = std::fs::read_dir(home)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with(".claude")))
        .map(|p| p.join("projects"))
        .collect();
    for dir in claude_dirs {
        scan_jsonl(db, find_files(&dir, &is_jsonl), &sessions, &mut report, |line, _, emit, _| {
            if let Some(e) = parse_claude_line(line) {
                emit(e);
            }
        })?;
    }

    // Codex: one rollout file per session, with the session's id and model near the top.
    let codex_files = find_files(&home.join(".codex/sessions"), &is_jsonl);
    for path in codex_files {
        let mut session = CodexSession::default();
        scan_jsonl(db, vec![path], &sessions, &mut report, |line, i, emit, _| {
            session.observe(line);
            if let Some(e) = parse_codex_line(line, i, &session) {
                emit(e);
            }
        })?;
    }

    // Pi: interactive sessions and the ones HQ launches in its own directory.
    for root in [home.join(".pi/agent/sessions"), home.join(".pi/hq-sessions")] {
        for path in find_files(&root, &is_jsonl) {
            let mut session = PiSession::default();
            scan_jsonl(db, vec![path], &sessions, &mut report, |line, _, emit, _| {
                session.observe(line);
                if let Some((e, _)) = parse_pi_line(line, &session) {
                    emit(e);
                }
            })?;
        }
    }

    // Kimi: the session id is the directory holding `wire.jsonl`; its model is not in the records.
    let kimi_files = find_files(&home.join(".kimi/sessions"), &|p| {
        p.file_name().is_some_and(|n| n == "wire.jsonl")
    });
    for path in kimi_files {
        scan_jsonl(db, vec![path], &sessions, &mut report, |line, i, emit, p| {
            let session = p
                .parent()
                .and_then(|d| d.file_name())
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            if let Some(e) = parse_kimi_line(line, &session, i, "") {
                emit(e);
            }
        })?;
    }

    scan_sqlite(db, &home.join(".local/share/opencode/opencode.db"), &sessions, &mut report, |conn, after| {
        read_opencode(conn, after)
    })?;
    scan_sqlite(db, &home.join(".copilot/session-store.db"), &sessions, &mut report, |conn, after| {
        read_copilot(conn, after)
    })?;
    Ok(report)
}

/// A SQLite source resumes from a cursor, kept in `size`: a millisecond time for OpenCode and a row
/// id for Copilot, so `after` means whatever that source orders by.
fn scan_sqlite(
    db: &Database,
    path: &Path,
    sessions: &[HarnessSessionRow],
    report: &mut ScanReport,
    read: impl Fn(&Connection, i64) -> rusqlite::Result<Vec<HarnessUsageEvent>>,
) -> Result<()> {
    if !path.is_file() {
        return Ok(());
    }
    let Some(conn) = open_readonly(path) else {
        return Ok(());
    };
    let after = state(db, path)?.map_or(0, |(cursor, _)| cursor);
    let Ok(events) = read(&conn, after) else {
        return Ok(());
    };
    report.calls_recorded += record(db, &events, sessions)?;
    report.sources_read += 1;
    // OpenCode's cursor is a time, Copilot's an id; both are the largest value just read. Events
    // carry seconds, so the time cursor is derived from the same rows by the reader's own ordering.
    let cursor = events
        .iter()
        .filter_map(|e| e.source_id.parse::<i64>().ok())
        .max()
        .or_else(|| events.iter().map(|e| e.ts * 1000).max())
        .map_or(after, |c| c.max(after));
    save_state(db, path, cursor, 0)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use hq_db::usage_ledger::{GroupBy, grouped_usage};
    use std::io::Write;

    fn write(path: &Path, lines: &[&str]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut f = File::create(path).unwrap();
        for l in lines {
            writeln!(f, "{l}").unwrap();
        }
    }

    fn claude_line(id: &str, session: &str, cwd: &str) -> String {
        format!(
            r#"{{"type":"assistant","sessionId":"{session}","cwd":"{cwd}","timestamp":"2026-10-08T10:00:00Z","message":{{"id":"{id}","model":"claude-haiku-5.5","usage":{{"input_tokens":2,"output_tokens":50,"cache_read_input_tokens":1000,"cache_creation_input_tokens":30}}}}}}"#
        )
    }

    fn home_with_claude() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(".claude/projects/-w/s1.jsonl");
        // The same message id appears twice, as Claude writes it once per content block.
        write(&file, &[&claude_line("m1", "s1", "/w"), &claude_line("m1", "s1", "/w"), &claude_line("m2", "s1", "/w")]);
        (dir, file)
    }

    #[test]
    fn a_growing_transcript_is_counted_once_per_call_however_often_it_is_read() {
        let (home, file) = home_with_claude();
        let db = Database::open_memory().unwrap();

        let first = scan(&db, home.path()).unwrap();
        assert_eq!(first.calls_recorded, 2, "duplicate lines of one message count once");

        // Unchanged file: skipped outright.
        assert_eq!(scan(&db, home.path()).unwrap().calls_recorded, 0);

        // The file grows; the old calls are not counted again, the new one is.
        let mut f = std::fs::OpenOptions::new().append(true).open(&file).unwrap();
        writeln!(f, "{}", claude_line("m3", "s1", "/w")).unwrap();
        drop(f);
        // Make the signature differ even on a coarse clock.
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(5);
        File::options().write(true).open(&file).unwrap().set_modified(later).unwrap();
        assert_eq!(scan(&db, home.path()).unwrap().calls_recorded, 1);

        let rows = db.with_conn(|c| grouped_usage(c, 0, GroupBy::Origin)).unwrap();
        assert_eq!((rows[0].key.as_str(), rows[0].calls), ("harness", 3));
        assert_eq!(rows[0].cache_read_tokens, 3000);
    }

    #[test]
    fn harness_calls_never_count_toward_a_global_budget() {
        let (home, _) = home_with_claude();
        let db = Database::open_memory().unwrap();
        scan(&db, home.path()).unwrap();
        let spent = db
            .with_conn(|c| {
                hq_db::usage_ledger::scope_spend(c, &hq_core::config::BudgetScope::Global, 0, i64::MAX)
            })
            .unwrap();
        assert_eq!(spent, 0.0);
        let harness = db
            .with_conn(|c| {
                hq_db::usage_ledger::scope_spend(
                    c,
                    &hq_core::config::BudgetScope::Origin("harness".into()),
                    0,
                    i64::MAX,
                )
            })
            .unwrap();
        assert!(harness > 0.0, "the same calls are visible to an origin:harness budget");
    }

    fn register(db: &Database, id: &str, harness: &str, cwd: &str) {
        use hq_db::harness_sessions_registry::{NewSession, Placement, insert};
        let placement = Placement {
            host: "local",
            agent_name: id,
            workspace_id: "w",
            pane_id: "p",
        };
        db.with_conn(|c| {
            insert(
                c,
                &NewSession {
                    id,
                    harness,
                    label: id,
                    cwd,
                    mission_id: None,
                    placement,
                },
            )
        })
        .unwrap();
    }

    fn event_at(db: &Database, harness: &'static str, cwd: &str) -> (HarnessUsageEvent, Vec<HarnessSessionRow>) {
        let now = chrono::Utc::now().timestamp();
        let sessions = db.with_conn(|c| list(c, None, 10)).unwrap();
        let event = HarnessUsageEvent {
            harness,
            source_id: "e".into(),
            session_ref: "ref".into(),
            cwd: Some(cwd.into()),
            ts: now,
            model: "m".into(),
            usage: Default::default(),
        };
        (event, sessions)
    }

    #[test]
    fn a_call_is_filed_under_the_one_hq_session_that_was_running_there() {
        let db = Database::open_memory().unwrap();
        register(&db, "hs-1", "claude-code", "/w");
        register(&db, "hs-2", "codex", "/w");
        let (event, sessions) = event_at(&db, "claude-code", "/w");
        assert_eq!(match_session(&sessions, &event).as_deref(), Some("hs-1"));
        // Another directory, or a harness HQ did not start there, matches nothing.
        let (other_dir, _) = event_at(&db, "claude-code", "/elsewhere");
        assert_eq!(match_session(&sessions, &other_dir), None);
        let (other_harness, _) = event_at(&db, "pi", "/w");
        assert_eq!(match_session(&sessions, &other_harness), None);
    }

    #[test]
    fn two_sessions_in_one_directory_are_ambiguous_and_match_neither() {
        let db = Database::open_memory().unwrap();
        register(&db, "hs-1", "claude-code", "/w");
        register(&db, "hs-2", "claude-code", "/w");
        let (event, sessions) = event_at(&db, "claude-code", "/w");
        assert_eq!(match_session(&sessions, &event), None);
    }

    #[test]
    fn subscription_harnesses_are_flat_and_recorded_dollars_are_kept() {
        let mut e = HarnessUsageEvent {
            harness: "github-copilot",
            source_id: "1".into(),
            session_ref: "r".into(),
            cwd: None,
            ts: 1,
            model: "gpt-x".into(),
            usage: hq_llm::cost::Usage { input: 10, output: 5, ..Default::default() },
        };
        assert_eq!(outcome_for(&e, &[]).cost_source, "flat");
        e.harness = "pi";
        e.usage.billed_usd = Some(0.02);
        let o = outcome_for(&e, &[]);
        assert_eq!((o.cost_source.as_str(), o.cost_usd, o.session_id.as_str()), ("provider", 0.02, "external:pi:r"));
    }
}
