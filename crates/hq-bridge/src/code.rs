//! The pure parts of pairing: the one-time code and its window, the commitment to a nonce, and the
//! six digits both people compare. Nothing here touches the network, so all of it is tested with a
//! fake clock.

use iroh::EndpointId;
use sha2::{Digest, Sha256};
use std::time::{Duration, Instant};
use subtle::ConstantTimeEq;

pub const CODE_BYTES: usize = 8;
pub const NONCE_BYTES: usize = 16;
pub const COMMIT_BYTES: usize = 32;
pub const BIND_BYTES: usize = 32;
pub const BIND_KEY_BYTES: usize = 16;
/// The longest a ticket can be made to live, whatever the caller asks for.
pub const MAX_TTL: Duration = Duration::from_secs(24 * 60 * 60);
/// How long a ticket works. Long enough to walk to the other machine, short enough to be useless
/// to someone who photographed the screen later.
pub const DEFAULT_TTL: Duration = Duration::from_secs(600);
/// Wrong codes allowed in all, across every source, before the ticket is burned. The per-source
/// limits (`limits.rs`) stop one machine long before this; it is only the backstop for an attacker
/// with many addresses, so it is large. The code is 64 bits: a thousand guesses succeed with odds
/// near 5e-17.
pub const DEFAULT_GLOBAL_ATTEMPTS: u32 = 1_000;
const SAS_DIGITS: u32 = 6;
const SAS_CONTEXT: &[u8] = b"hq/pair/sas/3";
const COMMIT_CONTEXT: &[u8] = b"hq/pair/commit/3";
const BIND_CONTEXT: &[u8] = b"hq/pair/bind/3";

pub type Code = [u8; CODE_BYTES];
pub type Nonce = [u8; NONCE_BYTES];
pub type Binding = [u8; BIND_BYTES];
pub type BindKey = [u8; BIND_KEY_BYTES];

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

/// The one open ticket: a code, a deadline, a backstop budget of wrong guesses shared by every
/// source, and a single use.
#[derive(Debug)]
pub struct Window {
    code: Code,
    deadline: Instant,
    attempts_left: u32,
    spent: bool,
}

