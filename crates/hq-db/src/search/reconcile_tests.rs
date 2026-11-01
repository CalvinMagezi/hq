use std::fs;
use std::path::{Path, PathBuf};

use super::*;

/// The real, fully migrated schema. A hand-picked subset of migrations once hid a table that a
/// later migration had dropped, so these tests use exactly what production runs.
fn db() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    crate::migrations::run(&conn).unwrap();
    conn
}

struct Vault {
    dir: tempfile::TempDir,
}

impl Vault {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("Notebooks")).unwrap();
        Self { dir }
    }

    fn root(&self) -> &Path {
        self.dir.path()
    }

    fn notebooks(&self) -> PathBuf {
        self.root().join("Notebooks")
    }

    fn write(&self, rel: &str, body: &str) {
        let path = self.root().join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, body).unwrap();
    }

    fn remove(&self, rel: &str) {
        fs::remove_file(self.root().join(rel)).unwrap();
    }

    fn sync(&self, conn: &Connection) -> SyncReport {
        sync_index(conn, &self.notebooks()).unwrap()
    }
}

fn count(conn: &Connection, sql: &str, path: &str) -> i64 {
    conn.query_row(sql, [path], |row| row.get(0)).unwrap()
}

/// How many rows each derived table holds for `path`: (fts, sync_state, vault_cache, embeddings).
fn rows(conn: &Connection, path: &str) -> (i64, i64, i64, i64) {
    (
        count(conn, "SELECT COUNT(*) FROM notes_fts WHERE path = ?1", path),
        count(
            conn,
            "SELECT COUNT(*) FROM sync_state WHERE path = ?1",
            path,
        ),
        count(
            conn,
            "SELECT COUNT(*) FROM vault_cache WHERE path = ?1",
            path,
        ),
        count(
            conn,
            "SELECT COUNT(*) FROM embeddings WHERE note_path = ?1",
            path,
        ),
    )
}

fn link_rows(conn: &Connection, path: &str) -> i64 {
    count(
        conn,
        "SELECT COUNT(*) FROM graph_links WHERE source_path = ?1 OR target_path = ?1",
        path,
    )
}

fn seed_links(conn: &Connection, path: &str) {
    add_graph_link(conn, path, "Notebooks/other.md", 0.9, "applied").unwrap();
}

#[test]
fn first_scan_indexes_every_note_and_a_second_pass_changes_nothing() {
    let conn = db();
    let vault = Vault::new();
    vault.write("Notebooks/a.md", "---\ntags: [x]\n---\nalpha body");
    vault.write("Notebooks/Projects/deep/b.md", "bravo body");

    let first = vault.sync(&conn);
    assert_eq!((first.indexed, first.removed, first.unchanged), (2, 0, 0));
    assert_eq!(rows(&conn, "Notebooks/a.md"), (1, 1, 1, 0));
    assert_eq!(rows(&conn, "Notebooks/Projects/deep/b.md"), (1, 1, 1, 0));
    assert_eq!(keyword_search(&conn, "alpha", 5).unwrap().len(), 1);

    let second = vault.sync(&conn);
    assert_eq!(
        (second.indexed, second.removed, second.unchanged),
        (0, 0, 2)
    );
}

#[test]
fn an_edited_note_is_reindexed_once_with_no_duplicate_rows() {
    let conn = db();
    let vault = Vault::new();
    vault.write("Notebooks/a.md", "first draft");
    vault.sync(&conn);

    vault.write("Notebooks/a.md", "second draft with zebra");
    let report = vault.sync(&conn);
    assert_eq!((report.indexed, report.unchanged), (1, 0));
    assert_eq!(rows(&conn, "Notebooks/a.md"), (1, 1, 1, 0));
    assert!(keyword_search(&conn, "first", 5).unwrap().is_empty());
    assert_eq!(keyword_search(&conn, "zebra", 5).unwrap().len(), 1);
}

