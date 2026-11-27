# The notes check shows a large, messy vault in full

**Reason skipped:** the plan for the notes check had a first-scan baseline, so a vault with thousands of old broken links would show only the new ones and the fixable ones. The report-only release has no fixes to offer yet, and a baseline that hides old problems before there is a way to fix them would hide the thing the person came to see.
**Date:** 2026-11-26
**Where:** `crates/hq-vault/src/health.rs` (`Report`), `crates/hq-tools/src/vault_health.rs`, `apps/hq-web/src/components/vault/VaultHealthCard.tsx`
**Remove when:** the fix engine lands: the card then leads with what can be fixed and what is new since the first scan, and the long tail moves behind Review.
