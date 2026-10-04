//! Hint index, system prompt enrichment, and skill body truncation.

use std::path::{Path, PathBuf};

use super::parse::*;

/// Lightweight in-memory index of skill metadata.
///
/// Built once at startup from the skills directory. Holds only names and
/// one-line descriptions — no skill content is ever loaded into the index.
/// Agents discover skills passively via the catalog block injected into
/// prompts and load full content on-demand with `load_skill`.
pub struct SkillHintIndex {
    pub(super) entries: Vec<SkillHintEntry>,
    skills_dir: PathBuf,
}

pub(super) struct SkillHintEntry {
    name: String,
    description: String,
    hints: Vec<String>,
    pub(super) auto_load: bool,
    pub(super) load_full: bool,
    next_skills: Vec<String>,
}

impl SkillHintIndex {
    /// Build the index by scanning the skills directory.
    ///
    /// Reads only frontmatter (name, description, hints) — no content loaded.
    /// Returns an empty index if the directory doesn't exist.
    pub fn build(skills_dir: &Path) -> Self {
        let metas = list_skills(skills_dir);
        let entries = metas
            .into_iter()
            .map(|m| SkillHintEntry {
                name: m.name,
                description: m.description,
                hints: m.hints,
                auto_load: m.auto_load,
                load_full: m.load_full,
                next_skills: m.next_skills,
            })
            .collect();

        Self {
            entries,
            skills_dir: skills_dir.to_path_buf(),
        }
    }

    /// Returns true if the index has any skills at all.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Names of catalog skills with a hint in `text`, which must already be lowercase.
    pub fn matching<'a>(&'a self, text: &'a str) -> impl Iterator<Item = &'a str> + 'a {
        self.entries
            .iter()
            .filter(move |e| e.hints.iter().any(|h| word_boundary_match(text, h)))
            .map(|e| e.name.as_str())
    }

    /// Generate a compact skill catalog block for prompt injection.
    ///
    /// Each skill gets one line: name, description, and hint keywords.
    /// Agents use `load_skill` tool to fetch full content on-demand.
    /// Typically <50 bytes per skill — negligible context cost.
    pub fn catalog_block(&self) -> String {
        if self.entries.is_empty() {
            return String::new();
        }

        let mut out = String::from("# Available HQ Skills\n\n");
        out.push_str(
            "Before starting a task, if a skill below matches it or is even partly relevant, \
             call `load_skill` with its name and follow it. If a loaded skill turns out wrong or \
             incomplete, fix it with `skill_manage` before you finish.\n\n",
        );

        // The catalog line is the frontmatter description; SUMMARY.md only feeds autoLoad injection.
        for entry in &self.entries {
            out.push_str(&format!("- **{}**: {}", entry.name, entry.description));
            if !entry.hints.is_empty() {
                out.push_str(&format!(" [hints: {}]", entry.hints.join(", ")));
            }
            out.push('\n');
        }

        out
    }
}

/// Check if `hint` appears at a word boundary in `text`.
///
/// Prevents "test" from matching "latest" or "contest".
/// Multi-word hints like "dark mode" are matched as a phrase.
fn word_boundary_match(text: &str, hint: &str) -> bool {
    let mut start = 0;
    let text_bytes = text.as_bytes();
    let hint_len = hint.len();

    while let Some(pos) = text[start..].find(hint) {
        let abs_pos = start + pos;
        let before_ok = abs_pos == 0 || !text_bytes[abs_pos - 1].is_ascii_alphanumeric();
        let after_pos = abs_pos + hint_len;
        let after_ok = after_pos >= text.len() || !text_bytes[after_pos].is_ascii_alphanumeric();

        if before_ok && after_ok {
            return true;
        }
        start = abs_pos + 1;
    }
    false
}