#[test]
fn a_deleted_note_leaves_nothing_behind_in_any_derived_table() {
    let conn = db();
    let vault = Vault::new();
    vault.write("Notebooks/gone.md", "soon deleted");
    vault.write("Notebooks/kept.md", "stays");
    vault.sync(&conn);
    store_embedding(&conn, "Notebooks/gone.md", &[1.0, 2.0, 3.0], "m").unwrap();
    store_embedding(&conn, "Notebooks/kept.md", &[3.0, 2.0, 1.0], "m").unwrap();
    seed_links(&conn, "Notebooks/gone.md");
    assert_eq!(rows(&conn, "Notebooks/gone.md"), (1, 1, 1, 1));

    vault.remove("Notebooks/gone.md");
    let report = vault.sync(&conn);

    assert_eq!((report.removed, report.unchanged), (1, 1));
    assert_eq!(rows(&conn, "Notebooks/gone.md"), (0, 0, 0, 0));
    assert_eq!(link_rows(&conn, "Notebooks/gone.md"), 0);
    assert_eq!(rows(&conn, "Notebooks/kept.md"), (1, 1, 1, 1));
    let semantic = semantic_search(&conn, &[1.0, 2.0, 3.0], "m", 10).unwrap();
    assert!(semantic.iter().all(|r| r.note_path != "Notebooks/gone.md"));
}

#[test]
fn a_note_written_by_a_tool_and_deleted_before_any_scan_is_still_cleaned_up() {
    let conn = db();
    let vault = Vault::new();
    // Tool and web writes index straight into FTS and never touch sync_state.
    index_note(&conn, "Notebooks/tool.md", "tool", "body", "").unwrap();
    store_embedding(&conn, "Notebooks/tool.md", &[1.0, 0.0], "m").unwrap();
    assert_eq!(rows(&conn, "Notebooks/tool.md"), (1, 0, 0, 1));

    let report = vault.sync(&conn);
    assert_eq!(report.removed, 1);
    assert_eq!(rows(&conn, "Notebooks/tool.md"), (0, 0, 0, 0));
}

#[test]
fn rows_for_paths_outside_notebooks_are_left_alone() {
    let conn = db();
    let vault = Vault::new();
    index_note(&conn, "Skills/x.md", "x", "body", "").unwrap();
    store_embedding(&conn, "Skills/x.md", &[1.0], "m").unwrap();

    let report = vault.sync(&conn);
    assert_eq!(report.removed, 0);
    assert_eq!(rows(&conn, "Skills/x.md"), (1, 0, 0, 1));
}

#[test]
fn hidden_folders_the_trash_meta_files_and_other_extensions_are_not_indexed() {
    let conn = db();
    let vault = Vault::new();
    vault.write("Notebooks/real.md", "indexed");
    vault.write("Notebooks/.obsidian/notes.md", "hidden folder");
    vault.write("Notebooks/_trash/old.md", "trash");
    vault.write("Notebooks/_meta.md", "meta");
    vault.write("Notebooks/.swap.md", "editor swap");
    vault.write("Notebooks/readme.txt", "text");
    vault.write("Notebooks/draft.md~", "backup");

    let report = vault.sync(&conn);
    assert_eq!(report.indexed, 1);
    assert_eq!(indexed_count(&conn).unwrap(), 1);
}

#[cfg(unix)]
#[test]
fn symlinks_are_not_followed_in_a_scan() {
    use std::os::unix::fs::symlink;

    let conn = db();
    let vault = Vault::new();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("secret.md"), "outside the vault").unwrap();
    vault.write("Notebooks/real.md", "inside");
    symlink(outside.path(), vault.notebooks().join("linked-dir")).unwrap();
    symlink(
        outside.path().join("secret.md"),
        vault.notebooks().join("linked-file.md"),
    )
    .unwrap();
    // A link back to an ancestor must not make the walk loop.
    symlink(vault.notebooks(), vault.notebooks().join("loop")).unwrap();

    let report = vault.sync(&conn);
    assert_eq!(report.indexed, 1);
    assert_eq!(indexed_count(&conn).unwrap(), 1);
    assert!(keyword_search(&conn, "outside", 5).unwrap().is_empty());
}

