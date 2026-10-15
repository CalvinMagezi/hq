//! Logging a skill load together with what is known about it.
//!
//! Both log sites (the `load_skill` tool and the session's auto-load) go through here, so an
//! invocation always records the same things: which version of the skill ran, which task the
//! session was on, and a coarse category for the work. Only hashes, ids and values from fixed
//! lists are stored; nothing the user asked for and nothing from a document.

use std::path::Path;

use hq_db::Database;
use hq_db::skill_invocations::{Context, InvocationTrigger, Purpose, Used};

/// Record that `name` was loaded in `session_id`. `task_id` is the task the session is working
/// on, when it is known. Returns the invocation id.
pub fn log_skill_load(
    db: &Database,
    skills_dir: &Path,
    name: &str,
    session_id: &str,
    trigger: InvocationTrigger,
    task_id: Option<&str>,
) -> anyhow::Result<String> {
    let hash = crate::skill_audit::live_hash(skills_dir, name);
    db.with_conn(|conn| {
        // Purpose is read from the task's tags only, never its title or description.
        let task = match task_id {
            Some(id) => hq_db::tasks::get_task(conn, id)?,
            None => None,
        };
        hq_db::skill_invocations::log_invocation_with(
            conn,
            name,
            session_id,
            trigger,
            &Context {
                skill_hash: hash.as_deref(),
                task_id: task.as_ref().map(|t| t.id.as_str()),
                purpose: task.as_ref().map(|t| Purpose::from_tags(&t.tags)),
                used: Some(Used::Loaded),
            },
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::HqTool;
    use crate::skills::LoadSkillTool;
    use serde_json::json;
    use std::sync::Arc;

    const SKILL: &str = "---\nname: sheets\ndescription: Work with spreadsheets\nhints:\n  - spreadsheet\n---\n\n# Sheets\n\n1. Open the sheet.\n";

    fn skills_dir() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("sheets")).unwrap();
        std::fs::write(dir.path().join("sheets").join("SKILL.md"), SKILL).unwrap();
        dir
    }

    fn task(db: &Database, tags: &[&str]) -> String {
        let tags: Vec<String> = tags.iter().map(|t| t.to_string()).collect();
        db.with_conn(|c| {
            hq_db::tasks::create_initiative(c, "in-1", "personal", None, "Work", "work", "WRK")?;
            hq_db::tasks::create_task(
                c,
                "tk-1",
                "in-1",
                &hq_db::tasks::NewTask {
                    title: "A private task title",
                    description: "Private details that must not be copied",
                    tags: &tags,
                    created_by: "test",
                    ..Default::default()
                },
            )
            .map(|t| t.id)
        })
        .unwrap()
    }

    type Row = (
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    );

    fn row(db: &Database) -> Row {
        db.with_conn(|c| {
            Ok(c.query_row(
                "SELECT skill_hash, task_id, purpose, used FROM skill_invocations",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )?)
        })
        .unwrap()
    }

    #[test]
    fn a_load_records_the_audit_digest_the_task_and_the_purpose_from_its_tags() {
        let dir = skills_dir();
        let db = Database::open_memory().unwrap();
        let id = task(&db, &["docs"]);

        log_skill_load(
            &db,
            dir.path(),
            "sheets",
            "s-1",
            InvocationTrigger::LoadSkill,
            Some(&id),
        )
        .unwrap();

        let audit = crate::skill_audit::AuditEntry::new(
            "sheets",
            1,
            SKILL,
            crate::skill_audit::Disposition::Adopted,
        );
        assert_eq!(
            row(&db),
            (
                Some(audit.sha256),
                Some(id),
                Some("writing".into()),
                Some("loaded".into())
            )
        );
    }

    #[test]
    fn with_no_task_the_purpose_and_task_stay_unknown() {
        let dir = skills_dir();
        let db = Database::open_memory().unwrap();
        log_skill_load(
            &db,
            dir.path(),
            "sheets",
            "s-1",
            InvocationTrigger::AutoLoad,
            None,
        )
        .unwrap();
        let (hash, task, purpose, _) = row(&db);
        assert!(hash.is_some());
        assert_eq!((task, purpose), (None, None));
    }

    #[test]
    fn an_unknown_task_id_is_not_stored() {
        let dir = skills_dir();
        let db = Database::open_memory().unwrap();
        log_skill_load(
            &db,
            dir.path(),
            "sheets",
            "s-1",
            InvocationTrigger::LoadSkill,
            Some("tk-nope"),
        )
        .unwrap();
        let (_, task, purpose, _) = row(&db);
        assert_eq!((task, purpose), (None, None));
    }

    #[test]
    fn nothing_the_user_or_the_task_wrote_is_stored() {
        let dir = skills_dir();
        let db = Database::open_memory().unwrap();
        let id = task(&db, &["research"]);
        log_skill_load(
            &db,
            dir.path(),
            "sheets",
            "s-1",
            InvocationTrigger::LoadSkill,
            Some(&id),
        )
        .unwrap();

        let everything: String = db
            .with_conn(|c| {
                let mut stmt = c.prepare("SELECT * FROM skill_invocations")?;
                let cols = stmt.column_count();
                let mut rows = stmt.query([])?;
                let mut out = String::new();
                while let Some(r) = rows.next()? {
                    for i in 0..cols {
                        out.push_str(&format!("{:?}|", r.get_ref(i)?));
                    }
                }
                Ok(out)
            })
            .unwrap();
        for private in [
            "A private task title",
            "Private details",
            "Open the sheet",
            "spreadsheets",
        ] {
            assert!(
                !everything.contains(private),
                "stored {private:?}: {everything}"
            );
        }
    }

    #[tokio::test]
    async fn the_load_skill_tool_takes_the_task_from_the_attested_identity_only() {
        let dir = skills_dir();
        let db = Arc::new(Database::open_memory().unwrap());
        let task_id = task(&db, &["code"]);
        let lease = db
            .with_conn(|c| {
                let who = hq_db::tasks::LeaseIdentity {
                    actor: "relay",
                    harness: "native",
                    external_session_ref: "relay",
                    ..Default::default()
                };
                Ok(hq_db::tasks::claim(c, &task_id, &who, 900, false)?
                    .session
                    .id)
            })
            .unwrap();
        let tool = LoadSkillTool::with_telemetry(dir.path().to_path_buf(), db.clone(), "s-1");

        // The runtime's markers name the lease: the task is known.
        tool.execute(json!({
            "name": "sheets",
            crate::provenance::NATIVE_SESSION_ARG: "s-1",
            crate::provenance::NATIVE_LEASES_ARG: [lease],
        }))
        .await
        .unwrap();
        // A bare call (nothing attested) records no task, whatever it claims in other fields.
        tool.execute(json!({ "name": "sheets", "task_id": task_id }))
            .await
            .unwrap();

        let tasks: Vec<Option<String>> = db
            .with_conn(|c| {
                let mut stmt = c.prepare("SELECT task_id FROM skill_invocations ORDER BY rowid")?;
                let rows = stmt.query_map([], |r| r.get(0))?;
                Ok(rows.collect::<rusqlite::Result<_>>()?)
            })
            .unwrap();
        assert_eq!(tasks, vec![Some(task_id), None]);
    }
}
