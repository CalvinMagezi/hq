use anyhow::Result;
use rusqlite::{Connection, TransactionBehavior};
use tracing::info;

const MIGRATIONS: &[(&str, &str)] = &[
    ("001_initial", include_str!("../sql/001_initial.sql")),
    (
        "002_graph_links",
        include_str!("../sql/002_graph_links.sql"),
    ),
    (
        "003_memory_system",
        include_str!("../sql/003_memory_system.sql"),
    ),
    (
        "004_harness_quotas",
        include_str!("../sql/004_harness_quotas.sql"),
    ),
    (
        "005_research_lessons",
        include_str!("../sql/005_research_lessons.sql"),
    ),
    (
        "007_codegraph_v2",
        include_str!("../sql/007_codegraph_v2.sql"),
    ),
    (
        "008_wa_signal_state",
        include_str!("../sql/008_wa_signal_state.sql"),
    ),
    (
        "009_earn_system",
        include_str!("../sql/009_earn_system.sql"),
    ),
    (
        "010_vault_cache",
        include_str!("../sql/010_vault_cache.sql"),
    ),
    (
        "011_vector_cache",
        include_str!("../sql/011_vector_cache.sql"),
    ),
    (
        "012_skill_invocations",
        include_str!("../sql/012_skill_invocations.sql"),
    ),
    (
        "012_task_outcomes",
        include_str!("../sql/012_task_outcomes.sql"),
    ),
    (
        "013_chat_threads",
        include_str!("../sql/013_chat_threads.sql"),
    ),
    (
        "014_coding_agent_discovery",
        include_str!("../sql/014_coding_agent_discovery.sql"),
    ),
    ("015_tool_usage", include_str!("../sql/015_tool_usage.sql")),
    (
        "016_proxy_calls",
        include_str!("../sql/016_proxy_calls.sql"),
    ),
    (
        "017_proxy_task_type",
        include_str!("../sql/017_proxy_task_type.sql"),
    ),
    (
        "018_vault_cache_token_count",
        include_str!("../sql/018_vault_cache_token_count.sql"),
    ),
    (
        "019_vault_cache_content",
        include_str!("../sql/019_vault_cache_content.sql"),
    ),
    (
        "020_identity_columns",
        include_str!("../sql/020_identity_columns.sql"),
    ),
    (
        "021_company_spend",
        include_str!("../sql/021_company_spend.sql"),
    ),
    (
        "022_workflow_runs",
        include_str!("../sql/022_workflow_runs.sql"),
    ),
    (
        "023_thread_note_path",
        include_str!("../sql/023_thread_note_path.sql"),
    ),
    ("024_calendar", include_str!("../sql/024_calendar.sql")),
    (
        "025_value_items",
        include_str!("../sql/025_value_items.sql"),
    ),
    (
        "026_proxy_turn_id",
        include_str!("../sql/026_proxy_turn_id.sql"),
    ),
    (
        "027_dispatch_governance",
        include_str!("../sql/027_dispatch_governance.sql"),
    ),
    (
        "028_hermes_sessions",
        include_str!("../sql/028_hermes_sessions.sql"),
    ),
    (
        "029_proxy_calls_quality",
        include_str!("../sql/029_proxy_calls_quality.sql"),
    ),
    (
        "030_self_update",
        include_str!("../sql/030_self_update.sql"),
    ),
    (
        "031_harness_sessions",
        include_str!("../sql/031_harness_sessions.sql"),
    ),
    ("032_missions", include_str!("../sql/032_missions.sql")),
    (
        "033_missions_teamlead",
        include_str!("../sql/033_missions_teamlead.sql"),
    ),
    (
        "034_background_turns",
        include_str!("../sql/034_background_turns.sql"),
    ),
    (
        "035_background_turn_watches",
        include_str!("../sql/035_background_turn_watches.sql"),
    ),
    (
        "036_harness_session_snapshot",
        include_str!("../sql/036_harness_session_snapshot.sql"),
    ),
    (
        "037_factory_proposals",
        include_str!("../sql/037_factory_proposals.sql"),
    ),
    (
        "038_factory_proposals_verify_block",
        include_str!("../sql/038_factory_proposals_verify_block.sql"),
    ),
    (
        "039_background_turns_blocked_on",
        include_str!("../sql/039_background_turns_blocked_on.sql"),
    ),
    (
        "040_factory_proposals_backoff",
        include_str!("../sql/040_factory_proposals_backoff.sql"),
    ),
    (
        "041_platform_threads",
        include_str!("../sql/041_platform_threads.sql"),
    ),
    (
        "042_missions_category",
        include_str!("../sql/042_missions_category.sql"),
    ),
    (
        "043_harness_sessions_herdr",
        include_str!("../sql/043_harness_sessions_herdr.sql"),
    ),
    ("044_tasks", include_str!("../sql/044_tasks.sql")),
    (
        "045_drop_missions_factory",
        include_str!("../sql/045_drop_missions_factory.sql"),
    ),
    (
        "046_drop_proxy_calls",
        include_str!("../sql/046_drop_proxy_calls.sql"),
    ),
    (
        "047_task_folders",
        include_str!("../sql/047_task_folders.sql"),
    ),
    (
        "048_drop_coding_agent_registry",
        include_str!("../sql/048_drop_coding_agent_registry.sql"),
    ),
    ("049_drop_plans", include_str!("../sql/049_drop_plans.sql")),
    (
        "050_drop_dead_memory_tables",
        include_str!("../sql/050_drop_dead_memory_tables.sql"),
    ),
    ("051_drop_facts", include_str!("../sql/051_drop_facts.sql")),
    (
        "052_purge_cognition_memories",
        include_str!("../sql/052_purge_cognition_memories.sql"),
    ),
    (
        "053_task_hierarchy_and_dependencies",
        include_str!("../sql/053_task_hierarchy_and_dependencies.sql"),
    ),
    (
        "054_drop_whatsapp_tables",
        include_str!("../sql/054_drop_whatsapp_tables.sql"),
    ),
    (
        "055_drop_calendar_tables",
        include_str!("../sql/055_drop_calendar_tables.sql"),
    ),
    (
        "056_drop_codegraph_tables",
        include_str!("../sql/056_drop_codegraph_tables.sql"),
    ),
    (
        "057_drop_dead_tables",
        include_str!("../sql/057_drop_dead_tables.sql"),
    ),
    (
        MEMORY_SCHEMA_MIGRATION,
        include_str!("../sql/058_memory_schema.sql"),
    ),
    (
        "059_drop_wikilink_graph_links",
        include_str!("../sql/059_drop_wikilink_graph_links.sql"),
    ),
    (
        "060_chat_message_meta",
        include_str!("../sql/060_chat_message_meta.sql"),
    ),
    (
        "061_harness_session_driver",
        include_str!("../sql/061_harness_session_driver.sql"),
    ),
    (
        "062_task_lifecycle_events",
        include_str!("../sql/062_task_lifecycle_events.sql"),
    ),
    ("063_task_graph", include_str!("../sql/063_task_graph.sql")),
    (
        "064_harness_session_goal",
        include_str!("../sql/064_harness_session_goal.sql"),
    ),
    (
        "065_graph_provenance",
        include_str!("../sql/065_graph_provenance.sql"),
    ),
    (
        "066_copilot_usage_samples",
        include_str!("../sql/066_copilot_usage_samples.sql"),
    ),
    (
        "067_subagent_runs",
        include_str!("../sql/067_subagent_runs.sql"),
    ),
    (
        "068_task_external_id",
        include_str!("../sql/068_task_external_id.sql"),
    ),
    ("069_hq_asks", include_str!("../sql/069_hq_asks.sql")),
    (
        "070_harness_session_drive_guards",
        include_str!("../sql/070_harness_session_drive_guards.sql"),
    ),
    (
        "071_harness_session_dismissals",
        include_str!("../sql/071_harness_session_dismissals.sql"),
    ),
    (
        "072_harness_session_dismiss_tail",
        include_str!("../sql/072_harness_session_dismiss_tail.sql"),
    ),
    (
        "073_approval_binding",
        include_str!("../sql/073_approval_binding.sql"),
    ),
    (
        "074_harness_session_tokens",
        include_str!("../sql/074_harness_session_tokens.sql"),
    ),
    (
        "075_task_messages",
        include_str!("../sql/075_task_messages.sql"),
    ),
    (
        "076_session_parent",
        include_str!("../sql/076_session_parent.sql"),
    ),
    (
        "077_session_archive",
        include_str!("../sql/077_session_archive.sql"),
    ),
    (
        "078_ledger_cost_attribution",
        include_str!("../sql/078_ledger_cost_attribution.sql"),
    ),
    (
        "079_harness_usage",
        include_str!("../sql/079_harness_usage.sql"),
    ),
    (
        "080_task_event_log",
        include_str!("../sql/080_task_event_log.sql"),
    ),
];

