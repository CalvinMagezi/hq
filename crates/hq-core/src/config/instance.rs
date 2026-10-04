use serde::{Deserialize, Serialize};

use super::default_true;

/// Whether this HQ instance is running locally or in the cloud.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "lowercase")]
pub enum InstanceType {
    #[default]
    Local,
    Cloud,
}

/// Feature flags for this instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstanceFeatures {
    /// Whether local Ollama models are available.
    #[serde(default = "default_true")]
    pub local_ollama: bool,
}

impl Default for InstanceFeatures {
    fn default() -> Self {
        Self {
            local_ollama: true,
        }
    }
}

impl InstanceFeatures {
    /// Features for a cloud/VPS instance (no local hardware).
    pub fn cloud() -> Self {
        Self {
            local_ollama: false,
        }
    }
}

/// Instance-specific configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstanceConfig {
    /// Instance type (local or cloud).
    #[serde(default)]
    pub instance_type: InstanceType,
    /// Available features on this instance.
    #[serde(default)]
    pub features: InstanceFeatures,
}

impl Default for InstanceConfig {
    fn default() -> Self {
        Self {
            instance_type: InstanceType::Local,
            features: InstanceFeatures::default(),
        }
    }
}
