//! Where an original goes when it leaves the phone: anywhere an object can be put and fetched by
//! the SHA-256 of its bytes. The shrink pipeline knows only this trait; the sinks (a folder, later
//! another HQ) and the sealing live in `hq-suite-sinks`.

use std::fmt;

/// A sink could not do what was asked. The text is for the person.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColdError(pub String);

impl fmt::Display for ColdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ColdError {}

pub type ColdResult<T> = std::result::Result<T, ColdError>;

pub trait ColdStore: Send + Sync {
    /// The sink's id, as the note and the journal name it.
    fn id(&self) -> &str;

    /// Stores `bytes`, which hash to `sha256`, and does not return until they have been read back
    /// and checked. Storing what is already there is fine and does nothing.
    fn put(&self, sha256: &str, bytes: &[u8]) -> ColdResult<()>;

    /// The bytes filed under `sha256`, checked against it.
    fn get(&self, sha256: &str) -> ColdResult<Vec<u8>>;

    /// Whether `sha256` is there. A sink that cannot be reached says no.
    fn has(&self, sha256: &str) -> bool;
}
