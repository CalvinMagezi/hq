use std::io;

/// Everything a backup or a restore can refuse or fail on.
#[derive(Debug, thiserror::Error)]
pub enum BackupError {
    /// The request itself is not acceptable: a target that is not empty, a missing file, a weak
    /// passphrase, an entry that tries to leave the folder it is restored into.
    #[error("{0}")]
    Refused(String),
    /// The key does not open the backup, or the first part of it was changed.
    #[error("the passphrase or key is wrong, or the backup was changed or damaged")]
    Decrypt,
    /// The backup opened but does not hold together: cut short, altered, a hash that differs.
    #[error("the backup is damaged: {0}")]
    Corrupt(String),
    /// The backup comes from a newer HQ than this one.
    #[error("{0}")]
    Newer(String),
    #[error("database error: {0}")]
    Database(String),
    #[error(transparent)]
    Io(#[from] io::Error),
}

pub type Result<T> = std::result::Result<T, BackupError>;

pub(crate) fn refused<T>(message: impl Into<String>) -> Result<T> {
    Err(BackupError::Refused(message.into()))
}

pub(crate) fn corrupt<T>(message: impl Into<String>) -> Result<T> {
    Err(BackupError::Corrupt(message.into()))
}
