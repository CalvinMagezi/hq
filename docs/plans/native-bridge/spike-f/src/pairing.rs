//! The pure parts of pairing: the ticket, the one-time code and its window, and the short
//! authentication string. Nothing here touches the network, so all of it is tested with a fake clock.

use anyhow::{Result, bail};
use data_encoding::{BASE32_NOPAD, BASE64URL_NOPAD};
use iroh::EndpointId;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::time::{Duration, Instant};
use subtle::ConstantTimeEq;

pub const TICKET_PREFIX: &str = "hqpair1.";
pub const CODE_BYTES: usize = 8;
/// How long a ticket works. Long enough to walk to the other machine, short enough to be useless
/// to someone who photographed the screen later.
pub const DEFAULT_TTL: Duration = Duration::from_secs(600);
/// Wrong codes allowed in all, across every caller, before the ticket is burned. The code is 64 bits,
/// so fifty guesses succeed with odds near 3e-18; the budget is large so that a stranger who knows
/// only the host's id cannot burn a ticket in five tries.
pub const DEFAULT_ATTEMPTS: u32 = 50;
const SAS_DIGITS: u32 = 6;
const SAS_CONTEXT: &[u8] = b"hq/pair/sas/2";
const COMMIT_CONTEXT: &[u8] = b"hq/pair/commit/2";
pub const NONCE_BYTES: usize = 16;
pub const COMMIT_BYTES: usize = 32;
/// Longest piece of text from the other machine that is shown to a person.
const MAX_SHOWN_CHARS: usize = 120;
/// A ticket this long cannot be a ticket; refuse it before parsing anything.
const MAX_TICKET_CHARS: usize = 2_000;

/// What the machine showing the QR code hands to the one scanning it. It holds no secret key.
/// The code only proves that the scanner saw the screen; the host's id is what it dials.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ticket {
    pub v: u8,
    pub id: String,
    pub code: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub relays: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub addrs: Vec<String>,
}

impl Ticket {
    pub fn new(
        id: EndpointId,
        code: &[u8; CODE_BYTES],
        relays: Vec<String>,
        addrs: Vec<String>,
    ) -> Self {
        Self {
            v: 1,
            id: id.to_string(),
            code: BASE32_NOPAD.encode(code),
            relays,
            addrs,
        }
    }

    pub fn encode(&self) -> String {
        let json = serde_json::to_vec(self).expect("a ticket always serializes");
        format!("{TICKET_PREFIX}{}", BASE64URL_NOPAD.encode(&json))
    }

    pub fn parse(text: &str) -> Result<Self> {
        let text = text.trim();
        if text.len() > MAX_TICKET_CHARS {
            bail!("that is too long to be a pairing ticket");
        }
        let Some(body) = text.strip_prefix(TICKET_PREFIX) else {
            bail!("a pairing ticket starts with {TICKET_PREFIX}");
        };
        let json = BASE64URL_NOPAD
            .decode(body.as_bytes())
            .map_err(|_| anyhow::anyhow!("the ticket is damaged"))?;
        let ticket: Ticket =
            serde_json::from_slice(&json).map_err(|_| anyhow::anyhow!("the ticket is damaged"))?;
        if ticket.v != 1 {
            bail!("this ticket is from a newer version");
        }
        ticket
            .id
            .parse::<EndpointId>()
            .map_err(|_| anyhow::anyhow!("the ticket names no machine"))?;
        ticket.code_bytes()?;
        Ok(ticket)
    }

    pub fn endpoint_id(&self) -> Result<EndpointId> {
        self.id
            .parse()
            .map_err(|_| anyhow::anyhow!("the ticket names no machine"))
    }

    pub fn code_bytes(&self) -> Result<[u8; CODE_BYTES]> {
        let raw = BASE32_NOPAD
            .decode(self.code.as_bytes())
            .map_err(|_| anyhow::anyhow!("the ticket's code is damaged"))?;
        raw.try_into()
            .map_err(|_| anyhow::anyhow!("the ticket's code is damaged"))
    }
}

/// Why an offered code was not taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Accepted,
    WrongCode,
    Expired,
    AlreadyUsed,
    TooManyAttempts,
}

impl Verdict {
    /// What the other side is told. It never says how close a wrong code was.
    pub fn reason(self) -> &'static str {
        match self {
            Verdict::Accepted => "ok",
            Verdict::WrongCode => "that code is not right",
            Verdict::Expired => "that ticket has expired",
            Verdict::AlreadyUsed => "that ticket was already used",
            Verdict::TooManyAttempts => "too many wrong codes: that ticket is closed",
        }
    }
}

