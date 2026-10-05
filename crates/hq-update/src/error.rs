use thiserror::Error;

#[derive(Debug, Error)]
pub enum UpdateError {
    #[error("unsupported {what} schema {found} (this updater understands {supported})")]
    UnsupportedSchema {
        what: &'static str,
        found: u64,
        supported: u32,
    },
    #[error("invalid {what}: {reason}")]
    Invalid { what: &'static str, reason: String },
    #[error("signature check failed for {what}: {reason}")]
    Signature { what: String, reason: String },
    #[error("no trusted public key available: {0}")]
    NoPublicKey(String),
    #[error("checksum mismatch for {name}: expected {expected}, got {actual}")]
    ChecksumMismatch {
        name: String,
        expected: String,
        actual: String,
    },
    #[error("{name} exceeds the size limit of {limit} bytes")]
    SizeLimit { name: String, limit: u64 },
    #[error("refusing downgrade from {current} to {target} (use --pin or --rollback)")]
    Downgrade { current: String, target: String },
    #[error(
        "release {version} needs updater {required} or newer, this is {have}; install a newer hq by hand"
    )]
    UpdaterTooOld {
        version: String,
        required: String,
        have: String,
    },
    #[error("version {0} is blocked after a failed update; publish a newer release or use --pin")]
    Blocked(String),
    #[error(
        "release {version} has no {platform} binary; this platform is not published for that release"
    )]
    NoPlatformArtifact { version: String, platform: String },
    #[error(
        "hq update on macOS cannot stop the service before swapping files, so --apply and --rollback are disabled; set HQ_UPDATE_ALLOW_DARWIN=1 or allow_darwin = true to accept that, or install a new release by hand"
    )]
    DarwinNotAllowed,
    #[error("another update is already running")]
    Locked,
    #[error("unsafe archive: {0}")]
    UnsafeArchive(String),
    #[error("staged binary failed its sanity check: {0}")]
    StagedBinary(String),
    #[error("nothing to roll back to")]
    NoRollbackTarget,
    #[error("download failed: {0}")]
    Download(String),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

impl From<std::io::Error> for UpdateError {
    fn from(e: std::io::Error) -> Self {
        UpdateError::Other(e.into())
    }
}

pub type Result<T> = std::result::Result<T, UpdateError>;
