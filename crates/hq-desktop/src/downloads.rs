//! Where a file the web window downloads goes, and what the page is told afterwards.
//!
//! The web window has no way to call the app, and WKWebView's own handling of a blob download
//! saves nothing the person can find. So the shell takes the download itself: it only accepts a
//! file the page made (a `blob:` address of the proxy's origin) or one the proxy served, puts it in
//! the person's Downloads folder under a safe, unused name, and then tells the page with a plain
//! DOM event. Everything that decides is here, as pure functions, so it is tested without a window.

use reqwest::Url;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// The window event the page listens for (`apps/hq-web/src/lib/desktopDownload.ts`).
pub const EVENT_NAME: &str = "hq-desktop-download";

/// Longest file name kept, in bytes. Most file systems stop at 255; this leaves room for the
/// ` (12)` the app may add.
const MAX_NAME_BYTES: usize = 120;
const FALLBACK_NAME: &str = "download";

/// Whether the shell may save this address. A download that came from anywhere but the proxy's own
/// origin is refused. `blob:http://127.0.0.1:<port>/<id>` is the page's own in-memory file and
/// counts only when the origin inside it is the proxy's.
pub fn download_allowed(url: &str, origin: &str) -> bool {
    let (Ok(url), Ok(origin)) = (Url::parse(url), Url::parse(origin)) else {
        return false;
    };
    let target = if url.scheme() == "blob" {
        match Url::parse(url.path()) {
            Ok(inner) => inner,
            Err(_) => return false,
        }
    } else {
        url
    };
    target.scheme() == "http"
        && origin.scheme() == "http"
        && target.host_str().is_some()
        && target.host_str() == origin.host_str()
        && target.port_or_known_default() == origin.port_or_known_default()
        && target.username().is_empty()
        && target.password().is_none()
}

/// A file name that is safe to create inside a folder: no separators, no leading dot, no control
/// characters, a bounded length, never `.` or `..`, and never a path. An unusable name becomes
/// `download`. The extension survives a shortening.
pub fn sanitize_file_name(suggested: &str) -> String {
    // Keep what follows the last separator of either kind, so `../../etc/x` becomes `x`.
    let last = suggested.rsplit(['/', '\\']).next().unwrap_or("");
    let cleaned: String = last
        .chars()
        .filter(|c| !c.is_control())
        .map(|c| match c {
            ':' | '"' | '<' | '>' | '|' | '*' | '?' => '-',
            c => c,
        })
        .collect();
    let trimmed = cleaned
        .trim_matches(|c: char| c.is_whitespace() || c == '.')
        .to_string();
    if trimmed.is_empty() {
        return FALLBACK_NAME.to_string();
    }
    shorten(&trimmed)
}

/// Cuts a name to the byte limit on a character boundary, keeping a short extension.
fn shorten(name: &str) -> String {
    if name.len() <= MAX_NAME_BYTES {
        return name.to_string();
    }
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 && name.len() - i <= 12 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    let mut keep = MAX_NAME_BYTES.saturating_sub(ext.len());
    while !stem.is_char_boundary(keep) {
        keep -= 1;
    }
    let stem = stem[..keep].trim_end_matches(|c: char| c.is_whitespace() || c == '.');
    if stem.is_empty() {
        return FALLBACK_NAME.to_string();
    }
    format!("{stem}{ext}")
}

/// `dir/name`, or `dir/name (1).ext`, `(2)`, … for the first one that does not exist yet.
/// `exists` is a parameter so the choice is testable without a disk.
pub fn unique_destination(dir: &Path, name: &str, exists: impl Fn(&Path) -> bool) -> PathBuf {
    let first = dir.join(name);
    if !exists(&first) {
        return first;
    }
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    let mut n = 1u32;
    loop {
        let candidate = dir.join(format!("{stem} ({n}){ext}"));
        // The counter ends: past this, take the name and let the file system say no.
        if !exists(&candidate) || n == u32::MAX {
            return candidate;
        }
        n += 1;
    }
}

/// The person's Downloads folder, when the system names one.
pub fn downloads_dir() -> Option<PathBuf> {
    dirs::download_dir().filter(|d| d.is_absolute())
}

/// Where to save a file the page asked for, given the name the web view suggested (the last part
/// of its default destination). `None` when there is no Downloads folder, so the caller refuses
/// rather than guessing a place.
pub fn destination_for(
    suggested: &Path,
    dir: Option<&Path>,
    exists: impl Fn(&Path) -> bool,
) -> Option<PathBuf> {
    let dir = dir?;
    let raw = suggested.file_name()?.to_string_lossy().into_owned();
    Some(unique_destination(dir, &sanitize_file_name(&raw), exists))
}

