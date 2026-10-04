//! Promote a declared backend to primary in `~/.hq/config.yaml`. Shared by the relay `model`
//! command and the agent's `model_switch` tool.

use super::HqConfig;

/// Result of resolving a user-typed `/model <name>` argument against the
/// configured backend chain.
pub enum ModelResolution {
    /// `name` matched a backend's `name` or its declared `model` string —
    /// promote that backend to primary.
    Backend(String),
    /// Nothing in the chain matched; carries the trimmed input for the reply.
    NoMatch(String),
}

/// Resolve a `/model <name>` argument against the configured backend chain,
/// matching either a backend's `name` (e.g. `deepseek`) or its declared
/// `model` string (e.g. `deepseek-v4-flash`), case-insensitively.
pub fn resolve_model_arg(backends: &super::BackendsConfig, name: &str) -> ModelResolution {
    let needle = name.trim().to_lowercase();
    for entry in &backends.backends {
        let name_match = entry.name.to_lowercase() == needle;
        let model_match = entry
            .model
            .as_deref()
            .map(|m| m.to_lowercase() == needle)
            .unwrap_or(false);
        if name_match || model_match {
            return ModelResolution::Backend(entry.name.clone());
        }
    }
    ModelResolution::NoMatch(name.trim().to_string())
}

/// Promote a declared backend to primary in `~/.hq/config.yaml`, moving the
/// old primary into the fallbacks (order otherwise preserved), and realign
/// `default_model` to the new primary's model so legacy paths don't drift.
///
/// Line-based, not serde: `HqConfig::save` rewrites the whole file and strips
/// comments, so we surgically replace only the two touched lines.
///
/// Returns `(new_primary_model, default_model_realigned)` on success.
pub fn set_primary_backend(name: &str) -> anyhow::Result<(Option<String>, bool)> {
    let path = HqConfig::config_file_path();
    let content = std::fs::read_to_string(&path)
        .map_err(|e| anyhow::anyhow!("cannot read {}: {e}", path.display()))?;
    let config = HqConfig::load()?;

    // Validate the target exists and is enabled before touching the file.
    let entry = config
        .backends
        .backend(name)
        .ok_or_else(|| anyhow::anyhow!("no backend named '{name}' in the chain"))?;
    if !entry.enabled {
        anyhow::bail!("backend '{name}' is disabled in config");
    }
    let new_model = entry.model.clone();

    // New fallbacks: old primary + remaining fallbacks, minus the promoted one.
    let mut fallbacks: Vec<String> = std::iter::once(config.backends.primary.clone())
        .chain(config.backends.fallbacks.iter().cloned())
        .filter(|n| n != name)
        .collect();
    fallbacks.dedup();

    let mut lines: Vec<String> = content.lines().map(|l| l.to_string()).collect();
    let mut replaced_primary = false;
    let mut fallback_start: Option<usize> = None;
    let mut fallback_end = 0usize;

    for i in 0..lines.len() {
        let line = lines[i].clone();
        let t = line.trim_start();
        let indent = line.len() - t.len();
        if t.starts_with("primary:") && indent == 2 {
            lines[i] = format!("{}primary: {}", " ".repeat(indent), name);
            replaced_primary = true;
        } else if t.starts_with("fallbacks:") && indent == 2 {
            fallback_start = Some(i);
            // Consume existing `- item` lines at deeper indent.
            let mut j = i + 1;
            while j < lines.len() {
                let lt = lines[j].trim_start();
                let lindent = lines[j].len() - lt.len();
                if lindent > indent && lt.starts_with("- ") {
                    j += 1;
                } else {
                    break;
                }
            }
            fallback_end = j;
        }
    }

    if !replaced_primary {
        anyhow::bail!("could not locate `primary:` line in {}", path.display());
    }

    if let Some(start) = fallback_start {
        let indent = lines[start].len() - lines[start].trim_start().len();
        let mut replacement = vec![format!("{}fallbacks:", " ".repeat(indent))];
        for f in &fallbacks {
            replacement.push(format!("{}- {}", " ".repeat(indent + 2), f));
        }
        lines.splice(start..fallback_end, replacement);
    }

    crate::fs_private::write_private(&path, lines.join("\n") + "\n")?;

    // Realign default_model so non-chain paths (status, legacy provider
    // construction, tool presets) resolve to the same model the chain runs.
    let mut realigned = false;
    if let Some(ref m) = new_model {
        HqConfig::set_key("default_model", m)?;
        realigned = true;
    }

    Ok((new_model, realigned))
}

