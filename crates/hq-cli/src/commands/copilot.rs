//! `hq copilot link`: point HQ at the GitHub Copilot seat this machine is signed in to.
//!
//! The route is GitHub's own CLI (`gh copilot`), the `github-copilot-cli` backend. It signs in
//! the way the CLI does, HQ never sees the GitHub token, and under `profile: lite` it is the one
//! model route accepted without a listing. Which models the seat offers is found by using
//! them: the first entry of `github_copilot.model_preference` that answers a one-word request
//! wins, and a seat with none of them gets a report instead of a bigger model.

use std::path::Path;

use anyhow::{Context, Result, bail};
use hq_core::config::HqConfig;
use hq_llm::copilot_pick::{Pick, ProbeFailure, pick_first_usable};
use serde_yaml::{Mapping, Value};

/// Name of the backend entry `link` writes.
const BACKEND_NAME: &str = "copilot";

pub async fn run(config: &HqConfig, sub: &str, model: Option<&str>, write: bool) -> Result<()> {
    match sub {
        "link" => link(config, model, write).await,
        other => bail!("unknown subcommand `{other}`; use `hq copilot link [--model M] [--write]`"),
    }
}

async fn link(config: &HqConfig, model: Option<&str>, write: bool) -> Result<()> {
    let Some(binary) = hq_core::machine::which_binary("gh") else {
        bail!(
            "the GitHub CLI (`gh`) is not on PATH. Install it from https://cli.github.com, run \
             `gh auth login` with your work account, and try again."
        );
    };
    let preference: Vec<String> = match model {
        Some(m) => vec![m.to_string()],
        None => config.github_copilot.model_preference.clone(),
    };
    if preference.iter().all(|m| m.trim().is_empty()) {
        bail!("no model to try: set github_copilot.model_preference or pass --model");
    }
    println!("Checking your Copilot seat through `gh copilot` (this sends one short test request per model).");

    let timeout = config.github_copilot.timeout_secs.clamp(10, 60);
    // An empty directory, so the CLI has no project files, instructions or git state to send.
    let probe_dir = std::env::temp_dir().join(format!("hq-copilot-probe-{}", std::process::id()));
    std::fs::create_dir_all(&probe_dir).context("creating the probe directory")?;
    let pick = pick_first_usable(&preference, |m| {
        let binary = binary.clone();
        let probe_dir = probe_dir.clone();
        async move {
            let args = probe_args(&m);
            let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
            hq_tools::run_external_cli_harness_strict(
                &binary,
                &arg_refs,
                "Reply with the single word: ok",
                Some(probe_dir.as_path()),
                timeout,
            )
            .await
            .map_err(|e| format!("{e:#}"))
            .and_then(|out| {
                if out.to_ascii_lowercase().contains("ok") {
                    Ok(())
                } else {
                    Err(format!("unexpected reply: {}", out.trim().chars().take(200).collect::<String>()))
                }
            })
        }
    })
    .await;
    let _ = std::fs::remove_dir(&probe_dir);

    match pick {
        Pick::Chosen { model, skipped } => {
            for (m, why) in &skipped {
                println!("  {m}: {}", describe(*why));
            }
            println!("  {model}: answered");
            println!("\nLinked model: {model}");
            let path = target_path();
            if write {
                apply(&path, &model)?;
                println!("Wrote {} (the first version is kept as config.yaml.bak; comments are not preserved).", path.display());
            } else {
                println!("\nAdd this to {} (or rerun with --write):\n", path.display());
                println!("{}", snippet(&model));
            }
            Ok(())
        }
        Pick::Inconclusive { model, detail } => bail!(
            "could not tell whether {model} is available: {}. Nothing was changed; run it again, or \
             name a model with --model.",
            detail.trim().chars().take(300).collect::<String>()
        ),
        Pick::NotSignedIn { model } => bail!(
            "`gh copilot` is not signed in (while trying {model}). Run `gh auth login` with the \
             account that has the Copilot seat, then run this again."
        ),
        Pick::NoneUsable { tried } => {
            for (m, why) in &tried {
                println!("  {m}: {}", describe(*why));
            }
            bail!(
                "none of the preferred models answered on this seat. Ask your administrator which \
                 Copilot models are enabled, then run `hq copilot link --model <id>`. HQ does not \
                 pick a different model for you."
            )
        }
    }
}

/// The headless invocation the `github-copilot-cli` backend uses, without tool permissions.
fn probe_args(model: &str) -> Vec<String> {
    ["copilot", "--", "-s", "--no-ask-user", "--model", model, "-p"]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

/// Where `--write` goes: the file `load()` reads when there is one, so a deployed config is not
/// shadowed by a new per-user file; otherwise the per-user path.
fn target_path() -> std::path::PathBuf {
    let read = HqConfig::config_read_path();
    if read.exists() { read } else { HqConfig::config_file_path() }
}

fn describe(why: ProbeFailure) -> &'static str {
    match why {
        ProbeFailure::ModelUnavailable => "not available on this seat",
        ProbeFailure::NotSignedIn => "not signed in",
        ProbeFailure::Other => "no answer (timeout or error)",
    }
}

fn snippet(model: &str) -> String {
    let doc = merged(Value::Mapping(Mapping::new()), model).unwrap_or(Value::Null);
    serde_yaml::to_string(&doc).unwrap_or_default()
}

