# Transfers between the owner's own wallets are not detected

**Reason skipped:** the parser cannot know that a payment to another of the owner's wallets is not spending. `flow` defaults from the message kind and the owner can set `transfer` by hand; no tile totals it and no check warns that a matching inflow exists on the other wallet.
**Date:** 2026-11-21
**Where:** `crates/hq-finance/src/ledger.rs`
**Remove when:** two providers are imported and the owner sees the same money counted twice. Match an outflow and an inflow of the same amount within minutes and propose `transfer` for review.
