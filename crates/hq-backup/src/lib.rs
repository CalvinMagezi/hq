//! Encrypted backup and restore of an HQ vault and its database.
//!
//! One file holds the vault's notes, a consistent snapshot of the database and, if asked for, the
//! config file. It is compressed and then sealed with ChaCha20-Poly1305 under a key made from a
//! passphrase (Argon2id) or read from a key file, and it lists its own contents with hashes.
//! Nothing here touches the network or runs another program, so it works the same on a server, a
//! laptop and a phone. See `docs/BACKUP.md`.

mod archive;
mod crypto;
mod error;
mod fsutil;
mod key;
mod retention;
mod snapshot;
pub mod status;

pub use archive::{
    BackupOptions, BackupReport, DB_RELATIVE, Entry, MANIFEST_FORMAT, Manifest, RestoreOptions,
    RestoreReport, create, restore, verify, verify_limited,
};
pub use error::{BackupError, Result};
pub use key::{Argon2Cost, Secret, read_private_file};
pub use retention::{EXTENSION, file_name, is_backup_name, list, newest, prune, sweep_partials};