const MEMORY_SCHEMA_MIGRATION: &str = "058_memory_schema";

/// Columns hq-memory used to ALTER in at startup. Older databases already have them.
const MEMORY_COLUMNS: &[(&str, &str, &str)] = &[
    ("memories", "harness", "TEXT NOT NULL DEFAULT ''"),
    ("memories", "raw_text", "TEXT NOT NULL DEFAULT ''"),
    ("memories", "summary", "TEXT NOT NULL DEFAULT ''"),
    ("memories", "entities", "TEXT NOT NULL DEFAULT '[]'"),
    ("memories", "topics", "TEXT NOT NULL DEFAULT '[]'"),
    ("memories", "last_accessed_at", "TEXT"),
    ("memories", "access_count", "INTEGER NOT NULL DEFAULT 0"),
    ("memories", "replay_count", "INTEGER NOT NULL DEFAULT 0"),
    ("memories", "delta_summary", "TEXT"),
    ("memories", "user_id", "TEXT"),
    (
        "consolidations",
        "connections",
        "TEXT NOT NULL DEFAULT '[]'",
    ),
    ("consolidations", "parent_id", "INTEGER"),
];

/// SQLite has no ADD COLUMN IF NOT EXISTS, so check table_info first.
fn add_missing_memory_columns(conn: &Connection) -> Result<()> {
    for (table, col, typedef) in MEMORY_COLUMNS {
        let exists: bool = conn.query_row(
            "SELECT COUNT(*) > 0 FROM pragma_table_info(?1) WHERE name = ?2",
            [table, col],
            |row| row.get(0),
        )?;
        if !exists {
            conn.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {col} {typedef};"))?;
        }
    }
    Ok(())
}

