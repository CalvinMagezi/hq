//! Describing a file in the vault for a client, and naming new ones under `Exports/`.
//!
//! Every tool that makes a file the owner is handed returns an entry built here, so the chat can
//! draw a card from data instead of from the model's words.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

/// Where delivered files and exports land, relative to the vault.
pub const EXPORTS_DIR: &str = "Exports";

/// A file type the owner can be handed: its extension, mime type and the card it gets.
pub struct FileType {
    pub ext: &'static str,
    pub mime: &'static str,
    /// `sheet`, `doc`, `slides`, `pdf` or `image`.
    pub kind: &'static str,
}

pub const FILE_TYPES: &[FileType] = &[
    FileType {
        ext: "xlsx",
        mime: "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        kind: "sheet",
    },
    FileType {
        ext: "docx",
        mime: "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        kind: "doc",
    },
    FileType {
        ext: "pptx",
        mime: "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        kind: "slides",
    },
    FileType {
        ext: "pdf",
        mime: "application/pdf",
        kind: "pdf",
    },
    FileType {
        ext: "csv",
        mime: "text/csv",
        kind: "sheet",
    },
    FileType {
        ext: "tsv",
        mime: "text/tab-separated-values",
        kind: "sheet",
    },
    FileType {
        ext: "png",
        mime: "image/png",
        kind: "image",
    },
    FileType {
        ext: "jpg",
        mime: "image/jpeg",
        kind: "image",
    },
    FileType {
        ext: "jpeg",
        mime: "image/jpeg",
        kind: "image",
    },
    FileType {
        ext: "webp",
        mime: "image/webp",
        kind: "image",
    },
    FileType {
        ext: "gif",
        mime: "image/gif",
        kind: "image",
    },
];

pub fn file_type(ext: &str) -> Option<&'static FileType> {
    FILE_TYPES.iter().find(|d| d.ext == ext)
}

pub fn extension_of(path: &Path) -> String {
    path.extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
}

/// The typed record of a file in the vault, as a chat message carries it: path, name, mime type,
/// size and the kind of card to draw. `rel` is vault-relative.
pub fn file_entry(vault: &Path, rel: &str) -> Value {
    let path = vault.join(rel);
    let known = file_type(&extension_of(&path));
    json!({
        "path": rel,
        "name": path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        "mime": known.map_or("application/octet-stream", |d| d.mime),
        "kind": known.map_or("file", |d| d.kind),
        "size": std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0),
    })
}

/// `Exports/<yyyy-mm>` for today.
pub fn exports_month_dir() -> PathBuf {
    PathBuf::from(EXPORTS_DIR).join(chrono::Local::now().format("%Y-%m").to_string())
}

/// `dir/stem.ext`, or `dir/stem-2.ext` and so on when that name is taken. Nothing is overwritten.
pub fn unique_path(dir: &Path, stem: &str, ext: &str) -> PathBuf {
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
pub fn clean_stem(raw: &str) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stems_are_clean_and_names_are_unique() {
        assert_eq!(clean_stem("../../etc/passwd"), "etc-passwd");
        assert_eq!(clean_stem(""), "file");
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("a.csv"), "").unwrap();
        assert_eq!(unique_path(d.path(), "a", "csv"), d.path().join("a-2.csv"));
    }

    #[test]
    fn an_entry_reads_name_kind_and_size_from_the_file() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("a.xlsx"), b"12345").unwrap();
        let e = file_entry(d.path(), "a.xlsx");
        assert_eq!(
            (e["name"].as_str(), e["kind"].as_str(), e["size"].as_u64()),
            (Some("a.xlsx"), Some("sheet"), Some(5))
        );
        assert_eq!(file_entry(d.path(), "b.bin")["kind"], "file");
    }
}
