# A payment message does not always state its fee

**Reason skipped:** some merchant-payment messages carry no fee, yet the balance moved by more than the amount (gaps of 100, 500 and 950 shillings in a real export). Inferring a fee from consecutive balances is a guess, so the ledger leaves `fee` empty and "Fees paid" can undercount.
**Date:** 2026-11-21
**Where:** `crates/hq-finance/src/validate.rs`, `crates/hq-finance/src/ledger.rs`
**Remove when:** a second message type or a statement import states the fee, or the owner asks for an "unexplained balance change" row.
