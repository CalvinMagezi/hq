//! 4-tier LLM JSON recovery parser.
//!
//! Small local models (1-4B params) frequently return malformed JSON wrapped
//! in prose, markdown fences, or YAML-like preamble. This module attempts
//! multiple extraction strategies before giving up, reducing expensive
//! retry round-trips from 3 to typically 0.

use serde::de::DeserializeOwned;
use tracing::debug;

/// Attempt to parse JSON from potentially messy LLM output using 4 fallback tiers:
///
/// 1. Direct parse (clean JSON)
/// 2. Brace/bracket extraction (JSON embedded in prose)
/// 3. Markdown fence extraction (```json ... ```)
/// 4. Preamble stripping (YAML-like lines before the first `{`)
pub fn parse_llm_json<T: DeserializeOwned>(raw: &str) -> Result<T, serde_json::Error> {
    let trimmed = raw.trim();

    // Tier 1: Direct parse
    if let Ok(v) = serde_json::from_str::<T>(trimmed) {
        debug!("json_recovery: tier 1 (direct) succeeded");
        return Ok(v);
    }

    // Tier 2: Extract between outermost matching braces or brackets
    if let Some(extracted) = extract_balanced(trimmed)
        && let Ok(v) = serde_json::from_str::<T>(extracted)
    {
        debug!("json_recovery: tier 2 (brace extraction) succeeded");
        return Ok(v);
    }

    // Tier 3: Markdown fence extraction
    if let Some(fenced) = extract_fenced(trimmed) {
        if let Ok(v) = serde_json::from_str::<T>(fenced) {
            debug!("json_recovery: tier 3 (fence extraction) succeeded");
            return Ok(v);
        }
        // Also try brace extraction on the fenced content
        if let Some(inner) = extract_balanced(fenced)
            && let Ok(v) = serde_json::from_str::<T>(inner)
        {
            debug!("json_recovery: tier 3b (fence + brace) succeeded");
            return Ok(v);
        }
    }

    // Tier 4: Strip preamble lines before the first `{` or `[`
    if let Some(stripped) = strip_preamble(trimmed) {
        if let Ok(v) = serde_json::from_str::<T>(stripped) {
            debug!("json_recovery: tier 4 (preamble strip) succeeded");
            return Ok(v);
        }
        // Try brace extraction on the stripped content
        if let Some(inner) = extract_balanced(stripped)
            && let Ok(v) = serde_json::from_str::<T>(inner)
        {
            debug!("json_recovery: tier 4b (preamble + brace) succeeded");
            return Ok(v);
        }
    }

    // All tiers failed; return the original parse error for diagnostics
    debug!("json_recovery: all 4 tiers failed");
    serde_json::from_str::<T>(trimmed)
}

/// Extract the substring between the first `{` and the last matching `}`,
/// or between the first `[` and the last matching `]`.
/// When both are present, picks whichever starts first (outermost).
fn extract_balanced(s: &str) -> Option<&str> {
    let mut best: Option<&str> = None;
    let mut best_start = usize::MAX;

    for (open, close) in [('{', '}'), ('[', ']')] {
        if let (Some(start), Some(end)) = (s.find(open), s.rfind(close))
            && end > start
        {
            let candidate = &s[start..=end];
            if is_plausibly_balanced(candidate, open, close) && start < best_start {
                best = Some(candidate);
                best_start = start;
            }
        }
    }
    best
}

/// Rough balance check: count of open chars should equal close chars.
/// Not a full parser, just a heuristic to avoid extracting partial JSON.
fn is_plausibly_balanced(s: &str, open: char, close: char) -> bool {
    let mut depth: i32 = 0;
    let mut in_string = false;
    let mut escape_next = false;

    for ch in s.chars() {
        if escape_next {
            escape_next = false;
            continue;
        }
        if ch == '\\' && in_string {
            escape_next = true;
            continue;
        }
        if ch == '"' {
            in_string = !in_string;
            continue;
        }
        if in_string {
            continue;
        }
        if ch == open {
            depth += 1;
        } else if ch == close {
            depth -= 1;
        }
    }
    depth == 0
}

