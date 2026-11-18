# The database, search and vault events

Notes for working under `crates/hq-db/`, moved out of the root `CLAUDE.md` so an assistant loads them only when it is in this area. The root file has the rules that apply everywhere.

## Entry points

| What | Path |
|------|------|
| Search (FTS5 + semantic) | `crates/hq-db/src/search/` |
| Vault change events: watcher, debounce queue, index and embedding freshness (`vault_watch:` config, `hq doctor` Search index, `hq vault repair`) | `crates/hq-daemon/src/vault_watch/` (`watcher.rs`, `queue.rs`, `pipeline.rs`, `status.rs`), `crates/hq-db/src/search/reconcile.rs` (the shared index writes and the full scan), `crates/hq-daemon/src/embeddings.rs` and `embedding_worker.rs`, `crates/hq-core/src/config/vault_watch.rs`, `docs/architecture/vault-events.md` |

## Rules

- **Search freshness has two sources.** The vault watcher (events, seconds) and the scheduled `embeddings` task's full scan (every 30 minutes, the correctness floor). Index writes go through `hq_db::search::reconcile`, never hand-written SQL on `notes_fts`, so a delete clears every derived row. Tests that touch the index use the real migrated schema (`crate::migrations::run`).
