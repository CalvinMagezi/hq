# The memory system

Notes for working under `crates/hq-memory/`, moved out of the root `CLAUDE.md` so an assistant loads them only when it is in this area. The root file has the rules that apply everywhere.

## Entry points

| What | Path |
|------|------|
| Memory system | `crates/hq-memory/src/lib.rs` |
| Recall: which memories to bring up for a message (meaningful words, coverage, ranking, silence, the fenced block). Not wired into a chat yet. Scored by the recall eval, whose CI gates must keep passing | `crates/hq-memory/src/recall.rs`, `crates/hq-memory/tests/recall_eval.rs` and `tests/fixtures/recall/` (`HQ_EVAL_REPORT=<file>` writes the report, `HQ_EVAL_ALL=1` lists every miss), keyword index in `crates/hq-db/src/memories_search.rs` |
| Finding a memory by meaning: one on-device vector per memory (`memory_embeddings`, same model label rule as notes), made a few per cycle by the `embeddings` daemon task only when `embeds_locally()` and the model is already on the machine (`local_embed::is_ready`, never fetches), and used by `recall_with_meaning` as a second arm with the same status and origin rules. The gain is not yet measured: `docs/techdebt/memory-meaning-not-measured.md` | `crates/hq-memory/src/memory_vectors.rs`, `crates/hq-db/src/memory_embeddings.rs` (`sql/156_memory_embeddings.sql`), `crates/hq-memory/src/recall.rs`, `crates/hq-cli/src/commands/start/daemon/tasks_periodic/embeddings.rs` |