#[test]
fn an_old_style_hash_costs_one_reindex_and_keeps_the_embedding() {
    let conn = db();
    let vault = Vault::new();
    vault.write("Notebooks/a.md", "unchanged text");
    // What the DefaultHasher-based sync stored: a short bare hex string.
    conn.execute(
        "INSERT INTO sync_state (path, content_hash, modified_at, synced_at)
         VALUES ('Notebooks/a.md', '3f9a1c2d4e5b6a70', 1, 1)",
        [],
    )
    .unwrap();
    index_note(&conn, "Notebooks/a.md", "a", "unchanged text", "").unwrap();
    store_embedding(&conn, "Notebooks/a.md", &[1.0, 2.0], "m").unwrap();

    let first = vault.sync(&conn);
    assert_eq!((first.indexed, first.unchanged), (1, 0));
    assert_eq!(rows(&conn, "Notebooks/a.md"), (1, 1, 1, 1));
    let stored: String = conn
        .query_row(
            "SELECT content_hash FROM sync_state WHERE path = 'Notebooks/a.md'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(stored.starts_with("b3:"));

    let second = vault.sync(&conn);
    assert_eq!((second.indexed, second.unchanged), (0, 1));
}

#[test]
fn a_note_that_cannot_be_read_keeps_its_rows() {
    let conn = db();
    let vault = Vault::new();
    vault.write("Notebooks/a.md", "good text");
    vault.sync(&conn);

    // Not valid UTF-8: the file exists, but the scan cannot read it as text.
    fs::write(vault.notebooks().join("a.md"), [0xff, 0xfe, 0x00, 0xc3]).unwrap();
    let report = vault.sync(&conn);

    assert_eq!((report.failed, report.removed, report.indexed), (1, 0, 0));
    assert_eq!(rows(&conn, "Notebooks/a.md"), (1, 1, 1, 0));
}

#[test]
fn a_missing_notebooks_folder_is_a_quiet_no_op() {
    let conn = db();
    let vault = Vault::new();
    index_note(&conn, "Notebooks/a.md", "a", "body", "").unwrap();
    let missing = vault.root().join("nowhere");

    assert_eq!(sync_index(&conn, &missing).unwrap(), SyncReport::default());
    assert_eq!(indexed_count(&conn).unwrap(), 1);
}

#[test]
fn the_cache_carries_the_files_mtime_not_the_time_of_indexing() {
    let conn = db();
    let vault = Vault::new();
    vault.write("Notebooks/old.md", "written long ago");
    let when = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000);
    fs::File::options()
        .write(true)
        .open(vault.notebooks().join("old.md"))
        .unwrap()
        .set_modified(when)
        .unwrap();

    vault.sync(&conn);
    assert_eq!(
        crate::vault_cache::get_mtime(&conn, "Notebooks/old.md").unwrap(),
        Some(1_000_000)
    );
    let recent = recent_notes(&conn, 5).unwrap();
    assert_eq!(recent[0].note_path, "Notebooks/old.md");
}

#[test]
fn reconcile_path_acts_on_the_file_as_it_is_now() {
    let conn = db();
    let vault = Vault::new();
    let rel = "Notebooks/Projects/p.md";

    vault.write(rel, "version one");
    assert_eq!(
        reconcile_path(&conn, vault.root(), rel).unwrap(),
        PathOutcome::Indexed
    );
    assert_eq!(
        reconcile_path(&conn, vault.root(), rel).unwrap(),
        PathOutcome::Unchanged
    );

    vault.write(rel, "version two");
    assert_eq!(
        reconcile_path(&conn, vault.root(), rel).unwrap(),
        PathOutcome::Indexed
    );
    assert_eq!(rows(&conn, rel), (1, 1, 1, 0));
    assert_eq!(keyword_search(&conn, "two", 5).unwrap().len(), 1);

    vault.remove(rel);
    assert_eq!(
        reconcile_path(&conn, vault.root(), rel).unwrap(),
        PathOutcome::Removed
    );
    assert_eq!(rows(&conn, rel), (0, 0, 0, 0));
    // A duplicate "deleted" signal changes nothing.
    assert_eq!(
        reconcile_path(&conn, vault.root(), rel).unwrap(),
        PathOutcome::Unchanged
    );
}

#[test]
fn reconcile_path_ignores_paths_the_index_does_not_cover() {
    let conn = db();
    let vault = Vault::new();
    vault.write("Other/a.md", "elsewhere");
    vault.write("Notebooks/a.txt", "not markdown");
    vault.write("Notebooks/.hidden/a.md", "hidden");
    let outside = vault.root().parent().unwrap().join("escape.md");

    for rel in [
        "Other/a.md",
        "Notebooks/a.txt",
        "Notebooks/.hidden/a.md",
        "Notebooks/../Other/a.md",
        "../escape.md",
        outside.to_str().unwrap(),
    ] {
        assert_eq!(
            reconcile_path(&conn, vault.root(), rel).unwrap(),
            PathOutcome::Ignored,
            "{rel}"
        );
    }
    assert_eq!(indexed_count(&conn).unwrap(), 0);
}