fn apply(conn: &Connection, migrations: &[(&str, &str)]) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_version (
            version TEXT PRIMARY KEY,
            applied_at TEXT NOT NULL DEFAULT (datetime('now'))
        );",
    )?;

    for (version, sql) in migrations {
        apply_one(conn, version, sql)?;
    }
    Ok(())
}

/// One migration and its `schema_version` row commit together, under the write
/// lock, with the version re-checked inside it. Two processes opening the same
/// database cannot both run an `ALTER TABLE ... ADD COLUMN`, and a crash between
/// the DDL and the bookkeeping cannot leave a half-recorded migration that fails
/// with "duplicate column" on the next start.
fn apply_one(conn: &Connection, version: &str, sql: &str) -> Result<()> {
    let tx = rusqlite::Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    let already_applied: bool = tx.query_row(
        "SELECT COUNT(*) > 0 FROM schema_version WHERE version = ?1",
        [version],
        |row| row.get(0),
    )?;
    if already_applied {
        return Ok(());
    }
    if version == MEMORY_SCHEMA_MIGRATION {
        add_missing_memory_columns(&tx)?;
    }
    tx.execute_batch(sql)?;
    tx.execute(
        "INSERT INTO schema_version (version) VALUES (?1)",
        [version],
    )?;
    tx.commit()?;
    info!(version = %version, "applied migration");
    Ok(())
}

