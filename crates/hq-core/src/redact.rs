//! Backstop redaction for secrets before text leaves the process or lands in a store.

use regex::Regex;
use std::sync::LazyLock;

/// Backstop redaction for common secret shapes, applied to anything pulled
/// from a transcript before it's written to `_pending/`. FR-008: this module
/// stores raw transcript slices, and a live Discord bot token was captured
/// verbatim (mode 0644) via this exact path. Not a substitute for not
/// copying transcripts at all — a pattern-based backstop, not a guarantee.
/// ponytail: known shapes only (Discord/Slack-style bot tokens, `sk-`/`ghp_`/
/// `AKIA`-prefixed keys, Bearer/Authorization headers, PEM blocks); a novel
/// secret shape slips through. Upgrade to entropy-based detection if that
/// proves insufficient in practice.
/// Discord-bot-token shape: base64.base64.base64 with a short middle segment
/// (this is the pattern that leaked in FR-008). Captures the outer two
/// segments so `redact_secrets` can additionally require each to contain a
/// digit — without that, this matches any three dot-separated runs of
/// word-ish characters with a 6-char middle run, which is common in dotted
/// module paths and hyphenated identifiers (`render`, `signal`, `common`,
/// `parser`, `config` are all 6 letters). Requiring a digit in each outer
/// run is expressible without lookahead (unsupported by this crate's
/// non-backtracking engine) by checking the captured groups in Rust instead
/// of in the pattern.
static BOT_TOKEN_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"([A-Za-z0-9_-]{20,})\.([A-Za-z0-9_-]{6})\.([A-Za-z0-9_-]{20,})")
        .expect("static secret pattern must compile")
});

static SECRET_PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
        // OpenRouter (sk-or-v1-...) and Anthropic (sk-ant-api03-...) keys carry dashes.
        r"\bsk-[A-Za-z0-9][A-Za-z0-9_-]{15,}",
        r"ghp_[A-Za-z0-9]{20,}",
        r"gh[oprsu]_[A-Za-z0-9]{20,}",
        r"AKIA[0-9A-Z]{16}",
        r"(?i)(?:bearer|authorization:\s*bearer)\s+[A-Za-z0-9._~+/-]{16,}=*",
        r"(?s)-----BEGIN [A-Z ]*PRIVATE KEY-----.*?-----END [A-Z ]*PRIVATE KEY-----",
    ]
    .iter()
    .map(|p| Regex::new(p).expect("static secret pattern must compile"))
    .collect()
});

/// `NAME_API_KEY=value` or `name_token: value`: the name stays so the line is
/// still readable, the value goes. The separator must follow the name directly,
/// so counters such as `max_tokens: 4096` are left alone.
static SECRET_ASSIGNMENT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?i)\b([A-Z0-9_]*(?:API_?KEY|TOKEN|SECRET|PASSWORD))(\s*[:=]\s*)(["']?)([^\s"']{8,})"#,
    )
    .expect("static assignment pattern must compile")
});

/// Replace anything matching a known secret shape with a placeholder. Best
/// effort — see [`SECRET_PATTERNS`]'s and [`BOT_TOKEN_PATTERN`]'s doc
/// comments for what this does and does not catch.
pub fn redact_secrets(text: &str) -> String {
    let mut out = BOT_TOKEN_PATTERN
        .replace_all(text, |caps: &regex::Captures| {
            // Digit + uppercase in both outer segments: real base64url
            // tokens mix case and digits; a lowercase-hex UUID (which does
            // contain digits) or a lowercase dotted identifier does not.
            let looks_token_like = |s: &str| {
                s.bytes().any(|b| b.is_ascii_digit()) && s.bytes().any(|b| b.is_ascii_uppercase())
            };
            if looks_token_like(&caps[1]) && looks_token_like(&caps[3]) {
                "[REDACTED]".to_string()
            } else {
                caps[0].to_string()
            }
        })
        .into_owned();
    for pattern in SECRET_PATTERNS.iter() {
        out = pattern.replace_all(&out, "[REDACTED]").into_owned();
    }
    SECRET_ASSIGNMENT
        .replace_all(&out, "$1$2$3[REDACTED]")
        .into_owned()
}

/// Whether `text` contains something shaped like a real credential: a key with a
/// known prefix, a private key block, a bot token, or an assignment whose value
/// is long and mixes letters with digits. Narrower than [`redact_secrets`], which
/// also rewrites `token: expired`-style prose; this one is for deciding whether
/// to refuse sending text out, where a false positive blocks legitimate work.
pub fn looks_like_credential(text: &str) -> bool {
    const CREDENTIAL_MIN_LEN: usize = 16;
    let credential_like = |value: &str| {
        value.len() >= CREDENTIAL_MIN_LEN
            && value.bytes().any(|b| b.is_ascii_digit())
            && value.bytes().any(|b| b.is_ascii_alphabetic())
    };
    let prefixed = SECRET_PATTERNS
        .iter()
        .enumerate()
        // Index 4 is the Bearer pattern, which also matches ordinary words after "bearer".
        .filter(|(i, _)| *i != 4)
        .any(|(_, p)| p.is_match(text));
    if prefixed {
        return true;
    }
    let bot_token = BOT_TOKEN_PATTERN.captures_iter(text).any(|c| {
        let tokenish = |s: &str| {
            s.bytes().any(|b| b.is_ascii_digit()) && s.bytes().any(|b| b.is_ascii_uppercase())
        };
        tokenish(&c[1]) && tokenish(&c[3])
    });
    bot_token
        || SECRET_ASSIGNMENT
            .captures_iter(text)
            .any(|c| credential_like(&c[4]))
        || BEARER_VALUE
            .captures_iter(text)
            .any(|c| credential_like(&c[1]))
}