#[cfg(unix)]
#[test]
fn reconcile_path_refuses_a_path_reached_through_a_symlink() {
    use std::os::unix::fs::symlink;

    let conn = db();
    let vault = Vault::new();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("n.md"), "outside").unwrap();
    symlink(outside.path(), vault.notebooks().join("Link")).unwrap();

    let outcome = reconcile_path(&conn, vault.root(), "Notebooks/Link/n.md").unwrap();
    assert_ne!(outcome, PathOutcome::Indexed);
    assert_eq!(indexed_count(&conn).unwrap(), 0);
}

#[test]
fn a_move_keeps_link_rows_but_a_delete_drops_them() {
    let conn = db();
    index_note(&conn, "Notebooks/moved.md", "m", "body", "").unwrap();
    index_note(&conn, "Notebooks/deleted.md", "d", "body", "").unwrap();
    seed_links(&conn, "Notebooks/moved.md");
    seed_links(&conn, "Notebooks/deleted.md");

    remove_note(&conn, "Notebooks/moved.md").unwrap();
    remove_path_everywhere(&conn, "Notebooks/deleted.md").unwrap();

    assert_eq!(rows(&conn, "Notebooks/moved.md"), (0, 0, 0, 0));
    assert_eq!(link_rows(&conn, "Notebooks/moved.md"), 1);
    assert_eq!(link_rows(&conn, "Notebooks/deleted.md"), 0);
}

#[test]
fn rebuild_index_applies_the_same_exclusions_as_the_scan() {
    let conn = db();
    let vault = Vault::new();
    vault.write("Notebooks/real.md", "indexed");
    vault.write("Notebooks/.obsidian/x.md", "hidden");
    vault.write("Notebooks/_trash/y.md", "trash");

    let (indexed, errors) = rebuild_index(&conn, &vault.notebooks()).unwrap();
    assert_eq!((indexed, errors), (1, 0));
}

#[test]
fn a_folder_that_moves_is_reconciled_from_the_folder_event_alone() {
    let conn = db();
    let vault = Vault::new();
    vault.write("Notebooks/A/x.md", "alpha content");
    vault.write("Notebooks/A/sub/y.md", "bravo content");
    vault.sync(&conn);

    fs::rename(vault.notebooks().join("A"), vault.notebooks().join("B")).unwrap();
    let gone = sync_subtree(&conn, vault.root(), "Notebooks/A").unwrap();
    let arrived = sync_subtree(&conn, vault.root(), "Notebooks/B").unwrap();

    assert_eq!(gone.removed, 2);
    assert_eq!((arrived.indexed, arrived.removed), (2, 0));
    assert_eq!(rows(&conn, "Notebooks/A/x.md"), (0, 0, 0, 0));
    assert_eq!(rows(&conn, "Notebooks/B/sub/y.md"), (1, 1, 1, 0));
    assert_eq!(keyword_search(&conn, "bravo", 5).unwrap().len(), 1);
}

#[test]
fn a_deleted_folder_removes_what_is_under_it_and_not_a_sibling_with_the_same_prefix() {
    let conn = db();
    let vault = Vault::new();
    vault.write("Notebooks/A/x.md", "in A");
    vault.write("Notebooks/A2/q.md", "in A2, a sibling sharing the prefix");
    vault.write("Notebooks/C/z.md", "in C");
    vault.sync(&conn);
    store_embedding(&conn, "Notebooks/A/x.md", &[1.0], "m").unwrap();

    fs::remove_dir_all(vault.notebooks().join("A")).unwrap();
    let report = sync_subtree(&conn, vault.root(), "Notebooks/A").unwrap();

    assert_eq!(report.removed, 1);
    assert_eq!(rows(&conn, "Notebooks/A/x.md"), (0, 0, 0, 0));
    assert_eq!(rows(&conn, "Notebooks/A2/q.md"), (1, 1, 1, 0));
    assert_eq!(rows(&conn, "Notebooks/C/z.md"), (1, 1, 1, 0));
}

