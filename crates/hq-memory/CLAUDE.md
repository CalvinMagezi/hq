# The memory system

Notes for working under `crates/hq-memory/`, moved out of the root `CLAUDE.md` so an assistant loads them only when it is in this area. The root file has the rules that apply everywhere.

## Entry points

| What | Path |
|------|------|
| Memory system | `crates/hq-memory/src/lib.rs` |
| Forget: delete a memory and every copy HQ made of it (insight notes, `MEMORY.md` lines, concept page references, archive lines), keep only a fingerprint (`hash.rs`) so the same words are not learned again, and say what it could not reach (`NOT_REACHED`). The owner's list, edit, pin, keep and out-of-date calls are `hq_db::memories_owner`; `hq memory list|search|view|edit|pin|unpin|keep|outdated|forget|export` | `crates/hq-memory/src/forget.rs`, `crates/hq-memory/src/hash.rs`, `crates/hq-db/src/memories_owner.rs` (`sql/158_memory_tombstones.sql`), `crates/hq-cli/src/commands/memory.rs` |
| Recall: which memories to bring up for a message (meaningful words, coverage, ranking, silence, the fenced block). Wired into chats by `crates/hq-agent/src/native_hq/recall.rs` (`daemon.memory_recall`: `shadow` by default, which only writes down what would fit; `query` brings it up; read the log with `hq memory recall-report`). Scored by the recall eval, whose CI gates must keep passing | `crates/hq-memory/src/recall.rs`, `crates/hq-memory/tests/recall_eval.rs` and `tests/fixtures/recall/` (`HQ_EVAL_REPORT=<file>` writes the report, `HQ_EVAL_ALL=1` lists every miss), keyword index in `crates/hq-db/src/memories_search.rs` |
| What gets written down (the ingester): the owner's own words only, "remember that ..." kept for good and pinned, repeats reinforce one note (`seen_count`), sensitive turns and orders to a model are not saved on their own or are held for review, a failed model call falls back to the on-device extractor. Shared lists live in `hq_core::sensitive` (also used by people notes and the skill scanner) | `crates/hq-memory/src/ingester.rs`, `remember.rs`, `db.rs` (`store_or_reinforce`), `crates/hq-core/src/sensitive.rs` |
