//! The search index and link graph after a note moves.

use hq_db::Database;
use hq_vault::VaultClient;
use std::sync::Arc;

pub(crate) fn reindex_moved_note(
    db: Option<&Arc<Database>>,
    vault: &VaultClient,
    from: &str,
    to: &str,
) {
    let Some(db) = db else { return };
    let from = from.to_string();
    let to = to.to_string();
    let note = vault.read_note(&to).ok();
    let _ = db.with_conn(move |conn| {
        let _ = hq_db::search::remove_note(conn, &from);
        // Its links belong to the note, not the old path.
        let _ = hq_db::search::move_graph_links(conn, &from, &to);
        if let Some(note) = note {
            let tags = note.tags.join(" ");
            let _ = hq_db::search::index_note(conn, &to, &note.title, &note.content, &tags);
        }
        Ok(())
    });
}
