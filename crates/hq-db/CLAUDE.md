# The database, search and vault events

Notes for working under `crates/hq-db/`, moved out of the root `CLAUDE.md` so an assistant loads them only when it is in this area. The root file has the rules that apply everywhere.

## Entry points

| What | Path |
|------|------|
| Search (FTS5 + semantic) | `crates/hq-db/src/search/` |
| Memory index: keyword search over what HQ remembers (`memories_fts`, `View::Recall` for what may surface on its own, `View::Owner` for the owner's own look), the `memories` status/origin/provenance columns, and the one-time timestamp fix (`sql/154_memory_index.sql`, `sql/155_memory_timestamps_text.sql`) | `crates/hq-db/src/memories_search.rs`, `crates/hq-db/tests/memory_index_scale.rs` (ignored timing diagnostic) |
| Vault change events: watcher, debounce queue, index and embedding freshness (`vault_watch:` config, `hq doctor` Search index, `hq vault repair`) | `crates/hq-daemon/src/vault_watch/` (`watcher.rs`, `queue.rs`, `pipeline.rs`, `status.rs`), `crates/hq-db/src/search/reconcile.rs` (the shared index writes and the full scan), `crates/hq-daemon/src/embeddings.rs` and `embedding_worker.rs`, `crates/hq-core/src/config/vault_watch.rs`, `docs/architecture/vault-events.md` |
| Stored notes-check scans, the items the owner ignored, repair runs for undo, the last scan's notes and the renames seen | `crates/hq-db/src/vault_health.rs` (`sql/153_vault_health.sql`, `sql/157_vault_repairs.sql`); the scan is `hq_vault::health`, run and stored by `hq_tools::vault_health` |

## Rules

- **Search freshness has two sources.** The vault watcher (events, seconds) and the scheduled `embeddings` task's full scan (every 30 minutes, the correctness floor). Index writes go through `hq_db::search::reconcile`, never hand-written SQL on `notes_fts`, so a delete clears every derived row. Tests that touch the index use the real migrated schema (`crate::migrations::run`).
