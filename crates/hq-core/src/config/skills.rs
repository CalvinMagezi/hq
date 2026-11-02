//! `skills:` — what the owner decides about the skills HQ ships.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillsConfig {
    #[serde(default)]
    pub builtin: BuiltinSkillsConfig,
}

/// The owner's alone: `config_manage` refuses the `skills` key and agents cannot write the config.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuiltinSkillsConfig {
    /// Built-in skills to leave out. To change one, copy it under another name and list it here.
    #[serde(default)]
    pub off: Vec<String>,
}
