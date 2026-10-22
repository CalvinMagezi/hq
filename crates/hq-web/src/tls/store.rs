//! Where the ACME account key and the certificate's private key live: a directory only the
//! owner can open, files only the owner can read, replaced by rename so a crash never leaves
//! half a key. Nothing here is logged, and nothing secret has a `Debug` that prints it.

use super::TlsError;
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::{Path, PathBuf};
use zeroize::Zeroizing;

const DIR_MODE: u32 = 0o700;
const FILE_MODE: u32 = 0o600;
/// Characters of the directory's hash that tell two ACME accounts apart in a file name.
const ACCOUNT_TAG_LEN: usize = 12;

/// A secret read from or about to be written to disk. It prints as nothing and is wiped when
/// it goes.
pub(crate) struct Secret(Zeroizing<Vec<u8>>);

impl Secret {
    pub fn new(bytes: Vec<u8>) -> Self {
        Self(Zeroizing::new(bytes))
    }

    pub fn expose(&self) -> &[u8] {
        &self.0
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(..)")
    }
}

/// The directory that holds the connector's TLS state.
#[derive(Debug, Clone)]
pub(crate) struct PrivateDir {
    root: PathBuf,
}

impl PrivateDir {
    /// Next to the database, outside anything the vault syncs as notes (like the Web Push key).
    pub fn under_vault(vault: &Path) -> Self {
        Self {
            root: vault.join("_data").join("connector_tls"),
        }
    }

    #[cfg(test)]
    pub fn at(root: PathBuf) -> Self {
        Self { root }
    }

    /// The file holding the key and chain issued for `host` by one ACME directory. Kept per
    /// directory: a certificate from staging means nothing to production, and must not be
    /// taken for one after the switch.
    pub fn issued_path(&self, host: &str, directory_url: &str) -> PathBuf {
        self.root
            .join(format!("{host}-{}.pem", directory_tag(directory_url)))
    }

    /// The file holding what a restart must remember about orders: failures in a row and the
    /// earliest time the next one may start.
    pub fn retry_path(&self, host: &str, directory_url: &str) -> PathBuf {
        self.root.join(format!(
            "{host}-{}.retry.json",
            directory_tag(directory_url)
        ))
    }

    /// The file holding the account for one ACME directory. Accounts are per directory: a
    /// staging account means nothing to production.
    pub fn account_path(&self, directory_url: &str) -> PathBuf {
        self.root
            .join(format!("account-{}.json", directory_tag(directory_url)))
    }

    /// Whether a file can be created and removed here. Checked before an ACME account is
    /// registered, so a store that cannot keep the account never lets one be made.
    pub fn check_writable(&self) -> Result<(), TlsError> {
        self.ensure()?;
        let probe = self.root.join("write-probe.tmp");
        let result = write_new_private(&probe, b"").and_then(|()| std::fs::remove_file(&probe));
        result.map_err(|e| TlsError::Store(format!("{} is not writable: {e}", self.root.display())))
    }

    pub fn remove(&self, path: &Path) {
        let _ = std::fs::remove_file(path);
    }

    pub fn read(&self, path: &Path) -> Result<Option<Secret>, TlsError> {
        match std::fs::read(path) {
            Ok(bytes) => Ok(Some(Secret::new(bytes))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(TlsError::Store(format!("reading {}: {e}", path.display()))),
        }
    }

    /// Writes `bytes` to `path`, created owner-only from the first byte, then renamed into place.
    pub fn write(&self, path: &Path, bytes: &[u8]) -> Result<(), TlsError> {
        self.ensure()?;
        let tmp = path.with_extension("tmp");
        let result = write_new_private(&tmp, bytes).and_then(|()| std::fs::rename(&tmp, path));
        if let Err(e) = result {
            let _ = std::fs::remove_file(&tmp);
            return Err(TlsError::Store(format!("writing {}: {e}", path.display())));
        }
        Ok(())
    }

    fn ensure(&self) -> Result<(), TlsError> {
        let mut dir = std::fs::DirBuilder::new();
        dir.recursive(true);
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut dir, DIR_MODE);
        dir.create(&self.root)
            .map_err(|e| TlsError::Store(format!("creating {}: {e}", self.root.display())))?;
        #[cfg(unix)]
        {
            // An existing directory keeps its mode; tighten one an older run or a person made.
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&self.root, std::fs::Permissions::from_mode(DIR_MODE))
                .map_err(|e| TlsError::Store(format!("securing {}: {e}", self.root.display())))?;
        }
        Ok(())
    }
}

