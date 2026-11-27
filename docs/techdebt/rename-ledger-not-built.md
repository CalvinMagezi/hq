# Notes renamed outside HQ leave their links broken until a person fixes them

**Reason skipped:** a move made through HQ now rewrites every link that reached the note (`hq_vault::relink`). A rename made in another editor or by a sync client is seen by the vault watcher as a delete and an add, and recording that as a rename needs a table and a repair that only the health work (P2 and P3 of the vault health plan) provides.
**Date:** 2026-11-26
**Where:** `crates/hq-daemon/src/vault_watch/pipeline.rs`, `crates/hq-db/src/search/reconcile.rs`
**Remove when:** the vault health fix engine lands: a delete and an add of the same content hash in one watcher batch is recorded as a rename, and links that named the old path are the first fixes it offers.
