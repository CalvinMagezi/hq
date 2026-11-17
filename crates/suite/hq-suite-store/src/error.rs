use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("file error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// A path that is absolute, climbs out of the vault, or passes through a link.
    #[error("`{0}` is not a safe path inside the vault")]
    UnsafePath(String),
    #[error("{0} already exists")]
    Exists(String),
    #[error("{0} does not exist")]
    Missing(String),
    /// Bytes on disk do not match the hash they are filed under.
    #[error("{0} does not match its hash")]
    Corrupt(String),
    #[error("journal: {0}")]
    Journal(String),
}

impl StoreError {
    pub(crate) fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        StoreError::Io {
            path: path.into(),
            source,
        }
    }
}

pub type Result<T> = std::result::Result<T, StoreError>;
