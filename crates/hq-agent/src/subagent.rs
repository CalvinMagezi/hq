//! Legacy compatibility helpers retained after `SpawnSubagentTool` removal.
//!
//! The unified [`AgentService`](crate::agents::AgentService) still reuses these
//! role-to-model and prompt-building helpers for in-process child sessions.

/// Map a child-agent role to its default router alias. Router aliases
/// resolve through whatever providers are actually configured (cloud or
/// local), unlike a literal model tag, which would silently point at an
/// unreachable Ollama model on a host with no local Ollama instance.
/// Returns `None` for `general`, which inherits the parent-selected model.
pub(crate) fn role_to_model_alias(role: &str) -> Option<&'static str> {
    match role {
        "coder" => Some("code"),
        "planner" => Some("plan"),
        "explorer" => Some("fast"),
        "verifier" => Some("verify"),
        _ => None,
    }
}

/// Whether `model` is one of the role router aliases rather than a model id.
pub(crate) fn is_role_alias(model: &str) -> bool {
    ["coder", "planner", "explorer", "verifier"]
        .iter()
        .any(|role| role_to_model_alias(role) == Some(model))
}

fn role_preamble(role: &str) -> &'static str {
    match role {
        "coder" => "\
You are a code implementation agent. Implement the specified changes precisely.

Rules:
- Use `grep` to orient on files before reading. Only read specific line ranges.
- Follow the patterns and conventions already present. Do not introduce new abstractions.
- Read before edit. `edit_file` old_string must match exactly (including whitespace).
- Do not add features, comments, or error handling beyond what was requested.
- After changes, verify they compile (`cargo check` or equivalent) and run relevant tests.
- NEVER invent function names or file paths. Verify with grep first.
- If ambiguous, implement the most conservative interpretation and flag it.",

        "explorer" => "\
You are a fast read-only explorer agent. Search and summarize code, files, and vault content. You CANNOT write or edit.

Strategy:
- Use `grep` FIRST to find ALL occurrences. Then `read_file` with specific line ranges only.
- NEVER answer from memory. NEVER invent file paths or function names.
- Return file paths with line numbers for every finding.
- Be concise. Structural summaries over prose.
- End with a '### Key Files' section listing the most relevant paths.",

        "planner" => "\
You are a planning agent. Analyze the task and produce a clear, actionable plan.\n\
Break down the work into concrete steps with file paths and function names where applicable.\n\
Do not implement anything. Output the plan only.",

        "verifier" => "\
You are a verification specialist. Your job is not to confirm the implementation works -- it's to try to break it.\n\
\n\
Strategy:\n\
- Run the actual tests and build commands. Reading code is not verification.\n\
- Check edge cases: empty inputs, missing fields, concurrent access, boundary values.\n\
- Include at least one adversarial probe (concurrency, idempotency, orphan operations).\n\
- Be skeptical. If something looks correct at a glance, dig deeper.\n\
\n\
Rationalizations to catch yourself making:\n\
- 'Code looks correct' -- run it and prove it.\n\
- 'Tests already pass' -- do they test the new behavior?\n\
- 'Probably fine' -- probably is not verified.\n\
\n\
Report pass/fail with evidence (command output, not opinions).",

        _ => "\
Complete the specific task you are given and return a clear, concise result.\n\
Focus only on the task. Do not introduce yourself or explain your role.",
    }
}

/// Build the system prompt for an in-process child agent.
pub(crate) fn build_subagent_system_prompt(
    context: &str,
    role: &str,
    depth: u32,
    max_depth: u32,
    soul_summary: &str,
) -> String {
    let preamble = role_preamble(role);

    let identity_block = if soul_summary.is_empty() {
        String::new()
    } else {
        format!("{}\n\n", soul_summary)
    };

    let mut prompt = format!(
        "{identity_block}You are a sub-agent (depth {depth}/{max_depth}) running inside HQ's single-harness architecture.\n\
         Model selection is handled by the built-in LLM router. There are no external harnesses.\n\n\
         {preamble}",
        identity_block = identity_block,
        depth = depth + 1,
        max_depth = max_depth,
        preamble = preamble
    );

    if depth + 1 >= max_depth - 1 {
        prompt.push_str("\n\nYou cannot spawn further sub-agents.");
    } else {
        prompt.push_str(
            "\n\nYou may spawn sub-agents if the task genuinely requires decomposition, but prefer doing work directly.\n\n\
             ## Coordination Rules\n\n\
             - Read-only sub-agents can run in parallel freely.\n\
             - Write-heavy sub-agents: one at a time per file set to avoid conflicts.\n\
             - Never fabricate results. If a sub-agent fails, report the failure.\n\
             - Never use one sub-agent to check on another. Verify results yourself.\n\
             - Be skeptical of sub-agent success claims. Check the actual output.",
        );
    }

    if !context.is_empty() {
        prompt.push_str("\n\n## Context from parent agent\n\n");
        prompt.push_str(context);
    }

    prompt
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_subagent_system_prompt_with_context() {
        let prompt = build_subagent_system_prompt("Some parent context", "general", 0, 3, "");
        assert!(prompt.contains("depth 1/3"));
        assert!(prompt.contains("Some parent context"));
        assert!(prompt.contains("may spawn sub-agents"));
    }

    #[test]
    fn build_subagent_system_prompt_at_limit() {
        let prompt = build_subagent_system_prompt("", "general", 1, 3, "");
        assert!(prompt.contains("depth 2/3"));
        assert!(prompt.contains("cannot spawn further"));
    }

    #[test]
    fn build_subagent_system_prompt_coder_role() {
        let prompt = build_subagent_system_prompt("", "coder", 0, 3, "");
        assert!(prompt.contains("code implementation agent"));
        assert!(prompt.contains("Do not add features"));
    }

    #[test]
    fn build_subagent_system_prompt_explorer_role() {
        let prompt = build_subagent_system_prompt("", "explorer", 0, 3, "");
        assert!(prompt.contains("read-only explorer"));
        assert!(prompt.contains("### Key Files"));
    }

    #[test]
    fn role_to_model_alias_maps_expected_roles() {
        assert_eq!(role_to_model_alias("coder"), Some("code"));
        assert_eq!(role_to_model_alias("planner"), Some("plan"));
        assert_eq!(role_to_model_alias("explorer"), Some("fast"));
        assert_eq!(role_to_model_alias("verifier"), Some("verify"));
        assert_eq!(role_to_model_alias("general"), None);
        assert_eq!(role_to_model_alias("unknown"), None);
    }
}