/// Most downloads the shell remembers at once; a page cannot start more than a few by hand.
const MAX_PENDING: usize = 64;

/// Where each running download was told to go. macOS reports no path when a download ends, so
/// without this the page would be told every download failed, even one that landed.
#[derive(Default)]
pub struct PendingDownloads(Mutex<HashMap<String, PathBuf>>);

impl PendingDownloads {
    pub fn remember(&self, url: &str, path: &Path) {
        let Ok(mut map) = self.0.lock() else { return };
        if map.len() >= MAX_PENDING {
            map.clear();
        }
        map.insert(url.to_string(), path.to_path_buf());
    }

    /// The path a finished download went to: the one the system reported, else the one remembered.
    pub fn resolve(&self, url: &str, reported: Option<PathBuf>) -> Option<PathBuf> {
        let kept = self.0.lock().ok().and_then(|mut map| map.remove(url));
        reported.or(kept)
    }
}

/// The script that tells the page how a download ended: one `CustomEvent` with `{name, path,
/// success}`. The detail is built with serde_json, so a file name cannot break out of the string.
pub fn finished_script(path: Option<&Path>, success: bool) -> String {
    let name = path
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let detail = serde_json::json!({
        "name": name,
        "path": path.map(|p| p.to_string_lossy().into_owned()).unwrap_or_default(),
        "success": success && path.is_some(),
    });
    // U+2028 and U+2029 are legal in a JS string since ES2019, but escape them for older engines.
    let detail = detail
        .to_string()
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029");
    format!(
        "window.dispatchEvent(new CustomEvent({}, {{ detail: {detail} }}));",
        serde_json::Value::String(EVENT_NAME.to_string())
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const ORIGIN: &str = "http://127.0.0.1:47474";

    /// The JSON object between `detail: ` and the closing ` }));`.
    fn detail_of(script: &str) -> serde_json::Value {
        let json = script.split_once("detail: ").unwrap().1;
        let end = json.rfind(" }));").unwrap();
        serde_json::from_str(&json[..end]).unwrap()
    }

    #[test]
    fn a_finished_download_is_named_from_where_it_was_sent() {
        let pending = PendingDownloads::default();
        let sent = PathBuf::from("/Users/x/Downloads/a.html");
        pending.remember("blob:http://127.0.0.1:1/id", &sent);
        assert_eq!(
            pending.resolve("blob:http://127.0.0.1:1/id", None),
            Some(sent)
        );
        assert_eq!(pending.resolve("blob:http://127.0.0.1:1/id", None), None);
        let reported = PathBuf::from("/Users/x/Downloads/b.html");
        pending.remember("u", Path::new("/Users/x/Downloads/other.html"));
        assert_eq!(pending.resolve("u", Some(reported.clone())), Some(reported));
        assert_eq!(pending.resolve("never-started", None), None);
    }

    #[test]
    fn a_blob_of_the_proxy_origin_and_the_proxy_itself_are_allowed() {
        assert!(download_allowed(
            "blob:http://127.0.0.1:47474/3f2a-11",
            ORIGIN
        ));
        assert!(download_allowed(
            "http://127.0.0.1:47474/api/note/export?x=1",
            ORIGIN
        ));
    }

    #[test]
    fn everything_else_is_refused() {
        for url in [
            "blob:http://evil.example.com/3f2a",
            "blob:https://127.0.0.1:47474/3f2a",
            "blob:null/3f2a",
            "blob:",
            "https://127.0.0.1:47474/x",
            "http://127.0.0.1:47475/x",
            "http://127.0.0.1:47474.evil.com/x",
            "http://user@127.0.0.1:47474/x",
            "blob:http://user@127.0.0.1:47474/x",
            "file:///etc/passwd",
            "data:text/plain,hi",
            "ftp://127.0.0.1:47474/x",
            "not a url",
            "",
        ] {
            assert!(!download_allowed(url, ORIGIN), "{url}");
        }
        assert!(!download_allowed(
            "blob:http://127.0.0.1:47474/x",
            "not an origin"
        ));
    }

    #[test]
    fn a_name_never_carries_a_path_or_a_leading_dot() {
        assert_eq!(sanitize_file_name("Plan.xlsx"), "Plan.xlsx");
        assert_eq!(sanitize_file_name("../../etc/passwd"), "passwd");
        assert_eq!(sanitize_file_name("..\\..\\evil.exe"), "evil.exe");
        assert_eq!(sanitize_file_name("/abs/path/file.csv"), "file.csv");
        assert_eq!(sanitize_file_name(".hidden"), "hidden");
        assert_eq!(sanitize_file_name("...rc"), "rc");
        assert_eq!(sanitize_file_name("a:b*c?.pdf"), "a-b-c-.pdf");
        assert_eq!(sanitize_file_name("tab\there\n.txt"), "tabhere.txt");
        assert_eq!(sanitize_file_name("  spaced.md  "), "spaced.md");
        assert_eq!(sanitize_file_name("trailing."), "trailing");
    }

    #[test]
    fn an_unusable_name_becomes_download() {
        for raw in ["", ".", "..", "...", "/", "a/..", "\\", "   ", "\u{0}"] {
            assert_eq!(sanitize_file_name(raw), "download", "{raw:?}");
        }
    }

    #[test]
    fn a_long_name_is_cut_on_a_character_boundary_and_keeps_its_extension() {
        let long = format!("{}.xlsx", "é".repeat(200));
        let out = sanitize_file_name(&long);
        assert!(out.len() <= MAX_NAME_BYTES, "{}", out.len());
        assert!(out.ends_with(".xlsx"));
        let no_ext = "a".repeat(500);
        assert_eq!(sanitize_file_name(&no_ext).len(), MAX_NAME_BYTES);
        // A "long extension" is part of the stem, not kept whole.
        let odd = format!("x.{}", "b".repeat(300));
        assert!(sanitize_file_name(&odd).len() <= MAX_NAME_BYTES);
    }

    #[test]
    fn an_existing_file_is_never_overwritten() {
        let dir = Path::new("/srv/example/Downloads");
        let taken = ["Plan.xlsx", "Plan (1).xlsx", "Plan (2).xlsx", "README"];
        let exists = |p: &Path| {
            p.parent() == Some(dir) && taken.contains(&p.file_name().unwrap().to_str().unwrap())
        };
        assert_eq!(
            unique_destination(dir, "Plan.xlsx", exists),
            dir.join("Plan (3).xlsx")
        );
        assert_eq!(
            unique_destination(dir, "New.csv", exists),
            dir.join("New.csv")
        );
        assert_eq!(
            unique_destination(dir, "README", exists),
            dir.join("README (1)")
        );
        // A leading dot is never produced by sanitizing, but the stem split must not break on one.
        assert_eq!(
            unique_destination(dir, "notes", |_| false),
            dir.join("notes")
        );
    }

    #[test]
    fn the_destination_is_inside_downloads_whatever_the_web_view_suggested() {
        let dir = Path::new("/srv/example/Downloads");
        let none = |_: &Path| false;
        let got = destination_for(
            Path::new("/tmp/elsewhere/../../etc/Plan.xlsx"),
            Some(dir),
            none,
        )
        .unwrap();
        assert_eq!(got, dir.join("Plan.xlsx"));
        assert_eq!(got.parent(), Some(dir));
        let dotted = destination_for(Path::new("/x/.ssh"), Some(dir), none).unwrap();
        assert_eq!(dotted, dir.join("ssh"));
        assert_eq!(destination_for(Path::new("/x/Plan.xlsx"), None, none), None);
        assert_eq!(destination_for(Path::new("/"), Some(dir), none), None);
    }

    #[test]
    fn the_script_carries_the_detail_as_data_not_code() {
        let path = PathBuf::from("/srv/example/Downloads/a\"); alert(1); (\".csv");
        let script = finished_script(Some(&path), true);
        assert!(script.starts_with("window.dispatchEvent(new CustomEvent(\"hq-desktop-download\""));
        let value = detail_of(&script);
        assert_eq!(value["name"], "a\"); alert(1); (\".csv");
        assert_eq!(value["path"], path.to_string_lossy().as_ref());
        assert_eq!(value["success"], true);
    }

    #[test]
    fn a_failed_download_says_so() {
        let value = |s: String| detail_of(&s);
        let failed = value(finished_script(None, false));
        assert_eq!(failed["success"], false);
        assert_eq!(failed["name"], "");
        // A "success" with no path is not one.
        assert_eq!(value(finished_script(None, true))["success"], false);
        let line = finished_script(Some(Path::new("/d/a\u{2028}b.txt")), true);
        assert!(!line.contains('\u{2028}'));
    }
}
