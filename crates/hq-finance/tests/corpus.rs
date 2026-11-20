//! Accuracy gate over redacted real messages. Raw text never enters the repo: the fixture is
//! produced by `examples/redact.rs`, which replaces every non-vocabulary word and scrambles numbers.

use hq_finance::parse::{Confidence, parse};
use serde::Deserialize;

#[derive(Deserialize)]
struct Msg {
    sender: String,
    body: String,
}

const MIN_PARSE_RATE: f64 = 0.90;

fn corpus() -> Vec<Msg> {
    let raw = include_str!("fixtures/mobile_money_redacted.json");
    serde_json::from_str(raw).expect("fixture parses")
}

/// A message that talks about a money movement, whatever the parser made of it.
fn looks_like_transaction(body: &str) -> bool {
    let b = body.to_ascii_lowercase();
    b.contains("ugx")
        && (b.contains("transaction id")
            || b.contains("txn id")
            || b.contains(" id")
            || b.contains("new balance"))
}

#[test]
fn transaction_messages_parse_with_amounts() {
    let msgs = corpus();
    let candidates: Vec<&Msg> = msgs
        .iter()
        .filter(|m| looks_like_transaction(&m.body))
        .collect();
    let (mut parsed, mut high, mut missed) = (0, 0, Vec::new());
    for m in &candidates {
        match parse(&m.sender, &m.body) {
            Some(d) => {
                parsed += 1;
                high += usize::from(d.confidence == Confidence::High);
            }
            None => missed.push(format!("{} | {}", m.sender, m.body.replace('\n', " "))),
        }
    }
    let rate = parsed as f64 / candidates.len() as f64;
    eprintln!(
        "candidates {} parsed {parsed} high-confidence {high} rate {rate:.2}",
        candidates.len()
    );
    for m in missed.iter().take(12) {
        eprintln!("MISSED {m}");
    }
    assert!(
        rate >= MIN_PARSE_RATE,
        "parse rate {rate:.2} below {MIN_PARSE_RATE}"
    );
}

#[test]
fn no_one_time_code_reaches_a_fixture() {
    assert!(corpus().iter().all(|m| !m.body.contains("<#>")));
}
