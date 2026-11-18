//! Bringing a finished file into the vault, and describing it for a client.
//!
//! `file_deliver` is the one door for a file a session made with a shell or a script: it checks
//! the type, copies the file under `Exports/<yyyy-mm>/` and returns the typed `files` entry that
//! the clients draw as a card. Exports are deliverables, not cache: the storage governor only
//! clears `_data/cache`, so nothing here is evicted.

use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use async_trait::async_trait;
use hq_convert::ingest::sniff::{Kind, sniff};
use serde_json::{Value, json};

use crate::registry::HqTool;

/// Where delivered files and exports land, relative to the vault.
pub(crate) const EXPORTS_DIR: &str = "Exports";
/// A delivered file may not be larger than this.
const MAX_DELIVER_BYTES: u64 = 50 * 1024 * 1024;
/// Vault folders that hold identity, threads and databases; files never belong there.
const PRIVATE_DIRS: [&str; 4] = ["_system", "_threads", "_data", "_trash"];
/// Macro-enabled Office types are refused outright.
const MACRO_EXTENSIONS: [&str; 6] = ["xlsm", "xlsb", "docm", "pptm", "dotm", "xltm"];
/// Folders a session's own files may come from, besides its working directory.
const SCRATCH_ROOTS: [&str; 4] = ["/tmp", "/var/tmp", "/private/tmp", "/private/var/folders"];

/// A type the owner can be handed: its extension, mime type, card kind and the content it must have.
struct Deliverable {
    ext: &'static str,
    mime: &'static str,
    kind: &'static str,
    content: Option<Kind>,
}

const DELIVERABLES: &[Deliverable] = &[
    Deliverable {
        ext: "xlsx",
        mime: "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        kind: "sheet",
        content: Some(Kind::Xlsx),
    },
    Deliverable {
        ext: "docx",
        mime: "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        kind: "doc",
        content: Some(Kind::Docx),
    },
    Deliverable {
        ext: "pptx",
        mime: "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        kind: "slides",
        content: Some(Kind::Pptx),
    },
    Deliverable {
        ext: "pdf",
        mime: "application/pdf",
        kind: "pdf",
        content: Some(Kind::Pdf),
    },
    Deliverable {
        ext: "csv",
        mime: "text/csv",
        kind: "sheet",
        content: None,
    },
    Deliverable {
        ext: "tsv",
        mime: "text/tab-separated-values",
        kind: "sheet",
        content: None,
    },
    Deliverable {
        ext: "png",
        mime: "image/png",
        kind: "image",
        content: Some(Kind::Image),
    },
    Deliverable {
        ext: "jpg",
        mime: "image/jpeg",
        kind: "image",
        content: Some(Kind::Image),
    },
    Deliverable {
        ext: "jpeg",
        mime: "image/jpeg",
        kind: "image",
        content: Some(Kind::Image),
    },
    Deliverable {
        ext: "webp",
        mime: "image/webp",
        kind: "image",
        content: Some(Kind::Image),
    },
    Deliverable {
        ext: "gif",
        mime: "image/gif",
        kind: "image",
        content: Some(Kind::Image),
    },
];

fn deliverable(ext: &str) -> Option<&'static Deliverable> {
    DELIVERABLES.iter().find(|d| d.ext == ext)
}

fn extension_of(path: &Path) -> String {
    path.extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
}

/// The typed record of a file in the vault, as a chat message carries it: path, name, mime type,
/// size and the kind of card to draw. `rel` is vault-relative.
pub(crate) fn file_entry(vault: &Path, rel: &str) -> Value {
    let path = vault.join(rel);
    let ext = extension_of(&path);
    let known = deliverable(&ext);
    json!({
        "path": rel,
        "name": path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        "mime": known.map_or("application/octet-stream", |d| d.mime),
        "kind": known.map_or("file", |d| d.kind),
        "size": std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0),
    })
}

/// `Exports/<yyyy-mm>` for today.
pub(crate) fn exports_month_dir() -> PathBuf {
    PathBuf::from(EXPORTS_DIR).join(chrono::Local::now().format("%Y-%m").to_string())
}

/// `dir/stem.ext`, or `dir/stem-2.ext` and so on when that name is taken. Nothing is overwritten.
pub(crate) fn unique_path(dir: &Path, stem: &str, ext: &str) -> PathBuf {
    let name = |n: u32| match (n, ext.is_empty()) {
        (1, true) => stem.to_string(),
        (1, false) => format!("{stem}.{ext}"),
        (n, true) => format!("{stem}-{n}"),
        (n, false) => format!("{stem}-{n}.{ext}"),
    };
    (1..)
        .map(|n| dir.join(name(n)))
        .find(|p| !p.exists())
        .expect("an unbounded range yields a free name")
}

