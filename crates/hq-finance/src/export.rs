//! The message export format: a JSON array of `{ "sender", "date", "body" }`, where `date` is the
//! time the phone received the message in milliseconds, as Android stores it. One file the owner
//! chose to hand over; nothing here reads an inbox.

use crate::parse::{Draft, parse_at};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct Message {
    sender: String,
    date: i64,
    body: String,
}

/// Drafts for every message that is a mobile-money transaction, oldest first. Messages from other
/// senders and notices that are not transactions are skipped without being kept.
pub fn drafts_from_export(json: &str) -> Result<Vec<Draft>, serde_json::Error> {
    let mut msgs: Vec<Message> = serde_json::from_str(json)?;
    msgs.sort_by_key(|m| m.date);
    Ok(msgs
        .iter()
        .filter_map(|m| parse_at(&m.sender, &m.body, m.date))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_transactions_in_time_order_and_drops_the_rest() {
        let json = r#"[
          {"sender":"MTNMobMoney","date":2000,"body":"You have received UGX 5,000 from <NAME> on 2026-03-05 09:00:00. fee:0. New balance: UGX 9,000. ID: 1000000002."},
          {"sender":"MTNMobMoney","date":1000,"body":"You have sent UGX 1,000 to <NAME>, 0700000000 on 2026-03-04 09:00:00, fee: 0. New balance: 4,000. ID :1000000001."},
          {"sender":"SOMEBANK","date":1500,"body":"You have paid UGX 5 ID: 1234567"},
          {"sender":"MoMoGlobal","date":1600,"body":"Pay for stuff with MoMo! Dial *677#"}
        ]"#;
        let drafts = drafts_from_export(json).unwrap();
        assert_eq!(drafts.len(), 2);
        assert_eq!(drafts[0].txn_id.as_deref(), Some("1000000001"));
    }

    #[test]
    fn a_malformed_export_is_an_error_not_an_empty_ledger() {
        assert!(drafts_from_export("{}").is_err());
    }
}