/// The one open ticket: a code, a deadline, a budget of wrong guesses shared by everyone who tries,
/// and a single use.
#[derive(Debug)]
pub struct Window {
    code: [u8; CODE_BYTES],
    deadline: Instant,
    attempts_left: u32,
    spent: bool,
}

impl Window {
    pub fn new(code: [u8; CODE_BYTES], now: Instant, ttl: Duration, attempts: u32) -> Self {
        Self {
            code,
            deadline: now + ttl,
            attempts_left: attempts,
            spent: false,
        }
    }

    /// Whether a pairing connection may be accepted at all right now.
    pub fn is_open(&self, now: Instant) -> bool {
        !self.spent && self.attempts_left > 0 && now < self.deadline
    }

    pub fn check(&mut self, now: Instant, offered: &[u8]) -> Verdict {
        if self.spent {
            return Verdict::AlreadyUsed;
        }
        if now >= self.deadline {
            return Verdict::Expired;
        }
        if self.attempts_left == 0 {
            return Verdict::TooManyAttempts;
        }
        if bool::from(self.code.as_slice().ct_eq(offered)) {
            self.spent = true;
            return Verdict::Accepted;
        }
        self.attempts_left -= 1;
        if self.attempts_left == 0 {
            Verdict::TooManyAttempts
        } else {
            Verdict::WrongCode
        }
    }
}

pub type Nonce = [u8; NONCE_BYTES];

pub fn new_nonce() -> Nonce {
    rand::random()
}

/// What the joiner sends before it has seen the host's nonce. Fixing its side first is what stops
/// a machine in the middle from trying joiner nonces until the digits on both screens agree: it
/// cannot learn the host's nonce before it is committed, so every try is a one in a million chance
/// that a person would see fail.
pub fn commit(nonce: &Nonce) -> [u8; COMMIT_BYTES] {
    let mut hash = Sha256::new();
    hash.update(COMMIT_CONTEXT);
    hash.update(nonce);
    hash.finalize().into()
}

/// Whether `offered` is the nonce `commitment` was made from.
pub fn opens(commitment: &[u8], offered: &Nonce) -> bool {
    bool::from(commitment.ct_eq(commit(offered).as_slice()))
}

/// Six digits both screens show. They depend on both machines' ids, the code and both nonces, so a
/// different machine than the one the person is holding gives different digits.
pub fn sas(
    host: &EndpointId,
    joiner: &EndpointId,
    code: &[u8; CODE_BYTES],
    joiner_nonce: &Nonce,
    host_nonce: &Nonce,
) -> String {
    let mut hash = Sha256::new();
    hash.update(SAS_CONTEXT);
    hash.update(host.as_bytes());
    hash.update(joiner.as_bytes());
    hash.update(code);
    hash.update(joiner_nonce);
    hash.update(host_nonce);
    let digest = hash.finalize();
    let number = u32::from_be_bytes([digest[0], digest[1], digest[2], digest[3]]);
    format!(
        "{:0width$}",
        number % 10u32.pow(SAS_DIGITS),
        width = SAS_DIGITS as usize
    )
}

/// Text that came from the other machine, made safe to print: control characters (a terminal
/// escape sequence, say) are replaced and the length is capped.
pub fn printable(text: &str) -> String {
    text.chars()
        .take(MAX_SHOWN_CHARS)
        .map(|c| if c.is_control() { '?' } else { c })
        .collect()
}

pub fn new_code() -> [u8; CODE_BYTES] {
    rand::random()
}

#[cfg(test)]
mod tests {
    use super::*;
    use iroh::SecretKey;

    fn id() -> EndpointId {
        SecretKey::generate().public()
    }

    fn at(base: Instant, secs: u64) -> Instant {
        base + Duration::from_secs(secs)
    }

    #[test]
    fn a_ticket_survives_text_and_holds_no_secret_key() {
        let (host, code) = (id(), new_code());
        let ticket = Ticket::new(
            host,
            &code,
            vec!["https://relay.example.com".into()],
            vec!["192.0.2.1:7000".into()],
        );
        let text = ticket.encode();
        assert!(
            text.starts_with(TICKET_PREFIX) && text.len() < 400,
            "{} chars",
            text.len()
        );
        let back = Ticket::parse(&text).unwrap();
        assert_eq!(back, ticket);
        assert_eq!(back.endpoint_id().unwrap(), host);
        assert_eq!(back.code_bytes().unwrap(), code);
        assert!(
            Ticket::parse(&format!("  {text}\n")).is_ok(),
            "whitespace around it is fine"
        );
    }

