//! Turns a real mobile-money message into a shareable fixture. Allowlist based: every word that is
//! not in `VOCAB` becomes `<NAME>`, so a name or merchant cannot survive by construction.

use regex::Regex;
use std::collections::HashSet;
use std::sync::LazyLock;

const VOCAB: &str = "you have received sent to from payment of for fee fees balance new your \
transaction id txn reference ref cash withdrawal withdraw deposit agent airtime bundle bought buy \
successfully successful failed failure reversed reversal tax excise duty on at is was has been and \
the a an by with via please dial momo mobile money mtn airtel wallet account transfer paid pay \
confirmed confirm loan repay repayment due limit available current amount date time financial \
charge charges bill token units thank using service advance momoadvance momoadvnce xtracash \
merchant ugx this that will be can in it as if or not no yes dear customer reason status sms code \
payments paying credit debit repaid disbursed approved increased decreased cashin cashout cash-in \
cash-out unit transactions umeme nwsc electricity water tv pin never share select enter request \
now success https example com x borrowed taken credited debited cleared requested processed \
interest processing been careful sure only valid expires expired remaining";

static VOCAB_SET: LazyLock<HashSet<&'static str>> =
    LazyLock::new(|| VOCAB.split_whitespace().collect());
static URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"https?://\S+|\b[\w\-]+\.(?:com|co\.ug|ug|net)\S*").unwrap());
// One pass, so a fake timestamp is never scrambled again as a number.
static DIGITS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?P<dt>\d{4}-\d{2}-\d{2}[ T]\d{2}:\d{2}:\d{2})|\d[\d,]*(?:\.\d+)?").unwrap()
});
static WORD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[A-Za-z][A-Za-z'\-]*").unwrap());
static NAME_RUN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<NAME>(?:\s*<NAME>)+").unwrap());

pub const NAME_TOKEN: &str = "<NAME>";
const OTP_MARKER: &str = "<#>";
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0100_0000_01b3;
const PLACEHOLDER_URL: &str = "https://example.com/x";

/// Returns None when the message must never leave the phone, such as a one-time-code message.
pub fn redact(body: &str, salt: &str) -> Option<String> {
    if body.contains(OTP_MARKER) {
        return None;
    }
    let no_url = URL.replace_all(body, PLACEHOLDER_URL);
    let words = WORD.replace_all(&no_url, |c: &regex::Captures| {
        if VOCAB_SET.contains(c[0].to_ascii_lowercase().as_str()) {
            c[0].to_string()
        } else {
            NAME_TOKEN.to_string()
        }
    });
    let collapsed = NAME_RUN.replace_all(&words, NAME_TOKEN);
    Some(
        DIGITS
            .replace_all(&collapsed, |c: &regex::Captures| {
                if c.name("dt").is_some() {
                    fake_datetime(&c[0], salt)
                } else {
                    scramble(&c[0], salt)
                }
            })
            .into_owned(),
    )
}

const KEEP_MAX_DIGITS: usize = 2;
const FAKE_YEAR: u64 = 2026;

/// A valid date and time derived from the original, so parsers still see a real timestamp shape.
fn fake_datetime(dt: &str, salt: &str) -> String {
    let h = hash(salt, dt);
    let sep = if dt.contains('T') { 'T' } else { ' ' };
    format!(
        "{FAKE_YEAR}-{:02}-{:02}{sep}{:02}:{:02}:{:02}",
        h % 12 + 1,
        (h >> 8) % 28 + 1,
        (h >> 16) % 24,
        (h >> 24) % 60,
        (h >> 32) % 60
    )
}

fn hash(salt: &str, s: &str) -> u64 {
    let mut h = FNV_OFFSET;
    for b in salt.bytes().chain(s.bytes()) {
        h = (h ^ u64::from(b)).wrapping_mul(FNV_PRIME);
    }
    h
}

/// Replaces every digit of a number longer than two digits (so zero fees survive) with a hash-derived one, keeping length, commas and decimal points. The same
/// input maps to the same output within a salt, so repeated ids still match as duplicates.
fn scramble(num: &str, salt: &str) -> String {
    if num.len() <= KEEP_MAX_DIGITS {
        return num.to_string();
    }
    let mut h = hash(salt, num);
    let mut first = true;
    num.chars()
        .map(|ch| {
            if !ch.is_ascii_digit() {
                return ch;
            }
            h = h.wrapping_mul(FNV_PRIME).rotate_left(13) ^ FNV_OFFSET;
            let d = if first {
                (h % 9) as u8 + 1
            } else {
                (h % 10) as u8
            };
            first = false;
            (b'0' + d) as char
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_otps_do_not_survive() {
        assert!(redact("<#> code 123456", "s").is_none());
        let out = redact(
            "You have sent UGX 50,000 to JANE DOE, 0771234567 on 2026-01-01",
            "s",
        )
        .unwrap();
        assert!(!out.contains("JANE") && !out.contains("0771234567") && !out.contains("50,000"));
        assert!(out.contains("<NAME>") && out.contains("UGX"));
    }

    #[test]
    fn same_number_maps_the_same() {
        assert_eq!(scramble("123456789", "s"), scramble("123456789", "s"));
        assert_eq!(scramble("1,234", "s").len(), 5);
    }
}