/// `doc` with the Copilot backend added and `github_copilot.model` set. An existing `primary`
/// is left alone; only an empty one becomes the new backend.
fn merged(doc: Value, model: &str) -> Result<Value> {
    /// The mapping at `v`, creating it when absent (null); anything else is the user's to fix.
    fn map<'a>(v: &'a mut Value, what: &str) -> Result<&'a mut Mapping> {
        if v.is_null() {
            *v = Value::Mapping(Mapping::new());
        }
        v.as_mapping_mut()
            .ok_or_else(|| anyhow::anyhow!("`{what}` in the config is not a mapping; fix it first"))
    }
    let key = |s: &str| Value::String(s.to_string());
    let mut doc = doc;
    let root = map(&mut doc, "the config root")?;

    let gh = root.entry(key("github_copilot")).or_insert(Value::Null);
    map(gh, "github_copilot")?.insert(key("model"), key(model));

    let backends = root.entry(key("backends")).or_insert(Value::Null);
    let backends = map(backends, "backends")?;
    let list = backends.entry(key("backends")).or_insert(Value::Null);
    if list.is_null() {
        *list = Value::Sequence(vec![]);
    }
    let list = list
        .as_sequence_mut()
        .ok_or_else(|| anyhow::anyhow!("`backends.backends` in the config is not a list; fix it first"))?;
    let existing = list
        .iter_mut()
        .find(|e| e.get("name").and_then(Value::as_str) == Some(BACKEND_NAME));
    match existing {
        Some(entry) => {
            let m = map(entry, "the `copilot` backend")?;
            if m.get("kind").and_then(Value::as_str) != Some("github-copilot-cli") {
                bail!(
                    "the config already has a backend named `{BACKEND_NAME}` of another kind; \
                     rename or remove it first"
                );
            }
            m.insert(key("model"), key(model));
            m.insert(key("enabled"), Value::Bool(true));
        }
        None => {
            let mut m = Mapping::new();
            m.insert(key("name"), key(BACKEND_NAME));
            m.insert(key("kind"), key("github-copilot-cli"));
            m.insert(key("model"), key(model));
            m.insert(key("enabled"), Value::Bool(true));
            list.push(Value::Mapping(m));
        }
    }
    let primary_empty = backends
        .get("primary")
        .and_then(Value::as_str)
        .is_none_or(|p| p.trim().is_empty());
    if primary_empty {
        backends.insert(key("primary"), key(BACKEND_NAME));
    }
    Ok(doc)
}

fn apply(path: &Path, model: &str) -> Result<()> {
    let current = if path.exists() {
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?
    } else {
        String::new()
    };
    let doc: Value = if current.trim().is_empty() {
        Value::Mapping(Mapping::new())
    } else {
        serde_yaml::from_str(&current).with_context(|| format!("{} is not valid YAML", path.display()))?
    };
    let doc = merged(doc, model)?;
    let out = serde_yaml::to_string(&doc)?;
    if let Some(parent) = path.parent() {
        hq_core::fs_private::create_private_dir_all(parent)?;
    }
    let backup = path.with_extension("yaml.bak");
    if !current.is_empty() && !backup.exists() {
        hq_core::fs_private::write_private(&backup, &current)?;
    }
    hq_core::fs_private::write_private(path, out)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use hq_core::config::{BackendEntry, BackendKind};

    fn backends_of(v: &Value) -> Vec<BackendEntry> {
        serde_yaml::from_value(v["backends"]["backends"].clone()).unwrap()
    }

    #[test]
    fn an_empty_config_gets_a_working_copilot_backend() {
        let v = merged(Value::Mapping(Mapping::new()), "gpt-6-luna").unwrap();
        let entries = backends_of(&v);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].kind, BackendKind::GithubCopilotCli);
        assert_eq!(entries[0].model.as_deref(), Some("gpt-6-luna"));
        assert_eq!(v["backends"]["primary"].as_str(), Some("copilot"));
        assert_eq!(v["github_copilot"]["model"].as_str(), Some("gpt-6-luna"));
    }

    #[test]
    fn relinking_updates_in_place_and_keeps_other_backends_and_the_primary() {
        let existing: Value = serde_yaml::from_str(
            "backends:\n  primary: work\n  backends:\n    - {name: work, kind: openrouter}\n    - {name: copilot, kind: github-copilot-cli, model: old, enabled: false}\nvault_path: /v\n",
        )
        .unwrap();
        let v = merged(existing, "claude-haiku-5.5").unwrap();
        let entries = backends_of(&v);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[1].model.as_deref(), Some("claude-haiku-5.5"));
        assert!(entries[1].enabled);
        assert_eq!(v["backends"]["primary"].as_str(), Some("work"), "a chosen primary stays");
        assert_eq!(v["vault_path"].as_str(), Some("/v"));
    }

    #[test]
    fn another_kind_under_the_same_name_or_a_malformed_config_is_refused() {
        let other: Value = serde_yaml::from_str("backends:\n  backends:\n    - {name: copilot, kind: openrouter}\n").unwrap();
        assert!(merged(other, "m").is_err());
        let bad: Value = serde_yaml::from_str("backends: [1]\n").unwrap();
        assert!(merged(bad, "m").is_err());
        assert!(merged(Value::String("x".into()), "m").is_err());
    }

    #[test]
    fn writing_keeps_a_backup_and_the_result_loads_as_a_config() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        std::fs::write(&path, "default_model: x\n").unwrap();
        apply(&path, "gpt-6-luna").unwrap();
        apply(&path, "claude-haiku-5.5").unwrap();
        assert_eq!(std::fs::read_to_string(dir.path().join("config.yaml.bak")).unwrap(), "default_model: x\n", "the first backup survives a second write");
        let doc: Value = serde_yaml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let backends: hq_core::config::BackendsConfig = serde_yaml::from_value(doc["backends"].clone()).unwrap();
        assert!(backends.is_configured());
        assert_eq!(doc["default_model"].as_str(), Some("x"));
        assert_eq!(doc["github_copilot"]["model"].as_str(), Some("claude-haiku-5.5"));
    }
}
