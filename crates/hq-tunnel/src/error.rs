//! One error type: a refusal with a reason a person can act on, or an I/O failure.

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Invalid(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

pub(crate) fn invalid<T>(reason: impl Into<String>) -> Result<T> {
    Err(Error::Invalid(reason.into()))
}