#[test]
fn a_folder_sync_leaves_other_folders_pending() {
    let conn = db();
    let vault = Vault::new();
    vault.write("Notebooks/A/x.md", "one");
    vault.write("Notebooks/C/z.md", "one");
    vault.sync(&conn);

    vault.write("Notebooks/A/x.md", "two");
    vault.write("Notebooks/C/z.md", "two");
    let report = sync_subtree(&conn, vault.root(), "Notebooks/A").unwrap();

    assert_eq!((report.indexed, report.unchanged), (1, 0));
    assert_eq!(keyword_search(&conn, "two", 5).unwrap().len(), 1);
    // The full scan still finds the folder that was not synced.
    assert_eq!(vault.sync(&conn).indexed, 1);
}

#[test]
fn a_folder_the_index_does_not_cover_is_ignored() {
    let conn = db();
    let vault = Vault::new();
    vault.write("Other/a.md", "elsewhere");
    for rel in ["Other", "Notebooks/.git", "Notebooks/_trash", "../up", ""] {
        assert_eq!(
            sync_subtree(&conn, vault.root(), rel).unwrap(),
            SyncReport::default(),
            "{rel}"
        );
    }
    assert_eq!(indexed_count(&conn).unwrap(), 0);
}

#[test]
fn syncing_the_notebooks_folder_is_a_full_scan() {
    let conn = db();
    let vault = Vault::new();
    vault.write("Notebooks/a.md", "a");
    vault.write("Notebooks/d/b.md", "b");
    let report = sync_subtree(&conn, vault.root(), "Notebooks").unwrap();
    assert_eq!(report.indexed, 2);
}

#[cfg(unix)]
#[test]
fn a_notebooks_folder_that_is_itself_a_symlink_still_syncs() {
    use std::os::unix::fs::symlink;

    let conn = db();
    let dir = tempfile::tempdir().unwrap();
    let real = dir.path().join("real-notes");
    fs::create_dir_all(real.join("Sub")).unwrap();
    fs::write(real.join("Sub/n.md"), "notes on a mounted drive").unwrap();
    let vault_root = dir.path().join("vault");
    fs::create_dir_all(&vault_root).unwrap();
    symlink(&real, vault_root.join("Notebooks")).unwrap();

    let report = sync_index(&conn, &vault_root.join("Notebooks")).unwrap();
    assert_eq!(report.indexed, 1);
    assert_eq!(
        reconcile_path(&conn, &vault_root, "Notebooks/Sub/n.md").unwrap(),
        PathOutcome::Unchanged
    );
    let folder = sync_subtree(&conn, &vault_root, "Notebooks/Sub").unwrap();
    assert_eq!((folder.indexed, folder.unchanged), (0, 1));
}

#[test]
fn freshness_counts_each_index_on_its_own() {
    let conn = db();
    let vault = Vault::new();
    assert_eq!(index_freshness(&conn).unwrap(), IndexFreshness::default());

    vault.write("Notebooks/a.md", "alpha");
    vault.write("Notebooks/b.md", "bravo");
    vault.write("Notebooks/c.md", "charlie");
    vault.sync(&conn);
    store_embedding(&conn, "Notebooks/a.md", &[1.0], "m").unwrap();
    // An embedding whose note is gone from the keyword index.
    store_embedding(&conn, "Notebooks/ghost.md", &[1.0], "m").unwrap();

    let f = index_freshness(&conn).unwrap();
    assert_eq!(f.keyword_indexed, 3);
    assert_eq!(f.embedded, 2);
    assert_eq!(f.awaiting_embedding, 2);
    assert_eq!(f.orphan_embeddings, 1);
    assert!(f.last_synced_at.is_some_and(|t| t > 1_700_000_000));
}

fn note_hash(conn: &Connection, path: &str) -> String {
    conn.query_row(
        "SELECT content_hash FROM sync_state WHERE path = ?1",
        [path],
        |r| r.get(0),
    )
    .unwrap()
}

fn pending(conn: &Connection, model: &str) -> Vec<(String, EmbedReason)> {
    pending_embeddings(conn, model, 100)
        .unwrap()
        .into_iter()
        .map(|p| (p.path, p.reason))
        .collect()
}

