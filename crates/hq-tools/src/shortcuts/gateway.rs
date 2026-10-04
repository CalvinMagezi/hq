use crate::registry::{HqTool, ToolPolicy};
use crate::shortcuts::vault;
use hq_db::Database;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;

/// Alias table: shorthand key -> canonical tool name.
const ALIASES: &[(&str, &str)] = &[
    ("vault", "vault_find"),
    ("find", "vault_find"),
    ("search", "vault_find"),
    ("note", "vault_note"),
    ("write", "vault_note"),
    ("log", "vault_log"),
    ("activity", "vault_log"),
];

pub struct HqGatewayTool {
    db: Arc<Database>,
    vault_path: PathBuf,
}

impl HqGatewayTool {
    pub fn new(db: Arc<Database>, vault_path: PathBuf) -> Self {
        Self { db, vault_path }
    }
}

/// Resolve a fuzzy tool name to a canonical shortcut name.
/// Returns None if no match is found (including empty input).
fn resolve_tool(tool: &str) -> Option<&'static str> {
    let lower = tool.to_lowercase();
    if lower.is_empty() {
        return None;
    }

    // 1. Exact match against alias keys or canonical names.
    for (key, canonical) in ALIASES {
        if *key == lower || *canonical == lower {
            return Some(canonical);
        }
    }

    // 2. Prefix match: alias key starts with the input.
    for (key, canonical) in ALIASES {
        if key.starts_with(lower.as_str()) {
            return Some(canonical);
        }
    }

    // 3. Substring match: alias key contains the input.
    for (key, canonical) in ALIASES {
        if key.contains(lower.as_str()) {
            return Some(canonical);
        }
    }

    None
}

/// Build dispatch args from a plain-English request for a given canonical tool.
fn build_args_from_request(canonical: &str, request: &str) -> Result<Value, Value> {
    match canonical {
        "vault_find" => Ok(json!({"topic": request})),
        "vault_note" => Ok(json!({"content": request})),
        "vault_log" => Ok(json!({"event": request})),
        _ => Ok(json!({})),
    }
}

#[async_trait::async_trait]
impl HqTool for HqGatewayTool {
    fn name(&self) -> &str {
        "hq"
    }

    fn behavioral_prompt(&self) -> Option<&str> {
        Some(
            "Alias dispatcher for the vault and code-search shortcuts. If the operation you want is not one of its aliases, call the underlying tool directly rather than guessing at an alias name.",
        )
    }

    fn description(&self) -> &str {
        "Universal HQ gateway. Pass a fuzzy tool name and plain-English request — resolves to \
         the correct tool automatically. Use when unsure of the exact tool name."
    }

    fn parameters(&self) -> Value {
        json!({"type":"object","properties":{"tool":{"type":"string","description":"Fuzzy tool name ('vault', 'note', 'log')"},"request":{"type":"string","description":"Plain English description of what you want to do"},"args":{"type":"object","description":"Optional structured arguments that override NL resolution"}},"required":["tool"]})
    }

    fn tool_policy(&self) -> ToolPolicy {
        ToolPolicy::Weak
    }

    fn category(&self) -> &str {
        "gateway"
    }

    async fn execute(&self, params: Value) -> anyhow::Result<Value> {
        let tool_input = match params.get("tool").and_then(|v| v.as_str()) {
            Some(s) => s.to_string(),
            None => return Ok(json!({"error": "missing required parameter: tool"})),
        };

        let canonical = match resolve_tool(&tool_input) {
            Some(c) => c,
            None => {
                let available: Vec<&str> = ALIASES.iter().map(|(k, _)| *k).collect();
                return Ok(json!({
                    "error": "unknown tool",
                    "tool": tool_input,
                    "available": available,
                }));
            }
        };

        // Determine dispatch args: explicit args > NL request > empty.
        let dispatch_args = if let Some(explicit) = params.get("args").filter(|v| v.is_object()) {
            explicit.clone()
        } else if let Some(request) = params.get("request").and_then(|v| v.as_str()) {
            match build_args_from_request(canonical, request) {
                Ok(a) => a,
                Err(e) => return Ok(e),
            }
        } else {
            json!({})
        };

        // Dispatch to the resolved tool.
        let result = self.dispatch(canonical, dispatch_args).await?;

        // Merge _resolved_to into the response.
        Ok(match result {
            Value::Object(map) => {
                let mut m = serde_json::Map::new();
                m.insert("_resolved_to".to_string(), json!(canonical));
                m.extend(map);
                Value::Object(m)
            }
            other => {
                json!({"_resolved_to": canonical, "result": other})
            }
        })
    }
}

impl HqGatewayTool {
    /// Construct a fresh instance of the target tool and call execute.
    async fn dispatch(&self, canonical: &str, args: Value) -> anyhow::Result<Value> {
        match canonical {
            "vault_find" => {
                vault::VaultFindShortcut::new(self.vault_path.clone(), self.db.clone())
                    .execute(args)
                    .await
            }
            "vault_note" => {
                vault::VaultNoteShortcut::new(self.vault_path.clone())
                    .execute(args)
                    .await
            }
            "vault_log" => {
                vault::VaultLogShortcut::new(self.vault_path.clone())
                    .execute(args)
                    .await
            }
            _ => Ok(json!({"error": "no handler for canonical tool", "canonical": canonical})),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every canonical name that appears in ALIASES must have a matching arm in
    /// dispatch(), so the two tables cannot drift apart.
    #[test]
    fn all_aliases_canonicals_are_dispatchable() {
        let known_canonicals: std::collections::HashSet<&str> = [
            "vault_find",
            "vault_note",
            "vault_log",
        ]
        .iter()
        .copied()
        .collect();

        for (_key, canonical) in ALIASES {
            assert!(
                known_canonicals.contains(canonical),
                "ALIASES canonical '{}' has no dispatch() arm — add it to both ALIASES and dispatch()",
                canonical
            );
        }
    }

    #[test]
    fn resolve_tool_exact_match() {
        assert_eq!(resolve_tool("vault"), Some("vault_find"));
    }

    #[test]
    fn resolve_tool_unknown_returns_none() {
        assert_eq!(resolve_tool("nonexistent_xyz"), None);
        assert_eq!(resolve_tool(""), None);
    }
}
