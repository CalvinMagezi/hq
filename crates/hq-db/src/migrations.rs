use anyhow::Result;
use rusqlite::{Connection, TransactionBehavior};
use tracing::info;

/// Every migration, in the order it runs: (name, SQL). The list is written by `build.rs` from the
/// files in `sql/` in file name order, so adding a migration is adding a file. The name is the
/// file name without `.sql` and is what `schema_version` records.
const MIGRATIONS: &[(&str, &str)] = include!(concat!(env!("OUT_DIR"), "/migrations_list.rs"));

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

/// Migrations that were renumbered after some databases had already run them under the old
/// name (the UX run's migrations moved up one when main took 110). A database that recorded the
/// old name has the change already, so it is recorded under the new name and not run again.
const RENAMED: &[(&str, &str)] = &[
    ("111_push_kinds", "110_push_kinds"),
    ("112_client_ids", "111_client_ids"),
    ("113_owner_approvals", "112_owner_approvals"),
    ("114_device_names", "113_device_names"),
    ("152_hot_path_indexes", "150_hot_path_indexes"),
];

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
    if let Some((_, old)) = RENAMED.iter().find(|(new, _)| *new == version) {
        let ran_as_old: bool = tx.query_row(
            "SELECT COUNT(*) > 0 FROM schema_version WHERE version = ?1",
            [old],
            |row| row.get(0),
        )?;
        if ran_as_old {
            tx.execute(
                "INSERT INTO schema_version (version) VALUES (?1)",
                [version],
            )?;
            tx.commit()?;
            info!(version = %version, renamed_from = %old, "migration already applied under its old name");
            return Ok(());
        }
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

/// The leading number of a migration name (`007_codegraph_v2` is 7).
fn migration_number(name: &str) -> Option<u32> {
    name.split('_').next()?.parse().ok()
}

/// The migrations recorded in `conn`'s `schema_version` table that are newer than this build: a
/// database from a later HQ, which this build must not open. Old databases also carry rows for
/// migrations that were retired from the code (`006_model_cards`), so a name this build does not
/// know is only "newer" when its number is above the highest this build has. A name with no
/// number is treated as newer. An empty list also covers a database with no such table.
pub fn newer_migrations(conn: &Connection) -> Result<Vec<String>> {
    let has_table: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'schema_version')",
        [],
        |row| row.get(0),
    )?;
    if !has_table {
        return Ok(Vec::new());
    }
    let highest_known = MIGRATIONS
        .iter()
        .filter_map(|(name, _)| migration_number(name))
        .max()
        .unwrap_or(0);
    let mut stmt = conn.prepare("SELECT version FROM schema_version")?;
    let applied = stmt
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(applied
        .into_iter()
        .filter(|name| !MIGRATIONS.iter().any(|(known, _)| known == name))
        .filter(|name| migration_number(name).is_none_or(|n| n > highest_known))
        .collect())
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

    /// Numbers that two migrations share, because the history has them: `012_skill_invocations`
    /// and `012_task_outcomes` were added on two branches. Nothing new belongs here.
    const KNOWN_SHARED_NUMBERS: &[u32] = &[12];
    /// Numbers no migration has, because none was ever written under them. Nothing new belongs
    /// here either: a gap is a mistake unless someone says why.
    const KNOWN_MISSING_NUMBERS: &[u32] = &[6];

    /// What is wrong with a list of migration names, in words a person can act on. Empty means
    /// the list is fine. The names are the file names without `.sql`.
    fn migration_number_problems(names: &[&str], shared: &[u32], missing: &[u32]) -> Vec<String> {
        let mut problems = Vec::new();
        let mut numbers: Vec<(u32, &str)> = Vec::new();
        for name in names {
            let (digits, rest) = name.split_at(name.len().min(3));
            let well_formed = digits.len() == 3
                && digits.bytes().all(|b| b.is_ascii_digit())
                && rest.starts_with('_')
                && rest.len() > 1
                && rest[1..]
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
            if well_formed {
                numbers.push((digits.parse().unwrap(), name));
            } else {
                problems.push(format!(
                    "`{name}.sql` is not named NNN_snake_case_description.sql (three digits, then lowercase words)"
                ));
            }
        }
        for pair in names.windows(2) {
            if pair[0] >= pair[1] {
                problems.push(format!("`{}` and `{}` are not in order", pair[0], pair[1]));
            }
        }
        let mut seen: std::collections::BTreeMap<u32, Vec<&str>> = Default::default();
        for (n, name) in &numbers {
            seen.entry(*n).or_default().push(name);
        }
        for (n, group) in &seen {
            if group.len() > 1 && !shared.contains(n) {
                problems.push(format!(
                    "{} share the number {n:03}: if neither has been released rename the later one to the next free number; if one has, rename it and add the old name to RENAMED so databases that ran it do not run it twice",
                    group.join(" and ")
                ));
            }
        }
        if let Some(&max) = seen.keys().next_back() {
            for n in 1..max {
                if !seen.contains_key(&n) && !missing.contains(&n) {
                    problems.push(format!(
                        "no migration is numbered {n:03}: numbers must be consecutive (renumber, or list {n} in KNOWN_MISSING_NUMBERS with the reason)"
                    ));
                }
            }
        }
        problems
    }

    #[test]
    fn migration_numbers_are_unique_and_in_order() {
        // build.rs lists sql/ in file name order, so a migration is just a new file; the price is
        // that two branches adding `140_*.sql` merge cleanly in git. This is what catches that.
        let names: Vec<&str> = super::MIGRATIONS.iter().map(|(name, _)| *name).collect();
        let problems =
            migration_number_problems(&names, KNOWN_SHARED_NUMBERS, KNOWN_MISSING_NUMBERS);
        assert!(
            problems.is_empty(),
            "migrations:\n  {}",
            problems.join("\n  ")
        );
        assert!(
            names.contains(&super::MEMORY_SCHEMA_MIGRATION),
            "the memory schema migration is named in the list"
        );
        // Every number the allowlists excuse is really still in that state, so they cannot rot.
        for n in KNOWN_SHARED_NUMBERS {
            let count = names
                .iter()
                .filter(|name| name.starts_with(&format!("{n:03}_")))
                .count();
            assert!(
                count > 1,
                "{n:03} is no longer shared: delete it from KNOWN_SHARED_NUMBERS"
            );
        }
        for n in KNOWN_MISSING_NUMBERS {
            let count = names
                .iter()
                .filter(|name| name.starts_with(&format!("{n:03}_")))
                .count();
            assert_eq!(
                count, 0,
                "{n:03} exists now: delete it from KNOWN_MISSING_NUMBERS"
            );
        }
    }

    #[test]
    fn the_number_check_catches_what_it_is_for() {
        let clean = ["001_a", "002_b", "003_c"];
        assert!(migration_number_problems(&clean, &[], &[]).is_empty());

        // Two branches each added number 004: merges cleanly in git, must fail here.
        let two = ["001_a", "002_b", "003_c", "004_x", "004_y"];
        let found = migration_number_problems(&two, &[], &[]);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(
            found[0].contains("004_x and 004_y share the number 004"),
            "{found:?}"
        );
        // ...unless history says it is fine.
        assert!(migration_number_problems(&two, &[4], &[]).is_empty());

        // A gap.
        let gap = ["001_a", "003_c"];
        let found = migration_number_problems(&gap, &[], &[]);
        assert!(
            found[0].contains("no migration is numbered 002"),
            "{found:?}"
        );
        assert!(migration_number_problems(&gap, &[], &[2]).is_empty());

        // A bad name, and a list out of order.
        let bad = ["001_a", "002-b", "3_c"];
        let found = migration_number_problems(&bad, &[], &[]);
        assert_eq!(
            found.len(),
            2,
            "the two badly named files and nothing else: {found:?}"
        );
        assert!(
            found.iter().all(|p| p.contains("is not named NNN_")),
            "{found:?}"
        );
        let unordered = ["002_b", "001_a"];
        assert!(
            migration_number_problems(&unordered, &[], &[])
                .iter()
                .any(|p| p.contains("not in order"))
        );
    }

    #[test]
    fn a_database_that_ran_a_migration_under_its_old_name_opens() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        super::run(&conn).unwrap();
        // As a database left by a build from before the renumbering: the columns exist, recorded
        // under the old names.
        for (new, old) in super::RENAMED {
            conn.execute("DELETE FROM schema_version WHERE version = ?1", [new])
                .unwrap();
            conn.execute("INSERT INTO schema_version (version) VALUES (?1)", [old])
                .unwrap();
        }
        super::run(&conn).expect("no renamed migration runs twice");
        for (new, _) in super::RENAMED {
            let recorded: bool = conn
                .query_row(
                    "SELECT COUNT(*) > 0 FROM schema_version WHERE version = ?1",
                    [new],
                    |r| r.get(0),
                )
                .unwrap();
            assert!(recorded, "{new}");
        }
    }

    #[test]
    fn retired_migrations_are_not_newer_but_a_higher_number_is() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        super::run(&conn).unwrap();
        assert!(super::newer_migrations(&conn).unwrap().is_empty());
        // A row for a migration that was retired from the code long ago (number below the highest).
        conn.execute(
            "INSERT INTO schema_version (version) VALUES ('006_model_cards')",
            [],
        )
        .unwrap();
        assert!(super::newer_migrations(&conn).unwrap().is_empty());
        // A migration from a later build, and a name that carries no number at all.
        conn.execute(
            "INSERT INTO schema_version (version) VALUES ('999_from_the_future')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO schema_version (version) VALUES ('mystery')",
            [],
        )
        .unwrap();
        let mut newer = super::newer_migrations(&conn).unwrap();
        newer.sort();
        assert_eq!(newer, ["999_from_the_future", "mystery"]);
        let bare = rusqlite::Connection::open_in_memory().unwrap();
        assert!(super::newer_migrations(&bare).unwrap().is_empty());
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
    fn connector_audit_reaches_a_database_that_ran_087_before_it_had_the_table() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let idx = super::MIGRATIONS
            .iter()
            .position(|(v, _)| *v == "089_mcp_connector_audit")
            .unwrap();
        super::apply(&conn, &super::MIGRATIONS[..idx]).unwrap();
        let audit = |c: &rusqlite::Connection| -> i64 {
            c.query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='mcp_connector_audit'",
                [],
                |r| r.get(0),
            )
            .unwrap()
        };
        assert_eq!(audit(&conn), 0, "087 as shipped has no call log");
        super::run(&conn).unwrap();
        assert_eq!(audit(&conn), 1);
    }

    #[test]
    fn web_device_tokens_reach_a_fresh_and_an_existing_database() {
        let tables = |c: &rusqlite::Connection| -> i64 {
            c.query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table'
                 AND name IN ('web_device_tokens','web_revoked_tokens')",
                [],
                |r| r.get(0),
            )
            .unwrap()
        };
        let fresh = rusqlite::Connection::open_in_memory().unwrap();
        super::run(&fresh).unwrap();
        assert_eq!(tables(&fresh), 2);

        let existing = rusqlite::Connection::open_in_memory().unwrap();
        let idx = super::MIGRATIONS
            .iter()
            .position(|(v, _)| *v == "090_web_device_tokens")
            .unwrap();
        super::apply(&existing, &super::MIGRATIONS[..idx]).unwrap();
        existing
            .execute_batch(
                "INSERT INTO push_subscriptions (endpoint, p256dh, auth, created_at)
                 VALUES ('https://push.example.com/a', 'k', 'a', '2026-01-01T00:00:00Z');",
            )
            .unwrap();
        assert_eq!(tables(&existing), 0);
        super::run(&existing).unwrap();
        assert_eq!(tables(&existing), 2);
        let kept: i64 = existing
            .query_row("SELECT COUNT(*) FROM push_subscriptions", [], |r| r.get(0))
            .unwrap();
        assert_eq!(kept, 1, "existing rows survive");
        super::run(&existing).unwrap();
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
    fn calibration_and_sweep_migrations_add_tables_keep_the_policy_and_start_unenforced() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let idx = super::MIGRATIONS
            .iter()
            .position(|(v, _)| *v == "096_task_calibration")
            .unwrap();
        super::apply(&conn, &super::MIGRATIONS[..idx]).unwrap();
        conn.execute(
            "UPDATE task_verification_policy SET config = '{\"mode\":\"shadow\"}' WHERE id = 1",
            [],
        )
        .unwrap();
        super::run(&conn).unwrap();
        let (config, since): (String, Option<String>) = conn
            .query_row(
                "SELECT config, enforcing_since FROM task_verification_policy WHERE id = 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            config, "{\"mode\":\"shadow\"}",
            "the policy in force survives"
        );
        assert_eq!(
            since, None,
            "nothing is enforcing yet, so nothing is legacy"
        );
        for table in [
            "task_audits",
            "task_canary_runs",
            "task_veto_holds",
            "task_sweeps",
            "task_sweep_items",
            "task_verification_skips",
            "task_reverify_requests",
        ] {
            let n: i64 = conn
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
                .unwrap();
            assert_eq!(n, 0, "{table} starts empty");
        }
        // A sweep and its items round-trip, and a bad status is refused.
        conn.execute_batch(
            "INSERT INTO spaces (id, name, slug) VALUES ('s', 'S', 's');
             INSERT INTO initiatives (id, space_id, name, slug, id_prefix) VALUES ('i', 's', 'I', 'i', 'I');
             INSERT INTO tasks (id, initiative_id, display_id, title, status)
                 VALUES ('t1', 'i', 'I-001', 'old', 'ready_for_review');
             INSERT INTO task_sweeps (id, scope, status) VALUES ('sw-1', '{}', 'planning');
             INSERT INTO task_sweep_items (sweep_id, task_id, rank, status_before)
                 VALUES ('sw-1', 't1', 1, 'ready_for_review');",
        )
        .unwrap();
        assert!(
            conn.execute(
                "INSERT INTO task_sweeps (id, scope, status) VALUES ('sw-2', '{}', 'bogus')",
                []
            )
            .is_err()
        );
    }

    #[test]
    fn task_verification_migration_seeds_an_off_policy_keeps_tasks_and_rolls_back() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let idx = super::MIGRATIONS
            .iter()
            .position(|(v, _)| *v == "095_task_verification")
            .unwrap();
        super::apply(&conn, &super::MIGRATIONS[..idx]).unwrap();
        conn.execute_batch(
            "INSERT INTO spaces (id, name, slug) VALUES ('s', 'S', 's');
             INSERT INTO initiatives (id, space_id, name, slug, id_prefix) VALUES ('i', 's', 'I', 'i', 'I');
             INSERT INTO tasks (id, initiative_id, display_id, title, status)
                 VALUES ('t1', 'i', 'I-001', 'old', 'complete');",
        )
        .unwrap();

        super::run(&conn).unwrap();
        let policy: String = conn
            .query_row(
                "SELECT config FROM task_verification_policy WHERE id = 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(policy, "{}", "a fresh policy is empty, which reads as off");
        let verdicts: i64 = conn
            .query_row("SELECT COUNT(*) FROM task_verdicts", [], |r| r.get(0))
            .unwrap();
        assert_eq!(verdicts, 0, "existing tasks get no invented verdict");
        let status: String = conn
            .query_row("SELECT status FROM tasks WHERE id = 't1'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(status, "complete", "existing tasks are untouched");
        assert!(
            conn.execute(
                "INSERT INTO task_verification_policy (id, config) VALUES (2, '{}')",
                []
            )
            .is_err(),
            "there is one policy row"
        );

        // The rollback documented in docs/plans/native-tasks.md: 097, then 096, then 095.
        conn.execute_batch(
            "DROP TABLE task_reverify_requests;
             DROP TABLE task_verification_skips;
             DROP TABLE task_sweep_items;
             DROP TABLE task_sweeps;
             ALTER TABLE task_verification_policy DROP COLUMN enforcing_since;
             DROP TABLE task_veto_holds;
             DROP TABLE task_canary_runs;
             DROP TABLE task_audits;
             DROP TABLE task_verification_waits;
             DROP TRIGGER task_verdicts_no_delete;
             DROP TRIGGER task_verdicts_no_update;
             DROP TABLE task_verdicts;
             DROP TABLE task_verification_policy;",
        )
        .unwrap();
        let left: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name LIKE 'task_verdict%' \
                 OR name LIKE 'task_verification%' OR name LIKE 'task_sweep%' \
                 OR name IN ('task_audits', 'task_canary_runs', 'task_veto_holds', 'task_reverify_requests')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(left, 0, "the documented rollback leaves nothing behind");
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
        assert_eq!(
            (event.as_str(), at.as_str()),
            ("entered_in_progress", "2020-01-01 00:00:00")
        );
        assert!(from.is_none(), "old events stay unknown, not guessed");
        conn.execute(
            "INSERT INTO task_events (task_id, event_type, from_status, to_status) \
             VALUES ('t1', 'entered_complete', 'in_progress', 'complete')",
            [],
        )
        .unwrap();
        assert!(
            conn.execute(
                "INSERT INTO task_events (task_id, event_type) VALUES ('t1', 'bogus')",
                []
            )
            .is_err()
        );
        let next: i64 = conn
            .query_row("SELECT MAX(id) FROM task_events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(next, 2, "ids continue after the copied rows");

        // The rollback documented in docs/plans/native-tasks.md.
        conn.execute_batch("ALTER TABLE tasks DROP COLUMN completed_at;")
            .unwrap();
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
        assert_eq!(
            id, 3,
            "id 2 belonged to a deleted event and is not handed out again"
        );
    }

    /// A database shaped like a long-lived install: many tasks with tags, comments, dependencies and events,
    /// moved through every task migration at once. Nothing may be lost or invented, and it must not crawl.
    #[test]
    fn a_large_legacy_task_database_survives_the_whole_task_v2_migration_intact() {
        const TASKS: i64 = 4000;
        const EVENTS_PER_TASK: i64 = 2;
        const MAX_MIGRATION_SECS: u64 = 30;
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let idx = super::MIGRATIONS
            .iter()
            .position(|(v, _)| *v == "080_task_event_log")
            .unwrap();
        super::apply(&conn, &super::MIGRATIONS[..idx]).unwrap();
        conn.execute_batch(
            "INSERT INTO spaces (id, name, slug) VALUES ('s', 'S', 's');
             INSERT INTO initiatives (id, space_id, name, slug, id_prefix) VALUES ('i', 's', 'I', 'i', 'I');",
        )
        .unwrap();
        let tx = conn.unchecked_transaction().unwrap();
        for n in 0..TASKS {
            let status = [
                "to_do",
                "in_progress",
                "blocked",
                "ready_for_review",
                "complete",
            ][(n % 5) as usize];
            tx.execute(
                "INSERT INTO tasks (id, initiative_id, display_id, title, status, priority) VALUES (?1, 'i', ?2, ?3, ?4, 'normal')",
                rusqlite::params![format!("t{n}"), format!("I-{n:05}"), format!("Task {n}"), status],
            )
            .unwrap();
            tx.execute(
                "INSERT INTO task_tags (task_id, tag) VALUES (?1, 'legacy')",
                [format!("t{n}")],
            )
            .unwrap();
            if n > 0 {
                tx.execute(
                    "INSERT INTO task_dependencies (task_id, depends_on_task_id) VALUES (?1, ?2)",
                    rusqlite::params![format!("t{n}"), format!("t{}", n - 1)],
                )
                .unwrap();
            }
            tx.execute(
                "INSERT INTO task_comments (task_id, author, body) VALUES (?1, 'someone', 'a note')",
                [format!("t{n}")],
            )
            .unwrap();
            for event in ["entered_in_progress", "entered_ready_for_review"] {
                tx.execute(
                    "INSERT INTO task_events (task_id, event_type) VALUES (?1, ?2)",
                    rusqlite::params![format!("t{n}"), event],
                )
                .unwrap();
            }
        }
        tx.commit().unwrap();
        let count = |sql: &str| -> i64 { conn.query_row(sql, [], |r| r.get(0)).unwrap() };
        let deps = count("SELECT COUNT(*) FROM task_dependencies");
        let before = (
            count("SELECT COUNT(*) FROM tasks"),
            count("SELECT COUNT(*) FROM task_tags"),
            count("SELECT COUNT(*) FROM task_comments"),
            count("SELECT COUNT(*) FROM task_events"),
        );

        let started = std::time::Instant::now();
        super::run(&conn).unwrap();
        assert!(
            started.elapsed().as_secs() < MAX_MIGRATION_SECS,
            "migration took {:?}",
            started.elapsed()
        );

        let after = (
            count("SELECT COUNT(*) FROM tasks"),
            count("SELECT COUNT(*) FROM task_tags"),
            count("SELECT COUNT(*) FROM task_comments"),
            count("SELECT COUNT(*) FROM task_events"),
        );
        assert_eq!(before, after);
        assert_eq!(count("SELECT COUNT(*) FROM task_dependencies"), deps);
        assert_eq!(after.3, TASKS * EVENTS_PER_TASK);
        assert_eq!(
            count("SELECT COUNT(*) FROM task_assignees"),
            0,
            "no assignee is invented"
        );
        assert_eq!(
            count("SELECT COUNT(*) FROM task_work_sessions"),
            0,
            "no lease is invented"
        );
        assert_eq!(
            count("SELECT COUNT(*) FROM tasks WHERE archived_at IS NOT NULL"),
            0,
            "nothing is archived by migrating"
        );
        assert_eq!(
            count("SELECT COUNT(*) FROM tasks WHERE shelved_at IS NOT NULL"),
            0,
            "nothing is shelved by migrating"
        );
        assert_eq!(
            count("SELECT COUNT(*) FROM tasks WHERE title LIKE 'Task %'"),
            TASKS
        );
        let integrity: String = conn
            .query_row("PRAGMA integrity_check", [], |r| r.get(0))
            .unwrap();
        assert_eq!(integrity, "ok");
        let fk_problems: i64 = conn
            .prepare("PRAGMA foreign_key_check")
            .unwrap()
            .query_map([], |_| Ok(()))
            .unwrap()
            .count() as i64;
        assert_eq!(fk_problems, 0);
        // A second run is a no-op.
        super::run(&conn).unwrap();
        assert_eq!(
            count("SELECT COUNT(*) FROM task_events"),
            TASKS * EVENTS_PER_TASK
        );
    }

    #[test]
    fn work_session_migration_keeps_old_events_and_rolls_back() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let idx = super::MIGRATIONS
            .iter()
            .position(|(v, _)| *v == "081_task_work_sessions")
            .unwrap();
        super::apply(&conn, &super::MIGRATIONS[..idx]).unwrap();
        conn.execute_batch(
            "INSERT INTO spaces (id, name, slug) VALUES ('s', 'S', 's');
             INSERT INTO initiatives (id, space_id, name, slug, id_prefix) VALUES ('i', 's', 'I', 'i', 'I');
             INSERT INTO tasks (id, initiative_id, display_id, title) VALUES ('t1', 'i', 'I-001', 'a');
             INSERT INTO task_events (task_id, event_type) VALUES ('t1', 'entered_in_progress');",
        )
        .unwrap();

        super::run(&conn).unwrap();
        let actor: Option<String> = conn
            .query_row(
                "SELECT actor FROM task_events WHERE task_id = 't1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(actor.is_none(), "old events stay unattributed, not guessed");
        assert!(
            conn.execute(
                "INSERT INTO task_work_sessions (id, task_id, actor, token_hash, end_reason) \
                 VALUES ('w', 't1', 'a', 'h', 'vanished')",
                [],
            )
            .is_err(),
            "an unknown end reason is refused"
        );

        // The rollback documented in docs/plans/native-tasks.md.
        conn.execute_batch(
            "DROP TABLE task_work_sessions;
             ALTER TABLE task_events DROP COLUMN actor;
             ALTER TABLE task_events DROP COLUMN work_session_id;",
        )
        .unwrap();
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

    #[test]
    fn existing_skill_invocations_migrate_with_nulls_and_keep_working() {
        use rusqlite::Connection;
        let conn = Connection::open_in_memory().unwrap();
        let (before, rest) = super::MIGRATIONS.split_at(
            super::MIGRATIONS
                .iter()
                .position(|(n, _)| *n == "093_skill_invocation_evidence")
                .unwrap(),
        );
        super::apply(&conn, before).unwrap();
        conn.execute(
            "INSERT INTO skill_invocations (id, skill_name, session_id, trigger, outcome_score, outcome_source)
             VALUES ('old', 'sheets', 's-1', 'load_skill', 0.75, 'session_result')",
            [],
        )
        .unwrap();

        super::apply(&conn, rest).unwrap();

        // Every new column reads NULL on a row that predates it.
        let nulls: i64 = conn
            .query_row(
                "SELECT (skill_hash IS NULL) + (task_id IS NULL) + (purpose IS NULL)
                      + (used IS NULL) + (outcome_status IS NULL) + (outcome_ref IS NULL)
                   FROM skill_invocations WHERE id = 'old'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(nulls, 6);
        // The aggregates the reviewer relies on still read the old row.
        assert_eq!(
            crate::skill_invocations::count_invocations(&conn, "sheets").unwrap(),
            1
        );
        let mean = crate::skill_invocations::mean_outcome(&conn, "sheets")
            .unwrap()
            .unwrap();
        assert!((mean - 0.75).abs() < 1e-9);
    }

    #[test]
    fn legacy_delegated_work_notices_are_cleaned_once() {
        let db = Database::open_memory().unwrap();
        let old = "Sub-agent run ee68156e (verifier) is partial and completed: it ended without a finished deliverable. Details: subagent_run_status.";
        let day_ago = (chrono::Utc::now() - chrono::Duration::hours(30)).to_rfc3339();
        let now = chrono::Utc::now().to_rfc3339();
        let rows: Vec<(String, String, String)> = db
            .with_conn(|c| {
                for (id, reviewed, created) in [
                    ("a", Some(5), &now),
                    ("b", None, &day_ago),
                    ("c", None, &now),
                ] {
                    c.execute(
                        "INSERT INTO subagent_runs (run_id, child_id, goal, exec_status, accept_status, started_at, last_activity_at, settled_at, reviewed_at)
                         VALUES (?1, 'x', 'g', 'completed', 'partial', 1, 1, 1, ?2)",
                        rusqlite::params![id, reviewed],
                    )?;
                    c.execute(
                        "INSERT INTO value_items (id, source_task, kind, title, body, dedup_key, state, created_at)
                         VALUES (?1, 'subagent_runs', 'action_needed', 'Delegated work needs attention', ?2, ?3, 'delivered', ?4)",
                        rusqlite::params![id, old, format!("subagent-run-{id}"), created],
                    )?;
                }
                for _ in 0..2 {
                    c.execute_batch(include_str!("../sql/147_legacy_delegated_work_notices.sql"))?;
                }
                let mut stmt = c.prepare("SELECT id, state, title || ' ' || body FROM value_items ORDER BY id")?;
                let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
                Ok(rows.collect::<Result<_, _>>()?)
            })
            .unwrap();
        let state = |id: &str| rows.iter().find(|r| r.0 == id).unwrap().1.clone();
        assert_eq!(state("a"), "dismissed");
        assert_eq!(state("b"), "dismissed");
        assert_eq!(state("c"), "delivered");
        for (_, _, text) in &rows {
            assert!(text.contains("A background task did not finish"));
            for leaked in ["ee68156e", "verifier", "subagent_run_status", "Sub-agent"] {
                assert!(!text.contains(leaked), "{text}");
            }
        }
    }

    #[test]
    fn settled_delegated_work_notices_are_dismissed_and_duplicates_collapsed() {
        let db = Database::open_memory().unwrap();
        let older = (chrono::Utc::now() - chrono::Duration::hours(23)).to_rfc3339();
        let newer = (chrono::Utc::now() - chrono::Duration::minutes(5)).to_rfc3339();
        let title = "A background task did not finish";
        let body = "A background task did not finish. Open the chat to see what is missing.";

        let rows: Vec<(String, String)> = db
            .with_conn(|c| {
                // Settled, unreviewed, young (5m old): must be dismissed.
                c.execute(
                    "INSERT INTO subagent_runs (run_id, child_id, goal, exec_status, accept_status, started_at, last_activity_at, settled_at, reviewed_at)
                     VALUES ('settled-young', 'x', 'g', 'completed', 'partial', 1, 1, 1, NULL)",
                    [],
                )?;
                c.execute(
                    "INSERT INTO value_items (id, source_task, kind, title, body, dedup_key, state, created_at, link)
                     VALUES ('v-settled-young', 'subagent_runs', 'action_needed', ?1, ?2, 'subagent-run-settled-young', 'delivered', ?3, '/chat?thread=t1')",
                    rusqlite::params![title, body, newer],
                )?;

                // Settled, unreviewed, 23h old: must be dismissed.
                c.execute(
                    "INSERT INTO subagent_runs (run_id, child_id, goal, exec_status, accept_status, started_at, last_activity_at, settled_at, reviewed_at)
                     VALUES ('settled-23h', 'x', 'g', 'failed', 'blocked', 1, 1, 1, NULL)",
                    [],
                )?;
                c.execute(
                    "INSERT INTO value_items (id, source_task, kind, title, body, dedup_key, state, created_at, link)
                     VALUES ('v-settled-23h', 'subagent_runs', 'action_needed', ?1, ?2, 'subagent-run-settled-23h', 'delivered', ?3, '/chat?thread=t1')",
                    rusqlite::params![title, body, older],
                )?;

                // Orphaned notice with no run row: must be dismissed.
                c.execute(
                    "INSERT INTO value_items (id, source_task, kind, title, body, dedup_key, state, created_at)
                     VALUES ('v-orphan', 'subagent_runs', 'action_needed', ?1, ?2, 'subagent-run-missing', 'pending', ?3)",
                    rusqlite::params![title, body, newer],
                )?;

                // Unsettled running run with a unique link: must stay open.
                c.execute(
                    "INSERT INTO subagent_runs (run_id, child_id, goal, exec_status, accept_status, started_at, last_activity_at, settled_at)
                     VALUES ('open-solo', 'x', 'g', 'running', 'unverified', 1, 1, NULL)",
                    [],
                )?;
                c.execute(
                    "INSERT INTO value_items (id, source_task, kind, title, body, dedup_key, state, created_at, link)
                     VALUES ('v-open-solo', 'subagent_runs', 'action_needed', ?1, ?2, 'subagent-run-open-solo', 'delivered', ?3, '/chat?thread=solo')",
                    rusqlite::params![title, body, older],
                )?;

                // Two identical open alerts for unsettled runs in the same chat: newest stays, older dismissed.
                for id in ["open-dup-1", "open-dup-2"] {
                    c.execute(
                        "INSERT INTO subagent_runs (run_id, child_id, goal, exec_status, accept_status, started_at, last_activity_at, settled_at)
                         VALUES (?1, 'x', 'g', 'running', 'unverified', 1, 1, NULL)",
                        [id],
                    )?;
                }
                c.execute(
                    "INSERT INTO value_items (id, source_task, kind, title, body, dedup_key, state, created_at, link)
                     VALUES ('v-dup-old', 'subagent_runs', 'action_needed', ?1, ?2, 'subagent-run-open-dup-1', 'delivered', ?3, '/chat?thread=dup')",
                    rusqlite::params![title, body, older],
                )?;
                c.execute(
                    "INSERT INTO value_items (id, source_task, kind, title, body, dedup_key, state, created_at, link)
                     VALUES ('v-dup-new', 'subagent_runs', 'action_needed', ?1, ?2, 'subagent-run-open-dup-2', 'delivered', ?3, '/chat?thread=dup')",
                    rusqlite::params![title, body, newer],
                )?;

                for _ in 0..2 {
                    c.execute_batch(include_str!(
                        "../sql/149_clear_settled_delegated_work_notices.sql"
                    ))?;
                }
                let mut stmt =
                    c.prepare("SELECT id, state FROM value_items ORDER BY id")?;
                let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
                Ok(rows.collect::<Result<_, _>>()?)
            })
            .unwrap();

        assert_eq!(rows.len(), 6, "rows are dismissed, never deleted");
        let state = |id: &str| rows.iter().find(|r| r.0 == id).unwrap().1.as_str();
        assert_eq!(state("v-settled-young"), "dismissed");
        assert_eq!(state("v-settled-23h"), "dismissed");
        assert_eq!(state("v-orphan"), "dismissed");
        assert_eq!(state("v-open-solo"), "delivered");
        assert_eq!(state("v-dup-old"), "dismissed");
        assert_eq!(state("v-dup-new"), "delivered");
    }

    /// The memory index migrations on a database that already holds memories: nothing is lost, an
    /// old row can be found, a Discord row is held back from recall, and a number written as a
    /// timestamp becomes text. Running them again changes nothing.
    #[test]
    fn the_memory_index_migrations_keep_existing_rows_and_fix_old_timestamps() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let idx = super::MIGRATIONS
            .iter()
            .position(|(v, _)| *v == "153_memory_index")
            .unwrap();
        super::apply(&conn, &super::MIGRATIONS[..idx]).unwrap();
        conn.execute_batch(
            "INSERT INTO memories (id, content, source, importance, created_at, harness, summary)
                 VALUES (1, 'old row with only content', 'chat-turn', 0.5, 1700000000, 'web', '');
             INSERT INTO memories (id, content, source, importance, created_at, harness, summary)
                 VALUES (2, 'from the shared channel', 'chat-turn', 0.5,
                         '2026-03-01T10:00:00+00:00', 'discord', 'from the shared channel');
             INSERT INTO memories (id, content, source, importance, created_at, harness, summary)
                 VALUES (3, 'written in milliseconds', 'chat-turn', 0.5, 1700000000000, 'telegram',
                         'written in milliseconds');
             INSERT INTO consolidations (memory_ids, insight, created_at) VALUES ('[1]', 'x', 1700000000);",
        )
        .unwrap();

        super::run(&conn).unwrap();

        let row = |id: i64| -> (String, String, String, i64, String) {
            conn.query_row(
                "SELECT status, origin, provenance, seen_count, created_at FROM memories WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .unwrap()
        };
        let (status, origin, provenance, seen, created) = row(1);
        assert_eq!(
            (status.as_str(), origin.as_str(), provenance.as_str(), seen),
            ("active", "owner", "model_inferred", 1)
        );
        assert_eq!(created, "2023-11-14T22:13:20+00:00", "seconds became text");
        assert_eq!(
            row(2).1,
            "unknown_channel",
            "a shared-channel row is held back"
        );
        assert_eq!(
            row(2).4,
            "2026-03-01T10:00:00+00:00",
            "a text timestamp is untouched"
        );
        assert_eq!(
            row(3).1,
            "owner",
            "Telegram is a one-to-one chat with the owner"
        );
        assert_eq!(
            row(3).4,
            "2023-11-14T22:13:20+00:00",
            "milliseconds became text"
        );
        let consolidation: String = conn
            .query_row("SELECT created_at FROM consolidations", [], |r| r.get(0))
            .unwrap();
        assert_eq!(consolidation, "2023-11-14T22:13:20+00:00");

        // The old row had no summary, only content; it is searchable by that.
        let hits = crate::memories_search::search_keyword(
            &conn,
            "content",
            crate::memories_search::View::Recall,
            10,
        )
        .unwrap();
        assert_eq!(hits.iter().map(|h| h.id).collect::<Vec<_>>(), vec![1]);
        let shared = crate::memories_search::search_keyword(
            &conn,
            "shared channel",
            crate::memories_search::View::Recall,
            10,
        )
        .unwrap();
        assert!(shared.is_empty(), "held back from recall");

        // Applying the same migrations to a database that has them changes nothing.
        super::run(&conn).unwrap();
        assert_eq!(row(1).4, "2023-11-14T22:13:20+00:00");
        let indexed: i64 = conn
            .query_row("SELECT COUNT(*) FROM memories_fts", [], |r| r.get(0))
            .unwrap();
        assert_eq!(indexed, 3);
    }
}
