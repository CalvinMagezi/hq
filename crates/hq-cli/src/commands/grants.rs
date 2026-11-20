//! `hq grants`: the sandbox access helpers asked HQ for, and how each request was decided.

use anyhow::{Result, bail};
use hq_core::config::HqConfig;
use hq_db::Database;
use hq_db::sandbox_grants::{self as store, Grant};
use std::io::Write;

const LIST_LIMIT: usize = 100;
const NONE: &str = "-";
const NO_PROFILE: &str = "none";

pub async fn run(
    config: &HqConfig,
    sub: &str,
    target: Option<String>,
    name: Option<String>,
    task: bool,
    json: bool,
) -> Result<()> {
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
        "profile" => {
            let (Some(target), Some(name)) = (target, name) else {
                bail!(
                    "usage: hq grants profile [--task] <session or task id> <profile name | none>"
                );
            };
            let known = &config.agent_host.grants.profiles;
            if name != NO_PROFILE && !known.contains_key(&name) {
                bail!("no profile '{name}' in agent_host.grants.profiles");
            }
            bind(&db, &target, &name, task, &mut out)
        }
        other => bail!("unknown subcommand '{other}': use list, revoke or profile"),
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

fn bind(db: &Database, target: &str, name: &str, task: bool, out: &mut impl Write) -> Result<()> {
    let scope = if task {
        format!("{}{target}", store::TASK_SCOPE)
    } else {
        target.to_string()
    };
    store::bind_profile(db, &scope, name, chrono::Utc::now().timestamp())?;
    writeln!(
        out,
        "{} now works under the '{name}' access profile. A running session picks it up when it is next started.",
        if task {
            format!("Task {target}")
        } else {
            format!("Session {target}")
        }
    )?;
    Ok(())
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

    fn filed(db: &Database, session: &str, state: &str, resource: &str) -> String {
        let d = SandboxDelta::default();
        store::file(
            db,
            &store::NewGrant {
                session_id: session,
                task_id: None,
                host: "local",
                repo: "",
                capability: "fs_write",
                resource,
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
        let a = filed(&db, "s1", store::ACTIVE, "/r/a");
        filed(&db, "s2", store::ACTIVE, "/r/a");
        filed(&db, "s2", store::ACTIVE, "/r/b");
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
    fn profile_binds_a_task_or_a_session_for_the_owner() {
        let db = Database::open_memory().unwrap();
        let mut out = Vec::new();
        bind(&db, "t1", "mobile", true, &mut out).unwrap();
        bind(&db, "s9", "web", false, &mut out).unwrap();
        assert_eq!(
            store::profile_for(&db, "sx", Some("t1"))
                .unwrap()
                .as_deref(),
            Some("mobile")
        );
        assert_eq!(
            store::profile_for(&db, "s9", None).unwrap().as_deref(),
            Some("web")
        );
        assert_eq!(
            store::profile_for(&db, "s9", Some("t1"))
                .unwrap()
                .as_deref(),
            Some("web"),
            "newest wins"
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
        filed(&db, "s1", store::REFUSED, "/r/a");
        let mut out = Vec::new();
        list(&db, false, &mut out).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(
            text.contains("refused") && text.contains("session s1"),
            "{text}"
        );
    }
}
