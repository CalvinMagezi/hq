# The notes check still shows a large, messy vault in full

**Reason skipped:** the plan had a first-scan baseline, so a vault with thousands of old broken links would show only what is new and what can be fixed. The fix engine now exists and the card leads with "Fix N" for the certain ones, the lists are capped at 50 per kind and the counts are exact, so a messy vault is not a wall of rows. But the card's headline is still the total, and a person with 4,000 old broken links they do not mean to fix sees that number for good until they ignore each item. Ignoring one at a time does not scale to that.
**Date:** 2026-11-27
**Where:** `crates/hq-vault/src/health.rs` (`Report`), `crates/hq-tools/src/vault_health.rs`, `apps/hq-web/src/components/vault/VaultHealthCard.tsx`, `apps/android/app/src/main/kotlin/example/hq/spike/ui/notes/VaultHealthCard.kt`
**Remove when:** an owner can say "leave everything that is here now alone" in one tap (a stored baseline of the first scan, with the card counting only what is new since), or ignoring a whole kind or a whole note is possible.