/// Enrich a base system prompt with the skill catalog and optional tool catalog.
///
/// This is the single function all entry points should call. Injects:
/// 1. Tool catalog (if provided)
/// 2. Skill catalog (one-line per skill)
/// 3. Auto-loaded skills that match the instruction (full SKILL.md or SUMMARY.md)
///
/// `tool_catalog` is an optional pre-built tool catalog block from
/// `ToolRegistry::catalog_block()`. Pass `None` if no tool registry is available.
///
/// `max_skill_tokens` caps the total tokens spent on auto-loaded skill content.
/// Pass `None` for unlimited. Estimated at ~4 chars per token.
///
/// Returns the enriched prompt and the names of the skills that were actually
/// auto-loaded, so the caller can record them. Without that second value the
/// `skill_invocations` table only ever saw explicit `load_skill` calls, which
/// is why auto-load usage was unmeasurable.
pub fn enrich_system_prompt(
    index: &SkillHintIndex,
    base_prompt: &str,
    instruction: &str,
    tool_catalog: Option<&str>,
    max_skill_tokens: Option<usize>,
) -> (String, Vec<String>) {
    let skill_catalog = index.catalog_block();
    let tool_cat = tool_catalog.unwrap_or("");

    // Collect matched auto-load skills with their content
    let mut matched_skills: Vec<(&SkillHintEntry, String)> = Vec::new();
    let instr_lower = instruction.to_lowercase();

    if !index.entries.is_empty() && !instruction.is_empty() {
        for entry in &index.entries {
            if entry.auto_load && !entry.hints.is_empty() {
                let matched = entry
                    .hints
                    .iter()
                    .any(|h| word_boundary_match(&instr_lower, h));
                if matched {
                    let content = if entry.load_full {
                        // Inject full SKILL.md content for workflow skills
                        parse_skill(&index.skills_dir, &entry.name).map(|s| s.content)
                    } else {
                        // Inject SUMMARY.md for lighter skills, falling back to
                        // a trimmed SKILL.md. A skill with autoLoad, no
                        // loadFull, and no SUMMARY.md used to match and then
                        // vanish silently — it looked configured and never
                        // fired. `hq skills validate` now rejects that shape;
                        // this keeps existing skills working meanwhile.
                        let summary_path = index.skills_dir.join(&entry.name).join("SUMMARY.md");
                        std::fs::read_to_string(summary_path).ok().or_else(|| {
                            tracing::warn!(
                                skill = %entry.name,
                                "autoLoad skill has no SUMMARY.md; falling back to a \
                                 truncated SKILL.md — add a SUMMARY.md or set loadFull"
                            );
                            parse_skill(&index.skills_dir, &entry.name)
                                .map(|s| truncate_skill_body(&s.content, SUMMARY_FALLBACK_CHARS))
                        })
                    };

                    if let Some(content) = content {
                        matched_skills.push((entry, content));
                    }
                }
            }
        }
    }

    // Apply token budget: loadFull skills get priority, others fall back to SUMMARY.md
    let max_chars = max_skill_tokens.map(|t| t * 4);
    let mut auto_loaded_content = String::new();
    let mut loaded_names: Vec<String> = Vec::new();
    let mut chars_used: usize = 0;

    for (entry, content) in &matched_skills {
        let content_len = content.len();
        let over_budget = max_chars
            .map(|max| chars_used + content_len > max)
            .unwrap_or(false);

        let final_content = if over_budget && entry.load_full {
            // Over budget: fall back to SUMMARY.md for this skill
            let summary_path = index.skills_dir.join(&entry.name).join("SUMMARY.md");
            std::fs::read_to_string(summary_path).ok()
        } else if over_budget {
            // Non-loadFull skill over budget: skip entirely
            None
        } else {
            Some(content.clone())
        };

        if let Some(text) = final_content {
            loaded_names.push(entry.name.clone());
            if !auto_loaded_content.is_empty() {
                auto_loaded_content.push_str("\n\n---\n\n");
            }
            auto_loaded_content.push_str(&text);

            // Append chaining cues if nextSkills is non-empty
            if !entry.next_skills.is_empty() {
                auto_loaded_content.push_str(&format!(
                    "\n\n> **Next skills:** {}",
                    entry.next_skills.join(", ")
                ));
            }

            chars_used += text.len();
        }
    }

    if skill_catalog.is_empty() && tool_cat.is_empty() && auto_loaded_content.is_empty() {
        return (base_prompt.to_string(), loaded_names);
    }

    let mut result = String::with_capacity(
        base_prompt.len() + skill_catalog.len() + tool_cat.len() + auto_loaded_content.len() + 128,
    );
    result.push_str(base_prompt);

    if !tool_cat.is_empty() {
        result.push_str("\n\n");
        result.push_str(tool_cat);
    }

    if !skill_catalog.is_empty() {
        result.push_str("\n\n");
        result.push_str(&skill_catalog);
    }

    if !auto_loaded_content.is_empty() {
        result.push_str("\n\n# Matched Quality Rules\n\n");
        result.push_str("These rules are automatically loaded because your instructions match established quality profiles.\n\n");
        result.push_str(&auto_loaded_content);
    }

    (result, loaded_names)
}

/// Trim a SKILL.md body to roughly `max_chars`, cutting on a line boundary.
/// Used only as the fallback when an autoLoad skill has no SUMMARY.md.
fn truncate_skill_body(body: &str, max_chars: usize) -> String {
    if body.len() <= max_chars {
        return body.to_string();
    }
    let clipped = &body[..body.floor_char_boundary(max_chars)];
    let cut = clipped.rfind('\n').unwrap_or(clipped.len());
    format!(
        "{}\n\n_(truncated — see the full SKILL.md)_",
        &clipped[..cut]
    )
}