/// The value after `Bearer`, for [`looks_like_credential`].
static BEARER_VALUE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\bbearer\s+([A-Za-z0-9][A-Za-z0-9._~+/-]{19,}=*)").expect("static pattern must compile")
});

const PLACEHOLDER: &str = "[REDACTED]";
const SECRET_KEY_SUFFIXES: [&str; 5] = ["api_key", "token", "secret", "password", "passphrase"];

/// What is safe to log about a shell command: the program, the number of
/// arguments, the length and the number of leading `NAME=value` assignments.
/// No argument text is logged at all, because credentials appear in every
/// shape (`curl -u user:pw`, `mysql -pSECRET`, `-H 'X-Api-Key: ..'`, URL userinfo).
pub fn loggable_command(command: &str) -> String {
    let mut rest = command.trim_start();
    let mut assignments = 0;
    while let Some((word, tail)) = rest.split_once(char::is_whitespace).or(Some((rest, ""))) {
        if !is_env_assignment(word) {
            break;
        }
        assignments += 1;
        rest = tail.trim_start();
    }
    let mut words = rest.split_whitespace();
    let program = redact_secrets(words.next().unwrap_or(""));
    let args = words.count();
    format!(
        "{program} (len {}, {args} arg(s), {assignments} env assignment(s))",
        command.len()
    )
}

fn is_env_assignment(word: &str) -> bool {
    word.split_once('=').is_some_and(|(name, _)| {
        !name.is_empty()
            && !name.starts_with(|c: char| c.is_ascii_digit())
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    })
}

/// Whether a config key names a secret value. `*_env` and `*_ref` keys hold the
/// name of a variable or secret, not the secret itself.
fn is_secret_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    !(key.ends_with("_env") || key.ends_with("_ref"))
        && SECRET_KEY_SUFFIXES.iter().any(|s| key.ends_with(s))
}

fn redact_value(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, v) in map.iter_mut() {
                if is_secret_key(key) && v.is_string() {
                    *v = serde_json::Value::String(PLACEHOLDER.into());
                } else {
                    redact_value(v);
                }
            }
        }
        serde_json::Value::Array(items) => items.iter_mut().for_each(redact_value),
        _ => {}
    }
}

