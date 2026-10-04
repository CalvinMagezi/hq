use crate::types::SecurityProfile;
use serde::{Deserialize, Serialize};

// ── Tool Validation (distilled from claude-code) ──────────────────────────────

/// The outcome of validating tool arguments before execution.
///
/// Mirrors claude-code's `ValidationResult` union type. A failing result
/// carries a user-facing message and a numeric error code so callers can
/// pattern-match on specific failure modes without parsing strings.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "result", rename_all = "lowercase")]
pub enum ValidationResult {
    /// Arguments are valid; proceed with execution.
    #[serde(rename = "ok")]
    Ok,
    /// Arguments failed validation; do not execute.
    #[serde(rename = "err")]
    Err {
        /// User-facing description of what went wrong.
        message: String,
        /// Stable numeric code so callers can branch without parsing `message`.
        /// Convention (aligned with claude-code FileEditTool error codes):
        ///   0 = secrets / policy violation
        ///   1 = no-op (old == new, empty diff)
        ///   2 = path denied by security policy
        ///   3 = file already exists (create conflict)
        ///   4 = file does not exist
        ///   5 = unsupported file type
        ///   6 = file not yet read (staleness gate)
        ///   7 = file modified since last read
        ///   8 = search string not found in file
        ///   9 = multiple matches without replace_all flag
        ///  10 = file exceeds size limit / blocked pattern
        error_code: u32,
        /// How the caller should respond to this failure.
        behavior: ValidationBehavior,
    },
}

impl ValidationResult {
    /// Convenience constructor for the success variant.
    pub fn ok() -> Self {
        Self::Ok
    }

    /// Convenience constructor for a blocking failure.
    pub fn block(message: impl Into<String>, error_code: u32) -> Self {
        Self::Err {
            message: message.into(),
            error_code,
            behavior: ValidationBehavior::Block,
        }
    }

    /// Convenience constructor for an ask-first failure.
    pub fn ask(message: impl Into<String>, error_code: u32) -> Self {
        Self::Err {
            message: message.into(),
            error_code,
            behavior: ValidationBehavior::Ask,
        }
    }

    /// Returns `true` if validation passed.
    pub fn is_ok(&self) -> bool {
        matches!(self, Self::Ok)
    }
}

/// What the tool executor should do when validation fails.
///
/// `Block` means the tool must not run and the error is returned to the LLM.
/// `Ask` means the executor may prompt the user for permission before retrying.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ValidationBehavior {
    /// Abort execution and surface the error message to the LLM.
    Block,
    /// Pause and ask the user before proceeding (permission escalation).
    Ask,
}

// ── Permission Modes (distilled from claude-code) ─────────────────────────────

/// Interactive permission mode for a session, extending `SecurityProfile`.
///
/// Mirrors claude-code's `PermissionMode` enum. While `SecurityProfile` sets
/// the compile-time path/call-count floor, `PermissionMode` governs how the
/// agent interacts with the user at runtime when permission is ambiguous.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum PermissionMode {
    /// Prompt the user for each tool call that isn't explicitly allowed. (default)
    #[default]
    Default,
    /// Show a plan first; require explicit approval before any tool runs.
    Plan,
    /// Auto-approve read-only and non-destructive tools; prompt for writes.
    AcceptEdits,
    /// Auto-approve everything that passes path and call-limit checks (power-user mode).
    BypassPermissions,
    /// Never prompt; deny any tool that hasn't been pre-approved.
    DontAsk,
}

impl PermissionMode {
    /// Display name shown in UI / log output.
    pub fn title(&self) -> &'static str {
        match self {
            Self::Default => "Default",
            Self::Plan => "Plan",
            Self::AcceptEdits => "Accept Edits",
            Self::BypassPermissions => "Bypass Permissions",
            Self::DontAsk => "Don't Ask",
        }
    }
}

// ── Permission Presets (distilled from deepseek-harness's permission-presets) ─

