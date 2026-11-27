# Finding a memory by meaning is built but its gain is not measured

**Reason skipped:** the on-device model (`potion-base-8M`, about 30 MB) could not be fetched where this was built, so the arm was tested with a toy embedder that proves it adds no noise and cannot widen who sees what, but cannot show it helps.
**Date:** 2026-11-27
**Where:** `crates/hq-memory/src/recall.rs` (`MIN_MEANING_SIMILARITY`, `add_meaning`, `score_by_meaning`), `crates/hq-memory/tests/recall_eval.rs` (`real_model_scores_meaning_against_keywords`)
**Remove when:** the ignored eval has been run with the real model, the paraphrase slice (20% on keywords alone) and the quiet-turn rate are read, and `MIN_MEANING_SIMILARITY` is set from them.

Run `cargo test -p hq-memory --test recall_eval -- --ignored --nocapture real_model` on a machine that can fetch the model. The 0.55 cut-off and the 0.60 weight are a starting guess taken from how the notes tagger scores related notes, not a measurement. Recall defaults to `shadow`, so a poor setting is seen in `hq memory recall-report` before it reaches a chat.
