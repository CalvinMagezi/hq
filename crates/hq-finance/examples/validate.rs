//! `cargo run -p hq-finance --example validate -- <raw.json>`: checks the parser against a real
//! export without printing any message text. Output is counts only.
//!
//! The ground truth is the wallet's own running balance. Between two messages that report a balance,
//! the balance moves by what the later message did plus whatever the events in between did, so the
//! gap must be explainable by the parsed amounts and fees. A misread amount, fee or direction
//! shows up as a gap nothing explains.

use hq_finance::parse::{Draft, Kind, dedupe, parse};
use std::collections::{BTreeMap, HashSet};
use std::{env, fs};

/// Some messages drop the cents from a balance, so equal means within a shilling.
const TRUNCATION_CENTS: i64 = 99;
const MAX_GAP_EVENTS: usize = 6;

fn near(set: &HashSet<i64>, b: i64) -> bool {
    (-TRUNCATION_CENTS..=TRUNCATION_CENTS).any(|d| set.contains(&(b + d)))
}

/// Every way an event could have moved the balance: not at all, or in or out by the amount, the
/// amount plus fee, or the amount less fee.
fn contributions(d: &Draft) -> Vec<i64> {
    let fee = d.fee_cents.unwrap_or(0);
    let mut v = vec![0];
    for base in [d.amount_cents, d.amount_cents + fee, d.amount_cents - fee] {
        v.push(base);
        v.push(-base);
    }
    v
}

fn main() {
    let path = env::args().nth(1).expect("raw json path");
    let raw: Vec<serde_json::Value> =
        serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    let mut rows: Vec<(i64, Draft)> = raw
        .iter()
        .filter_map(|m| {
            let d = parse(m["sender"].as_str()?, m["body"].as_str()?)?;
            Some((m["date"].as_i64()?, d))
        })
        .collect();
    rows.sort_by_key(|(t, _)| *t);

    let mut by_kind: BTreeMap<String, usize> = BTreeMap::new();
    for (_, d) in &rows {
        *by_kind.entry(format!("{:?}", d.kind)).or_default() += 1;
    }
    let merged = dedupe(
        rows.iter()
            .map(|(_, d)| d.clone())
            .filter(|d| !matches!(d.kind, Kind::Failed | Kind::WithdrawalRequested))
            .collect(),
    );
    let events: Vec<&Draft> = merged.iter().collect();
    let anchors: Vec<usize> = (0..events.len())
        .filter(|&i| events[i].balance_cents.is_some())
        .collect();
    let (mut ok, mut bad, mut skipped) = (0, 0, 0);
    let mut bad_kinds = BTreeMap::<String, usize>::new();
    for w in anchors.windows(2) {
        let (a, b) = (w[0], w[1]);
        if b - a - 1 > MAX_GAP_EVENTS {
            skipped += 1;
            continue;
        }
        let target = events[b].balance_cents.unwrap() - events[a].balance_cents.unwrap();
        let mut reach: HashSet<i64> = HashSet::from([0]);
        for d in &events[a + 1..=b] {
            let c = contributions(d);
            reach = reach
                .iter()
                .flat_map(|r| c.iter().map(move |x| r + x))
                .collect();
        }
        if near(&reach, target) {
            ok += 1;
        } else {
            bad += 1;
            if env::var_os("VALIDATE_DETAIL").is_some() {
                let parts: Vec<String> = events[a + 1..=b]
                    .iter()
                    .map(|d| {
                        format!(
                            "{:?}:{}+{}",
                            d.kind,
                            d.amount_cents / 100,
                            d.fee_cents.unwrap_or(0) / 100
                        )
                    })
                    .collect();
                println!("gap {} over [{}]", target / 100, parts.join(", "));
            }
            *bad_kinds
                .entry(format!("{:?}", events[b].kind))
                .or_default() += 1;
        }
    }
    println!(
        "parsed {} drafts, {} distinct events",
        rows.len(),
        events.len()
    );
    println!("by kind: {by_kind:?}");
    println!(
        "replay: {ok} gaps explained, {bad} unexplained, {skipped} skipped; unexplained at {bad_kinds:?}"
    );
    let no_id = rows.iter().filter(|(_, d)| d.txn_id.is_none()).count();
    let no_time = rows.iter().filter(|(_, d)| d.occurred_at.is_none()).count();
    println!("missing id {no_id}, missing time {no_time}");
}