impl Window {
    pub fn new(code: Code, now: Instant, ttl: Duration, attempts: u32) -> Self {
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

    pub fn deadline(&self) -> Instant {
        self.deadline
    }

    /// Closes the ticket without using it.
    pub fn close(&mut self) {
        self.spent = true;
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

pub fn new_code() -> Code {
    rand::random()
}

pub fn new_nonce() -> Nonce {
    rand::random()
}

fn framed(hash: &mut Sha256, bytes: &[u8]) {
    hash.update((bytes.len() as u16).to_be_bytes());
    hash.update(bytes);
}

/// What the joiner sends before it has seen the host's nonce. Fixing its side first is what stops
/// a machine in the middle from trying joiner nonces until the digits on both screens agree: it
/// cannot learn the host's nonce before it is committed, so every try is a one in a million chance
/// that a person would see fail. The protocol name is part of the hash, so a commitment made for
/// one purpose or version opens to nothing in another.
pub fn commit(alpn: &[u8], nonce: &Nonce) -> [u8; COMMIT_BYTES] {
    let mut hash = Sha256::new();
    hash.update(COMMIT_CONTEXT);
    framed(&mut hash, alpn);
    hash.update(nonce);
    hash.finalize().into()
}

/// Whether `offered` is the nonce `commitment` was made from.
pub fn opens(alpn: &[u8], commitment: &[u8], offered: &Nonce) -> bool {
    bool::from(commitment.ct_eq(commit(alpn, offered).as_slice()))
}

/// The host's promise of its long-term id, sent before the person confirms and without the id in
/// it: a hash with the host's fresh nonce and a fresh secret key. The id and the key arrive only
/// after the digits matched, and the joiner checks them against this, so the digits the person
/// compared cover the identity that gets stored. Without the secret key, nobody who has only the
/// binding and the public nonce can test a guessed id against it, so a ticket holder who only
/// burns a ticket learns nothing about the host's identity, not even a yes or no.
pub fn bind_host(host: &EndpointId, host_nonce: &Nonce, key: &BindKey) -> Binding {
    let mut hash = Sha256::new();
    hash.update(BIND_CONTEXT);
    hash.update(host.as_bytes());
    hash.update(host_nonce);
    hash.update(key);
    hash.finalize().into()
}

pub fn new_bind_key() -> BindKey {
    rand::random()
}

/// Everything both screens' digits depend on.
pub struct SasInputs<'a> {
    pub alpn: &'a [u8],
    /// The throwaway endpoint the ticket named.
    pub pairing_endpoint: &'a EndpointId,
    pub host_binding: &'a Binding,
    pub joiner: &'a EndpointId,
    pub code: &'a Code,
    pub joiner_nonce: &'a Nonce,
    pub host_nonce: &'a Nonce,
}

/// Six digits both screens show. A different machine than the one the person is holding, a
/// different protocol or a different ticket gives different digits.
pub fn sas(i: &SasInputs) -> String {
    let mut hash = Sha256::new();
    hash.update(SAS_CONTEXT);
    framed(&mut hash, i.alpn);
    hash.update(i.pairing_endpoint.as_bytes());
    hash.update(i.host_binding);
    hash.update(i.joiner.as_bytes());
    hash.update(i.code);
    hash.update(i.joiner_nonce);
    hash.update(i.host_nonce);
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
pub fn printable(text: &str, max_chars: usize) -> String {
    text.chars()
        .take(max_chars)
        .map(|c| if hides_or_reorders(c) { '?' } else { c })
        .collect()
}

/// Control characters, and the invisible ones that can reorder or hide text on a screen: the
/// bidirectional overrides and isolates, zero-width characters, line and paragraph separators.
fn hides_or_reorders(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{00AD}'
                | '\u{034F}'
                | '\u{061C}'
                | '\u{115F}'..='\u{1160}'
                | '\u{17B4}'..='\u{17B5}'
                | '\u{180B}'..='\u{180F}'
                | '\u{200B}'..='\u{200F}'
                | '\u{2028}'..='\u{202E}'
                | '\u{2060}'..='\u{206F}'
                | '\u{3164}'
                | '\u{FE00}'..='\u{FE0F}'
                | '\u{FEFF}'
                | '\u{FFA0}'
                | '\u{FFF9}'..='\u{FFFB}'
                | '\u{E0000}'..='\u{E007F}'
        )
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
    fn the_right_code_works_once() {
        let t0 = Instant::now();
        let code = new_code();
        let mut window = Window::new(code, t0, DEFAULT_TTL, DEFAULT_GLOBAL_ATTEMPTS);
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
        let mut window = Window::new(code, t0, DEFAULT_TTL, DEFAULT_GLOBAL_ATTEMPTS);
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
        let mut window = Window::new(new_code(), t0, DEFAULT_TTL, DEFAULT_GLOBAL_ATTEMPTS);
        for offered in [&b""[..], b"a", &[0u8; CODE_BYTES + 1]] {
            assert_eq!(window.check(t0, offered), Verdict::WrongCode);
        }
    }

    #[test]
    fn the_digits_agree_on_both_sides_and_change_with_any_input() {
        let (pairing_endpoint, joiner, other) = (id(), id(), id());
        let (code, nj, nh) = (new_code(), new_nonce(), new_nonce());
        let key = new_bind_key();
        let binding = bind_host(&id(), &nh, &key);
        let alpn = b"hq/pair/3".as_slice();
        let base = || SasInputs {
            alpn,
            pairing_endpoint: &pairing_endpoint,
            host_binding: &binding,
            joiner: &joiner,
            code: &code,
            joiner_nonce: &nj,
            host_nonce: &nh,
        };
        let digits = sas(&base());
        assert_eq!(digits.len(), 6);
        assert!(digits.chars().all(|c| c.is_ascii_digit()));
        assert_eq!(digits, sas(&base()), "both sides compute the same");

        assert_ne!(
            digits,
            sas(&SasInputs {
                joiner: &other,
                ..base()
            })
        );
        assert_ne!(
            digits,
            sas(&SasInputs {
                pairing_endpoint: &other,
                ..base()
            })
        );
        assert_ne!(
            digits,
            sas(&SasInputs {
                host_binding: &bind_host(&other, &nh, &key),
                ..base()
            }),
            "the host's long-term id counts"
        );
        assert_ne!(
            digits,
            sas(&SasInputs {
                code: &new_code(),
                ..base()
            })
        );
        assert_ne!(
            digits,
            sas(&SasInputs {
                joiner_nonce: &new_nonce(),
                ..base()
            }),
            "the joiner's nonce counts"
        );
        assert_ne!(
            digits,
            sas(&SasInputs {
                host_nonce: &new_nonce(),
                ..base()
            }),
            "the host's nonce counts"
        );
        assert_ne!(
            digits,
            sas(&SasInputs {
                alpn: b"hq/register/3",
                ..base()
            }),
            "a registration never shows the digits of a pairing"
        );
    }

    #[test]
    fn a_commitment_opens_only_to_its_own_nonce_and_protocol() {
        let alpn = b"hq/pair/3".as_slice();
        let nonce = new_nonce();
        let commitment = commit(alpn, &nonce);
        assert!(opens(alpn, &commitment, &nonce));
        assert!(!opens(alpn, &commitment, &new_nonce()));
        assert!(
            !opens(b"hq/register/3", &commitment, &nonce),
            "another protocol's commitment is not this one's"
        );
        assert!(
            !opens(alpn, &commitment[..COMMIT_BYTES - 1], &nonce),
            "short is wrong"
        );
        assert!(
            !commitment.windows(NONCE_BYTES).any(|w| w == nonce),
            "the commitment does not hold the nonce"
        );
    }

    #[test]
    fn the_host_binding_hides_the_id_and_depends_on_the_nonce_and_the_key() {
        let host = id();
        let (nonce, key) = (new_nonce(), new_bind_key());
        let binding = bind_host(&host, &nonce, &key);
        assert_eq!(binding, bind_host(&host, &nonce, &key));
        assert_ne!(binding, bind_host(&host, &new_nonce(), &key));
        assert_ne!(binding, bind_host(&id(), &nonce, &key));
        assert_ne!(
            binding,
            bind_host(&host, &nonce, &new_bind_key()),
            "without the key a guessed id cannot be tested against the binding"
        );
        assert!(
            !binding
                .windows(host.as_bytes().len())
                .any(|w| w == host.as_bytes()),
            "the id is not in it"
        );
    }

    #[test]
    fn text_from_the_other_machine_cannot_carry_a_terminal_escape() {
        let shown = printable("ok\u{1b}[2J\u{7}\nnext", 120);
        assert!(!shown.chars().any(char::is_control), "{shown:?}");
        let bidi = printable("a\u{202E}b\u{200B}c\u{2066}d\u{FEFF}e\u{2028}f", 120);
        assert_eq!(
            bidi, "a?b?c?d?e?f",
            "text that reorders or hides is replaced"
        );
        let hidden = printable("a\u{061C}b\u{E0041}c\u{3164}d\u{FFF9}e\u{180E}f", 120);
        assert_eq!(
            hidden, "a?b?c?d?e?f",
            "tag characters and blank fillers too"
        );
        assert_eq!(
            printable("Pixel 8 (Anna\u{2019}s)", 120),
            "Pixel 8 (Anna\u{2019}s)"
        );
        assert!(printable(&"x".repeat(10_000), 120).chars().count() <= 120);
    }
}