/// A named bundle of `(SecurityProfile, PermissionMode)`.
///
/// `SecurityProfile` and `PermissionMode` are two independent knobs; most
/// callers only ever want one of a handful of well-known combinations. This
/// gives CLI flags and chat commands a single name to pick instead of two
/// raw enum values, mirroring dsh's named permission-preset table
/// (`read-only` / `workspace-write` / `danger-full-access`). Purely additive:
/// `ToolGuardian::new`/`with_default_mode` are unaffected.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum PermissionPreset {
    /// Read-only: no tool that isn't read-only may run. `Standard` profile
    /// (the minimum security floor) + `PermissionMode::DontAsk`.
    ReadOnly,
    /// Writes allowed within the governed workspace paths, auto-approved.
    /// `Guarded` profile (the default) + `PermissionMode::AcceptEdits`.
    WorkspaceWrite,
    /// Every governed check except path/call-limit is skipped. `Admin`
    /// profile + `PermissionMode::BypassPermissions`. Power-user only.
    DangerFullAccess,
}

impl PermissionPreset {
    /// Resolve to the `(SecurityProfile, PermissionMode)` pair this preset bundles.
    pub fn profile_and_mode(&self) -> (SecurityProfile, PermissionMode) {
        match self {
            Self::ReadOnly => (SecurityProfile::Standard, PermissionMode::DontAsk),
            Self::WorkspaceWrite => (SecurityProfile::Guarded, PermissionMode::AcceptEdits),
            Self::DangerFullAccess => (SecurityProfile::Admin, PermissionMode::BypassPermissions),
        }
    }

    /// The canonical kebab-case name used in CLI flags and chat commands.
    pub fn name(&self) -> &'static str {
        match self {
            Self::ReadOnly => "read-only",
            Self::WorkspaceWrite => "workspace-write",
            Self::DangerFullAccess => "danger-full-access",
        }
    }

    /// One-line human description, e.g. for a `/permission` command's help text.
    pub fn description(&self) -> &'static str {
        match self {
            Self::ReadOnly => "No writes: only read-only tools may run.",
            Self::WorkspaceWrite => "Writes within the workspace are auto-approved.",
            Self::DangerFullAccess => "All governed permission prompts are bypassed.",
        }
    }

    /// All presets, in the order they should be listed to a user.
    pub fn all() -> &'static [PermissionPreset] {
        &[Self::ReadOnly, Self::WorkspaceWrite, Self::DangerFullAccess]
    }

    /// Case-insensitive lookup by name, accepting common aliases
    /// (`readonly`, `read_only`, `danger`, `full-access`, `bypass`, …).
    pub fn from_name(name: &str) -> Option<Self> {
        match name.to_lowercase().replace(['_', ' '], "-").as_str() {
            "read-only" | "readonly" | "read" => Some(Self::ReadOnly),
            "workspace-write" | "workspacewrite" | "workspace" | "write" => {
                Some(Self::WorkspaceWrite)
            }
            "danger-full-access" | "dangerfullaccess" | "danger" | "full-access" | "bypass" => {
                Some(Self::DangerFullAccess)
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_preset_resolves_to_expected_profile_and_mode() {
        assert_eq!(
            PermissionPreset::ReadOnly.profile_and_mode(),
            (SecurityProfile::Standard, PermissionMode::DontAsk)
        );
        assert_eq!(
            PermissionPreset::WorkspaceWrite.profile_and_mode(),
            (SecurityProfile::Guarded, PermissionMode::AcceptEdits)
        );
        assert_eq!(
            PermissionPreset::DangerFullAccess.profile_and_mode(),
            (SecurityProfile::Admin, PermissionMode::BypassPermissions)
        );
    }

    #[test]
    fn permission_preset_from_name_accepts_canonical_and_alias_forms() {
        for name in ["read-only", "readonly", "read", "READ-ONLY"] {
            assert_eq!(
                PermissionPreset::from_name(name),
                Some(PermissionPreset::ReadOnly)
            );
        }
        for name in ["workspace-write", "workspace", "write"] {
            assert_eq!(
                PermissionPreset::from_name(name),
                Some(PermissionPreset::WorkspaceWrite)
            );
        }
        for name in ["danger-full-access", "danger", "bypass", "full-access"] {
            assert_eq!(
                PermissionPreset::from_name(name),
                Some(PermissionPreset::DangerFullAccess)
            );
        }
    }

    #[test]
    fn permission_preset_from_name_rejects_unknown_names() {
        assert_eq!(PermissionPreset::from_name("nonsense"), None);
        assert_eq!(PermissionPreset::from_name(""), None);
    }

    #[test]
    fn permission_preset_name_round_trips_through_from_name() {
        for preset in PermissionPreset::all() {
            assert_eq!(PermissionPreset::from_name(preset.name()), Some(*preset));
        }
    }
}
