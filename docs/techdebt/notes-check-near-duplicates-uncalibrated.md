# Near-duplicate notes use a threshold nobody has tuned on a real vault

**Reason skipped:** the on-device model puts related notes at roughly 0.4 to 0.6 cosine and the check wants "almost the same", so it asks for 0.97 and then 80 percent of the words shared. Those two numbers are a conservative guess from synthetic notes, not measured on a real vault, and only the first 512 characters of a note are embedded.
**Date:** 2026-11-27
**Where:** `crates/hq-tools/src/vault_health.rs` (`SIMILAR_COSINE`, `SIMILAR_WORDS`, `SIMILAR_MAX_NOTES`)
**Remove when:** the opt-in census (`crates/hq-vault/tests/links_diag.rs`) has been run on a real vault and the two numbers are set from what it shows. Until then the pairs are only listed, never removed.
