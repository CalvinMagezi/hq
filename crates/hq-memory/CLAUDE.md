# The memory system

Notes for working under `crates/hq-memory/`, moved out of the root `CLAUDE.md` so an assistant loads them only when it is in this area. The root file has the rules that apply everywhere.

## Entry points

| What | Path |
|------|------|
| Memory system | `crates/hq-memory/src/lib.rs` |
| Recall: which memories to bring up for a message (meaningful words, coverage, ranking, silence, the fenced block). Wired into chats by `crates/hq-agent/src/native_hq/recall.rs` (`daemon.memory_recall`: `shadow` by default, which only writes down what would fit; `query` brings it up; read the log with `hq memory recall-report`). Scored by the recall eval, whose CI gates must keep passing | `crates/hq-memory/src/recall.rs`, `crates/hq-memory/tests/recall_eval.rs` and `tests/fixtures/recall/` (`HQ_EVAL_REPORT=<file>` writes the report, `HQ_EVAL_ALL=1` lists every miss), keyword index in `crates/hq-db/src/memories_search.rs` |
