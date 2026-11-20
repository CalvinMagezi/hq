//! Checks parsed drafts against the one ground truth a message export carries: the wallet's running
//! balance. Between two messages that report a balance, the balance moves by what the later message
//! did plus whatever the events in between did, so the gap must be explainable by the parsed
//! amounts and fees. A misread amount, fee or direction shows up as a gap nothing explains.
//!
//! The report is counts only, so it is safe to read, share and put in an issue.

use crate::parse::{Draft, Kind, dedupe};
use serde::Serialize;
use std::collections::{BTreeMap, HashSet};

/// Some messages drop the cents from a balance, so equal means within a shilling.
const TRUNCATION_CENTS: i64 = 99;
const MAX_GAP_EVENTS: usize = 6;

#[derive(Debug, Default, Serialize, PartialEq, Eq)]
pub struct Report {
    pub drafts: usize,
    pub events: usize,
    pub by_kind: BTreeMap<String, usize>,
    pub gaps_explained: usize,
    pub gaps_unexplained: usize,
    pub gaps_skipped: usize,
    pub unexplained_at: BTreeMap<String, usize>,
    pub missing_id: usize,
    pub missing_time: usize,
}

impl Report {
    /// Share of checked gaps the parsed amounts explain, or None when nothing could be checked.
    pub fn explained_share(&self) -> Option<f64> {
        let total = self.gaps_explained + self.gaps_unexplained;
        (total > 0).then(|| self.gaps_explained as f64 / total as f64)
    }
}

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

/// `drafts` are in the order the messages arrived.
pub fn replay(drafts: &[Draft]) -> Report {
    let mut report = Report {
        drafts: drafts.len(),
        ..Report::default()
    };
    for d in drafts {
        *report.by_kind.entry(format!("{:?}", d.kind)).or_default() += 1;
        report.missing_id += usize::from(d.txn_id.is_none());
        report.missing_time += usize::from(d.occurred_at.is_none());
    }
    let merged = dedupe(
        drafts
            .iter()
            .filter(|d| !matches!(d.kind, Kind::Failed | Kind::WithdrawalRequested))
            .cloned()
            .collect(),
    );
    report.events = merged.len();
    let anchors: Vec<usize> = (0..merged.len())
        .filter(|&i| merged[i].balance_cents.is_some())
        .collect();
    for w in anchors.windows(2) {
        let (a, b) = (w[0], w[1]);
        if b - a - 1 > MAX_GAP_EVENTS {
            report.gaps_skipped += 1;
            continue;
        }
        let target = merged[b].balance_cents.unwrap_or(0) - merged[a].balance_cents.unwrap_or(0);
        let mut reach: HashSet<i64> = HashSet::from([0]);
        for d in &merged[a + 1..=b] {
            let c = contributions(d);
            reach = reach
                .iter()
                .flat_map(|r| c.iter().map(move |x| r + x))
                .collect();
        }
        if near(&reach, target) {
            report.gaps_explained += 1;
        } else {
            report.gaps_unexplained += 1;
            *report
                .unexplained_at
                .entry(format!("{:?}", merged[b].kind))
                .or_default() += 1;
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse;

    fn d(body: &str) -> Draft {
        parse("MTNMobMoney", body).unwrap()
    }

    #[test]
    fn a_consistent_chain_is_explained_and_a_misread_amount_is_not() {
        let a = "You have sent UGX 10,000 to <NAME>, 0700000000 on 2026-03-04 10:00:00, fee: 100. Reason: . New balance: 90,000. ID :1000000001.";
        let b = "You have sent UGX 5,000 to <NAME>, 0700000000 on 2026-03-04 11:00:00, fee: 100. Reason: . New balance: 84,900. ID :1000000002.";
        let wrong = b.replace("84,900", "80,000");
        assert_eq!(replay(&[d(a), d(b)]).gaps_explained, 1);
        assert_eq!(replay(&[d(a), d(&wrong)]).gaps_unexplained, 1);
    }

    #[test]
    fn the_report_holds_no_message_text() {
        let json = serde_json::to_string(&replay(&[])).unwrap();
        assert!(!json.contains("NAME"));
    }
}
