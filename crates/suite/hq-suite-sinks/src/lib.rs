//! A folder the owner keeps, holding originals sealed one at a time.
//!
//! The folder can be an SD card, a USB drive, a mounted share or any path the process can write.
//! Each original is sealed under a key made from a recovery code the owner writes down, filed by
//! the SHA-256 of its bytes, and read back before the sink says it is stored. The code is shown
//! once, when the sink is made, and is never kept: losing it means the sealed originals cannot be
//! opened, which is the point of sealing them. The notes (and their pictures) stay readable on the
//! phone whatever happens to the sink.
//!
//! ```text
//! <root>/suite-sink.json         id, salt and a check value (not secret)
//! <root>/objects/<hh>/<sha>.hqo  one sealed original
//! ```

use std::fs;
use std::path::{Path, PathBuf};

use hq_backup::object::{self, ObjectKey, SALT_LEN};
use hq_suite_store::fsx;
use hq_suite_store::{ColdError, ColdResult, ColdStore};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const META_FILE: &str = "suite-sink.json";
const OBJECTS: &str = "objects";
const EXTENSION: &str = "hqo";
const FORMAT: u32 = 1;
/// What the check value seals. Opening it proves the code is the right one before any original
/// is touched.
const CHECK_TEXT: &[u8] = b"hq-suite-sink";

/// What a sink keeps about itself in the open: its name, the salt its key is made with, and a
/// sealed check value. None of it is secret; without the recovery code it opens nothing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Meta {
    format: u32,
    id: String,
    salt: String,
    check: String,
}

impl Meta {
    /// A new sink's description, its key and its recovery code (shown once, never kept).
    pub fn make(id: &str) -> ColdResult<(Meta, ObjectKey, String)> {
        if !valid_id(id) {
            return Err(err(
                "a sink's name is lowercase letters, digits and dashes, up to 32 characters",
            ));
        }
        let code = object::new_recovery_code().map_err(|e| err(e.to_string()))?;
        let salt = object::random_salt().map_err(|e| err(e.to_string()))?;
        let key = ObjectKey::from_recovery_code(&code, &salt).map_err(|e| err(e.to_string()))?;
        let check = object::seal(CHECK_TEXT, &key).map_err(|e| err(e.to_string()))?;
        let meta = Meta {
            format: FORMAT,
            id: id.to_string(),
            salt: hex::encode(salt),
            check: hex::encode(check),
        };
        Ok((meta, key, code))
    }

    /// The key the recovery code makes, once the check value proves the code is the right one.
    pub fn unlock(&self, code: &str) -> ColdResult<ObjectKey> {
        if self.format != FORMAT {
            return Err(err(
                "this sink was made by a newer HQ; update HQ to open it",
            ));
        }
        if !valid_id(&self.id) {
            return Err(err("the sink's description is damaged"));
        }
        let damaged = || err("the sink's description is damaged");
        let salt: [u8; SALT_LEN] = hex::decode(&self.salt)
            .ok()
            .and_then(|b| b.try_into().ok())
            .ok_or_else(damaged)?;
        let key = ObjectKey::from_recovery_code(code, &salt).map_err(|e| err(e.to_string()))?;
        let check = hex::decode(&self.check).map_err(|_| damaged())?;
        match object::open(&check, &key) {
            Ok(p) if p == CHECK_TEXT => Ok(key),
            _ => Err(err("that recovery code does not open this sink")),
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }
}

/// The original sealed under `key`, after checking it hashes to `sha256`.
pub fn seal_object(key: &ObjectKey, sha256: &str, bytes: &[u8]) -> ColdResult<Vec<u8>> {
    if sha256_hex(bytes) != sha256 {
        return Err(err("the bytes do not hash to the name they were given"));
    }
    object::seal(bytes, key).map_err(|e| err(e.to_string()))
}

/// A sealed original opened under `key` and checked against the hash it was filed under.
pub fn open_object(key: &ObjectKey, sha256: &str, sealed: &[u8]) -> ColdResult<Vec<u8>> {
    let plain = object::open(sealed, key).map_err(|_| {
        err("that original could not be opened: it is damaged or sealed under another code")
    })?;
    if sha256_hex(&plain) != sha256 {
        return Err(err(
            "that original came back different from what was stored",
        ));
    }
    Ok(plain)
}

/// A sink that can say which originals it holds, which a restore drill needs.
pub trait Listable: ColdStore {
    /// Every hash stored, sorted.
    fn list_hashes(&self) -> Vec<String>;
}

impl Listable for FolderSink {
    fn list_hashes(&self) -> Vec<String> {
        self.list()
    }
}

/// What a sink can say about itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Health {
    pub reachable: bool,
    pub objects: usize,
}

#[derive(Debug)]
pub struct FolderSink {
    root: PathBuf,
    id: String,
    key: ObjectKey,
}

