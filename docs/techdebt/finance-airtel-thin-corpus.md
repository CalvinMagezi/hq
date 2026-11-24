# The Airtel Money parser is thinly tested

**Reason skipped:** the redacted corpus holds about ten Airtel Money messages, so its formats are the least covered. A person on Airtel may hit messages the parser drops without saying.
**Date:** 2026-11-21
**Where:** `crates/hq-finance/src/parse.rs`, `crates/hq-finance/tests/fixtures/mobile_money_redacted.json`
**Remove when:** more Airtel exports are available to add as redacted fixtures. `hq finance check` reports how many transaction-like messages did not parse.
