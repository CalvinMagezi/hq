//! Rule-based task type inference from prompt text.
//! Runs synchronously on every proxy call — no LLM, no I/O.

#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TaskType {
    QuickReasoning,
    CodeEditing,
    Planning,
    Research,
    LongForm,
    Agentic,
    /// Vault-centric work: reading, searching, writing notes, memory, knowledge base.
    /// The native `hq` harness owns this category — it has first-class vault tools,
    /// sub-agent dispatch, and deep vault-context injection that external harnesses lack.
    Vault,
    Unknown,
}

impl TaskType {
    pub fn as_str(&self) -> &'static str {
        match self {
            TaskType::QuickReasoning => "quick-reasoning",
            TaskType::CodeEditing => "code-editing",
            TaskType::Planning => "planning",
            TaskType::Research => "research",
            TaskType::LongForm => "long-form",
            TaskType::Agentic => "agentic",
            TaskType::Vault => "vault",
            TaskType::Unknown => "unknown",
        }
    }
}

impl std::fmt::Display for TaskType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

pub fn classify_task(prompt: &str) -> TaskType {
    let p = prompt.to_lowercase();
    let words: std::collections::HashSet<&str> = p.split_whitespace().collect();

    // Vault-centric work is checked first: it is the category where the native hq
    // harness has a structural edge (vault tools, sub-agents, context injection).
    // Requires an explicit vault/note/memory signal so generic prompts don't match.
    if has_any(&words, &["vault", "note", "notes", "notebook", "remember"])
        || contains_any(
            &p,
            &[
                "write a note",
                "save to the vault",
                "save to vault",
                "in my vault",
                "search the vault",
                "knowledge base",
                "vault note",
                "my notes",
                "frontmatter",
                ".vault",
                "notebooks/",
                "_system/",
                "to memory",
                "save to memory",
                "my memory",
            ],
        )
    {
        return TaskType::Vault;
    }

    if has_any(&words, &["edit", "fix", "refactor", "implement", "bug"])
        || contains_any(
            &p,
            &[
                "add a method",
                "create a struct",
                "update the code",
                "change the",
                "compile error",
                "test fails",
            ],
        )
        || p.contains("```")
        || p.contains(".rs")
        || p.contains(".ts")
        || p.contains(".py")
        || p.contains("fn ")
        || p.contains("impl ")
        || p.contains("async fn")
    {
        return TaskType::CodeEditing;
    }

    if has_any(
        &words,
        &[
            "run", "execute", "bash", "shell", "terminal", "deploy", "git", "commit", "push",
            "install",
        ],
    ) && prompt.len() > 100
    {
        return TaskType::Agentic;
    }

    if has_any(
        &words,
        &[
            "plan",
            "design",
            "architecture",
            "approach",
            "strategy",
            "breakdown",
            "phases",
            "roadmap",
            "spec",
            "proposal",
        ],
    ) {
        return TaskType::Planning;
    }

    if has_any(
        &words,
        &[
            "research",
            "search",
            "analyze",
            "compare",
            "benchmark",
            "review",
            "investigate",
        ],
    ) || contains_any(&p, &["look up"])
    {
        return TaskType::Research;
    }

    if has_any(
        &words,
        &[
            "write", "draft", "essay", "document", "report", "article", "blog", "email", "letter",
        ],
    ) && prompt.len() > 80
    {
        return TaskType::LongForm;
    }

    if prompt.len() < 200
        || has_any(
            &words,
            &["summarize", "classify", "answer", "quick", "brief"],
        )
        || contains_any(&p, &["one word", "one sentence"])
    {
        return TaskType::QuickReasoning;
    }

    TaskType::Unknown
}

fn has_any(words: &std::collections::HashSet<&str>, targets: &[&str]) -> bool {
    targets.iter().any(|t| {
        if t.contains(' ') {
            // Multi-word phrases: substring match against the full lowercased prompt string.
            // The words set only contains individual tokens, so phrase targets need a different check.
            // Callers pass `p` (the lowercased prompt) implicitly via the closure below, but since
            // this helper only receives the word set, multi-word targets are checked via the caller.
            // This path is unreachable — multi-word targets should not be passed here.
            false
        } else {
            words.contains(t)
        }
    })
}

/// Check if the lowercased prompt contains any of the given substrings (single or multi-word).
fn contains_any(prompt: &str, targets: &[&str]) -> bool {
    targets.iter().any(|t| prompt.contains(t))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_editing_from_rust_snippet() {
        let p = "Fix this function:\n```rust\nfn foo() {}\n```";
        assert_eq!(classify_task(p), TaskType::CodeEditing);
    }

    #[test]
    fn planning_from_design_prompt() {
        let p = "Design the architecture for a distributed task queue";
        assert_eq!(classify_task(p), TaskType::Planning);
    }

    #[test]
    fn quick_reasoning_short_prompt() {
        let p = "Is Rust memory safe?";
        assert_eq!(classify_task(p), TaskType::QuickReasoning);
    }

    #[test]
    fn vault_from_note_write() {
        let p = "Write a note summarizing the Northwind Q3 roadmap and save it to the vault";
        assert_eq!(classify_task(p), TaskType::Vault);
    }

    #[test]
    fn vault_from_search() {
        let p = "Search the vault for everything about ExampleProject and summarize the open questions";
        assert_eq!(classify_task(p), TaskType::Vault);
    }

    #[test]
    fn vault_from_memory() {
        let p = "Remember that the new daemon port is 5678";
        assert_eq!(classify_task(p), TaskType::Vault);
    }

    #[test]
    fn code_edit_not_misread_as_vault() {
        let p = "Fix the borrow checker error in session.rs at line 42";
        assert_eq!(classify_task(p), TaskType::CodeEditing);
    }
}
