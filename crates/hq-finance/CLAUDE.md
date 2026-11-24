# CLAUDE.md for `crates/hq-finance/`

Personal finance from mobile-money messages. Read `docs/finance/README.md` first: it lists every
place a copy of the data can go and why each is closed.

## Rules

- **No path to a model, an index or the network.** The manifest depends on `chrono`, `regex`
  and `serde*` only. `tests/boundary.rs` fails if a model, vault, database, agent, web or HTTP
  crate appears. A feature that needs one of them belongs in another crate that takes values
  from this one, never the reverse.
- **Counts, never text.** `validate::Report` and every CLI line print numbers only. Do not add a
  field that carries a message, a name, a number or an id.
- **Real messages never enter the repo.** Fixtures come from `examples/redact.rs`, which keeps
  only words in a fixed vocabulary, scrambles every number with a random salt that is never
  stored, and drops one-time-code messages. Widening `VOCAB` is a privacy change: add generic
  words only, never a name, a merchant or a place.
- **Drafts, not facts.** A parsed message is a `Draft` until the owner confirms the row in Sheets.
  Totals count `confirmed` rows only. Parsing returns `None` for a message that is not a
  transaction; it never guesses an amount.
- **The Suite does the maths.** The ledger is hqmd text (`ledger.rs`). Do not compute totals in
  Rust for display. `crates/suite/hq-suite-adapter/tests/finance_ledger.rs` proves the Suite
  reads what this crate writes; change both together.
- The ledger lives at `_data/finance/ledger.md`. `_data` is a private vault folder: agent read
  tools refuse it and the vault listing skips it.

## Checking against a real export

```bash
HQ_FINANCE_EXPORT=<export.json> cargo test -p hq-finance --test pipeline -- --ignored --nocapture
hq finance check <export.json>
```

Both print counts only. Make a redacted fixture with
`cargo run -p hq-finance --example redact -- <export.json> <out.json>`.