pub fn run(conn: &Connection) -> Result<()> {
    apply(conn, MIGRATIONS)?;

    // One-time: enable incremental auto_vacuum on existing databases.
    // PRAGMA auto_vacuum can only take effect after a full VACUUM.
    // 0 = NONE (disabled), 1 = FULL, 2 = INCREMENTAL.
    let auto_vacuum_mode: i64 = conn
        .query_row("PRAGMA auto_vacuum", [], |row| row.get(0))
        .unwrap_or(2); // assume already enabled if query fails

    if auto_vacuum_mode == 0 {
        // Set the mode first, then VACUUM to rebuild the file with it active.
        // VACUUM cannot run inside a transaction — execute_batch runs each
        // statement independently so this is safe.
        conn.execute_batch("PRAGMA auto_vacuum = INCREMENTAL;")?;
        conn.execute_batch("VACUUM;")?;
        info!("one-time: enabled incremental auto_vacuum via full VACUUM");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn memory_columns_are_plain_identifiers_and_types() {
        let ident =
            |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_lowercase() || c == '_');
        let typedef = |s: &str| {
            s.chars()
                .all(|c| c.is_ascii_alphanumeric() || " _'[](),.".contains(c))
        };
        for (table, col, def) in super::MEMORY_COLUMNS {
            assert!(ident(table) && ident(col), "{table}.{col}");
            assert!(typedef(def), "{table}.{col}: {def}");
        }
    }

    use crate::pool::Database;

    #[test]
    fn dead_tables_are_dropped() {
        let db = Database::open_memory().unwrap();
        let count: i64 = db
            .with_conn(|c| {
                Ok(c.query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name IN
                     ('model_cards','model_synergy','model_card_daily','sblu_training_runs',
                      'bounties','earn_usage','hermes_sessions','harness_quotas','spans',
                      'span_events','traces','locks','link_state','usage_records')",
                    [],
                    |r| r.get(0),
                )?)
            })
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn wikilink_graph_rows_are_deleted_and_suggestions_kept() {
        let db = Database::open_memory().unwrap();
        let remaining: Vec<String> = db
            .with_conn(|c| {
                c.execute_batch(
                    "INSERT INTO graph_links VALUES ('/abs/a.md', 'B', 1.0, 'wikilink', 0);
                     INSERT INTO graph_links VALUES ('a.md', 'b.md', 0.9, 'suggested', 0);",
                )?;
                c.execute_batch(include_str!("../sql/059_drop_wikilink_graph_links.sql"))?;
                let mut stmt = c.prepare("SELECT link_type FROM graph_links")?;
                let rows = stmt.query_map([], |r| r.get(0))?;
                Ok(rows.collect::<Result<Vec<String>, _>>()?)
            })
            .unwrap();
        assert_eq!(remaining, vec!["suggested".to_string()]);
    }

    #[test]
    fn tasks_tables_created() {
        let db = Database::open_memory().unwrap();
        let count: i64 = db
            .with_conn(|c| {
                Ok(c.query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name IN
                     ('spaces','initiatives','tasks','task_tags','task_comments')",
                    [],
                    |r| r.get(0),
                )?)
            })
            .unwrap();
        assert_eq!(count, 5);
    }

    #[test]
    fn folders_table_and_initiative_folder_id_created() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            let folder_table: i64 = c.query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='folders'",
                [],
                |r| r.get(0),
            )?;
            assert_eq!(folder_table, 1);
            let has_folder_id: i64 = c.query_row(
                "SELECT COUNT(*) FROM pragma_table_info('initiatives') WHERE name='folder_id'",
                [],
                |r| r.get(0),
            )?;
            assert_eq!(has_folder_id, 1);
            Ok(())
        })
        .unwrap();
    }

    /// What hq-memory's open_memory_tables ran at startup before migration 058.
    const LEGACY_MEMORY_DDL: &str = "
        CREATE TABLE IF NOT EXISTS entity_nodes (
            id INTEGER PRIMARY KEY AUTOINCREMENT, canonical TEXT NOT NULL UNIQUE,
            display_name TEXT NOT NULL, entity_type TEXT NOT NULL,
            mention_count INTEGER NOT NULL DEFAULT 1, created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS entity_edges (
            source_id INTEGER NOT NULL, target_id INTEGER NOT NULL,
            relationship TEXT NOT NULL, weight REAL NOT NULL DEFAULT 1.0,
            source_memory_id INTEGER, updated_at TEXT NOT NULL,
            PRIMARY KEY (source_id, target_id, relationship));
        CREATE INDEX IF NOT EXISTS idx_memories_topics ON memories(topics);
        CREATE INDEX IF NOT EXISTS idx_memories_replay ON memories(replay_count DESC);
        CREATE INDEX IF NOT EXISTS idx_entities_canonical ON entity_nodes(canonical);";

    fn pre_058() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let idx = super::MIGRATIONS
            .iter()
            .position(|(v, _)| *v == super::MEMORY_SCHEMA_MIGRATION)
            .unwrap();
        super::apply(&conn, &super::MIGRATIONS[..idx]).unwrap();
        conn
    }

    fn assert_memory_schema(conn: &rusqlite::Connection) {
        for (table, col, _) in super::MEMORY_COLUMNS {
            let n: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM pragma_table_info(?1) WHERE name = ?2",
                    [table, col],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(n, 1, "{table}.{col} missing");
        }
        let tables: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name IN
                 ('entity_nodes','entity_edges','workflow_runs','workflow_run_events','company_spend')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(tables, 2, "entity tables present, dead tables dropped");
    }

    #[test]
    fn memory_schema_migration_on_fresh_db() {
        let conn = pre_058();
        super::run(&conn).unwrap();
        assert_memory_schema(&conn);
    }

    #[test]
    fn memory_schema_migration_on_db_built_by_legacy_ddl() {
        let conn = pre_058();
        // The legacy code swallowed errors, since some columns (access_count) predate it.
        for (table, col, typedef) in super::MEMORY_COLUMNS {
            let _ = conn.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {col} {typedef};"));
        }
        conn.execute_batch(LEGACY_MEMORY_DDL).unwrap();
        super::run(&conn).unwrap();
        assert_memory_schema(&conn);
    }

    #[test]
    fn task_lifecycle_migration_keeps_existing_rows_unknown_and_rolls_back() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let idx = super::MIGRATIONS
            .iter()
            .position(|(v, _)| *v == "062_task_lifecycle_events")
            .unwrap();
        super::apply(&conn, &super::MIGRATIONS[..idx]).unwrap();
        conn.execute_batch(
            "INSERT INTO spaces (id, name, slug) VALUES ('s', 'S', 's');
             INSERT INTO initiatives (id, space_id, name, slug, id_prefix) VALUES ('i', 's', 'I', 'i', 'I');
             INSERT INTO tasks (id, initiative_id, display_id, title, status)
                 VALUES ('t1', 'i', 'I-001', 'old', 'in_progress');",
        )
        .unwrap();

        super::run(&conn).unwrap();
        let (started, review): (Option<String>, Option<String>) = conn
            .query_row(
                "SELECT work_started_at, first_ready_for_review_at FROM tasks WHERE id = 't1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert!(started.is_none() && review.is_none());
        let events: i64 = conn
            .query_row("SELECT COUNT(*) FROM task_events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(events, 0);

        // The rollback documented in docs/plans/native-tasks.md.
        conn.execute_batch(
            "DROP TABLE task_events;
             ALTER TABLE tasks DROP COLUMN work_started_at;
             ALTER TABLE tasks DROP COLUMN first_ready_for_review_at;",
        )
        .unwrap();
        let title: String = conn
            .query_row("SELECT title FROM tasks WHERE id = 't1'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(title, "old");
    }

    #[test]
    fn task_event_log_migration_keeps_old_events_and_widens_the_types() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let idx = super::MIGRATIONS
            .iter()
            .position(|(v, _)| *v == "080_task_event_log")
            .unwrap();
        super::apply(&conn, &super::MIGRATIONS[..idx]).unwrap();
        conn.execute_batch(
            "INSERT INTO spaces (id, name, slug) VALUES ('s', 'S', 's');
             INSERT INTO initiatives (id, space_id, name, slug, id_prefix) VALUES ('i', 's', 'I', 'i', 'I');
             INSERT INTO tasks (id, initiative_id, display_id, title, status)
                 VALUES ('t1', 'i', 'I-001', 'old', 'in_progress');
             INSERT INTO task_events (task_id, event_type, occurred_at)
                 VALUES ('t1', 'entered_in_progress', '2020-01-01 00:00:00');",
        )
        .unwrap();

        super::run(&conn).unwrap();
        let (event, at, from): (String, String, Option<String>) = conn
            .query_row(
                "SELECT event_type, occurred_at, from_status FROM task_events WHERE task_id = 't1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!((event.as_str(), at.as_str()), ("entered_in_progress", "2020-01-01 00:00:00"));
        assert!(from.is_none(), "old events stay unknown, not guessed");
        conn.execute(
            "INSERT INTO task_events (task_id, event_type, from_status, to_status) \
             VALUES ('t1', 'entered_complete', 'in_progress', 'complete')",
            [],
        )
        .unwrap();
        assert!(
            conn.execute("INSERT INTO task_events (task_id, event_type) VALUES ('t1', 'bogus')", [])
                .is_err()
        );
        let next: i64 = conn
            .query_row("SELECT MAX(id) FROM task_events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(next, 2, "ids continue after the copied rows");

        // The rollback documented in docs/plans/native-tasks.md.
        conn.execute_batch("ALTER TABLE tasks DROP COLUMN completed_at;").unwrap();
    }

    #[test]
    fn task_event_log_migration_does_not_reuse_the_id_of_a_deleted_event() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let idx = super::MIGRATIONS
            .iter()
            .position(|(v, _)| *v == "080_task_event_log")
            .unwrap();
        super::apply(&conn, &super::MIGRATIONS[..idx]).unwrap();
        conn.execute_batch(
            "INSERT INTO spaces (id, name, slug) VALUES ('s', 'S', 's');
             INSERT INTO initiatives (id, space_id, name, slug, id_prefix) VALUES ('i', 's', 'I', 'i', 'I');
             INSERT INTO tasks (id, initiative_id, display_id, title) VALUES ('t1', 'i', 'I-001', 'a');
             INSERT INTO task_events (task_id, event_type) VALUES ('t1', 'entered_in_progress');
             INSERT INTO task_events (task_id, event_type) VALUES ('t1', 'entered_ready_for_review');
             DELETE FROM task_events WHERE id = 2;",
        )
        .unwrap();

        super::run(&conn).unwrap();
        conn.execute(
            "INSERT INTO task_events (task_id, event_type) VALUES ('t1', 'entered_complete')",
            [],
        )
        .unwrap();
        let id: i64 = conn
            .query_row("SELECT MAX(id) FROM task_events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(id, 3, "id 2 belonged to a deleted event and is not handed out again");
    }

    #[test]
    fn hq_asks_migration_creates_the_table_and_a_rerun_changes_nothing() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        super::run(&conn).unwrap();
        super::run(&conn).expect("applying twice is a no-op");
        let objects: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name IN ('hq_asks', 'idx_hq_asks_external')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(objects, 2);
        let applied: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM schema_version WHERE version = '069_hq_asks'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(applied, 1);
        let insert = "INSERT INTO hq_asks (ask_id, thread_id, external_id, scope, mode, caller, fingerprint, created_at) \
                      VALUES (?1, 't', 'same', 'full', 'read_only', 'c', 'f', 'now')";
        conn.execute(insert, ["a1"]).unwrap();
        assert!(
            conn.execute(insert, ["a2"]).is_err(),
            "one ask per external_id and scope"
        );
    }

    #[test]
    fn a_failing_migration_leaves_neither_its_ddl_nor_a_version_row() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE schema_version (version TEXT PRIMARY KEY, applied_at TEXT NOT NULL DEFAULT (datetime('now')));")
            .unwrap();
        let failing =
            "CREATE TABLE half (x); ALTER TABLE half ADD COLUMN y; ALTER TABLE half ADD COLUMN y;";
        assert!(super::apply_one(&conn, "999_bad", failing).is_err());
        let tables: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name = 'half'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let versions: i64 = conn
            .query_row("SELECT COUNT(*) FROM schema_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            (tables, versions),
            (0, 0),
            "the retry starts from a clean slate"
        );
    }

    #[test]
    fn two_processes_opening_one_database_do_not_both_run_a_migration() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("hq-migrations-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("vault.db");
        let handles: Vec<_> = (0..4)
            .map(|_| {
                let path = path.clone();
                std::thread::spawn(move || {
                    let conn = rusqlite::Connection::open(path).unwrap();
                    conn.busy_timeout(std::time::Duration::from_secs(30))
                        .unwrap();
                    super::run(&conn)
                })
            })
            .collect();
        for h in handles {
            h.join()
                .unwrap()
                .expect("concurrent open must not fail with duplicate column");
        }
        let conn = rusqlite::Connection::open(&path).unwrap();
        let applied: i64 = conn
            .query_row("SELECT COUNT(*) FROM schema_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(applied as usize, super::MIGRATIONS.len());
        super::run(&conn).expect("a second run is a no-op");
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