/// A file name with no folders in it and nothing a file system dislikes.
pub(crate) fn clean_stem(raw: &str) -> String {
    let stem: String = raw
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, ' ' | '-' | '_' | '(' | ')') {
                c
            } else {
                '-'
            }
        })
        .collect();
    let stem = stem
        .trim_matches(|c: char| c == '-' || c == ' ')
        .to_string();
    if stem.is_empty() { "file".into() } else { stem }
}

fn in_private_dir(vault: &Path, abs: &Path) -> bool {
    PRIVATE_DIRS.iter().any(|d| abs.starts_with(vault.join(d)))
}

/// Whether `abs` sits in a place a session legitimately makes files: temp space or its own cwd.
fn in_session_space(abs: &Path) -> bool {
    let mut roots: Vec<PathBuf> = SCRATCH_ROOTS.iter().map(PathBuf::from).collect();
    roots.push(std::env::temp_dir());
    if let Ok(cwd) = std::env::current_dir() {
        roots.push(cwd);
    }
    roots
        .iter()
        .filter_map(|r| r.canonicalize().ok())
        .any(|r| abs.starts_with(r))
}

/// Everything `file_deliver` checks before it copies, with the reason in words the agent can act on.
fn check_source(vault: &Path, raw: &Path) -> Result<(PathBuf, &'static Deliverable)> {
    let meta = std::fs::symlink_metadata(raw)
        .map_err(|_| anyhow::anyhow!("no such file: {}", raw.display()))?;
    if meta.file_type().is_symlink() || !meta.is_file() {
        bail!("{} is not a plain file", raw.display());
    }
    let abs = raw.canonicalize()?;
    if in_private_dir(vault, &abs) {
        bail!("{} is in a folder HQ keeps for itself", raw.display());
    }
    if !abs.starts_with(vault) && !in_session_space(&abs) {
        bail!(
            "{} is outside this session's working folder and temp space; make the file there, or use office_export",
            raw.display()
        );
    }
    let ext = extension_of(&abs);
    if MACRO_EXTENSIONS.contains(&ext.as_str()) {
        bail!(
            "macro-enabled Office files (.{ext}) are not delivered; save a macro-free copy first"
        );
    }
    let Some(kind) = deliverable(&ext) else {
        let allowed: Vec<&str> = DELIVERABLES.iter().map(|d| d.ext).collect();
        bail!(
            "type .{ext} is not delivered; allowed: {}",
            allowed.join(", ")
        );
    };
    if meta.len() > MAX_DELIVER_BYTES {
        bail!(
            "{} is {} bytes; the limit is {MAX_DELIVER_BYTES}",
            raw.display(),
            meta.len()
        );
    }
    Ok((abs, kind))
}

fn check_content(bytes: &[u8], ext: &str, want: &Deliverable) -> Result<()> {
    match want.content {
        Some(kind) if sniff(bytes, ext).kind != kind => {
            bail!("the file's content is not a real .{ext} file")
        }
        None if std::str::from_utf8(bytes).is_err() => {
            bail!("the file is not text, so it cannot be a .{ext}")
        }
        _ => Ok(()),
    }
}

/// Copies a checked file into `Exports/<yyyy-mm>/` (or reports it where it already sits in the vault).
fn deliver(vault: &Path, raw: &Path, name: Option<&str>) -> Result<Value> {
    let vault = vault.canonicalize()?;
    let (abs, kind) = check_source(&vault, raw)?;
    let bytes = std::fs::read(&abs)?;
    check_content(&bytes, kind.ext, kind)?;

    if let Ok(rel) = abs.strip_prefix(&vault) {
        let rel = rel.to_string_lossy().into_owned();
        return Ok(result(&vault, &rel));
    }
    let suffix = format!(".{}", kind.ext);
    let raw_stem = match name {
        Some(n) => n.strip_suffix(&suffix).unwrap_or(n),
        None => abs.file_stem().and_then(|s| s.to_str()).unwrap_or("file"),
    };
    let stem = clean_stem(raw_stem);
    let dir = vault.join(exports_month_dir());
    std::fs::create_dir_all(&dir)?;
    let dest = unique_path(&dir, &stem, kind.ext);
    let part = dest.with_extension(format!("{}.part", kind.ext));
    std::fs::write(&part, &bytes)?;
    std::fs::rename(&part, &dest)?;
    let rel = dest.strip_prefix(&vault)?.to_string_lossy().into_owned();
    Ok(result(&vault, &rel))
}

fn result(vault: &Path, rel: &str) -> Value {
    json!({
        "files": [file_entry(vault, rel)],
        "web_path": rel,
        "output_path": vault.join(rel).to_string_lossy(),
    })
}

pub struct FileDeliverTool {
    vault_path: PathBuf,
}

impl FileDeliverTool {
    pub fn new(vault_path: PathBuf) -> Self {
        Self { vault_path }
    }
}

#[async_trait]
impl HqTool for FileDeliverTool {
    fn name(&self) -> &str {
        "file_deliver"
    }