    #[test]
    fn what_is_not_a_ticket_is_refused_without_a_panic() {
        let good = Ticket::new(id(), &new_code(), vec![], vec![]).encode();
        let json = |extra: &str| {
            format!(
                "{TICKET_PREFIX}{}",
                BASE64URL_NOPAD.encode(extra.as_bytes())
            )
        };
        for bad in [
            String::new(),
            "hqpair1.".into(),
            "hqpair2.AAAA".into(),
            good.replace(TICKET_PREFIX, "other."),
            format!("{}!", good),
            format!("{good}{}", "A".repeat(MAX_TICKET_CHARS)),
            json("not json"),
            json(r#"{"v":1,"id":"nope","code":"AAAAAAAAAAAAA"}"#),
            json(&format!(
                r#"{{"v":9,"id":"{}","code":"AAAAAAAAAAAAA"}}"#,
                id()
            )),
            json(&format!(r#"{{"v":1,"id":"{}","code":"A"}}"#, id())),
        ] {
            assert!(Ticket::parse(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_right_code_works_once() {
        let t0 = Instant::now();
        let code = new_code();
        let mut window = Window::new(code, t0, DEFAULT_TTL, DEFAULT_ATTEMPTS);
        assert!(window.is_open(t0));
        assert_eq!(window.check(at(t0, 1), &code), Verdict::Accepted);
        assert!(!window.is_open(at(t0, 1)));
        assert_eq!(
            window.check(at(t0, 2), &code),
            Verdict::AlreadyUsed,
            "a replay is refused"
        );
    }

    #[test]
    fn a_ticket_expires() {
        let t0 = Instant::now();
        let code = new_code();
        let mut window = Window::new(code, t0, DEFAULT_TTL, DEFAULT_ATTEMPTS);
        assert!(window.is_open(at(t0, 599)));
        assert!(!window.is_open(at(t0, 600)));
        assert_eq!(window.check(at(t0, 600), &code), Verdict::Expired);
    }

    #[test]
    fn wrong_codes_burn_the_ticket_for_everyone_including_the_right_one() {
        let t0 = Instant::now();
        let code = new_code();
        let mut window = Window::new(code, t0, DEFAULT_TTL, 3);
        assert_eq!(window.check(t0, b"nope0000"), Verdict::WrongCode);
        assert_eq!(window.check(t0, b"nope0001"), Verdict::WrongCode);
        assert_eq!(window.check(t0, b"nope0002"), Verdict::TooManyAttempts);
        assert!(!window.is_open(t0));
        assert_eq!(
            window.check(t0, &code),
            Verdict::TooManyAttempts,
            "too late for the real one"
        );
    }

    #[test]
    fn a_code_of_the_wrong_length_is_just_wrong() {
        let t0 = Instant::now();
        let mut window = Window::new(new_code(), t0, DEFAULT_TTL, DEFAULT_ATTEMPTS);
        for offered in [&b""[..], b"a", &[0u8; CODE_BYTES + 1]] {
            assert_eq!(window.check(t0, offered), Verdict::WrongCode);
        }
    }

    #[test]
    fn the_digits_agree_on_both_sides_and_change_with_any_input() {
        let (host, joiner, other) = (id(), id(), id());
        let (code, nj, nh) = (new_code(), new_nonce(), new_nonce());
        let digits = sas(&host, &joiner, &code, &nj, &nh);
        assert_eq!(digits.len(), 6);
        assert!(digits.chars().all(|c| c.is_ascii_digit()));
        assert_eq!(
            digits,
            sas(&host, &joiner, &code, &nj, &nh),
            "both sides compute the same"
        );
        assert_ne!(digits, sas(&host, &other, &code, &nj, &nh));
        assert_ne!(digits, sas(&other, &joiner, &code, &nj, &nh));
        assert_ne!(digits, sas(&host, &joiner, &new_code(), &nj, &nh));
        assert_ne!(
            digits,
            sas(&host, &joiner, &code, &new_nonce(), &nh),
            "the joiner's nonce counts"
        );
        assert_ne!(
            digits,
            sas(&host, &joiner, &code, &nj, &new_nonce()),
            "the host's nonce counts"
        );
    }

    #[test]
    fn a_commitment_opens_only_to_its_own_nonce() {
        let nonce = new_nonce();
        let commitment = commit(&nonce);
        assert!(opens(&commitment, &nonce));
        assert!(!opens(&commitment, &new_nonce()));
        assert!(
            !opens(&commitment[..COMMIT_BYTES - 1], &nonce),
            "short is wrong"
        );
        assert!(
            !commitment.windows(NONCE_BYTES).any(|w| w == nonce),
            "the commitment does not hold the nonce"
        );
    }

    #[test]
    fn text_from_the_other_machine_cannot_carry_a_terminal_escape() {
        let shown = printable("ok\u{1b}[2J\u{7}\nnext");
        assert!(!shown.chars().any(char::is_control), "{shown:?}");
        assert!(printable(&"x".repeat(10_000)).chars().count() <= MAX_SHOWN_CHARS);
    }
}