fn indent_of(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

/// Add `entry` to the `backends.backends` list of a config file's text, matching the list's own
/// indentation and leaving everything else, comments included, untouched.
pub fn insert_backend(content: &str, entry: &super::BackendEntry) -> anyhow::Result<String> {
    let lines: Vec<&str> = content.lines().collect();
    let top = lines
        .iter()
        .position(|l| l.starts_with("backends:"))
        .ok_or_else(|| anyhow::anyhow!("no top-level `backends:` section in the config"))?;
    let list_key = (top + 1..lines.len())
        .find(|&i| indent_of(lines[i]) == 2 && lines[i].trim_start().starts_with("backends:"))
        .ok_or_else(|| anyhow::anyhow!("no `backends:` list inside the backends section"))?;
    let dash_indent = (list_key + 1..lines.len())
        .find(|&i| lines[i].trim_start().starts_with("- "))
        .map(|i| indent_of(lines[i]))
        .ok_or_else(|| {
            anyhow::anyhow!("the backends list has no entries to place a new one after")
        })?;
    let mut last = list_key;
    for (i, line) in lines.iter().enumerate().skip(list_key + 1) {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let indent = indent_of(line);
        let belongs =
            indent > dash_indent || (indent == dash_indent && line.trim_start().starts_with("- "));
        if !belongs {
            break;
        }
        last = i;
    }
    let body = serde_yaml::to_string(entry)?;
    let pad = " ".repeat(dash_indent);
    let mut block = Vec::new();
    for (n, field) in body.lines().enumerate() {
        block.push(if n == 0 {
            format!("{pad}- {field}")
        } else {
            format!("{pad}  {field}")
        });
    }
    let mut out: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
    out.splice(last + 1..last + 1, block);
    Ok(out.join("\n") + "\n")
}

/// Declare a new backend in `~/.hq/config.yaml`. The result is parsed back to confirm the entry
/// is there before the file is replaced, and the previous file is kept next to it.
pub fn declare_backend(entry: &super::BackendEntry) -> anyhow::Result<()> {
    let path = HqConfig::config_file_path();
    let content = std::fs::read_to_string(&path)
        .map_err(|e| anyhow::anyhow!("cannot read {}: {e}", path.display()))?;
    let updated = insert_backend(&content, entry)?;
    let parsed: serde_yaml::Value = serde_yaml::from_str(&updated)
        .map_err(|e| anyhow::anyhow!("the edited config would not parse: {e}"))?;
    let present = parsed["backends"]["backends"]
        .as_sequence()
        .is_some_and(|items| {
            items
                .iter()
                .any(|i| i["name"].as_str() == Some(entry.name.as_str()))
        });
    if !present {
        anyhow::bail!("the new backend did not land in the config; nothing was written");
    }
    let backup = path.with_extension("yaml.bak-model-switch");
    crate::fs_private::write_private(&backup, &content)?;
    crate::fs_private::write_private(&path, updated)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::{BackendEntry, BackendKind, WireApi};
    use super::*;

    const CONFIG: &str = "default_model: x\nbackends:\n  version: 1\n  primary: copilot\n  fallbacks:\n    - luna\n  backends:\n  - name: copilot\n    kind: github-copilot-api\n    model: claude-sonnet-5\n    enabled: true\n  # keep this comment\n  - name: luna\n    kind: openai-compatible\n    endpoint: https://api.githubcopilot.com\n    wire: responses\n    model: gpt-6-luna\n    enabled: true\nrelay:\n  model: null\n";

    fn entry() -> BackendEntry {
        BackendEntry {
            name: "gpt-6-astra".into(),
            kind: BackendKind::OpenaiCompatible,
            endpoint: Some("https://api.githubcopilot.com".into()),
            credential_env: None,
            model: Some("gpt-6-astra".into()),
            effort: None,
            wire: WireApi::Responses,
            enabled: true,
        }
    }

    #[test]
    fn a_new_entry_lands_after_the_last_one_and_the_rest_is_untouched() {
        let out = insert_backend(CONFIG, &entry()).unwrap();
        assert!(out.contains("# keep this comment"));
        assert!(out.contains("relay:\n  model: null"));
        let parsed: serde_yaml::Value = serde_yaml::from_str(&out).unwrap();
        let names: Vec<&str> = parsed["backends"]["backends"]
            .as_sequence()
            .unwrap()
            .iter()
            .map(|i| i["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["copilot", "luna", "gpt-6-astra"]);
        assert_eq!(
            parsed["backends"]["backends"][2]["wire"].as_str(),
            Some("responses")
        );
        assert_eq!(parsed["relay"]["model"], serde_yaml::Value::Null);
    }

    #[test]
    fn a_config_without_a_backends_list_is_refused() {
        assert!(insert_backend("default_model: x\n", &entry()).is_err());
    }
}
