//! Mechanical AI-slop detection shared across fiction (`hq-tools::novel`) and
//! business-document generation (`hq-tools::brand`).
//!
//! Lives in `hq-core` rather than `hq-tools` so any crate can score prose
//! without depending on `hq-tools` (same reasoning `critic.rs` documents for
//! itself).

use regex::Regex;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SlopViolation {
    pub line_number: usize,
    pub pattern_id: String,
    pub fragment: String,
    pub description: String,
}

struct SlopRule {
    id: &'static str,
    pattern: Regex,
    description: &'static str,
}

pub struct SlopDetector {
    rules: Vec<SlopRule>,
}

fn banned_word(id: &'static str, word: &str, description: &'static str) -> SlopRule {
    SlopRule {
        id,
        pattern: Regex::new(&format!(r"(?i)\b{word}\b")).unwrap(),
        description,
    }
}

/// A single em-dash is enough to flag — the earlier `—.*—.*—` pattern only
/// fired on 3+ em-dashes on one line and missed the common one-per-sentence
/// AI tell entirely.
fn em_dash_rule() -> SlopRule {
    SlopRule {
        id: "em_dash_present",
        pattern: Regex::new(r"—").unwrap(),
        description: "Em-dash present; prefer commas, periods, or colons",
    }
}

/// `regex`'s Unicode tables have no `Emoji` property, so this lists explicit
/// codepoint ranges (pictographs, emoticons, transport, symbols, dingbats,
/// regional-indicator flag letters). Arrows (`\u{2190}-\u{21FF}`) are
/// deliberately excluded — `→` shows up constantly in ordinary technical
/// prose and code comments.
fn emoji_rule() -> SlopRule {
    SlopRule {
        id: "emoji_present",
        pattern: Regex::new(
            r"[\x{1F300}-\x{1FAFF}\x{2600}-\x{26FF}\x{2700}-\x{27BF}\x{1F1E6}-\x{1F1FF}]",
        )
        .unwrap(),
        description: "Emoji/icon present in business prose",
    }
}

impl SlopDetector {
    /// Fiction-writing anti-patterns (`hq-tools::novel`'s existing rule set,
    /// minus `triadic_sensory_list` — that rule matched any ordinary
    /// Oxford-comma list and was unconditionally wrong, not a false positive
    /// specific to one domain).
    pub fn fiction() -> Self {
        let rules = vec![
            banned_word(
                "banned_word_delve",
                "delve",
                "Banned AI-cliché word 'delve'",
            ),
            banned_word(
                "banned_word_tapestry",
                "tapestry",
                "Banned AI-cliché word 'tapestry'",
            ),
            banned_word(
                "banned_word_landscape",
                "landscape",
                "Banned AI-cliché word 'landscape' in abstract context",
            ),
            banned_word(
                "banned_word_leverage",
                "leverage",
                "Banned corporate-speak word 'leverage'",
            ),
            banned_word(
                "banned_word_robust",
                "robust",
                "Banned AI-cliché word 'robust'",
            ),
            banned_word(
                "banned_word_seamless",
                "seamless",
                "Banned AI-cliché word 'seamless'",
            ),
            SlopRule {
                id: "cliche_opener_landscape",
                pattern: Regex::new(r"(?i)^In today's .* landscape").unwrap(),
                description: "Banned cliché opener 'In today's ... landscape'",
            },
            SlopRule {
                id: "cliche_opener_dive_in",
                pattern: Regex::new(r"(?i)^Let's dive in").unwrap(),
                description: "Banned cliché opener 'Let's dive in'",
            },
            SlopRule {
                id: "thought_about_formulation",
                pattern: Regex::new(r"(?i)\bthought about\b").unwrap(),
                description: "Weak 'thought about' formulation; show, don't tell",
            },
            SlopRule {
                id: "show_dont_tell_surge",
                pattern: Regex::new(r"(?i)\bfelt a surge of\b").unwrap(),
                description: "Weak 'felt a surge of' formulation; show, don't tell",
            },
            SlopRule {
                id: "show_dont_tell_shiver",
                pattern: Regex::new(r"(?i)\bshiver ran down\b").unwrap(),
                description: "Cliché 'shiver ran down' formulation",
            },
            em_dash_rule(),
            emoji_rule(),
        ];
        Self { rules }
    }

