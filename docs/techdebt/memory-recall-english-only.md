# Memory search is tuned for English only

**Reason skipped:** the owner chose English for the first version of query-aware recall. The memory index uses the same tokenizer as chat search (`porter unicode61 remove_diacritics 2`), whose stemmer is English. Luganda and Swahili words are matched whole, so a different prefix form of the same word is a miss, and the local embedder (`potion-base-8M`) has not been measured on either language.
**Date:** 2026-11-27
**Where:** `crates/hq-db/sql/153_memory_index.sql`, `crates/hq-db/src/memories_search.rs`, `crates/hq-memory/tests/fixtures/recall/`
**Remove when:** the recall eval has Luganda, Swahili and mixed-language cases reviewed by a native speaker, and the scores show whether a different tokenizer (for example trigram) or a multilingual embedding model is needed.

What already helps: the entities column is weighted above the summary, and names and places are the same in any language. The eval fixtures carry a `lang` field on every row, so adding a language is adding data, not code. Nothing here blocks other languages; it just has not been measured, so it is not claimed.