/// Extract content from markdown code fences: ```json ... ``` or ``` ... ```
fn extract_fenced(s: &str) -> Option<&str> {
    // Try ```json first, then plain ```
    for prefix in ["```json", "```"] {
        if let Some(start_idx) = s.find(prefix) {
            let content_start = start_idx + prefix.len();
            let rest = &s[content_start..];
            if let Some(end_idx) = rest.find("```") {
                let content = rest[..end_idx].trim();
                if !content.is_empty() {
                    return Some(content);
                }
            }
        }
    }
    None
}

/// Strip lines before the first line starting with `{` or `[`.
/// Handles cases where the model outputs explanatory text before JSON.
fn strip_preamble(s: &str) -> Option<&str> {
    for (i, line) in s.lines().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('{') || trimmed.starts_with('[') {
            // Return everything from this line onward
            let offset: usize = s.lines().take(i).map(|l| l.len() + 1).sum();
            let remainder = &s[offset..];
            if !remainder.is_empty() {
                return Some(remainder.trim_end());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Debug, Deserialize, PartialEq)]
    struct TestObj {
        name: String,
        count: u32,
    }

    #[test]
    fn tier1_clean_json() {
        let input = r#"{"name": "hello", "count": 42}"#;
        let result: TestObj = parse_llm_json(input).unwrap();
        assert_eq!(
            result,
            TestObj {
                name: "hello".into(),
                count: 42
            }
        );
    }

    #[test]
    fn tier1_with_whitespace() {
        let input = r#"
            {"name": "hello", "count": 42}
        "#;
        let result: TestObj = parse_llm_json(input).unwrap();
        assert_eq!(
            result,
            TestObj {
                name: "hello".into(),
                count: 42
            }
        );
    }

    #[test]
    fn tier2_prose_wrapped() {
        let input = r#"Here is the JSON output:
{"name": "test", "count": 7}
Hope this helps!"#;
        let result: TestObj = parse_llm_json(input).unwrap();
        assert_eq!(
            result,
            TestObj {
                name: "test".into(),
                count: 7
            }
        );
    }

    #[test]
    fn tier2_inline_prose() {
        let input = r#"The result is {"name": "inline", "count": 1} as requested."#;
        let result: TestObj = parse_llm_json(input).unwrap();
        assert_eq!(
            result,
            TestObj {
                name: "inline".into(),
                count: 1
            }
        );
    }

    #[test]
    fn tier3_markdown_fence() {
        let input = r#"Here's the output:

```json
{"name": "fenced", "count": 99}
```

That should work."#;
        let result: TestObj = parse_llm_json(input).unwrap();
        assert_eq!(
            result,
            TestObj {
                name: "fenced".into(),
                count: 99
            }
        );
    }

    #[test]
    fn tier3_plain_fence() {
        let input = r#"```
{"name": "plain", "count": 5}
```"#;
        let result: TestObj = parse_llm_json(input).unwrap();
        assert_eq!(
            result,
            TestObj {
                name: "plain".into(),
                count: 5
            }
        );
    }

    #[test]
    fn tier4_preamble_lines() {
        let input = r#"Sure! Here's what I came up with:
Output format: JSON
---
{"name": "preamble", "count": 3}"#;
        let result: TestObj = parse_llm_json(input).unwrap();
        assert_eq!(
            result,
            TestObj {
                name: "preamble".into(),
                count: 3
            }
        );
    }

    #[test]
    fn tier2_array() {
        let input = r#"Results: [{"name": "a", "count": 1}, {"name": "b", "count": 2}]"#;
        let result: Vec<TestObj> = parse_llm_json(input).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].name, "a");
    }

    #[test]
    fn nested_braces_in_strings() {
        let input = r#"{"name": "has {braces}", "count": 10}"#;
        let result: TestObj = parse_llm_json(input).unwrap();
        assert_eq!(
            result,
            TestObj {
                name: "has {braces}".into(),
                count: 10
            }
        );
    }

    #[test]
    fn all_tiers_fail() {
        let input = "This is just plain text with no JSON at all.";
        let result = parse_llm_json::<TestObj>(input);
        assert!(result.is_err());
    }

    #[test]
    fn escaped_quotes_in_strings() {
        let input = r#"{"name": "say \"hi\"", "count": 0}"#;
        let result: TestObj = parse_llm_json(input).unwrap();
        assert_eq!(result.name, r#"say "hi""#);
    }
}
