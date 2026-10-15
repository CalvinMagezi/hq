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
    let semantic = semantic_search(&conn, &[1.0, 2.0, 3.0], 10).unwrap();
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
