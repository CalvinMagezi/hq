use std::time::Duration;

#[derive(Debug, thiserror::Error)]
pub enum HostError {
    #[error("cannot sandbox the agent: {0}")]
    Sandbox(String),
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
    #[error("{0} is too large")]
    TooLarge(&'static str),
    #[error("the host is running as many agents as it allows ({0})")]
    TooManyAgents(usize),
    #[error(
        "the host is locked down and starts no agent ({0}); only `hq host unlock` on that machine lifts it"
    )]
    LockedDown(String),
    #[error("'{name}' needs these variables to resume: {missing}")]
    MissingEnv { name: String, missing: String },
    #[error(
        "'{0}' is waiting at a dialog, so the text was not typed: answer the dialog with keys first"
    )]
    Blocked(String),
    #[error("'{0}' is not waiting at a dialog this host can read")]
    NoDialog(String),
    #[error(
        "'{name}' is at a different dialog now (number {current}); read it again before answering"
    )]
    StaleDialog { name: String, current: u64 },
    #[error("the dialog has no option {0}")]
    NoSuchOption(u32),
    #[error(
        "option {0} opens a text field instead of answering; decline the dialog and send the message as a prompt"
    )]
    TypedOption(u32),
    #[error("could not tell which option the cursor is on, so nothing was pressed")]
    NoCursor,
    #[error("the cursor did not reach option {0}, so Enter was not pressed; read the dialog again")]
    AnswerUnverified(u32),
}

impl HostError {
    /// Stable machine-readable code, shared with the control API later.
    pub fn code(&self) -> &'static str {
        match self {
            HostError::Sandbox(_) => "sandbox_unavailable",
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
            HostError::TooLarge(_) => "too_large",
            HostError::TooManyAgents(_) => "too_many_agents",
            HostError::MissingEnv { .. } => "missing_env",
            HostError::LockedDown(_) => "locked_down",
            HostError::Blocked(_) => "agent_blocked",
            HostError::NoDialog(_) => "no_dialog",
            HostError::StaleDialog { .. } => "stale_dialog",
            HostError::NoSuchOption(_) => "invalid_option",
            HostError::TypedOption(_) => "typed_option",
            HostError::NoCursor => "no_cursor",
            HostError::AnswerUnverified(_) => "answer_unverified",
        }
    }
}
