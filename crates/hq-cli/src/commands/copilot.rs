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

    let timeout = config.github_copilot.timeout_secs.min(60);
    let pick = pick_first_usable(&preference, |m| {
        let binary = binary.clone();
        async move {
            let args = probe_args(&m);
            let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
            hq_tools::run_external_cli_harness_strict(
                &binary,
                &arg_refs,
                "Reply with the single word: ok",
                None,
                timeout,
            )
            .await
            .map(|_| ())
            .map_err(|e| format!("{e:#}"))
        }
    })
    .await;

    match pick {
        Pick::Chosen { model, skipped } => {
            for (m, why) in &skipped {
                println!("  {m}: {}", describe(*why));
            }
            println!("  {model}: answered");
            println!("\nLinked model: {model}");
            let path = HqConfig::config_file_path();
            if write {
                apply(&path, &model)?;
                println!("Wrote {} (the previous file is kept as config.yaml.bak).", path.display());
            } else {
                println!("\nAdd this to {} (or rerun with --write):\n", path.display());
                println!("{}", snippet(&model));
            }
            Ok(())
        }
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

fn describe(why: ProbeFailure) -> &'static str {
    match why {
        ProbeFailure::ModelUnavailable => "not available on this seat",
        ProbeFailure::NotSignedIn => "not signed in",
        ProbeFailure::Other => "no answer (timeout or error)",
    }
}

fn snippet(model: &str) -> String {
    let doc = merged(Value::Mapping(Mapping::new()), model);
    serde_yaml::to_string(&doc).unwrap_or_default()
}

/// `doc` with the Copilot backend added and `github_copilot.model` set. An existing `primary`
/// is left alone; only an empty one becomes the new backend.
fn merged(doc: Value, model: &str) -> Value {
    fn map(v: &mut Value) -> &mut Mapping {
        if !v.is_mapping() {
            *v = Value::Mapping(Mapping::new());
        }
        v.as_mapping_mut().expect("just made a mapping")
    }
    let key = |s: &str| Value::String(s.to_string());
    let mut doc = doc;
    let root = map(&mut doc);

    let gh = root.entry(key("github_copilot")).or_insert(Value::Null);
    map(gh).insert(key("model"), key(model));

    let backends = root.entry(key("backends")).or_insert(Value::Null);
    let backends = map(backends);
    let list = backends.entry(key("backends")).or_insert(Value::Sequence(vec![]));
    if !list.is_sequence() {
        *list = Value::Sequence(vec![]);
    }
    let list = list.as_sequence_mut().expect("just made a sequence");
    let existing = list
        .iter_mut()
        .find(|e| e.get("name").and_then(Value::as_str) == Some(BACKEND_NAME));
    match existing {
        Some(entry) => {
            let m = map(entry);
            m.insert(key("kind"), key("github-copilot-cli"));
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
    doc
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
    let out = serde_yaml::to_string(&merged(doc, model))?;
    if let Some(parent) = path.parent() {
        hq_core::fs_private::create_private_dir_all(parent)?;
    }
    if !current.is_empty() {
        hq_core::fs_private::write_private(&path.with_extension("yaml.bak"), &current)?;
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
        let v = merged(Value::Mapping(Mapping::new()), "gpt-6-luna");
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
        let v = merged(existing, "claude-haiku-5.5");
        let entries = backends_of(&v);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[1].model.as_deref(), Some("claude-haiku-5.5"));
        assert!(entries[1].enabled);
        assert_eq!(v["backends"]["primary"].as_str(), Some("work"), "a chosen primary stays");
        assert_eq!(v["vault_path"].as_str(), Some("/v"));
    }

    #[test]
    fn writing_keeps_a_backup_and_the_result_loads_as_a_config() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        std::fs::write(&path, "default_model: x\n").unwrap();
        apply(&path, "gpt-6-luna").unwrap();
        assert_eq!(std::fs::read_to_string(dir.path().join("config.yaml.bak")).unwrap(), "default_model: x\n");
        let doc: Value = serde_yaml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let backends: hq_core::config::BackendsConfig = serde_yaml::from_value(doc["backends"].clone()).unwrap();
        assert!(backends.is_configured());
        assert_eq!(doc["default_model"].as_str(), Some("x"));
        assert_eq!(doc["github_copilot"]["model"].as_str(), Some("gpt-6-luna"));
    }
}
