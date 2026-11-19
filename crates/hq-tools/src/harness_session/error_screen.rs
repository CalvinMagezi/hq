//! Telling a coding agent's error screen from a finished turn. A CLI whose login or API key
//! failed stops at an error and goes quiet, which the host reads as `done`; marking the task
//! ready for review on that would be a false finish. The match is on the last rows only, so an
//! old error scrolled up the screen does not condemn a session that recovered.

/// Rows from the bottom of the screen that are searched.
const TAIL_ROWS: usize = 25;
/// Longest error line kept.
const MAX_LINE_CHARS: usize = 200;

/// Phrases (lowercase) an agent CLI prints when its credentials or the API refuse it.
const PHRASES: &[&str] = &[
    "authentication_error",
    "apikeyhelper",
    "invalid api key",
    "please run /login",
    "run /login",
    "oauth token has expired",
    "api error: 401",
    "not logged in",
    "select login method",
    "invalid x-api-key",
];

/// The error line when the bottom of `screen` shows an authentication or API-key failure.
/// Secrets in the line are redacted and it is cut to a short length.
pub fn detect(screen: &str) -> Option<String> {
    let rows: Vec<&str> = screen
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    let tail = &rows[rows.len().saturating_sub(TAIL_ROWS)..];
    // The last matching row is the freshest.
    let line = tail.iter().rev().find(|row| {
        let lower = row.to_lowercase();
        PHRASES.iter().any(|p| lower.contains(p))
    })?;
    let line: String = line.chars().take(MAX_LINE_CHARS).collect();
    Some(hq_core::redact::redact_secrets(&line))
}

#[cfg(test)]
mod tests {
    use super::detect;

    #[test]
    fn auth_and_key_failures_are_found_with_their_line() {
        for (screen, want) in [
            (
                "working...\napiKeyHelper failed: exit 1\n> ",
                "apiKeyHelper failed",
            ),
            (
                "API Error: 401 {\"type\":\"error\",\"error\":{\"type\":\"authentication_error\"}}\n",
                "authentication_error",
            ),
            ("Invalid API key · Please run /login\n", "Invalid API key"),
            ("OAuth token has expired. Run /login to continue", "expired"),
        ] {
            let line = detect(screen).unwrap_or_default();
            assert!(line.contains(want), "{screen:?} gave {line:?}");
        }
    }

    #[test]
    fn a_normal_finish_and_an_old_error_are_not_errors() {
        assert_eq!(detect("Done. I edited 3 files.\n> "), None);
        let mut screen = String::from("401 authentication_error from the proxy\n");
        for n in 0..40 {
            screen.push_str(&format!("progress line {n}\n"));
        }
        assert_eq!(detect(&screen), None, "scrolled well above the tail");
    }

    #[test]
    fn a_key_in_the_line_is_redacted() {
        // Built at run time so no secret-shaped literal sits in the source.
        let key = format!("sk-{}", "abcdefghijklmnopqrstuvwxyz0123456789");
        let line = detect(&format!("Invalid API key {key}\n")).unwrap();
        assert!(!line.contains("abcdefghijklmnop"), "{line}");
    }
}
