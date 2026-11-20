# Fees a payment message does not state

Some merchant-payment messages carry no fee, yet the balance moved by more than the amount (gaps of 100, 500 and 950 shillings in a real export). The ledger leaves `fee` empty, so "Fees paid" can undercount. Inferring a fee from consecutive balances is a guess, so it is not done.

Fix when: a second message type or a statement import states the fee, or the owner asks for an "unexplained balance change" row.

File: `crates/hq-finance/src/validate.rs`, `ledger.rs`.
