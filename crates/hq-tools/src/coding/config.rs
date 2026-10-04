use anyhow::Result;
use async_trait::async_trait;
use serde_json::{Value, json};

use crate::registry::HqTool;

pub struct ConfigTool;

const MAX_KEY_CHARS: usize = 64;
const MAX_VALUE_CHARS: usize = 500;

/// A top-level key made of plain identifier characters, so it can never carry YAML structure.
fn check_key(key: &str) -> Result<()> {
    let plain = !key.is_empty()
        && key.chars().count() <= MAX_KEY_CHARS
        && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if !plain {
        anyhow::bail!("key must be 1 to {MAX_KEY_CHARS} letters, digits or underscores");
    }
    Ok(())
}

/// One line of YAML for `value`: no control characters, quoted by the YAML serializer when needed.
fn yaml_scalar(value: &str) -> Result<String> {
    if value.chars().any(char::is_control) || value.chars().count() > MAX_VALUE_CHARS {
        anyhow::bail!("value must be one line of at most {MAX_VALUE_CHARS} characters with no control characters");
    }
    Ok(serde_yaml::to_string(value)?.trim_end().to_string())
}

/// Whether `line` is the top-level entry for exactly `key`.
fn is_entry(line: &str, key: &str) -> bool {
    line.split_once(':').is_some_and(|(k, _)| k == key)
}

#[async_trait]
impl HqTool for ConfigTool {
    fn name(&self) -> &str {
        "config_manage"
    }

    fn description(&self) -> &str {
        "Get or set Agent HQ configuration values. Supports theme, vim_mode, \
         brief_mode, and any key in ~/.hq/config.yaml."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "required": ["action"],
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["get", "set", "list"],
                    "description": "Action: get a value, set a value, or list all config"
                },
                "key": {
                    "type": "string",
                    "description": "Config key (e.g., 'theme', 'vim_mode', 'brief_mode')"
                },
                "value": {
                    "type": "string",
                    "description": "Value to set"
                }
            }
        })
    }

    fn category(&self) -> &str {
        "config"
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let action = args["action"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing action"))?;

        let config_path = dirs::home_dir()
            .map(|h| h.join(".hq/config.yaml"))
            .ok_or_else(|| anyhow::anyhow!("cannot determine home directory"))?;

        match action {
            "list" => {
                if config_path.exists() {
                    let content = tokio::fs::read_to_string(&config_path).await?;
                    Ok(json!({"config": content}))
                } else {
                    Ok(
                        json!({"config": "# No config file found", "path": config_path.display().to_string()}),
                    )
                }
            }
            "get" => {
                let key = args["key"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("missing key"))?;
                check_key(key)?;
                if config_path.exists() {
                    let content = tokio::fs::read_to_string(&config_path).await?;
                    for line in content.lines().filter(|l| is_entry(l, key)) {
                        if let Some((_, value)) = line.split_once(':') {
                            return Ok(json!({"key": key, "value": value.trim()}));
                        }
                    }
                    Ok(json!({"key": key, "value": null, "note": "key not found"}))
                } else {
                    Ok(json!({"key": key, "value": null, "note": "no config file"}))
                }
            }
            "set" => {
                let key = args["key"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("missing key"))?;
                let value = args["value"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("missing value"))?;
                check_key(key)?;
                let scalar = yaml_scalar(value)?;

                if let Some(parent) = config_path.parent() {
                    tokio::fs::create_dir_all(parent).await?;
                }

                let mut content = if config_path.exists() {
                    tokio::fs::read_to_string(&config_path).await?
                } else {
                    String::new()
                };

                let key_line = format!("{key}: {scalar}");
                let mut found = false;
                let lines: Vec<String> = content
                    .lines()
                    .map(|line| {
                        if is_entry(line, key) {
                            found = true;
                            key_line.clone()
                        } else {
                            line.to_string()
                        }
                    })
                    .collect();

                content = lines.join("\n");
                if !found {
                    if !content.is_empty() && !content.ends_with('\n') {
                        content.push('\n');
                    }
                    content.push_str(&key_line);
                    content.push('\n');
                }

                hq_core::fs_private::write_private(&config_path, &content)?;
                Ok(json!({"success": true, "key": key, "value": value}))
            }
            _ => Ok(json!({"error": format!("unknown action: {}", action)})),
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_plain_and_exact_and_values_stay_on_one_line() {
        assert!(check_key("theme").is_ok());
        for bad in ["", "a b", "a:b", "x\nweb_token", "a.b", "../x"] {
            assert!(check_key(bad).is_err(), "{bad:?}");
        }
        assert!(yaml_scalar("a\nweb_allowed_origins: [\"*\"]").is_err());
        assert!(yaml_scalar("a\rb").is_err());
        assert_eq!(yaml_scalar("dark").unwrap(), "dark");
        assert_eq!(yaml_scalar("x: y # z").unwrap(), "'x: y # z'", "structure is quoted, not interpreted");
        assert!(is_entry("theme: dark", "theme"));
        assert!(!is_entry("theme_extra: 1", "theme"), "exact keys, not prefixes");
        assert!(!is_entry("  theme: 1", "theme"), "nested entries are not top-level");
    }
}
