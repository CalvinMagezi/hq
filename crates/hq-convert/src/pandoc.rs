//! Running pandoc on text a model or a note supplies.
//!
//! Every such run passes `--sandbox`, so pandoc's readers and writers may read only the files
//! named on its command line (the input, a `--reference-doc`). That is not the whole answer:
//! for PDF output pandoc fetches a document's images before the sandboxed writer runs, and
//! before 3.6 `--embed-resources` was not sandboxed at all. An image whose target is
//! `/home/u/.ssh/id_ed25519` or `http://169.254.169.254/` would be read or requested either way.
//! So a document is first read into pandoc's JSON form (reading Markdown does no IO), every
//! image is checked here ([`confine_images`]), and only then written. What is allowed is a
//! `data:` URI, or, when a vault is given, an image file inside it, which is inlined as a
//! `data:` URI so the writer never has to open it.
//!
//! A pandoc older than [`MIN_VERSION`] has no `--sandbox` and is refused rather than run
//! without one.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use base64::Engine as _;
use serde_json::{Value, json};
use tokio::process::Command;

use crate::types::ConvertError;

/// The first pandoc with `--sandbox`.
pub const MIN_VERSION: (u32, u32) = (2, 15);

/// The largest image inlined from the vault. Bigger ones are left out with their alt text.
const MAX_IMAGE_BYTES: u64 = 25 * 1024 * 1024;

/// Image types that are inlined, by extension, with their media type.
const IMAGE_TYPES: &[(&str, &str)] = &[
    ("png", "image/png"),
    ("jpg", "image/jpeg"),
    ("jpeg", "image/jpeg"),
    ("gif", "image/gif"),
    ("webp", "image/webp"),
    ("svg", "image/svg+xml"),
];

/// Which images a document may keep.
#[derive(Debug, Clone, Copy)]
pub enum ImageRule<'a> {
    /// Only `data:` URIs. Everything else is replaced by its alt text.
    DataOnly,
    /// `data:` URIs, and image files inside this vault (a relative target is taken from its
    /// root), inlined as `data:` URIs.
    Vault(&'a Path),
}

/// `pandoc 3.1.3` (or `pandoc.exe 2.19.2`) on the first line of `pandoc --version`.
pub(crate) fn parse_version(text: &str) -> Option<(u32, u32, u32)> {
    let mut words = text.lines().next()?.split_whitespace();
    if !words.next()?.starts_with("pandoc") {
        return None;
    }
    let mut parts = words.next()?.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next().map_or(Some(0), |p| p.parse().ok())?;
    let patch = parts
        .next()
        .and_then(|p| {
            p.chars()
                .take_while(char::is_ascii_digit)
                .collect::<String>()
                .parse()
                .ok()
        })
        .unwrap_or(0);
    Some((major, minor, patch))
}

type Version = (u32, u32, u32);

fn version_of(bin: &Path) -> Option<Version> {
    static SEEN: OnceLock<Mutex<HashMap<PathBuf, Version>>> = OnceLock::new();
    let seen = SEEN.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(v) = seen.lock().ok()?.get(bin) {
        return Some(*v);
    }
    let out = std::process::Command::new(bin)
        .arg("--version")
        .output()
        .ok()?;
    let v = parse_version(&String::from_utf8_lossy(&out.stdout))?;
    seen.lock().ok()?.insert(bin.to_path_buf(), v);
    Some(v)
}

/// Refuse a pandoc that cannot be sandboxed. The version is read once per binary.
pub fn ensure_sandbox(bin: &Path) -> Result<(), ConvertError> {
    check_version(version_of(bin))
}

fn check_version(version: Option<(u32, u32, u32)>) -> Result<(), ConvertError> {
    let (want_major, want_minor) = MIN_VERSION;
    match version {
        Some((major, minor, _)) if (major, minor) >= MIN_VERSION => Ok(()),
        Some((major, minor, patch)) => Err(ConvertError::OutboundFailed(format!(
            "pandoc {major}.{minor}.{patch} is too old: HQ only runs pandoc with --sandbox, which \
             needs pandoc {want_major}.{want_minor} or newer. Install a current release from \
             https://pandoc.org/installing.html"
        ))),
        None => Err(ConvertError::OutboundFailed(
            "could not read the version of pandoc (`pandoc --version`), so it cannot be run \
             sandboxed"
                .into(),
        )),
    }
}

