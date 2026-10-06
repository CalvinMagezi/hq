use std::time::Duration;

#[derive(Debug, thiserror::Error)]
pub enum HostError {
    #[error("no agent named '{0}'")]
    NotFound(String),
    #[error("an agent named '{0}' already exists")]
    NameTaken(String),
    #[error("invalid agent name '{0}': use [a-z][a-z0-9_-] and at most 32 characters")]
    InvalidName(String),
    #[error("invalid key {0:?}: use logical names such as enter, esc, down or ctrl+c")]
    InvalidKey(String),
    #[error("could not start '{command}': {detail}")]
    Spawn { command: String, detail: String },
    #[error("invalid size {rows}x{cols}: rows and columns must each be 1 to {max}")]
    InvalidSize { rows: u16, cols: u16, max: u16 },
    #[error("resume_argv must not be empty")]
    InvalidResume,
    #[error("i/o error: {0}")]
    Io(String),
    #[error("timed out after {0:?}")]
    Timeout(Duration),
    #[error("'{0}' has exited")]
    Exited(String),
}

impl HostError {
    /// Stable machine-readable code, shared with the control API later.
    pub fn code(&self) -> &'static str {
        match self {
            HostError::NotFound(_) => "agent_not_found",
            HostError::NameTaken(_) => "name_taken",
            HostError::InvalidName(_) => "invalid_name",
            HostError::InvalidKey(_) => "invalid_keys",
            HostError::Spawn { .. } => "spawn_failed",
            HostError::InvalidSize { .. } => "invalid_size",
            HostError::InvalidResume => "invalid_resume",
            HostError::Io(_) => "io",
            HostError::Timeout(_) => "timeout",
            HostError::Exited(_) => "agent_exited",
        }
    }
}
