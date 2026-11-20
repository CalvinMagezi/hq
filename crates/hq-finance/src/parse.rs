//! Label-based extraction of one mobile-money message into a draft. Nothing here is confirmed:
//! every result is a draft the owner reviews, with the confidence the parser has in it.

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

pub const PARSER_VERSION: u32 = 1;
const CENTS_PER_UNIT: i64 = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Provider {
    Mtn,
    Airtel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    Sent,
    Paid,
    Received,
    LoanDisbursed,
    LoanRepayment,
    Utility,
    Withdrawal,
    WithdrawalRequested,
    Purchase,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Confidence {
    High,
    Medium,
    Low,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewState {
    Draft,
    Confirmed,
    Rejected,
}

/// Amounts are hundredths of a shilling because fees carry decimals.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Draft {
    pub provider: Provider,
    pub kind: Kind,
    pub amount_cents: i64,
    pub fee_cents: Option<i64>,
    pub balance_cents: Option<i64>,
    pub txn_id: Option<String>,
    pub occurred_at: Option<String>,
    pub confidence: Confidence,
    pub review: ReviewState,
    pub parser_version: u32,
}

impl Draft {
    /// The same transaction can arrive as two messages (payment and receipt), so the id alone is not
    /// enough: the kind keeps a loan disbursement and its fee apart.
    pub fn dedupe_key(&self) -> Option<String> {
        self.txn_id
            .as_ref()
            .map(|id| format!("{:?}|{id}|{:?}", self.provider, self.kind))
    }
}

const NUM: &str = r"(?:UGX\s*)*(\d[\d,]*(?:\.\d+)?)";

static DATETIME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(\d{4}-\d{2}-\d{2})[ T](\d{2}:\d{2}:\d{2})").unwrap());
static TXN_ID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(?:transaction id|txn id|\bid)\s*:?\s*(\d{6,})").unwrap());
static FEE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"(?i)(?:fee|charge)\s*(?:of\s*)?:?\s*{NUM}")).unwrap());
static BALANCE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"(?i)(?:new balance|balance is now|balance)\s*(?:is)?\s*:?\s*{NUM}"
    ))
    .unwrap()
});
static LOAN_REPAY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"(?i){NUM}\s*has been\s*\S*\s*to repay")).unwrap());
static UTILITY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"(?i)amount:\s*{NUM}\s*UGX")).unwrap());
static VERB: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)you have (sent|paid|received)\b").unwrap());
static VERB_AMOUNT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"(?i)you have (?:sent|paid|received)\s+{NUM}")).unwrap());
static PAID_MERCHANT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)you have paid .*?UGX\s+(\d[\d,]*(?:\.\d+)?)").unwrap());
static ADVANCE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"(?i)you have \S+\s+{NUM}\s+with .*from MOMOADVANCE"
    ))
    .unwrap()
});
static WITHDRAWN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"(?i)\bhas \S+\s+{NUM}\s+at a fee of .*from your mobile money account"
    ))
    .unwrap()
});
static WITHDRAW_REQUEST: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"(?i)requested a withdrawal of\s+{NUM}")).unwrap());
static BUNDLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"(?i)^\s*(?:bundle|airtime)\b.*?{NUM}")).unwrap());
static GENERIC_SENT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"(?i)you have \S+\s+{NUM}\s+to\b")).unwrap());
static GENERIC_RECEIVED: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"(?i)you have \S+\s+{NUM}\s+from\b")).unwrap());
static FAILED: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)has failed|payment of .* failed").unwrap());

pub fn provider_for(sender: &str) -> Option<Provider> {
    let s = sender.to_ascii_lowercase();
    if s.contains("airtel") {
        Some(Provider::Airtel)
    } else if s.contains("mtn") || s.contains("momo") {
        Some(Provider::Mtn)
    } else {
        None
    }
}

fn cents(s: &str) -> Option<i64> {
    let clean: String = s.chars().filter(|c| *c != ',').collect();
    let (whole, frac) = clean.split_once('.').unwrap_or((&clean, ""));
    let frac2 = format!("{frac:0<2}");
    Some(whole.parse::<i64>().ok()? * CENTS_PER_UNIT + frac2.get(..2)?.parse::<i64>().ok()?)
}

