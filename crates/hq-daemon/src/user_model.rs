//! Dialectic User Model — `_system/USER.md`.
//!
//! A structured, confidence-tagged user profile. The notification gate reads
//! and updates it; the operator can also edit it by hand.
//!
//! Storage layout (YAML frontmatter in `.vault/_system/USER.md`):
//!
//! ```yaml
//! ---
//! notif.github.ci.failed: { value: "urgent", confidence: 0.9, last_updated: "..." }
//! ...
//! ---
//! # Inferred Notes
//! (free-form LLM observations)
//! ```
//!
//! Every trait carries `confidence` in `[0.0, 1.0]` and an ISO timestamp. A
//! higher-confidence value replaces a lower one; on ties, the newer timestamp
//! wins, so every writer gets the same merge semantics.

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const USER_MD: &str = "_system/USER.md";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserTrait {
    #[serde(default)]
    pub value: Value,
    #[serde(default)]
    pub confidence: f64,
    #[serde(default)]
    pub last_updated: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UserModel {
    /// Sorted by key for deterministic rendering.
    #[serde(flatten)]
    pub traits: BTreeMap<String, UserTrait>,
    /// Free-form LLM-authored observations appended to the body of USER.md.
    #[serde(skip)]
    pub notes: String,
}

impl UserModel {
    /// Merge a single-trait update using the resolution rules described in the
    /// module docstring. Returns true if the incoming update won.
    pub fn merge(&mut self, key: &str, incoming: UserTrait) -> bool {
        let existing = self.traits.get(key);
        let take = match existing {
            None => true,
            Some(e) if incoming.confidence > e.confidence + f64::EPSILON => true,
            Some(e)
                if (incoming.confidence - e.confidence).abs() < f64::EPSILON
                    && incoming.last_updated > e.last_updated =>
            {
                true
            }
            _ => false,
        };
        if take {
            self.traits.insert(key.to_string(), incoming);
        }
        take
    }
}

fn user_md_path(vault_path: &Path) -> PathBuf {
    vault_path.join(USER_MD)
}

pub fn load_user_model(vault_path: &Path) -> Result<UserModel> {
    let path = user_md_path(vault_path);
    if !path.exists() {
        return Ok(UserModel::default());
    }
    let raw = std::fs::read_to_string(&path)?;
    let matter = gray_matter::Matter::<gray_matter::engine::YAML>::new();
    let result = matter.parse(&raw);
    let traits = match result.data.as_ref() {
        Some(gray_matter::Pod::Hash(map)) => map
            .iter()
            .filter_map(|(k, v)| pod_to_trait(v).map(|t| (k.clone(), t)))
            .collect::<BTreeMap<_, _>>(),
        _ => BTreeMap::new(),
    };
    Ok(UserModel {
        traits,
        notes: result.content,
    })
}

fn pod_to_trait(pod: &gray_matter::Pod) -> Option<UserTrait> {
    // Serialize the Pod through serde_json so we reuse our UserTrait derive.
    let json = pod_to_json(pod)?;
    serde_json::from_value::<UserTrait>(json).ok()
}

fn pod_to_json(pod: &gray_matter::Pod) -> Option<Value> {
    match pod {
        gray_matter::Pod::Null => Some(Value::Null),
        gray_matter::Pod::String(s) => Some(Value::String(s.clone())),
        gray_matter::Pod::Integer(i) => Some(Value::from(*i)),
        gray_matter::Pod::Float(f) => serde_json::Number::from_f64(*f).map(Value::Number),
        gray_matter::Pod::Boolean(b) => Some(Value::Bool(*b)),
        gray_matter::Pod::Array(arr) => {
            Some(Value::Array(arr.iter().filter_map(pod_to_json).collect()))
        }
        gray_matter::Pod::Hash(map) => {
            let mut obj = serde_json::Map::new();
            for (k, v) in map {
                if let Some(jv) = pod_to_json(v) {
                    obj.insert(k.clone(), jv);
                }
            }
            Some(Value::Object(obj))
        }
    }
}

pub fn save_user_model(vault_path: &Path, model: &UserModel) -> Result<()> {
    let path = user_md_path(vault_path);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut rendered = String::from("---\n");
    for (key, t) in &model.traits {
        let line = serde_yaml::to_string(&serde_json::json!({ key: t })).unwrap_or_default();
        rendered.push_str(&line);
    }
    rendered.push_str("---\n\n");
    if model.notes.trim().is_empty() {
        rendered.push_str("# Inferred Notes\n\n");
        rendered.push_str("The daemon refines this profile over time. Higher-confidence ");
        rendered.push_str("values replace lower ones; ties favor the newer timestamp.\n");
    } else {
        rendered.push_str(&model.notes);
    }
    std::fs::write(&path, rendered)?;
    Ok(())
}

/// Relay callback for the keep/mute reaction on a notification:
/// bump the targeted trait with the reply text at confidence 0.9.
pub fn apply_user_reply(vault_path: &Path, trait_key: &str, reply_text: &str) -> Result<()> {
    let mut model = load_user_model(vault_path)?;
    let incoming = UserTrait {
        value: Value::String(reply_text.trim().to_string()),
        confidence: 0.9,
        last_updated: Some(Utc::now()),
    };
    model.merge(trait_key, incoming);
    save_user_model(vault_path, &model)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn merge_prefers_higher_confidence() {
        let mut m = UserModel::default();
        let before = UserTrait {
            value: Value::String("A".into()),
            confidence: 0.3,
            last_updated: Some(Utc::now()),
        };
        m.merge("working_hours", before);

        let after = UserTrait {
            value: Value::String("B".into()),
            confidence: 0.8,
            last_updated: Some(Utc::now()),
        };
        assert!(m.merge("working_hours", after));
        assert_eq!(m.traits["working_hours"].value, Value::String("B".into()));
    }

    #[test]
    fn merge_drops_lower_confidence() {
        let mut m = UserModel::default();
        let strong = UserTrait {
            value: Value::String("A".into()),
            confidence: 0.9,
            last_updated: Some(Utc::now()),
        };
        m.merge("working_hours", strong);

        let weak = UserTrait {
            value: Value::String("B".into()),
            confidence: 0.4,
            last_updated: Some(Utc::now()),
        };
        assert!(!m.merge("working_hours", weak));
        assert_eq!(m.traits["working_hours"].value, Value::String("A".into()));
    }

    #[test]
    fn roundtrip_save_load_preserves_traits() {
        let tmp = tempdir().unwrap();
        let vault = tmp.path();
        let mut m = UserModel::default();
        m.merge(
            "working_hours",
            UserTrait {
                value: Value::String("09:00-18:00".into()),
                confidence: 0.85,
                last_updated: Some(Utc::now()),
            },
        );
        save_user_model(vault, &m).unwrap();

        let loaded = load_user_model(vault).unwrap();
        let wh = &loaded.traits["working_hours"];
        assert!(matches!(&wh.value, Value::String(s) if s == "09:00-18:00"));
        assert!((wh.confidence - 0.85).abs() < 1e-6);
    }

    #[test]
    fn apply_user_reply_bumps_trait_to_high_confidence() {
        let tmp = tempdir().unwrap();
        let vault = tmp.path();
        apply_user_reply(vault, "working_hours", "09:00-17:30 EAT").unwrap();
        let m = load_user_model(vault).unwrap();
        assert_eq!(
            m.traits["working_hours"].value,
            Value::String("09:00-17:30 EAT".into())
        );
        assert!((m.traits["working_hours"].confidence - 0.9).abs() < 1e-6);
    }
}
