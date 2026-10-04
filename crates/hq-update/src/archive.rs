//! Strict `.tar.gz` unpacking. Artifacts are untrusted until their checksum
//! and the manifest signature check out, and even then only plain files and
//! directories under the destination are ever created.

use crate::error::{Result, UpdateError};
use flate2::read::GzDecoder;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Component, Path, PathBuf};
use tar::{Archive, EntryType};

fn unsafe_archive(reason: impl Into<String>) -> UpdateError {
    UpdateError::UnsafeArchive(reason.into())
}

/// Relative, normal components only (a leading `./` is tolerated).
fn safe_relative(path: &Path) -> Result<PathBuf> {
    let mut clean = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => clean.push(part),
            Component::CurDir => {}
            other => {
                return Err(unsafe_archive(format!(
                    "entry path {} has a {other:?} component",
                    path.display()
                )));
            }
        }
    }
    if clean.as_os_str().is_empty() {
        return Err(unsafe_archive("entry with an empty path"));
    }
    Ok(clean)
}

fn open_archive(path: &Path) -> Result<Archive<GzDecoder<File>>> {
    Ok(Archive::new(GzDecoder::new(File::open(path)?)))
}

fn copy_limited(entry: &mut impl Read, dest: &mut File, limit: u64, name: &str) -> Result<u64> {
    let written = io::copy(&mut entry.take(limit.saturating_add(1)), dest)?;
    if written > limit {
        return Err(UpdateError::SizeLimit {
            name: name.to_string(),
            limit,
        });
    }
    Ok(written)
}

/// Unpacks an archive that must hold exactly one regular file named
/// `expected_name` to `dest` with mode 0755.
pub fn extract_single_file(
    archive: &Path,
    expected_name: &str,
    dest: &Path,
    max_bytes: u64,
) -> Result<()> {
    let mut archive = open_archive(archive)?;
    let mut found = false;
    for entry in archive
        .entries()
        .map_err(|e| unsafe_archive(e.to_string()))?
    {
        let mut entry = entry.map_err(|e| unsafe_archive(e.to_string()))?;
        match entry.header().entry_type() {
            EntryType::XGlobalHeader => continue,
            EntryType::Regular => {}
            EntryType::Directory
                if entry
                    .path()
                    .map(|p| p.as_os_str() == "." || p.as_os_str() == "./")
                    .unwrap_or(false) =>
            {
                continue;
            }
            other => return Err(unsafe_archive(format!("unexpected entry type {other:?}"))),
        }
        let rel = safe_relative(&entry.path().map_err(|e| unsafe_archive(e.to_string()))?)?;
        if rel != Path::new(expected_name) || found {
            return Err(unsafe_archive(format!(
                "unexpected entry {} (only `{expected_name}` is allowed, once)",
                rel.display()
            )));
        }
        let mut out = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o755)
            .open(dest)?;
        copy_limited(&mut entry, &mut out, max_bytes, expected_name)?;
        out.sync_all()?;
        found = true;
    }
    if !found {
        return Err(unsafe_archive(format!(
            "archive does not contain `{expected_name}`"
        )));
    }
    Ok(())
}

pub struct TreeLimits {
    pub max_total_bytes: u64,
    pub max_files: u64,
}