/// `Debug` body for a config type that holds credentials: its serialized form
/// with every secret-named string replaced. Use as `fmt_redacted(self, "Name", f)`
/// so a stray `{:?}` or `?config` in a log call cannot leak a key.
pub fn fmt_redacted<T: serde::Serialize>(
    value: &T,
    name: &str,
    f: &mut std::fmt::Formatter<'_>,
) -> std::fmt::Result {
    match serde_json::to_value(value) {
        Ok(mut v) => {
            redact_value(&mut v);
            write!(f, "{name} {v}")
        }
        Err(_) => write!(f, "{name} {{ <unprintable> }}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_known_secret_shapes() {
        assert_eq!(
            redact_secrets("token: MTIzNDU2Nzg5MDEyMzQ1Njc4.AAAAAA.FakeToken0ForRedaction1Test2FakeToken3X done"),
            "token: [REDACTED] done"
        );
        assert_eq!(
            redact_secrets("key sk-abcdefghijklmnopqrstuvwx here"),
            "key [REDACTED] here"
        );
        assert_eq!(redact_secrets("plain sentence, nothing secret here"), "plain sentence, nothing secret here");
    }

    #[test]
    fn redacts_dashed_provider_keys() {
        let openrouter = format!("key {} end", ["sk", "or", "v1", &"a1b2".repeat(16)].join("-"));
        assert_eq!(redact_secrets(&openrouter), "key [REDACTED] end");
        let anthropic = format!("key {} end", ["sk", "ant", "api03", &"Zx9_".repeat(20)].join("-"));
        assert_eq!(redact_secrets(&anthropic), "key [REDACTED] end");
        assert_eq!(redact_secrets("a desk-organizer-for-small-offices"), "a desk-organizer-for-small-offices");
    }

    #[test]
    fn redacts_values_of_secret_assignments() {
        assert_eq!(
            redact_secrets("OPENROUTER_API_KEY=abcd1234efgh5678"), // gitleaks:allow
            "OPENROUTER_API_KEY=[REDACTED]"
        );
        assert_eq!(
            redact_secrets("  telegram_token: \"123456:ABCdefGHIjkl\""),
            "  telegram_token: \"[REDACTED]\""
        );
        assert_eq!(redact_secrets("GITHUB_TOKEN = ghx0123456789"), "GITHUB_TOKEN = [REDACTED]"); // gitleaks:allow
        let config = "openrouter_api_key: sk-or-v1-0123456789abcdef0123456789abcdef0123456789abcdef\nGITHUB_TOKEN=\"ghx_notatypicalshape99\"\nmodel: sonnet"; // gitleaks:allow
        assert_eq!(
            redact_secrets(config),
            "openrouter_api_key: [REDACTED]\nGITHUB_TOKEN=\"[REDACTED]\"\nmodel: sonnet"
        );
        assert_eq!(
            redact_secrets("OPENROUTER_API_KEY=sk-or-v1-0123456789abcdef0123456789abcdef0123456789abcdef hq doctor"), // gitleaks:allow
            "OPENROUTER_API_KEY=[REDACTED] hq doctor"
        );
        assert_eq!(
            redact_secrets("key is sk-ant-api03-AbCdEf0123456789_AbCdEf0123456789-xyz"),
            "key is [REDACTED]"
        );
        for untouched in ["max_tokens: 100", "max_tokens: 4096", "input_token: 1234", "password_hint: ask the owner"] {
            assert_eq!(redact_secrets(untouched), untouched);
        }
    }

    #[test]
    fn does_not_redact_dotted_identifiers_or_paths_that_only_look_token_shaped() {
        // Regression: the original bot-token pattern matched any three
        // dot-separated runs of word-ish characters with a 6-char middle
        // run — extremely common in dotted module paths and hyphenated
        // identifiers, none of which are secrets.
        let module_path = "python -m anthropic_skills_office_validate.render.docx_schema_validator_module";
        assert_eq!(redact_secrets(module_path), module_path);

        let file_path = "see crates/hq-relay-adapter.protocol-implementation-notes for details";
        assert_eq!(redact_secrets(file_path), file_path);
    }

    #[test]
    fn loggable_command_never_prints_argument_text() {
        let secret_shapes = [
            "curl -u admin:hunter2 https://example.com", // gitleaks:allow
            "mysql -phunter2 -h db",
            "tool --password hunter2",
            "curl -H 'X-Api-Key: abc123' https://example.com",
            concat!("git clone https://user:", "hunter2@host/repo.git"),
            "export TOKEN=hunter2; cmd",
            "GITHUB_TOKEN=ghp_abcdefghijklmnopqrstu1234 MYSQL_PWD=hunter2 gh pr list", // gitleaks:allow
        ];
        for cmd in secret_shapes {
            let out = loggable_command(cmd);
            assert!(!out.contains("hunter2") && !out.contains("abc123") && !out.contains("ghp_"), "{cmd} -> {out}");
            assert!(out.contains(&format!("len {}", cmd.len())), "{out}");
        }
        assert_eq!(
            loggable_command("GITHUB_TOKEN=x MYSQL_PWD=y gh pr list"),
            "gh (len 37, 2 arg(s), 2 env assignment(s))"
        );
        assert_eq!(loggable_command(""), " (len 0, 0 arg(s), 0 env assignment(s))");
        assert!(loggable_command("echo a=b").starts_with("echo (len 8, 1 arg(s)"));
    }

    #[test]
    fn secret_keys_are_recognised_but_env_names_are_not() {
        for k in ["openrouter_api_key", "web_auth_token", "discord_token", "api_key", "client_secret", "DB_PASSWORD"] {
            assert!(is_secret_key(k), "{k}");
        }
        for k in ["api_key_env", "credential_env", "secret_ref", "max_tokens", "model"] {
            assert!(!is_secret_key(k), "{k}");
        }
    }

    #[test]
    fn credential_shapes_are_detected_but_error_message_prose_is_not() {
        for text in [
            "sk-or-v1-0123456789abcdef0123456789abcdef", // gitleaks:allow
            concat!("ghp_", "abcdefghijklmnopqrstuvwxyz0123456789"),
            concat!("key AKIA", "ABCDEFGHIJKLMNOP leaked"),
            "OPENAI_API_KEY=abcd1234efgh5678ijkl", // gitleaks:allow
            "Authorization: Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9", // gitleaks:allow
            "-----BEGIN RSA PRIVATE KEY-----\nMIIB\n-----END RSA PRIVATE KEY-----", // gitleaks:allow
        ] {
            assert!(looks_like_credential(text), "{text}");
        }
        for text in [
            "invalid token: signature has expired",
            "password: incorrect",
            "bearer authentication scheme explained",
            "bearer /api/v1/users/me returns 401",
            "AWS AKIA key rotation best practice",
            "max_tokens: 4096 openai",
            "sk-learn pipeline",
            "API_KEY=changeme_placeholder", // gitleaks:allow
        ] {
            assert!(!looks_like_credential(text), "{text}");
        }
    }
}
