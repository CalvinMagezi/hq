//! Frontmatter helpers shared by every crate that reads vault notes, including
//! `hq-db`, which cannot depend on `hq-vault` without a cycle.

/// Split a note into its YAML frontmatter (without the fences) and its body.
///
/// The opening fence must be the first line, and the first line that is only
/// `---` closes it, so a `---` inside a value or a horizontal rule later in
/// the body does not end the frontmatter early. CRLF line endings work. A note
/// without a closed frontmatter block returns `(None, content)` unchanged.
pub fn split_frontmatter(content: &str) -> (Option<&str>, &str) {
    let Some(rest) = content
        .strip_prefix("---\n")
        .or_else(|| content.strip_prefix("---\r\n"))
    else {
        return (None, content);
    };
    let mut offset = 0;
    for line in rest.split_inclusive('\n') {
        if line.trim_end() == "---" {
            return (Some(&rest[..offset]), &rest[offset + line.len()..]);
        }
        offset += line.len();
    }
    (None, content)
}

/// The note body without its frontmatter, leading blank lines removed.
pub fn strip_frontmatter(raw: &str) -> &str {
    split_frontmatter(raw).1.trim_start_matches(['\r', '\n'])
}

/// Extract tags from YAML frontmatter as a list of strings.
///
/// Supports both inline (`tags: [foo, bar]`) and block list formats:
/// ```yaml
/// tags:
///   - foo
///   - bar
/// ```
pub fn extract_tags_from_frontmatter(raw: &str) -> Vec<String> {
    let Some(frontmatter) = split_frontmatter(raw).0 else {
        return Vec::new();
    };

    let mut tags = Vec::new();
    let mut in_tags = false;
    for line in frontmatter.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("tags:") {
            let inline = trimmed.strip_prefix("tags:").unwrap().trim();
            if inline.starts_with('[') {
                // Inline array: tags: [foo, bar]
                let inner = inline.trim_start_matches('[').trim_end_matches(']');
                for tag in inner.split(',') {
                    let t = tag.trim().trim_matches('"').trim_matches('\'');
                    if !t.is_empty() {
                        tags.push(t.to_string());
                    }
                }
                return tags;
            }
            in_tags = true;
            continue;
        }
        if in_tags {
            if trimmed.starts_with("- ") {
                let tag = trimmed
                    .strip_prefix("- ")
                    .unwrap()
                    .trim()
                    .trim_matches('"')
                    .trim_matches('\'');
                if !tag.is_empty() {
                    tags.push(tag.to_string());
                }
            } else if !trimmed.is_empty() && !trimmed.starts_with('#') {
                // End of tags list
                break;
            }
        }
    }
    tags
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_basic_frontmatter() {
        let raw = "---\ntitle: Hello\n---\nBody text.";
        assert_eq!(strip_frontmatter(raw), "Body text.");
    }

    #[test]
    fn strip_no_frontmatter() {
        let raw = "Just plain markdown.";
        assert_eq!(strip_frontmatter(raw), raw);
    }

    #[test]
    fn extract_inline_tags() {
        let raw = "---\ntags: [rust, hq, test]\n---\nContent.";
        assert_eq!(
            extract_tags_from_frontmatter(raw),
            vec!["rust", "hq", "test"]
        );
    }

    #[test]
    fn extract_block_tags() {
        let raw = "---\ntags:\n  - alpha\n  - beta\n---\nContent.";
        assert_eq!(extract_tags_from_frontmatter(raw), vec!["alpha", "beta"]);
    }

    #[test]
    fn split_closes_on_the_first_fence_line_only() {
        let raw = "---\ntitle: a---b\n---\nbody\n\n---\nnot frontmatter\n";
        let (fm, body) = split_frontmatter(raw);
        assert_eq!(fm, Some("title: a---b\n"));
        assert_eq!(body, "body\n\n---\nnot frontmatter\n");
    }

    #[test]
    fn split_handles_crlf_and_unclosed_blocks() {
        assert_eq!(split_frontmatter("---\r\nx: 1\r\n---\r\nbody"), (Some("x: 1\r\n"), "body"));
        assert_eq!(split_frontmatter("---\nx: 1\nno close"), (None, "---\nx: 1\nno close"));
        assert_eq!(strip_frontmatter("---\nx: 1\n---\n\nBody"), "Body");
    }

    #[test]
    fn extract_no_frontmatter() {
        let raw = "No frontmatter here.";
        assert!(extract_tags_from_frontmatter(raw).is_empty());
    }
}