#[test]
fn a_note_with_no_embedding_is_pending_and_one_made_from_its_current_text_is_not() {
    let conn = db();
    let vault = Vault::new();
    vault.write("Notebooks/a.md", "alpha");
    vault.sync(&conn);
    assert_eq!(
        pending(&conn, "m"),
        vec![("Notebooks/a.md".to_string(), EmbedReason::Missing)]
    );

    let hash = note_hash(&conn, "Notebooks/a.md");
    store_embedding_of(&conn, "Notebooks/a.md", &[1.0], "m", Some(&hash)).unwrap();
    assert!(pending(&conn, "m").is_empty());
    assert_eq!(stale_embedding_count(&conn).unwrap(), 0);
}

#[test]
fn an_edit_makes_the_embedding_pending_again() {
    let conn = db();
    let vault = Vault::new();
    vault.write("Notebooks/a.md", "first");
    vault.sync(&conn);
    let first = note_hash(&conn, "Notebooks/a.md");
    store_embedding_of(&conn, "Notebooks/a.md", &[1.0], "m", Some(&first)).unwrap();

    vault.write("Notebooks/a.md", "second, quite different");
    vault.sync(&conn);

    assert_eq!(
        pending(&conn, "m"),
        vec![("Notebooks/a.md".to_string(), EmbedReason::Edited)]
    );
    assert_eq!(stale_embedding_count(&conn).unwrap(), 1);
    assert_eq!(index_freshness(&conn).unwrap().stale_embeddings, 1);

    let second = note_hash(&conn, "Notebooks/a.md");
    assert_ne!(first, second);
    store_embedding_of(&conn, "Notebooks/a.md", &[2.0], "m", Some(&second)).unwrap();
    assert!(pending(&conn, "m").is_empty());
    assert_eq!(
        get_embedding(&conn, "Notebooks/a.md").unwrap(),
        Some(vec![2.0])
    );
}

#[test]
fn a_different_model_makes_every_embedding_pending() {
    let conn = db();
    let vault = Vault::new();
    vault.write("Notebooks/a.md", "alpha");
    vault.sync(&conn);
    let hash = note_hash(&conn, "Notebooks/a.md");
    store_embedding_of(&conn, "Notebooks/a.md", &[1.0], "old-model", Some(&hash)).unwrap();

    assert!(pending(&conn, "old-model").is_empty());
    assert_eq!(
        pending(&conn, "new-model"),
        vec![("Notebooks/a.md".to_string(), EmbedReason::OtherModel)]
    );
}

#[test]
fn new_notes_are_embedded_before_re_embedding_old_ones() {
    let conn = db();
    let vault = Vault::new();
    vault.write("Notebooks/a-old.md", "one");
    vault.write("Notebooks/z-new.md", "two");
    vault.sync(&conn);
    let hash = note_hash(&conn, "Notebooks/a-old.md");
    store_embedding_of(&conn, "Notebooks/a-old.md", &[1.0], "m", Some(&hash)).unwrap();
    vault.write("Notebooks/a-old.md", "one, edited");
    vault.sync(&conn);

    let order: Vec<_> = pending(&conn, "m").into_iter().map(|(_, r)| r).collect();
    assert_eq!(order, vec![EmbedReason::Missing, EmbedReason::Edited]);
    assert_eq!(pending_embeddings(&conn, "m", 1).unwrap().len(), 1);
}

#[test]
fn an_embedding_from_before_the_hash_column_is_stale_only_if_the_note_changed_since() {
    let conn = db();
    let vault = Vault::new();
    vault.write("Notebooks/fresh.md", "written long ago");
    vault.write("Notebooks/edited.md", "edited after embedding");
    let long_ago = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000);
    fs::File::options()
        .write(true)
        .open(vault.notebooks().join("fresh.md"))
        .unwrap()
        .set_modified(long_ago)
        .unwrap();
    vault.sync(&conn);

    // Legacy rows: no hash, embedded at 'now' in milliseconds.
    store_embedding(&conn, "Notebooks/fresh.md", &[1.0], "m").unwrap();
    store_embedding(&conn, "Notebooks/edited.md", &[1.0], "m").unwrap();
    // The second note's file changed a minute after its embedding was made.
    conn.execute(
        "UPDATE sync_state SET modified_at = ?1 WHERE path = 'Notebooks/edited.md'",
        [chrono::Utc::now().timestamp() + 60],
    )
    .unwrap();

    assert_eq!(
        pending(&conn, "m"),
        vec![("Notebooks/edited.md".to_string(), EmbedReason::Edited)]
    );
}