/// A pandoc failure, with the one case people hit explained: a pandoc built without its data
/// files inside the binary (Debian and Ubuntu packages) cannot open them under `--sandbox`, so
/// docx, pptx, odt and epub fail. Release builds from pandoc.org embed them.
pub(crate) fn failure(msg: String) -> ConvertError {
    if msg.contains("Could not find data file") {
        return ConvertError::OutboundFailed(format!(
            "{msg}\nThis pandoc reads its data files from disk, which --sandbox does not allow. \
             Install a release build from https://pandoc.org/installing.html (they carry their \
             data files inside the binary)."
        ));
    }
    ConvertError::OutboundFailed(msg)
}

/// `--sandbox <input> -f <reader> -t json`.
pub(crate) fn reader_args(input: &Path, reader: &str) -> Vec<String> {
    vec![
        "--sandbox".into(),
        input.to_string_lossy().into_owned(),
        "-f".into(),
        reader.into(),
        "-t".into(),
        "json".into(),
    ]
}

/// Read `input` with `reader` into pandoc's JSON form, sandboxed, and confine its images.
/// Returns the JSON text to hand to the writer and how many images were left out.
pub(crate) async fn read_confined(
    bin: &Path,
    input: &Path,
    reader: &str,
    rule: ImageRule<'_>,
) -> Result<(Vec<u8>, usize), ConvertError> {
    let mut cmd = Command::new(bin);
    cmd.kill_on_drop(true).args(reader_args(input, reader));
    let out = crate::run_tool(cmd, "pandoc").await.map_err(failure)?;
    let mut ast: Value = serde_json::from_slice(&out)
        .map_err(|e| ConvertError::OutboundFailed(format!("pandoc gave unreadable JSON: {e}")))?;
    let dropped = confine_images(&mut ast, rule);
    let bytes = serde_json::to_vec(&ast)
        .map_err(|e| ConvertError::OutboundFailed(format!("could not write the document: {e}")))?;
    Ok((bytes, dropped))
}

/// Replace every image the rule does not allow by its alt text, and inline the allowed vault
/// images as `data:` URIs. Returns how many images were left out.
pub fn confine_images(ast: &mut Value, rule: ImageRule<'_>) -> usize {
    let vault = match rule {
        ImageRule::Vault(v) => v.canonicalize().ok(),
        ImageRule::DataOnly => None,
    };
    let mut dropped = 0;
    walk(ast, vault.as_deref(), &mut dropped);
    dropped
}

fn walk(node: &mut Value, vault: Option<&Path>, dropped: &mut usize) {
    match node {
        Value::Array(items) => {
            for item in items {
                walk(item, vault, dropped);
            }
        }
        Value::Object(map) => {
            if map.get("t").and_then(Value::as_str) == Some("Image")
                && let Some(replacement) = confine_one(map.get("c"), vault, dropped)
            {
                *node = replacement;
                // The alt text of a dropped image is still walked: it may hold another image.
                walk(node, vault, dropped);
                return;
            }
            for child in map.values_mut() {
                walk(child, vault, dropped);
            }
        }
        _ => {}
    }
}

/// `None` keeps the image as it is (after rewriting its target in place, if inlined).
fn confine_one(
    content: Option<&Value>,
    vault: Option<&Path>,
    dropped: &mut usize,
) -> Option<Value> {
    let parts = content.and_then(Value::as_array);
    let target = parts
        .and_then(|p| p.get(2))
        .and_then(Value::as_array)
        .and_then(|t| t.first())
        .and_then(Value::as_str)
        .unwrap_or("");
    if target.len() >= 5 && target[..5].eq_ignore_ascii_case("data:") {
        return None;
    }
    let attr = parts
        .and_then(|p| p.first())
        .cloned()
        .unwrap_or_else(|| json!(["", [], []]));
    if let Some(uri) = vault.and_then(|v| vault_data_uri(target, v)) {
        let title = parts
            .and_then(|p| p.get(2))
            .and_then(Value::as_array)
            .and_then(|t| t.get(1))
            .cloned()
            .unwrap_or_else(|| json!(""));
        let alt = parts
            .and_then(|p| p.get(1))
            .cloned()
            .unwrap_or_else(|| json!([]));
        return Some(json!({"t": "Image", "c": [attr, alt, [uri, title]]}));
    }
    *dropped += 1;
    let mut alt = parts
        .and_then(|p| p.get(1))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if alt.is_empty() {
        alt.push(json!({"t": "Str", "c": "[image]"}));
    }
    Some(json!({"t": "Span", "c": [attr, alt]}))
}

