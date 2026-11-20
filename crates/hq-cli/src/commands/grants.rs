//! `hq grants`: the sandbox access helpers asked HQ for, and how each request was decided.

use anyhow::{Result, bail};
use hq_core::config::HqConfig;
use hq_db::Database;
use hq_db::sandbox_grants::{self as store, Grant};
use std::io::Write;

const LIST_LIMIT: usize = 100;
const NONE: &str = "-";

pub async fn run(config: &HqConfig, sub: &str, target: Option<String>, json: bool) -> Result<()> {
    let db = Database::open(&config.db_path())?;
    let mut out = std::io::stdout();
    match sub {
        "list" => list(&db, json, &mut out),
        "revoke" => {
            let Some(target) = target else {
                bail!("usage: hq grants revoke <grant id | session id>");
            };
            revoke(&db, &target, &mut out)
        }
        other => bail!("unknown subcommand '{other}': use list or revoke"),
    }
}

fn list(db: &Database, json: bool, out: &mut impl Write) -> Result<()> {
    let grants = store::recent(db, LIST_LIMIT)?;
    if json {
        writeln!(out, "{}", serde_json::to_string_pretty(&grants)?)?;
        return Ok(());
    }
    if grants.is_empty() {
        writeln!(out, "No sandbox access has been requested.")?;
        return Ok(());
    }
    for g in &grants {
        writeln!(out, "{}", line(g))?;
    }
    Ok(())
}

fn line(g: &Grant) -> String {
    format!(
        "{}  {:<8} {:<7} {} {}  session {}  by {}{}",
        &g.id[..g.id.len().min(8)],
        g.state,
        g.capability,
        g.resource,
        if g.decision_note.is_empty() {
            String::new()
        } else {
            format!("({})", g.decision_note)
        },
        g.session_id,
        g.decided_by,
        g.task_id
            .as_deref()
            .map(|t| format!("  task {t}"))
            .unwrap_or_else(|| format!("  task {NONE}")),
    )
}

fn revoke(db: &Database, target: &str, out: &mut impl Write) -> Result<()> {
    let now = chrono::Utc::now().timestamp();
    let mut n = usize::from(store::revoke(db, target, now)?);
    if n == 0 {
        n = store::revoke_session(db, target, now)?;
    }
    if n == 0 {
        bail!("no live grant or session '{target}'");
    }
    writeln!(
        out,
        "Revoked {n}. A running session keeps the access until it is stopped; resume it to start without it."
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use hq_core::grants::SandboxDelta;

    fn filed(db: &Database, session: &str, state: &str) -> String {
        let d = SandboxDelta::default();
        store::file(
            db,
            &store::NewGrant {
                session_id: session,
                task_id: None,
                host: "local",
                repo: "",
                capability: "fs_write",
                resource: "/r/x",
                delta: &d,
                state,
                decided_by: store::BY_POLICY,
                helper_reason: "",
                decision_note: "",
                expires_at: None,
            },
            1,
        )
        .unwrap()
    }

    #[test]
    fn revoke_takes_a_grant_id_or_a_whole_session_and_refuses_unknown_targets() {
        let db = Database::open_memory().unwrap();
        let a = filed(&db, "s1", store::ACTIVE);
        filed(&db, "s2", store::ACTIVE);
        filed(&db, "s2", store::ACTIVE);
        let mut out = Vec::new();
        revoke(&db, &a, &mut out).unwrap();
        revoke(&db, "s2", &mut out).unwrap();
        assert!(revoke(&db, "nobody", &mut out).is_err());
        assert!(store::delta_for_session(&db, "s2", 9).unwrap().is_empty());
        let text = String::from_utf8(out).unwrap();
        assert!(
            text.contains("Revoked 1") && text.contains("Revoked 2"),
            "{text}"
        );
    }

    #[test]
    fn list_prints_a_line_per_grant_and_says_so_when_empty() {
        let db = Database::open_memory().unwrap();
        let mut out = Vec::new();
        list(&db, false, &mut out).unwrap();
        assert!(
            String::from_utf8(out)
                .unwrap()
                .contains("No sandbox access")
        );
        filed(&db, "s1", store::REFUSED);
        let mut out = Vec::new();
        list(&db, false, &mut out).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(
            text.contains("refused") && text.contains("session s1"),
            "{text}"
        );
    }
}