/// A short stable name for a directory URL, with or without a trailing slash.
fn directory_tag(directory_url: &str) -> String {
    let tag: String = Sha256::digest(directory_url.trim_end_matches('/').as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    tag[..ACCOUNT_TAG_LEN].to_string()
}

fn write_new_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    // A leftover from a crash is ours to replace; create_new then refuses a link planted at the path.
    let _ = std::fs::remove_file(path);
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut opts, FILE_MODE);
    let mut file = opts.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir() -> (tempfile::TempDir, PrivateDir) {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = PrivateDir::at(tmp.path().join("connector_tls"));
        (tmp, store)
    }

    #[cfg(unix)]
    fn mode(path: &Path) -> u32 {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[test]
    #[cfg(unix)]
    fn files_and_their_directory_are_owner_only_and_stay_so_on_rewrite() {
        let (_tmp, store) = dir();
        let path = store.issued_path("hq.example.com", "https://acme.example.org/dir");
        store.write(&path, b"first").unwrap();
        store.write(&path, b"second").unwrap();
        assert_eq!(mode(&path), FILE_MODE);
        assert_eq!(mode(path.parent().unwrap()), DIR_MODE);
        assert_eq!(store.read(&path).unwrap().unwrap().expose(), b"second");
        let leftovers = std::fs::read_dir(path.parent().unwrap()).unwrap().count();
        assert_eq!(leftovers, 1, "no temporary file is left behind");
    }

    #[test]
    #[cfg(unix)]
    fn a_directory_that_was_open_to_others_is_closed() {
        let (_tmp, store) = dir();
        std::fs::create_dir_all(&store.root).unwrap();
        std::fs::set_permissions(
            &store.root,
            std::os::unix::fs::PermissionsExt::from_mode(0o755),
        )
        .unwrap();
        store
            .write(
                &store.issued_path("hq.example.com", "https://acme.example.org/dir"),
                b"k",
            )
            .unwrap();
        assert_eq!(mode(&store.root), DIR_MODE);
    }

    #[test]
    #[cfg(unix)]
    fn a_link_planted_at_the_temporary_path_is_not_followed() {
        let (tmp, store) = dir();
        let path = store.issued_path("hq.example.com", "https://acme.example.org/dir");
        std::fs::create_dir_all(&store.root).unwrap();
        let victim = tmp.path().join("victim");
        std::fs::write(&victim, b"keep").unwrap();
        std::os::unix::fs::symlink(&victim, path.with_extension("tmp")).unwrap();
        store.write(&path, b"secret").unwrap();
        assert_eq!(std::fs::read(&victim).unwrap(), b"keep");
        assert_eq!(store.read(&path).unwrap().unwrap().expose(), b"secret");
    }

    #[test]
    fn a_store_that_can_be_written_says_so_and_leaves_nothing_behind() {
        let (_tmp, store) = dir();
        store.check_writable().unwrap();
        let left = std::fs::read_dir(&store.root).unwrap().count();
        assert_eq!(left, 0, "the probe file is removed");
    }

    #[test]
    fn a_store_that_cannot_be_written_says_so() {
        let (_tmp, store) = dir();
        std::fs::create_dir_all(store.root.join("write-probe.tmp")).unwrap();
        let err = store.check_writable().unwrap_err();
        assert!(err.to_string().contains("not writable"), "{err}");
    }

    #[test]
    fn a_missing_file_is_none_and_accounts_are_kept_per_directory() {
        let (_tmp, store) = dir();
        assert!(
            store
                .read(&store.issued_path("nothing.example.com", "https://acme.example.org/dir"))
                .unwrap()
                .is_none()
        );
        let a = store.account_path("https://acme.example.org/dir");
        let b = store.account_path("https://acme-staging.example.org/dir");
        assert_ne!(a, b);
        assert_eq!(a, store.account_path("https://acme.example.org/dir"));
        assert_eq!(a, store.account_path("https://acme.example.org/dir/"));
        assert_ne!(
            store.issued_path("hq.example.com", "https://acme.example.org/dir"),
            store.issued_path("hq.example.com", "https://acme-staging.example.org/dir"),
            "a staging certificate is never found under the production name"
        );
    }

    #[test]
    fn a_secret_never_prints() {
        let secret = Secret::new(b"-----BEGIN PRIVATE KEY-----".to_vec());
        assert_eq!(format!("{secret:?}"), "Secret(..)");
    }
}