    /// Business-document anti-patterns: banned words, openers and closers,
    /// em-dashes and emoji.
    pub fn business() -> Self {
        let rules = vec![
            banned_word(
                "banned_word_delve",
                "delve",
                "Banned AI-cliché word 'delve'",
            ),
            banned_word(
                "banned_word_landscape",
                "landscape",
                "Banned AI-cliché word 'landscape'",
            ),
            banned_word(
                "banned_word_tapestry",
                "tapestry",
                "Banned AI-cliché word 'tapestry'",
            ),
            banned_word(
                "banned_word_leverage",
                "leverage",
                "Banned corporate-speak word 'leverage'",
            ),
            banned_word(
                "banned_word_robust",
                "robust",
                "Banned AI-cliché word 'robust'",
            ),
            banned_word(
                "banned_word_seamless",
                "seamless",
                "Banned AI-cliché word 'seamless'",
            ),
            banned_word(
                "banned_word_cutting_edge",
                "cutting-edge",
                "Banned AI-cliché word 'cutting-edge'",
            ),
            banned_word(
                "banned_word_innovative",
                "innovative",
                "Banned AI-cliché word 'innovative'",
            ),
            SlopRule {
                id: "opener_todays_landscape",
                pattern: Regex::new(r"(?i)In today's .* landscape").unwrap(),
                description: "Banned opener \"In today's ... landscape\"",
            },
            SlopRule {
                id: "opener_dive_in",
                pattern: Regex::new(r"(?i)Let's dive in").unwrap(),
                description: "Banned opener \"Let's dive in\"",
            },
            SlopRule {
                id: "opener_heres_the_thing",
                pattern: Regex::new(r"(?i)Here's the thing").unwrap(),
                description: "Banned opener \"Here's the thing\"",
            },
            SlopRule {
                id: "opener_important_to_note",
                pattern: Regex::new(r"(?i)It's important to note that").unwrap(),
                description: "Banned opener \"It's important to note that\"",
            },
            SlopRule {
                id: "closer_final_thoughts",
                pattern: Regex::new(r"(?i)^#+\s*Final [Tt]houghts").unwrap(),
                description: "Banned closer heading \"Final thoughts\"",
            },
            SlopRule {
                id: "closer_in_conclusion",
                pattern: Regex::new(r"(?i)^In conclusion,?").unwrap(),
                description: "Banned closer paragraph opener \"In conclusion\"",
            },
            SlopRule {
                id: "not_just_x_its_y",
                pattern: Regex::new(r"(?i)not just \w+.{0,20}, (it'?s|but) ").unwrap(),
                description: "Banned structural tell: \"It's not just X, it's Y\"",
            },
            em_dash_rule(),
            emoji_rule(),
        ];
        Self { rules }
    }

    pub fn detect(&self, content: &str) -> Vec<SlopViolation> {
        let mut violations = Vec::new();
        for (i, line) in content.lines().enumerate() {
            for rule in &self.rules {
                if let Some(mat) = rule.pattern.find(line) {
                    violations.push(SlopViolation {
                        line_number: i + 1,
                        pattern_id: rule.id.to_string(),
                        fragment: mat.as_str().to_string(),
                        description: rule.description.to_string(),
                    });
                }
            }
        }
        violations
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(violations: &[SlopViolation]) -> Vec<&str> {
        violations.iter().map(|v| v.pattern_id.as_str()).collect()
    }

    #[test]
    fn business_catches_banned_word() {
        let v = SlopDetector::business().detect("We should leverage this opportunity.");
        assert!(ids(&v).contains(&"banned_word_leverage"));
    }

    #[test]
    fn business_catches_single_em_dash() {
        // Regression case: the old `—.*—.*—` pattern needed 3 em-dashes on
        // one line and missed exactly this, the common AI tell.
        let v = SlopDetector::business().detect("The rollout went well — mostly.");
        assert!(ids(&v).contains(&"em_dash_present"));
    }

    #[test]
    fn business_catches_emoji() {
        let v = SlopDetector::business().detect("Deployment succeeded \u{2705}");
        assert!(ids(&v).contains(&"emoji_present"));
    }

    #[test]
    fn business_clean_oxford_comma_list_has_no_violations() {
        // Regression case: the removed `triadic_sensory_list` rule fired on
        // any ordinary 3-item list, which was unconditionally wrong.
        let v = SlopDetector::business().detect("We shipped auth, billing, and search.");
        assert!(v.is_empty(), "expected no violations, got {v:?}");
    }

    #[test]
    fn business_catches_opener_and_closer() {
        let v = SlopDetector::business()
            .detect("Here's the thing about this quarter.\nIn conclusion, it went fine.");
        let found = ids(&v);
        assert!(found.contains(&"opener_heres_the_thing"));
        assert!(found.contains(&"closer_in_conclusion"));
    }

    #[test]
    fn fiction_rules_still_catch_existing_patterns() {
        let v = SlopDetector::fiction().detect("A shiver ran down her spine.");
        assert!(ids(&v).contains(&"show_dont_tell_shiver"));
    }

    #[test]
    fn fiction_no_longer_flags_triadic_lists() {
        let v = SlopDetector::fiction().detect("The room smelled of smoke, dust, and rain.");
        assert!(v.is_empty(), "expected no violations, got {v:?}");
    }
}
