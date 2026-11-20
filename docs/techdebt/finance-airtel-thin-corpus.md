# Airtel Money parser is thinly tested

The redacted corpus holds about ten Airtel Money messages, so its formats are the least covered. A real Airtel user may hit messages the parser drops silently.

Fix when: more Airtel exports are available. `hq finance check` reports how many transaction-like messages did not parse.

File: `crates/hq-finance/src/parse.rs`.