#[test]
fn notes_with_no_sync_state_are_pending_only_when_they_have_no_embedding() {
    let conn = db();
    // Written through a tool: in the keyword index, never seen by the scan.
    index_note(&conn, "Notebooks/tool.md", "tool", "body", "").unwrap();
    assert_eq!(pending(&conn, "m").len(), 1);
    store_embedding(&conn, "Notebooks/tool.md", &[1.0], "m").unwrap();
    assert!(pending(&conn, "m").is_empty());
}

/// Rows left by the old delete path, for notes that exist nowhere: in every table, inside and
/// outside Notebooks/.
fn seed_ghost(conn: &Connection, path: &str) {
    index_note(conn, path, "ghost", "left behind", "").unwrap();
    store_embedding(conn, path, &[1.0, 0.0], "m").unwrap();
    crate::vault_cache::write_note_cached(conn, path, "left behind", "ghost", None, None).unwrap();
    conn.execute(
        "INSERT INTO sync_state (path, content_hash, modified_at, synced_at) VALUES (?1, 'h', 1, 1)",
        [path],
    )
    .unwrap();
    seed_links(conn, path);
}

#[test]
fn semantic_search_never_returns_a_note_that_is_gone_from_the_index() {
    let conn = db();
    index_note(&conn, "Notebooks/live.md", "live", "body", "").unwrap();
    store_embedding(&conn, "Notebooks/live.md", &[1.0, 0.0], "m").unwrap();
    // A stale embedding the old delete path left behind: no keyword-index row.
    store_embedding(&conn, "Notebooks/deleted.md", &[1.0, 0.0], "m").unwrap();

    let found = semantic_search(&conn, &[1.0, 0.0], "m", 10).unwrap();
    let paths: Vec<_> = found.iter().map(|r| r.note_path.as_str()).collect();
    assert_eq!(paths, vec!["Notebooks/live.md"]);

    store_embedding(&conn, "Notebooks/other.md", &[0.9, 0.1], "m").unwrap();
    index_note(&conn, "Notebooks/other.md", "other", "body", "").unwrap();
    let similar = find_similar_notes(&conn, "Notebooks/live.md", 10, 0.1).unwrap();
    let paths: Vec<_> = similar.iter().map(|r| r.note_path.as_str()).collect();
    assert_eq!(
        paths,
        vec!["Notebooks/other.md"],
        "the deleted note is not similar to anything"
    );
}

#[test]
fn which_embeddings_a_capped_search_loads_does_not_depend_on_row_order() {
    let conn = db();
    for (i, name) in ["a", "b", "c"].iter().enumerate() {
        let path = format!("Notebooks/{name}.md");
        index_note(&conn, &path, name, "body", "").unwrap();
        store_embedding(&conn, &path, &[1.0, 0.0], "m").unwrap();
        // Same score for all three, so only the tie-break decides the order.
        conn.execute(
            "UPDATE embeddings SET embedded_at = ?1 WHERE note_path = ?2",
            rusqlite::params![1_000 + i as i64, path],
        )
        .unwrap();
    }
    let order = |conn: &Connection| -> Vec<String> {
        semantic_search(conn, &[1.0, 0.0], "m", 10)
            .unwrap()
            .into_iter()
            .map(|r| r.note_path)
            .collect()
    };
    let first = order(&conn);
    assert_eq!(first, order(&conn));
}