fn grab(re: &Regex, body: &str) -> Option<i64> {
    cents(re.captures(body)?.get(1)?.as_str())
}

fn classify(body: &str) -> Option<(Kind, i64)> {
    if FAILED.is_match(body) {
        return Some((Kind::Failed, 0));
    }
    if let Some(a) = grab(&LOAN_REPAY, body) {
        return Some((Kind::LoanRepayment, a));
    }
    if let Some(a) = grab(&ADVANCE, body) {
        return Some((Kind::LoanDisbursed, a));
    }
    if let Some(a) = grab(&UTILITY, body) {
        return Some((Kind::Utility, a));
    }
    if let Some(a) = grab(&WITHDRAW_REQUEST, body) {
        return Some((Kind::WithdrawalRequested, a));
    }
    if let Some(a) = grab(&WITHDRAWN, body) {
        return Some((Kind::Withdrawal, a));
    }
    if let Some(a) = grab(&BUNDLE, body) {
        return Some((Kind::Purchase, a));
    }
    let Some(verb) = VERB.captures(body) else {
        if let Some(a) = grab(&GENERIC_RECEIVED, body) {
            return Some((Kind::Received, a));
        }
        return Some((Kind::Sent, grab(&GENERIC_SENT, body)?));
    };
    let kind = match verb[1].to_ascii_lowercase().as_str() {
        "sent" => Kind::Sent,
        "received" => Kind::Received,
        _ => Kind::Paid,
    };
    // "You have paid <merchant> 901368 UGX 25,771": the first number is a merchant code.
    let amount = grab(&VERB_AMOUNT, body).or_else(|| grab(&PAID_MERCHANT, body))?;
    Some((kind, amount))
}

/// None means "not a transaction message" (promotion, notice), which is not an error.
pub fn parse(sender: &str, body: &str) -> Option<Draft> {
    let provider = provider_for(sender)?;
    let (kind, amount_cents) = classify(body)?;
    let txn_id = TXN_ID.captures(body).map(|c| c[1].to_string());
    let occurred_at = DATETIME
        .captures(body)
        .map(|c| format!("{} {}", &c[1], &c[2]));
    let confidence = match (&txn_id, &occurred_at) {
        (Some(_), Some(_)) => Confidence::High,
        (Some(_), None) => Confidence::Medium,
        _ => Confidence::Low,
    };
    Some(Draft {
        provider,
        kind,
        amount_cents,
        fee_cents: grab(&FEE, body),
        balance_cents: grab(&BALANCE, body),
        txn_id,
        occurred_at,
        confidence,
        review: ReviewState::Draft,
        parser_version: PARSER_VERSION,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sent_message_fields() {
        let b = "You have sent UGX 32,000 to <NAME>, 0700000000 on 2026-03-04 10:11:12, fee: 218. Reason: . New balance: 80,455. ID :47152250553.";
        let d = parse("MTNMobMoney", b).unwrap();
        assert_eq!(
            (d.kind, d.amount_cents, d.fee_cents, d.balance_cents),
            (Kind::Sent, 3_200_000, Some(21_800), Some(8_045_500))
        );
        assert_eq!(d.txn_id.as_deref(), Some("47152250553"));
        assert_eq!(d.confidence, Confidence::High);
    }

    #[test]
    fn merchant_code_is_not_the_amount() {
        let b = "You have paid <NAME> 901368 UGX 25,771.  Fee: UGX 0.  New balance: UGX 605.10. ID: 19702351163.";
        let d = parse("MTNMobMoney", b).unwrap();
        assert_eq!(
            (d.amount_cents, d.fee_cents, d.balance_cents),
            (2_577_100, Some(0), Some(60_510))
        );
    }

    #[test]
    fn promotions_are_not_transactions() {
        assert!(parse("MoMoGlobal", "Pay for <NAME> with MoMo! Dial *677#").is_none());
        assert!(parse("DFCUBANK", "You have paid UGX 5 ID: 1234567").is_none());
    }
}