    fn description(&self) -> &str {
        "Bring a file you made (with a script or a shell) into the vault so the owner can open it: \
         copies it to Exports/<yyyy-mm>/ and returns a `files` entry the chat shows as a card. Only \
         xlsx, docx, pptx, pdf, csv, tsv and common images, up to 50 MB, from your own working or temp \
         folder; macro-enabled files are refused and nothing is overwritten. A file left anywhere else \
         is invisible to the owner."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Absolute path of the finished file." },
                "name": { "type": "string", "description": "Optional file name for the vault copy (no folders)." }
            },
            "required": ["path"]
        })
    }

    fn category(&self) -> &str {
        "convert"
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let path = args
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("missing required arg: path"))?;
        let name = args.get("name").and_then(Value::as_str);
        let (vault, path, name) = (
            self.vault_path.clone(),
            PathBuf::from(path),
            name.map(str::to_owned),
        );
        tokio::task::spawn_blocking(move || deliver(&vault, &path, name.as_deref())).await?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn zip_with(name: &str) -> Vec<u8> {
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut z = zip::ZipWriter::new(&mut buf);
            let o = zip::write::SimpleFileOptions::default();
            z.start_file("[Content_Types].xml", o).unwrap();
            z.write_all(b"<Types/>").unwrap();
            z.start_file(name, o).unwrap();
            z.write_all(b"<x/>").unwrap();
            z.finish().unwrap();
        }
        buf.into_inner()
    }

    fn setup() -> (tempfile::TempDir, tempfile::TempDir) {
        (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap())
    }

    #[test]
    fn a_real_workbook_lands_under_exports_and_is_described() {
        let (vault, scratch) = setup();
        let src = scratch.path().join("stocks.xlsx");
        std::fs::write(&src, zip_with("xl/workbook.xml")).unwrap();
        let out = deliver(vault.path(), &src, Some("AI stocks")).unwrap();
        let file = &out["files"][0];
        let rel = file["path"].as_str().unwrap();
        assert!(
            rel.starts_with("Exports/") && rel.ends_with("/AI stocks.xlsx"),
            "{rel}"
        );
        assert_eq!(file["kind"], "sheet");
        assert!(vault.path().join(rel).exists());
        assert!(src.exists(), "the source is copied, not moved");
    }

    #[test]
    fn a_second_delivery_never_overwrites_the_first() {
        let (vault, scratch) = setup();
        let src = scratch.path().join("a.pdf");
        std::fs::write(&src, b"%PDF-1.4 one").unwrap();
        let first = deliver(vault.path(), &src, None).unwrap();
        let second = deliver(vault.path(), &src, None).unwrap();
        assert_ne!(first["web_path"], second["web_path"]);
    }

    #[test]
    fn macro_files_other_types_and_fake_content_are_refused() {
        let (vault, scratch) = setup();
        let put = |name: &str, bytes: &[u8]| {
            let p = scratch.path().join(name);
            std::fs::write(&p, bytes).unwrap();
            p
        };
        let cases = [
            (put("m.xlsm", &zip_with("xl/workbook.xml")), "macro"),
            (put("run.sh", b"echo hi"), "not delivered"),
            (put("fake.xlsx", b"just text"), "not a real"),
            (put("bin.csv", &[0xff, 0xfe, 0x00, 0xc3]), "not text"),
        ];
        for (path, want) in cases {
            let err = deliver(vault.path(), &path, None).unwrap_err().to_string();
            assert!(err.contains(want), "{err}");
        }
    }

    #[test]
    fn files_outside_session_space_and_private_vault_folders_are_refused() {
        let (vault, _scratch) = setup();
        std::fs::create_dir_all(vault.path().join("_data")).unwrap();
        let private = vault.path().join("_data/x.csv");
        std::fs::write(&private, "a,b\n1,2\n").unwrap();
        let err = deliver(vault.path(), &private, None)
            .unwrap_err()
            .to_string();
        assert!(err.contains("keeps for itself"), "{err}");
        let missing = Path::new("/definitely/not/here.csv");
        assert!(deliver(vault.path(), missing, None).is_err());
    }

    #[test]
    fn a_file_already_in_the_vault_is_described_not_copied() {
        let (vault, _scratch) = setup();
        std::fs::create_dir_all(vault.path().join("Notebooks")).unwrap();
        std::fs::write(vault.path().join("Notebooks/t.csv"), "a,b\n1,2\n").unwrap();
        let out = deliver(vault.path(), &vault.path().join("Notebooks/t.csv"), None).unwrap();
        assert_eq!(out["web_path"], "Notebooks/t.csv");
        assert!(!vault.path().join("Exports").exists());
    }

    #[test]
    fn stems_are_clean_and_names_are_unique() {
        assert_eq!(clean_stem("../../etc/passwd"), "etc-passwd");
        assert_eq!(clean_stem(""), "file");
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("a.csv"), "").unwrap();
        assert_eq!(unique_path(d.path(), "a", "csv"), d.path().join("a-2.csv"));
    }
}