/// Unpacks a directory tree into `dest_dir` (created fresh by the caller).
/// Rejects links, devices, absolute paths, `..`, duplicate paths and
/// anything past the limits. Files get mode 0644, directories 0755.
pub fn extract_tree(archive: &Path, dest_dir: &Path, limits: &TreeLimits) -> Result<()> {
    let mut archive = open_archive(archive)?;
    let (mut total, mut files) = (0u64, 0u64);
    for entry in archive
        .entries()
        .map_err(|e| unsafe_archive(e.to_string()))?
    {
        let mut entry = entry.map_err(|e| unsafe_archive(e.to_string()))?;
        let kind = entry.header().entry_type();
        if kind == EntryType::XGlobalHeader {
            continue;
        }
        let raw = entry
            .path()
            .map_err(|e| unsafe_archive(e.to_string()))?
            .into_owned();
        if kind == EntryType::Directory && raw.components().all(|c| matches!(c, Component::CurDir))
        {
            continue;
        }
        let target = dest_dir.join(safe_relative(&raw)?);
        match kind {
            EntryType::Directory => {
                fs::create_dir_all(&target)?;
            }
            EntryType::Regular => {
                files += 1;
                if files > limits.max_files {
                    return Err(unsafe_archive("too many files"));
                }
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent)?;
                }
                let remaining = limits.max_total_bytes.saturating_sub(total);
                let mut out = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o644)
                    .open(&target)
                    .map_err(|e| unsafe_archive(format!("{}: {e}", raw.display())))?;
                total += copy_limited(&mut entry, &mut out, remaining, "web archive contents")?;
            }
            other => {
                return Err(unsafe_archive(format!(
                    "{}: entry type {other:?} is not allowed",
                    raw.display()
                )));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use flate2::{Compression, write::GzEncoder};

    pub enum Item<'a> {
        File(&'a str, &'a [u8]),
        Dir(&'a str),
        Symlink(&'a str, &'a str),
        Hardlink(&'a str, &'a str),
    }

    /// Raw path bytes bypass the builder's own `..` refusal.
    pub fn make_tgz(path: &Path, items: &[Item]) {
        let gz = GzEncoder::new(File::create(path).unwrap(), Compression::fast());
        let mut b = tar::Builder::new(gz);
        for item in items {
            let mut h = tar::Header::new_gnu();
            match item {
                Item::File(name, data) => {
                    h.set_entry_type(EntryType::Regular);
                    h.set_size(data.len() as u64);
                    set_raw_path(&mut h, name);
                    h.set_mode(0o644);
                    h.set_cksum();
                    b.append(&h, *data).unwrap();
                }
                Item::Dir(name) => {
                    h.set_entry_type(EntryType::Directory);
                    h.set_size(0);
                    set_raw_path(&mut h, name);
                    h.set_mode(0o755);
                    h.set_cksum();
                    b.append(&h, io::empty()).unwrap();
                }
                Item::Symlink(name, to) | Item::Hardlink(name, to) => {
                    h.set_entry_type(if matches!(item, Item::Symlink(..)) {
                        EntryType::Symlink
                    } else {
                        EntryType::Link
                    });
                    h.set_size(0);
                    set_raw_path(&mut h, name);
                    h.set_link_name(to).unwrap();
                    h.set_cksum();
                    b.append(&h, io::empty()).unwrap();
                }
            }
        }
        b.into_inner().unwrap().finish().unwrap();
    }

    fn set_raw_path(h: &mut tar::Header, name: &str) {
        let field = &mut h.as_old_mut().name;
        field.fill(0);
        field[..name.len()].copy_from_slice(name.as_bytes());
    }

    fn limits() -> TreeLimits {
        TreeLimits {
            max_total_bytes: 1024,
            max_files: 10,
        }
    }

    #[test]
    fn single_file_happy_path_and_mode() {
        let d = tempfile::tempdir().unwrap();
        let tgz = d.path().join("a.tgz");
        make_tgz(&tgz, &[Item::File("hq", b"binary")]);
        let out = d.path().join("out");
        extract_single_file(&tgz, "hq", &out, 100).unwrap();
        assert_eq!(fs::read(&out).unwrap(), b"binary");
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&out).unwrap().permissions().mode() & 0o777,
            0o755
        );
    }

    #[test]
    fn single_file_refuses_everything_else() {
        let d = tempfile::tempdir().unwrap();
        let cases: Vec<Vec<Item>> = vec![
            vec![Item::File("other", b"x")],
            vec![Item::File("../hq", b"x")],
            vec![Item::File("/abs/hq", b"x")],
            vec![Item::File("sub/hq", b"x")],
            vec![Item::File("hq", b"x"), Item::File("extra", b"y")],
            vec![Item::File("hq", b"x"), Item::File("hq", b"y")],
            vec![Item::Symlink("hq", "/bin/sh")],
            vec![Item::Hardlink("hq", "/etc/passwd")],
            vec![],
        ];
        for (i, items) in cases.iter().enumerate() {
            let tgz = d.path().join(format!("c{i}.tgz"));
            make_tgz(&tgz, items);
            let out = d.path().join(format!("o{i}"));
            let err = extract_single_file(&tgz, "hq", &out, 100).unwrap_err();
            assert!(
                matches!(err, UpdateError::UnsafeArchive(_)),
                "case {i}: {err}"
            );
        }
    }

    #[test]
    fn single_file_size_limit() {
        let d = tempfile::tempdir().unwrap();
        let tgz = d.path().join("a.tgz");
        make_tgz(&tgz, &[Item::File("hq", &[7u8; 50])]);
        let err = extract_single_file(&tgz, "hq", &d.path().join("o"), 10).unwrap_err();
        assert!(matches!(err, UpdateError::SizeLimit { .. }), "{err}");
    }

    #[test]
    fn tree_happy_path_with_dot_prefix() {
        let d = tempfile::tempdir().unwrap();
        let tgz = d.path().join("w.tgz");
        make_tgz(
            &tgz,
            &[
                Item::Dir("./"),
                Item::File("./index.html", b"<html>"),
                Item::Dir("assets"),
                Item::File("assets/app.js", b"js"),
            ],
        );
        let out = d.path().join("out");
        fs::create_dir(&out).unwrap();
        extract_tree(&tgz, &out, &limits()).unwrap();
        assert_eq!(fs::read(out.join("index.html")).unwrap(), b"<html>");
        assert_eq!(fs::read(out.join("assets/app.js")).unwrap(), b"js");
    }

    #[test]
    fn tree_refuses_traversal_links_and_duplicates() {
        let d = tempfile::tempdir().unwrap();
        let cases: Vec<Vec<Item>> = vec![
            vec![Item::File("../escape", b"x")],
            vec![Item::File("a/../../escape", b"x")],
            vec![Item::File("/etc/escape", b"x")],
            vec![Item::Symlink("link", "/etc")],
            vec![Item::Hardlink("link", "index.html")],
            vec![Item::File("a", b"1"), Item::File("a", b"2")],
        ];
        for (i, items) in cases.iter().enumerate() {
            let tgz = d.path().join(format!("c{i}.tgz"));
            make_tgz(&tgz, items);
            let out = d.path().join(format!("o{i}"));
            fs::create_dir(&out).unwrap();
            let err = extract_tree(&tgz, &out, &limits()).unwrap_err();
            assert!(
                matches!(err, UpdateError::UnsafeArchive(_)),
                "case {i}: {err}"
            );
        }
        assert!(!d.path().join("escape").exists());
    }

    #[test]
    fn tree_limits() {
        let d = tempfile::tempdir().unwrap();
        let tgz = d.path().join("w.tgz");
        make_tgz(&tgz, &[Item::File("big", &[1u8; 2000])]);
        let out = d.path().join("o");
        fs::create_dir(&out).unwrap();
        assert!(matches!(
            extract_tree(&tgz, &out, &limits()).unwrap_err(),
            UpdateError::SizeLimit { .. }
        ));
        let names: Vec<String> = (0..11).map(|i| format!("f{i}")).collect();
        let items: Vec<Item> = names.iter().map(|n| Item::File(n, b"x")).collect();
        make_tgz(&tgz, &items);
        let out = d.path().join("o2");
        fs::create_dir(&out).unwrap();
        assert!(matches!(
            extract_tree(&tgz, &out, &limits()).unwrap_err(),
            UpdateError::UnsafeArchive(_)
        ));
    }
}