/// The target as a `data:` URI, when it names an image file inside the vault.
fn vault_data_uri(target: &str, vault: &Path) -> Option<String> {
    if target.contains("://") || target.starts_with("//") {
        return None;
    }
    let decoded = percent_decode(target);
    let path = Path::new(&decoded);
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        vault.join(path)
    };
    let real = path.canonicalize().ok()?;
    if !real.starts_with(vault) {
        return None;
    }
    let ext = real.extension()?.to_str()?.to_ascii_lowercase();
    let (_, mime) = IMAGE_TYPES.iter().find(|(e, _)| *e == ext)?;
    let meta = std::fs::metadata(&real).ok()?;
    if !meta.is_file() || meta.len() > MAX_IMAGE_BYTES {
        return None;
    }
    let bytes = std::fs::read(&real).ok()?;
    Some(format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    ))
}

/// A file as a `data:` URI with an image media type, for HQ's own assets (a brand logo).
pub(crate) fn image_data_uri(path: &Path) -> Option<String> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    let (_, mime) = IMAGE_TYPES.iter().find(|(e, _)| *e == ext)?;
    let bytes = std::fs::read(path).ok()?;
    Some(format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    ))
}

pub(crate) fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(hex) = s.get(i + 1..i + 3)
            && let Ok(v) = u8::from_str_radix(hex, 16)
        {
            out.push(v);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(target: &str, alt: &str) -> Value {
        let alt: Vec<Value> = if alt.is_empty() {
            vec![]
        } else {
            vec![json!({"t": "Str", "c": alt})]
        };
        json!({"t": "Image", "c": [["", [], []], alt, [target, ""]]})
    }

    fn doc(inlines: Vec<Value>) -> Value {
        json!({
            "pandoc-api-version": [1, 23, 1],
            "meta": {},
            "blocks": [{"t": "Para", "c": inlines}]
        })
    }

    fn inlines(ast: &Value) -> &Vec<Value> {
        ast["blocks"][0]["c"].as_array().unwrap()
    }

    #[test]
    fn versions_parse_from_the_first_line() {
        assert_eq!(
            parse_version("pandoc 3.1.3\nFeatures: +server"),
            Some((3, 1, 3))
        );
        assert_eq!(parse_version("pandoc.exe 2.19.2"), Some((2, 19, 2)));
        assert_eq!(parse_version("pandoc 3.6"), Some((3, 6, 0)));
        assert_eq!(parse_version("pandoc 2.9.2.1"), Some((2, 9, 2)));
        assert_eq!(parse_version("something else 1.0"), None);
        assert_eq!(parse_version(""), None);
    }

    #[test]
    fn a_pandoc_without_sandbox_is_refused_with_the_version_it_needs() {
        assert!(check_version(Some((2, 15, 0))).is_ok());
        assert!(check_version(Some((3, 1, 3))).is_ok());
        for old in [(2, 14, 2), (2, 9, 2), (1, 19, 0)] {
            let err = check_version(Some(old)).unwrap_err().to_string();
            assert!(err.contains("too old") && err.contains("2.15"), "{err}");
        }
        assert!(check_version(None).is_err());
    }

    #[test]
    fn the_reader_runs_sandboxed_and_writes_json() {
        let args = reader_args(Path::new("/tmp/in.md"), "markdown-raw_tex");
        assert_eq!(args[0], "--sandbox");
        assert_eq!(
            args[1..],
            ["/tmp/in.md", "-f", "markdown-raw_tex", "-t", "json"]
        );
    }

    #[test]
    fn only_data_uris_survive_without_a_vault() {
        let mut ast = doc(vec![
            image("/home/u/.ssh/id_ed25519", "key"),
            image("http://169.254.169.254/latest/meta-data", ""),
            image("file:///etc/passwd", "p"),
            image("relative/chart.png", "chart"),
            image("data:image/png;base64,AAAA", "inline"),
            image("DATA:image/png;base64,AAAA", "shouted"),
        ]);
        assert_eq!(confine_images(&mut ast, ImageRule::DataOnly), 4);
        let out = inlines(&ast);
        let kinds: Vec<&str> = out.iter().map(|n| n["t"].as_str().unwrap()).collect();
        assert_eq!(kinds, ["Span", "Span", "Span", "Span", "Image", "Image"]);
        assert_eq!(out[0]["c"][1][0]["c"], "key", "the alt text stays");
        assert_eq!(
            out[1]["c"][1][0]["c"], "[image]",
            "an empty alt says what was there"
        );
        let text = serde_json::to_string(&ast).unwrap();
        assert!(
            !text.contains("id_ed25519") && !text.contains("169.254"),
            "{text}"
        );
    }

    #[test]
    fn nested_images_and_figures_are_reached() {
        let mut ast = json!({
            "pandoc-api-version": [1, 23, 1],
            "meta": {"title": {"t": "MetaInlines", "c": [image("/etc/passwd", "t")]}},
            "blocks": [
                {"t": "Figure", "c": [["", [], []], [null, []], [{"t": "Plain", "c": [image("/etc/hosts", "f")]}]]},
                {"t": "Para", "c": [{"t": "Link", "c": [["", [], []], [image("/etc/shadow", "l")], ["https://example.com", ""]]}]},
                {"t": "Para", "c": [{"t": "Image", "c": [["", [], []], [image("/etc/group", "inner")], ["/etc/motd", ""]]}]}
            ]
        });
        assert_eq!(confine_images(&mut ast, ImageRule::DataOnly), 5);
        let text = serde_json::to_string(&ast).unwrap();
        assert!(!text.contains("/etc/"), "{text}");
    }

    #[test]
    fn vault_images_are_inlined_and_everything_outside_is_left_out() {
        let vault = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(vault.path().join("img")).unwrap();
        std::fs::write(vault.path().join("img/chart.png"), b"PNGDATA").unwrap();
        std::fs::write(vault.path().join("notes.txt"), b"not an image").unwrap();
        std::fs::write(outside.path().join("secret.png"), b"SECRET").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(
            outside.path().join("secret.png"),
            vault.path().join("img/link.png"),
        )
        .unwrap();
        let inside = vault.path().join("img/chart.png");
        let mut ast = doc(vec![
            image(&inside.to_string_lossy(), "abs"),
            image("img/chart.png", "rel"),
            image(
                &format!("{}/img/../../x.png", vault.path().display()),
                "escape",
            ),
            image(&outside.path().join("secret.png").to_string_lossy(), "out"),
            image("img/link.png", "link"),
            image("notes.txt", "txt"),
            image("https://example.com/a.png", "remote"),
        ]);
        let dropped = confine_images(&mut ast, ImageRule::Vault(vault.path()));
        let out = inlines(&ast);
        let want = format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(b"PNGDATA")
        );
        assert_eq!(out[0]["c"][2][0], want.as_str());
        assert_eq!(out[1]["c"][2][0], want.as_str());
        for (i, node) in out.iter().enumerate().skip(2) {
            assert_eq!(node["t"], "Span", "{i}: {node}");
        }
        assert_eq!(dropped, out.len() - 2);
        assert!(!serde_json::to_string(&ast).unwrap().contains("U0VDUkVU"));
    }

    #[test]
    fn the_data_file_failure_says_what_to_install() {
        let err = failure("Could not find data file data/docx/[Content_Types].xml".into());
        assert!(err.to_string().contains("pandoc.org"), "{err}");
        let other = failure("something else".into()).to_string();
        assert!(!other.contains("pandoc.org"), "{other}");
    }
}