fn err(msg: impl Into<String>) -> ColdError {
    ColdError(msg.into())
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 32
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

pub fn valid_hash(h: &str) -> bool {
    h.len() == 64
        && h.bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

impl FolderSink {
    /// Makes a sink in `root` (created if needed) and returns it with its recovery code. The code
    /// is the only way to open what is sealed into it later. Refuses a folder that is already a
    /// sink, so a second run can never replace the key of the first.
    pub fn create(root: &Path, id: &str) -> ColdResult<(FolderSink, String)> {
        let meta_path = root.join(META_FILE);
        if meta_path.exists() {
            return Err(err(
                "that folder is already a sink; open it with its recovery code",
            ));
        }
        fs::create_dir_all(root.join(OBJECTS))
            .map_err(|e| err(format!("cannot use {}: {e}", root.display())))?;
        let (meta, key, code) = Meta::make(id)?;
        let text = serde_json::to_vec_pretty(&meta).map_err(|e| err(e.to_string()))?;
        fsx::atomic_write(&meta_path, &text).map_err(|e| err(e.to_string()))?;
        Ok((
            FolderSink {
                root: root.to_path_buf(),
                id: id.to_string(),
                key,
            },
            code,
        ))
    }

    /// Opens the sink in `root` with its recovery code. A wrong code is refused here, before any
    /// original is stored or fetched.
    pub fn open(root: &Path, code: &str) -> ColdResult<FolderSink> {
        let text = fs::read(root.join(META_FILE)).map_err(|_| {
            err(format!(
                "{} is not a sink, or it cannot be reached",
                root.display()
            ))
        })?;
        let meta: Meta =
            serde_json::from_slice(&text).map_err(|_| err("the sink's description is damaged"))?;
        let key = meta.unlock(code)?;
        Ok(FolderSink {
            root: root.to_path_buf(),
            id: meta.id,
            key,
        })
    }

    fn path_of(&self, sha256: &str) -> ColdResult<PathBuf> {
        if !valid_hash(sha256) {
            return Err(err("that is not a hash this sink files by"));
        }
        Ok(self
            .root
            .join(OBJECTS)
            .join(&sha256[..2])
            .join(format!("{sha256}.{EXTENSION}")))
    }

    /// Every hash stored, sorted.
    pub fn list(&self) -> Vec<String> {
        let mut out = Vec::new();
        let Ok(shards) = fs::read_dir(self.root.join(OBJECTS)) else {
            return out;
        };
        for shard in shards.flatten() {
            let Ok(files) = fs::read_dir(shard.path()) else {
                continue;
            };
            for f in files.flatten() {
                let name = f.file_name().to_string_lossy().into_owned();
                if let Some(h) = name.strip_suffix(&format!(".{EXTENSION}"))
                    && valid_hash(h)
                {
                    out.push(h.to_string());
                }
            }
        }
        out.sort();
        out
    }

    pub fn health(&self) -> Health {
        Health {
            reachable: self.root.join(META_FILE).is_file(),
            objects: self.list().len(),
        }
    }
}

impl ColdStore for FolderSink {
    fn id(&self) -> &str {
        &self.id
    }

    fn put(&self, sha256: &str, bytes: &[u8]) -> ColdResult<()> {
        let path = self.path_of(sha256)?;
        if path.is_file() && self.get(sha256).is_ok() {
            return Ok(());
        }
        let sealed = seal_object(&self.key, sha256, bytes)?;
        fsx::atomic_write(&path, &sealed)
            .map_err(|e| err(format!("cannot write to the sink: {e}")))?;
        // Read back through the same path a restore takes, so "stored" means "restorable".
        self.get(sha256)
            .map(drop)
            .map_err(|e| err(format!("the sink did not keep it: {e}")))
    }

    fn get(&self, sha256: &str) -> ColdResult<Vec<u8>> {
        let path = self.path_of(sha256)?;
        let sealed = fs::read(&path).map_err(|_| err("that original is not in this sink"))?;
        open_object(&self.key, sha256, &sealed)
    }

    fn has(&self, sha256: &str) -> bool {
        self.path_of(sha256)
            .ok()
            .and_then(|p| fs::metadata(p).ok())
            .is_some_and(|m| m.is_file() && m.len() > 0)
    }
}

/// What a restore drill found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Drill {
    pub checked: usize,
    pub failed: Vec<String>,
}

/// Fetches up to `sample` stored originals, spread evenly over the sink, and checks each opens and
/// hashes to its name. Run before trusting a sink with the only copy of something.
pub fn restore_drill<S: Listable + ?Sized>(sink: &S, sample: usize) -> Drill {
    let all = sink.list_hashes();
    let picks: Vec<&String> = if sample == 0 || all.is_empty() {
        Vec::new()
    } else if sample >= all.len() {
        all.iter().collect()
    } else {
        (0..sample).map(|i| &all[i * all.len() / sample]).collect()
    };
    let failed = picks
        .iter()
        .filter(|h| sink.get(h).is_err())
        .map(|h| h.to_string())
        .collect();
    Drill {
        checked: picks.len(),
        failed,
    }
}

#[cfg(test)]
mod tests;
