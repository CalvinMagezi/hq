//! `hq shortcuts` subcommand.
//!
//! Smoke-tests the shortcut tools after install: the built-in read-only probes,
//! or `.vault/Notebooks/Tests/shortcut_probes.yaml` when a vault defines its own.

use anyhow::Result;
use hq_core::config::HqConfig;
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;

/// A single probe entry from the YAML file.
#[derive(Debug, Deserialize)]
struct Probe {
    name: String,
    tool: String,
    #[serde(default)]
    args: serde_json::Map<String, Value>,
    expect_keys: Vec<String>,
}

/// Read-only probes used when the vault has no probe file of its own.
const BUILTIN_PROBES: &str = r#"
probes:
  - name: vault_find answers a topic
    tool: vault_find
    args: { topic: "hq" }
    expect_keys: [notes]
  - name: hq gateway resolves an alias
    tool: hq
    args: { tool: "find", request: "hq" }
    expect_keys: [_resolved_to, notes]
"#;

/// Top-level YAML structure.
#[derive(Debug, Deserialize)]
struct ProbeFile {
    probes: Vec<Probe>,
}

pub async fn run(config: &HqConfig, sub: &str) -> Result<()> {
    match sub {
        "test" => run_test(config).await,
        _ => {
            println!("Usage: hq shortcuts <subcommand>");
            println!();
            println!("Subcommands:");
            println!("  test    Run shortcut smoke tests (built-in, or the vault's shortcut_probes.yaml)");
            Ok(())
        }
    }
}

async fn run_test(config: &HqConfig) -> Result<()> {
    let (probe_file, source) = load_probes(config).await?;
    println!("Probes from {source}");

    if probe_file.probes.is_empty() {
        println!("No probes defined in {source}.");
        return Ok(());
    }

    // Open the database and build the shortcut tool registry.
    let vault_path = config.vault_path.clone();
    let db = tokio::task::spawn_blocking(move || open_db_at(&vault_path)).await??;
    let tools =
        hq_tools::shortcuts::create_shortcut_tools(config.vault_path.clone(), db);

    // Index tools by name for O(1) lookup.
    let tool_map: std::collections::HashMap<&str, &dyn hq_tools::registry::HqTool> =
        tools.iter().map(|t| (t.name(), t.as_ref())).collect();

    println!("Running {} shortcut probe(s)...\n", probe_file.probes.len());

    let mut pass = 0usize;
    let mut fail = 0usize;

    for probe in &probe_file.probes {
        let result = run_probe(probe, &tool_map).await;
        match result {
            Ok(()) => {
                println!("  [OK] {}", probe.name);
                pass += 1;
            }
            Err(e) => {
                println!("  [FAIL] {}: {e}", probe.name);
                fail += 1;
            }
        }
    }

    println!();
    println!("Results: {pass} passed, {fail} failed.");

    if fail > 0 {
        std::process::exit(1);
    }
    Ok(())
}

async fn load_probes(config: &HqConfig) -> Result<(ProbeFile, String)> {
    let probe_path = config
        .vault_path
        .join("Notebooks")
        .join("Tests")
        .join("shortcut_probes.yaml");
    let (text, source) = match tokio::fs::read_to_string(&probe_path).await {
        Ok(text) => (text, probe_path.display().to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            (BUILTIN_PROBES.to_string(), "the built-in probe set".to_string())
        }
        Err(e) => return Err(e.into()),
    };
    let probes = serde_yaml::from_str(&text)
        .map_err(|e| anyhow::anyhow!("failed to parse {source}: {e}"))?;
    Ok((probes, source))
}

async fn run_probe(
    probe: &Probe,
    tool_map: &std::collections::HashMap<&str, &dyn hq_tools::registry::HqTool>,
) -> Result<()> {
    let tool = tool_map
        .get(probe.tool.as_str())
        .ok_or_else(|| anyhow::anyhow!("unknown tool '{}'", probe.tool))?;

    let args = Value::Object(probe.args.clone());
    let output = tool
        .execute(args)
        .await
        .map_err(|e| anyhow::anyhow!("execute error: {e}"))?;

    let obj = output
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("expected JSON object, got: {output}"))?;

    for key in &probe.expect_keys {
        if !obj.contains_key(key.as_str()) {
            anyhow::bail!(
                "missing expected key '{}' in response (got: {})",
                key,
                serde_json::to_string(obj).unwrap_or_default()
            );
        }
    }

    Ok(())
}

fn open_db_at(vault_path: &std::path::Path) -> Result<Arc<hq_db::Database>> {
    let db_path = vault_path.join("_data").join("vault.db");
    std::fs::create_dir_all(db_path.parent().unwrap()).ok();
    let db = hq_db::Database::open(&db_path)?;
    Ok(Arc::new(db))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn builtin_probes_pass_on_an_empty_vault() {
        let file: ProbeFile = serde_yaml::from_str(BUILTIN_PROBES).unwrap();
        let vault = tempfile::tempdir().unwrap();
        let db = Arc::new(hq_db::Database::open_memory().unwrap());
        let tools = hq_tools::shortcuts::create_shortcut_tools(vault.path().to_path_buf(), db);
        let tool_map: std::collections::HashMap<&str, &dyn hq_tools::registry::HqTool> =
            tools.iter().map(|t| (t.name(), t.as_ref())).collect();
        assert!(!file.probes.is_empty());
        for probe in &file.probes {
            run_probe(probe, &tool_map).await.expect(&probe.name);
        }
    }
}