#[test]
fn repair_reports_leftovers_in_every_table_and_removes_them_only_when_asked() {
    let conn = db();
    let vault = Vault::new();
    vault.write("Notebooks/real.md", "a real note");
    vault.write("Skills/real.md", "a real note outside Notebooks");
    vault.sync(&conn);
    index_note(
        &conn,
        "Skills/real.md",
        "real",
        "a real note outside Notebooks",
        "",
    )
    .unwrap();
    seed_ghost(&conn, "Notebooks/ghost.md");
    seed_ghost(&conn, "Skills/ghost.md");
    // A note that exists on disk but has an embedding and no keyword row is not an orphan.
    vault.write("Notebooks/unindexed.md", "present, just not indexed yet");
    store_embedding(&conn, "Notebooks/unindexed.md", &[1.0], "m").unwrap();
    // Something that could never be a note.
    store_embedding(&conn, "../../etc/passwd", &[1.0], "m").unwrap();

    let dry = repair_orphans(&conn, vault.root(), false).unwrap();
    assert_eq!(
        dry.orphan_paths().into_iter().collect::<Vec<_>>(),
        vec!["../../etc/passwd", "Notebooks/ghost.md", "Skills/ghost.md"]
    );
    assert_eq!(dry.removed, 0);
    assert_eq!(
        rows(&conn, "Notebooks/ghost.md"),
        (1, 1, 1, 1),
        "a dry run changes nothing"
    );

    let applied = repair_orphans(&conn, vault.root(), true).unwrap();
    assert_eq!(applied.removed, 3);
    for ghost in ["Notebooks/ghost.md", "Skills/ghost.md"] {
        assert_eq!(rows(&conn, ghost), (0, 0, 0, 0), "{ghost}");
        assert_eq!(link_rows(&conn, ghost), 0, "{ghost}");
    }
    assert_eq!(rows(&conn, "Notebooks/real.md"), (1, 1, 1, 0));
    assert_eq!(
        rows(&conn, "Notebooks/unindexed.md").3,
        1,
        "its embedding stays"
    );
    assert!(get_embedding(&conn, "../../etc/passwd").unwrap().is_none());

    let again = repair_orphans(&conn, vault.root(), true).unwrap();
    assert!(again.orphan_paths().is_empty());
    assert_eq!(again.removed, 0);
}

#[test]
fn a_moved_notes_links_follow_it_in_both_directions() {
    let conn = db();
    add_graph_link(&conn, "Notebooks/old.md", "Notebooks/a.md", 0.9, "applied").unwrap();
    add_graph_link(
        &conn,
        "Notebooks/b.md",
        "Notebooks/old.md",
        0.8,
        "suggested",
    )
    .unwrap();
    add_graph_link(&conn, "Notebooks/c.md", "Notebooks/d.md", 0.7, "applied").unwrap();

    assert_eq!(
        move_graph_links(&conn, "Notebooks/old.md", "Notebooks/new.md").unwrap(),
        2
    );

    let related = |path: &str| get_related_paths(&conn, path, 10).unwrap();
    assert!(related("Notebooks/old.md").is_empty());
    let mut moved = related("Notebooks/new.md");
    moved.sort();
    assert_eq!(moved, vec!["Notebooks/a.md", "Notebooks/b.md"]);
    assert_eq!(related("Notebooks/a.md"), vec!["Notebooks/new.md"]);
    assert_eq!(
        related("Notebooks/c.md"),
        vec!["Notebooks/d.md"],
        "other links are untouched"
    );
}

#[test]
fn moving_a_note_over_one_it_was_linked_to_drops_the_self_link_and_keeps_the_rest() {
    let conn = db();
    add_graph_link(
        &conn,
        "Notebooks/old.md",
        "Notebooks/new.md",
        0.9,
        "applied",
    )
    .unwrap();
    add_graph_link(&conn, "Notebooks/old.md", "Notebooks/x.md", 0.6, "applied").unwrap();
    // The destination already has a link to x; the move replaces it rather than failing.
    add_graph_link(
        &conn,
        "Notebooks/new.md",
        "Notebooks/x.md",
        0.2,
        "suggested",
    )
    .unwrap();

    move_graph_links(&conn, "Notebooks/old.md", "Notebooks/new.md").unwrap();

    assert_eq!(link_rows(&conn, "Notebooks/old.md"), 0);
    assert_eq!(
        get_related_paths(&conn, "Notebooks/new.md", 10).unwrap(),
        vec!["Notebooks/x.md"]
    );
    let self_links: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM graph_links WHERE source_path = target_path",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(self_links, 0);
    assert_eq!(
        move_graph_links(&conn, "Notebooks/same.md", "Notebooks/same.md").unwrap(),
        0
    );
}
