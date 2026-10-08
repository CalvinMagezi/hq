//! Vault note → shareable PDF.
//!
//! A note is not ready to hand to someone else as-is: it carries YAML
//! frontmatter, `[[wikilinks]]` that mean nothing outside the vault, and
//! `![[embeds]]` that point at vault-relative files. [`prepare_note`] turns the
//! raw text into plain Markdown a stranger can read, and [`export_note_pdf`]
//! renders it.
//!
//! Rendering picks the first engine it finds (see [`PdfEngine::detect`]):
//!
//! 1. a Chromium-family browser in headless mode (best typography, Unicode and
//!    emoji just work, and most machines already have one),
//! 2. WeasyPrint,
//! 3. `xelatex` through pandoc (the heaviest install, kept as a last resort).
//!
//! The first two go pandoc → standalone HTML → PDF, so the same stylesheet,
//! including optional brand colours and font, applies to both.
//!
//! A PDF is meant to leave the machine, so images are confined to the vault:
//! anything that resolves outside it, and every remote image, is replaced by a
//! placeholder rather than embedded. Raw HTML in the note is dropped.

use std::path::{Component, Path, PathBuf};
use std::sync::LazyLock;
use std::time::Duration;

use regex::{Captures, Regex};
use tokio::process::Command;
use tracing::{debug, warn};
use which::which;

use crate::brand::BrandKit;
use crate::outbound::OutboundConverter;
use crate::types::{ConvertError, ExportFormat};

const RENDER_TIMEOUT: Duration = Duration::from_secs(120);
const IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "svg"];
/// Bound on the vault walk that looks an embed up by bare file name.
const MAX_WALK_ENTRIES: usize = 20_000;

/// The renderer used to turn a note into a PDF.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PdfEngine {
    /// A Chromium-family browser binary, run headless.
    Browser(PathBuf),
    Weasyprint(PathBuf),
    Xelatex,
}

impl PdfEngine {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Browser(_) => "chromium",
            Self::Weasyprint(_) => "weasyprint",
            Self::Xelatex => "xelatex",
        }
    }

    /// Pick an engine. `HQ_PDF_ENGINE` (`chromium`, `weasyprint`, `xelatex`)
    /// forces one and fails if it is missing; `HQ_PDF_BROWSER` names the browser
    /// binary when it is not on `PATH`.
    pub fn detect() -> Result<Self, ConvertError> {
        let forced = std::env::var("HQ_PDF_ENGINE")
            .ok()
            .map(|v| v.trim().to_ascii_lowercase())
            .filter(|v| !v.is_empty());
        match forced.as_deref() {
            Some("chromium" | "chrome" | "browser") => find_browser()
                .map(Self::Browser)
                .ok_or(ConvertError::NoPdfEngine),
            Some("weasyprint") => which("weasyprint")
                .map(Self::Weasyprint)
                .map_err(|_| ConvertError::NoPdfEngine),
            Some("xelatex") => which("xelatex")
                .map(|_| Self::Xelatex)
                .map_err(|_| ConvertError::NoPdfEngine),
            Some(other) => Err(ConvertError::Other(format!(
                "unknown HQ_PDF_ENGINE '{other}' (use chromium, weasyprint or xelatex)"
            ))),
            None => find_browser()
                .map(Self::Browser)
                .or_else(|| which("weasyprint").ok().map(Self::Weasyprint))
                .or_else(|| which("xelatex").ok().map(|_| Self::Xelatex))
                .ok_or(ConvertError::NoPdfEngine),
        }
    }
}

fn find_browser() -> Option<PathBuf> {
    if let Ok(custom) = std::env::var("HQ_PDF_BROWSER") {
        let p = PathBuf::from(custom.trim());
        if p.is_file() {
            return Some(p);
        }
        return which(&p).ok();
    }
    const NAMES: &[&str] = &[
        "google-chrome",
        "google-chrome-stable",
        "chromium",
        "chromium-browser",
        "chrome",
        "microsoft-edge",
        "microsoft-edge-stable",
    ];
    if let Some(found) = NAMES.iter().find_map(|n| which(n).ok()) {
        return Some(found);
    }
    const MAC_APPS: &[&str] = &[
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        "/Applications/Chromium.app/Contents/MacOS/Chromium",
        "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
        "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser",
    ];
    MAC_APPS.iter().map(PathBuf::from).find(|p| p.is_file())
}

/// A note reduced to what a reader outside the vault should see.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedNote {
    pub title: String,
    pub markdown: String,
}

/// Outcome of a successful export.
#[derive(Debug, Clone)]
pub struct NotePdf {
    pub path: PathBuf,
    pub size_bytes: u64,
    pub title: String,
    pub engine: &'static str,
}

static EMBED_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"!\[\[([^\]|#]+)(?:#[^\]|]*)?(?:\|([^\]]*))?\]\]").unwrap());
static WIKILINK_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\[\[([^\]|#]+)(?:#[^\]|]*)?(?:\|([^\]]*))?\]\]").unwrap());
static IMAGE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"!\[([^\]]*)\]\((?:<([^>]*)>|([^)\s]*))(?:\s+"[^"]*")?\)"#).unwrap()
});
static CALLOUT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(\s*>\s*)\[!(\w+)\][+-]?\s*(.*)$").unwrap());
static COMMENT_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"%%.*?%%").unwrap());

/// Reduce raw note text to shareable Markdown.
///
/// `note_dir` and `vault` are absolute; relative image references resolve
/// against the note's folder, then the vault root, and never leave `vault`.
pub fn prepare_note(
    raw: &str,
    fallback_title: &str,
    note_dir: &Path,
    vault: &Path,
) -> PreparedNote {
    prepare_note_with(raw, fallback_title, note_dir, vault, false)
}

/// Like [`prepare_note`], with `keep_callouts` left on the `> [!kind] Title`
/// syntax for renderers that draw real callout boxes. `prepare_note` flattens
/// them into a bold label, which is what a pandoc-based renderer needs.
pub fn prepare_note_with(
    raw: &str,
    fallback_title: &str,
    note_dir: &Path,
    vault: &Path,
    keep_callouts: bool,
) -> PreparedNote {
    let (frontmatter, body) = split_frontmatter(raw);
    let fm_title = frontmatter
        .as_deref()
        .and_then(|fm| serde_yaml::from_str::<serde_yaml::Value>(fm).ok())
        .and_then(|v| v.get("title").and_then(|t| t.as_str().map(str::to_owned)))
        .map(|t| t.trim().to_owned())
        .filter(|t| !t.is_empty());

    let mut lines: Vec<String> = Vec::new();
    let mut fence: Option<String> = None;
    for line in body.lines() {
        let trimmed = line.trim_start();
        if let Some(marker) = fence_marker(trimmed) {
            match &fence {
                None => fence = Some(marker),
                Some(open) if marker.starts_with(open.as_str()) => fence = None,
                Some(_) => {}
            }
            lines.push(line.to_owned());
            continue;
        }
        if fence.is_some() {
            lines.push(line.to_owned());
            continue;
        }
        lines.push(rewrite_line(line, note_dir, vault, keep_callouts));
    }

    // The title block already prints the title, so a leading H1 that says the
    // same thing would appear twice.
    let first = lines.iter().position(|l| !l.trim().is_empty());
    let mut title = fm_title.clone();
    if let Some(i) = first
        && let Some(h1) = lines[i].strip_prefix("# ")
    {
        let h1 = h1.trim().to_owned();
        if title.is_none() {
            title = Some(h1);
            lines.remove(i);
        } else if title
            .as_deref()
            .is_some_and(|t| t.eq_ignore_ascii_case(&h1))
        {
            lines.remove(i);
        }
    }

    PreparedNote {
        title: title.unwrap_or_else(|| fallback_title.to_owned()),
        markdown: lines.join("\n"),
    }
}

/// Returns `(frontmatter, body)`. Text without a closed `---` block comes back whole.
fn split_frontmatter(raw: &str) -> (Option<String>, &str) {
    let text = raw.strip_prefix('\u{feff}').unwrap_or(raw);
    let Some(rest) = text
        .strip_prefix("---\n")
        .or_else(|| text.strip_prefix("---\r\n"))
    else {
        return (None, text);
    };
    let mut offset = 0;
    for line in rest.split_inclusive('\n') {
        if line.trim_end() == "---" || line.trim_end() == "..." {
            let fm = rest[..offset].to_owned();
            return (Some(fm), &rest[offset + line.len()..]);
        }
        offset += line.len();
    }
    (None, text)
}

fn fence_marker(trimmed: &str) -> Option<String> {
    for ch in ['`', '~'] {
        let n = trimmed.chars().take_while(|c| *c == ch).count();
        if n >= 3 {
            return Some(ch.to_string().repeat(n));
        }
    }
    None
}

fn rewrite_line(line: &str, note_dir: &Path, vault: &Path, keep_callouts: bool) -> String {
    let line = COMMENT_RE.replace_all(line, "");
    // Standard images first: the embed rule emits absolute paths that the
    // standard-image rule would otherwise reject on a second pass.
    let line = IMAGE_RE.replace_all(&line, |c: &Captures| {
        let alt = &c[1];
        let dest = c.get(2).or_else(|| c.get(3)).map_or("", |m| m.as_str());
        rewrite_image(alt, dest, note_dir, vault)
    });
    let line = EMBED_RE.replace_all(&line, |c: &Captures| {
        let target = c[1].trim();
        let alt = c
            .get(2)
            .map(|m| m.as_str())
            .filter(|a| a.parse::<u32>().is_err());
        if is_image_name(target) {
            match find_in_vault(target, note_dir, vault) {
                Some(p) => image_md(alt.unwrap_or(""), &p),
                None => format!("*[image not found: {target}]*"),
            }
        } else {
            // Another note embedded in this one: its text is not ours to inline.
            display_name(target)
        }
    });
    let line = WIKILINK_RE.replace_all(&line, |c: &Captures| {
        c.get(2)
            .map(|m| m.as_str().trim().to_owned())
            .filter(|a| !a.is_empty())
            .unwrap_or_else(|| display_name(c[1].trim()))
    });
    if keep_callouts {
        return line.into_owned();
    }
    match CALLOUT_RE.captures(&line) {
        Some(c) => {
            let kind = capitalise(&c[2]);
            let title = c[3].trim();
            let label = if title.is_empty() {
                kind
            } else {
                format!("{kind}: {title}")
            };
            format!("{}**{label}**", &c[1])
        }
        None => line.into_owned(),
    }
}

fn rewrite_image(alt: &str, dest: &str, note_dir: &Path, vault: &Path) -> String {
    let dest = dest.trim();
    if dest.starts_with("data:") {
        return format!("![{alt}]({dest})");
    }
    if dest.contains("://") {
        // Fetching remote images at export time would make the export depend on
        // the network and on whoever controls the URL.
        let label = if alt.is_empty() { "image" } else { alt };
        return format!("[{label}]({dest})");
    }
    match find_in_vault(&percent_decode(dest), note_dir, vault) {
        Some(p) => image_md(alt, &p),
        None => format!(
            "*[image unavailable: {}]*",
            if alt.is_empty() { dest } else { alt }
        ),
    }
}

fn image_md(alt: &str, path: &Path) -> String {
    format!("![{alt}](<{}>)", path.display())
}

fn is_image_name(name: &str) -> bool {
    Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| IMAGE_EXTS.contains(&e.to_ascii_lowercase().as_str()))
}

fn display_name(target: &str) -> String {
    let last = target.rsplit('/').next().unwrap_or(target);
    last.strip_suffix(".md").unwrap_or(last).to_owned()
}

fn capitalise(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(f) => f.to_uppercase().collect::<String>() + &chars.as_str().to_lowercase(),
        None => String::new(),
    }
}

fn percent_decode(s: &str) -> String {
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

/// Resolve `name` to an existing file inside `vault`: next to the note, then
/// from the vault root, then by bare file name anywhere in the vault. Returns
/// `None` for absolute paths, `..` escapes and symlinks that leave the vault.
fn find_in_vault(name: &str, note_dir: &Path, vault: &Path) -> Option<PathBuf> {
    let rel = Path::new(name.trim_start_matches('/'));
    if rel.as_os_str().is_empty()
        || rel
            .components()
            .any(|c| !matches!(c, Component::Normal(_) | Component::CurDir))
    {
        return None;
    }
    let root = std::fs::canonicalize(vault).ok()?;
    let inside = |p: PathBuf| -> Option<PathBuf> {
        let canon = std::fs::canonicalize(&p).ok()?;
        (canon.is_file() && canon.starts_with(&root)).then_some(canon)
    };
    if let Some(hit) = inside(note_dir.join(rel)).or_else(|| inside(vault.join(rel))) {
        return Some(hit);
    }
    if rel.components().count() == 1 {
        let wanted = rel.file_name()?.to_owned();
        let mut budget = MAX_WALK_ENTRIES;
        let mut stack = vec![vault.to_path_buf()];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                budget = budget.checked_sub(1)?;
                let path = entry.path();
                let hidden = entry.file_name().to_string_lossy().starts_with('.');
                match entry.file_type() {
                    Ok(t) if t.is_dir() && !hidden => stack.push(path),
                    Ok(t) if t.is_file() && entry.file_name() == wanted => {
                        if let Some(hit) = inside(path) {
                            return Some(hit);
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    None
}

/// Keep only characters that can sit inside a CSS value without ending it.
fn css_safe(value: &str, extra: &str) -> String {
    value
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || " #,.()%-".contains(*c) || extra.contains(*c))
        .collect()
}

fn stylesheet(brand: Option<&BrandKit>) -> String {
    let accent = brand
        .map(|b| css_safe(&b.primary_color, ""))
        .filter(|c| !c.trim().is_empty())
        .unwrap_or_else(|| "#1f4e79".to_owned());
    let family = brand
        .map(|b| css_safe(&b.font, ""))
        .filter(|f| !f.trim().is_empty())
        .map(|f| format!("\"{}\", ", f.trim()))
        .unwrap_or_default();
    let footer = brand
        .map(|b| css_safe(&b.brand, ""))
        .filter(|b| !b.trim().is_empty())
        .map(|b| format!("\"{}\"", b.trim()))
        .unwrap_or_else(|| "\"\"".to_owned());
    format!(
        r#"
@page {{
  size: A4;
  margin: 22mm 20mm 24mm 20mm;
  @bottom-center {{ content: counter(page) " / " counter(pages); font: 9pt sans-serif; color: #777; }}
  @bottom-left {{ content: {footer}; font: 9pt sans-serif; color: #777; }}
}}
:root {{ --accent: {accent}; }}
html {{ font-size: 11pt; }}
body {{
  font-family: {family}-apple-system, "Segoe UI", "Helvetica Neue", Arial, "Noto Sans", sans-serif;
  line-height: 1.55; color: #1b1b1b; max-width: none; margin: 0; padding: 0;
  overflow-wrap: anywhere;
}}
header#title-block-header {{ border-bottom: 2px solid var(--accent); margin-bottom: 1.4em; padding-bottom: .4em; }}
h1.title {{ font-size: 2em; margin: 0; color: var(--accent); line-height: 1.2; }}
h1, h2, h3, h4 {{ color: var(--accent); line-height: 1.25; break-after: avoid; }}
h1 {{ font-size: 1.6em; }} h2 {{ font-size: 1.3em; }} h3 {{ font-size: 1.1em; }}
p, li {{ orphans: 3; widows: 3; }}
a {{ color: var(--accent); }}
img {{ max-width: 100%; height: auto; break-inside: avoid; }}
blockquote {{ margin: 1em 0; padding: .1em 1em; border-left: 4px solid var(--accent); color: #444; background: #f6f7f8; }}
code {{ font-family: "SF Mono", Menlo, Consolas, "DejaVu Sans Mono", monospace; font-size: .88em; background: #f2f3f4; padding: .1em .3em; border-radius: 3px; }}
pre {{ background: #f2f3f4; padding: .8em 1em; border-radius: 4px; white-space: pre-wrap; break-inside: avoid; }}
pre code {{ background: none; padding: 0; }}
/* pandoc's print stylesheet hangs wrapped code lines by 5em; keep them flush. */
pre > code.sourceCode > span {{ text-indent: 0; padding-left: 0; }}
table {{ border-collapse: collapse; width: 100%; margin: 1em 0; break-inside: avoid; }}
th, td {{ border: 1px solid #cfd3d7; padding: .35em .6em; text-align: left; vertical-align: top; }}
th {{ background: #eef0f2; }}
hr {{ border: 0; border-top: 1px solid #cfd3d7; margin: 1.5em 0; }}
"#
    )
}

/// Resolve a note reference to an absolute file inside `vault`.
///
/// Accepts an exact vault-relative path, the same without `.md`, or a bare note
/// name found anywhere under `Notebooks/`. Rejects absolute paths, `..`, and
/// anything whose real path leaves the vault. Only Markdown and plain-text
/// files qualify, so a PDF export cannot be pointed at an arbitrary file.
pub fn resolve_note(vault: &Path, reference: &str) -> Option<PathBuf> {
    let reference = reference.trim();
    let rel = Path::new(reference);
    if reference.is_empty()
        || rel
            .components()
            .any(|c| !matches!(c, Component::Normal(_) | Component::CurDir))
    {
        return None;
    }
    let root = std::fs::canonicalize(vault).ok()?;
    let accept = |p: PathBuf| -> Option<PathBuf> {
        let canon = std::fs::canonicalize(&p).ok()?;
        let ext = canon.extension()?.to_str()?.to_ascii_lowercase();
        (canon.is_file()
            && canon.starts_with(&root)
            && matches!(ext.as_str(), "md" | "markdown" | "txt"))
        .then_some(canon)
    };
    if let Some(hit) = accept(vault.join(rel)) {
        return Some(hit);
    }
    if rel.extension().is_none()
        && let Some(hit) = accept(vault.join(format!("{reference}.md")))
    {
        return Some(hit);
    }
    if rel.components().count() == 1 {
        let wanted = format!("{}.md", reference.strip_suffix(".md").unwrap_or(reference));
        let mut budget = MAX_WALK_ENTRIES;
        let mut stack = vec![vault.join("Notebooks")];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                budget = budget.checked_sub(1)?;
                let name = entry.file_name();
                match entry.file_type() {
                    Ok(t) if t.is_dir() && !name.to_string_lossy().starts_with('.') => {
                        stack.push(entry.path())
                    }
                    Ok(t)
                        if t.is_file() && name.to_string_lossy().eq_ignore_ascii_case(&wanted) =>
                    {
                        if let Some(hit) = accept(entry.path()) {
                            return Some(hit);
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    None
}

/// Export the note at `note_abs` (inside `vault`) to a PDF at `dest`.
pub async fn export_note_pdf(
    vault: &Path,
    note_abs: &Path,
    dest: &Path,
    brand: Option<&BrandKit>,
) -> Result<NotePdf, ConvertError> {
    let engine = PdfEngine::detect()?;
    let raw = std::fs::read_to_string(note_abs)?;
    let stem = note_abs
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Note".to_owned());
    let note_dir = note_abs.parent().unwrap_or(vault);
    let prepared = prepare_note(&raw, &stem, note_dir, vault);

    if let Some(parent) = dest.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    render(&prepared, &engine, dest, brand).await?;

    let size_bytes = std::fs::metadata(dest)
        .map_err(|_| ConvertError::OutboundFailed("renderer finished but wrote no file".into()))?
        .len();
    if size_bytes == 0 {
        return Err(ConvertError::OutboundFailed(
            "renderer wrote an empty file".into(),
        ));
    }
    Ok(NotePdf {
        path: dest.to_path_buf(),
        size_bytes,
        title: prepared.title,
        engine: engine.label(),
    })
}

/// Like [`export_note_pdf`] but returns the PDF bytes and leaves nothing on disk.
pub async fn export_note_pdf_bytes(
    vault: &Path,
    note_abs: &Path,
    brand: Option<&BrandKit>,
) -> Result<(NotePdf, Vec<u8>), ConvertError> {
    let tmp = tempfile::tempdir()?;
    let dest = tmp.path().join("note.pdf");
    let info = export_note_pdf(vault, note_abs, &dest, brand).await?;
    let bytes = std::fs::read(&dest)?;
    Ok((info, bytes))
}

async fn render(
    prepared: &PreparedNote,
    engine: &PdfEngine,
    dest: &Path,
    brand: Option<&BrandKit>,
) -> Result<(), ConvertError> {
    if *engine == PdfEngine::Xelatex {
        let doc = format!(
            "---\ntitle: {}\n---\n\n{}",
            serde_yaml::to_string(&prepared.title)
                .unwrap_or_else(|_| "Note".into())
                .trim(),
            prepared.markdown
        );
        return OutboundConverter::convert(&doc, &ExportFormat::Pdf, dest, brand).await;
    }

    let pandoc = OutboundConverter::check_pandoc()?;
    let tmp = tempfile::tempdir()?;
    let md = tmp.path().join("note.md");
    let css = tmp.path().join("style.css");
    let html = tmp.path().join("note.html");
    std::fs::write(&md, &prepared.markdown)?;
    std::fs::write(&css, stylesheet(brand))?;

    let mut cmd = Command::new(&pandoc);
    cmd.kill_on_drop(true)
        .arg(&md)
        // Raw HTML is off so a note cannot smuggle in <img src=file://...> or script.
        .args([
            "-f",
            "gfm-raw_html+footnotes",
            "-t",
            "html5",
            "--standalone",
        ])
        .arg("--embed-resources")
        .arg(format!("--css={}", css.display()))
        .arg("--metadata")
        .arg(format!("title={}", prepared.title))
        .arg("-o")
        .arg(&html);
    if let Some(logo) = brand
        .and_then(|b| b.logo_light.as_ref())
        .filter(|p| p.is_file())
    {
        let header = tmp.path().join("logo.html");
        std::fs::write(
            &header,
            format!(
                "<img src=\"{}\" style=\"height:36px;margin-bottom:8px\" alt=\"\">",
                logo.display()
            ),
        )?;
        cmd.arg(format!("--include-before-body={}", header.display()));
    }
    run_with_timeout(cmd, "pandoc").await?;

    match engine {
        PdfEngine::Browser(bin) => {
            let profile = tmp.path().join("profile");
            let mut cmd = Command::new(bin);
            cmd.kill_on_drop(true)
                .args(["--headless", "--disable-gpu", "--disable-dev-shm-usage"])
                .args([
                    "--no-pdf-header-footer",
                    "--no-first-run",
                    "--hide-scrollbars",
                ])
                // A private profile keeps this off any running browser's lock.
                .arg(format!("--user-data-dir={}", profile.display()))
                .arg(format!("--print-to-pdf={}", dest.display()));
            if needs_no_sandbox() {
                cmd.arg("--no-sandbox");
            }
            cmd.arg(file_url(&html));
            debug!(browser = %bin.display(), "rendering note pdf");
            run_with_timeout(cmd, "browser").await
        }
        PdfEngine::Weasyprint(bin) => {
            let mut cmd = Command::new(bin);
            cmd.kill_on_drop(true).arg(&html).arg(dest);
            run_with_timeout(cmd, "weasyprint").await
        }
        PdfEngine::Xelatex => unreachable!("handled above"),
    }
}

async fn run_with_timeout(cmd: Command, name: &str) -> Result<(), ConvertError> {
    match tokio::time::timeout(RENDER_TIMEOUT, crate::run_tool(cmd, name)).await {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(msg)) => {
            warn!(error = %msg, "note pdf render failed");
            Err(ConvertError::OutboundFailed(msg))
        }
        Err(_) => Err(ConvertError::OutboundFailed(format!(
            "{name} did not finish within {}s",
            RENDER_TIMEOUT.as_secs()
        ))),
    }
}

fn file_url(path: &Path) -> String {
    let mut out = String::from("file://");
    for b in path.to_string_lossy().bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Chromium refuses to start its sandbox as root (containers, CI), so only
/// there is it turned off. `HQ_PDF_NO_SANDBOX=1` forces it for other setups.
fn needs_no_sandbox() -> bool {
    if std::env::var("HQ_PDF_NO_SANDBOX").is_ok_and(|v| v == "1") {
        return true;
    }
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find_map(|l| l.strip_prefix("Uid:").map(str::to_owned))
        })
        .and_then(|rest| rest.split_whitespace().next().map(|u| u == "0"))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vault() -> tempfile::TempDir {
        let v = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(v.path().join("Notebooks/Inbox")).unwrap();
        std::fs::write(v.path().join("Notebooks/Inbox/chart.png"), b"png").unwrap();
        v
    }

    fn prep(raw: &str, v: &tempfile::TempDir) -> PreparedNote {
        prepare_note(raw, "Fallback", &v.path().join("Notebooks/Inbox"), v.path())
    }

    #[test]
    fn frontmatter_is_dropped_and_title_read_from_it() {
        let v = vault();
        let p = prep(
            "---\ntitle: Quarterly plan\ntags: [a]\n---\n# Quarterly plan\n\nBody",
            &v,
        );
        assert_eq!(p.title, "Quarterly plan");
        assert_eq!(p.markdown.trim(), "Body");
    }

    #[test]
    fn leading_h1_becomes_the_title_when_frontmatter_has_none() {
        let v = vault();
        let p = prep("# Field notes\n\nText", &v);
        assert_eq!(p.title, "Field notes");
        assert_eq!(p.markdown.trim(), "Text");
    }

    #[test]
    fn title_falls_back_to_file_stem() {
        let v = vault();
        let p = prep("Just text", &v);
        assert_eq!(p.title, "Fallback");
        assert_eq!(p.markdown, "Just text");
    }

    #[test]
    fn unclosed_frontmatter_is_left_alone() {
        let v = vault();
        let p = prep("---\nnot closed\nbody", &v);
        assert!(p.markdown.contains("not closed"));
    }

    #[test]
    fn wikilinks_become_plain_text() {
        let v = vault();
        let p = prep(
            "See [[Projects/Alpha]], [[Beta|the beta]] and [[Gamma#Intro]].",
            &v,
        );
        assert_eq!(p.markdown, "See Alpha, the beta and Gamma.");
    }

    #[test]
    fn fenced_code_is_untouched() {
        let v = vault();
        let src = "```\n[[not a link]] %%keep%%\n```\n[[a link]]";
        let p = prep(src, &v);
        assert_eq!(p.markdown, "```\n[[not a link]] %%keep%%\n```\na link");
    }

    #[test]
    fn image_embed_resolves_inside_the_vault() {
        let v = vault();
        let p = prep("![[chart.png|300]]", &v);
        let canon = std::fs::canonicalize(v.path().join("Notebooks/Inbox/chart.png")).unwrap();
        assert_eq!(p.markdown, format!("![](<{}>)", canon.display()));
    }

    #[test]
    fn missing_image_gets_a_placeholder() {
        let v = vault();
        assert_eq!(
            prep("![[nope.png]]", &v).markdown,
            "*[image not found: nope.png]*"
        );
    }

    #[test]
    fn note_embed_degrades_to_its_name() {
        let v = vault();
        assert_eq!(prep("![[Other note]]", &v).markdown, "Other note");
    }

    #[test]
    fn images_outside_the_vault_are_never_embedded() {
        let v = vault();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret.png"), b"x").unwrap();
        let abs = format!("![s]({})", outside.path().join("secret.png").display());
        assert!(prep(&abs, &v).markdown.starts_with("*[image unavailable"));
        assert!(
            prep("![s](../../../secret.png)", &v)
                .markdown
                .starts_with("*[image unavailable")
        );
    }

    #[test]
    fn symlink_out_of_the_vault_is_refused() {
        let v = vault();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret.png"), b"x").unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(
                outside.path().join("secret.png"),
                v.path().join("Notebooks/Inbox/link.png"),
            )
            .unwrap();
            assert!(
                prep("![](link.png)", &v)
                    .markdown
                    .starts_with("*[image unavailable")
            );
        }
    }

    #[test]
    fn remote_images_become_links() {
        let v = vault();
        assert_eq!(
            prep("![logo](https://example.com/a.png)", &v).markdown,
            "[logo](https://example.com/a.png)"
        );
    }

    #[test]
    fn relative_markdown_image_with_encoded_space() {
        let v = vault();
        std::fs::write(v.path().join("Notebooks/Inbox/my pic.png"), b"x").unwrap();
        let p = prep("![pic](my%20pic.png)", &v);
        assert!(p.markdown.starts_with("![pic](<") && p.markdown.contains("my pic.png"));
    }

    #[test]
    fn callouts_become_bold_labels() {
        let v = vault();
        assert_eq!(
            prep("> [!warning] Careful\n> body", &v).markdown,
            "> **Warning: Careful**\n> body"
        );
    }

    #[test]
    fn callouts_stay_intact_when_asked_to_keep_them() {
        let v = vault();
        let p = prepare_note_with(
            "> [!warning] Careful\n> body",
            "Fallback",
            &v.path().join("Notebooks/Inbox"),
            v.path(),
            true,
        );
        assert_eq!(p.markdown, "> [!warning] Careful\n> body");
    }

    #[test]
    fn resolve_note_handles_exact_extensionless_and_bare_names() {
        let v = vault();
        let note = v.path().join("Notebooks/Inbox/Plan.md");
        std::fs::write(&note, "x").unwrap();
        let canon = std::fs::canonicalize(&note).unwrap();
        assert_eq!(
            resolve_note(v.path(), "Notebooks/Inbox/Plan.md"),
            Some(canon.clone())
        );
        assert_eq!(
            resolve_note(v.path(), "Notebooks/Inbox/Plan"),
            Some(canon.clone())
        );
        assert_eq!(resolve_note(v.path(), "plan"), Some(canon));
    }

    #[test]
    fn resolve_note_refuses_escapes_and_non_notes() {
        let v = vault();
        assert_eq!(resolve_note(v.path(), "../x.md"), None);
        assert_eq!(resolve_note(v.path(), "/etc/passwd"), None);
        assert_eq!(resolve_note(v.path(), "Notebooks/Inbox/chart.png"), None);
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("s.md"), "x").unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(
                outside.path().join("s.md"),
                v.path().join("Notebooks/s.md"),
            )
            .unwrap();
            assert_eq!(resolve_note(v.path(), "Notebooks/s.md"), None);
        }
    }

    #[test]
    fn css_values_cannot_break_out() {
        let kit_color = css_safe("red; } body { display:none", "");
        assert!(!kit_color.contains(';') && !kit_color.contains('{'));
    }

    #[test]
    fn file_url_escapes_spaces() {
        assert_eq!(
            file_url(Path::new("/tmp/a b/x.html")),
            "file:///tmp/a%20b/x.html"
        );
    }

    /// Needs pandoc plus a Chromium-family browser, WeasyPrint or xelatex.
    /// Run with: `cargo test -p hq-convert -- --ignored note_pdf_renders`
    #[tokio::test]
    #[ignore = "needs pandoc and a PDF engine on this machine"]
    async fn note_pdf_renders() {
        let v = vault();
        let note = v.path().join("Notebooks/Inbox/Plan.md");
        std::fs::write(
            &note,
            "---\ntitle: Plan\n---\nHello **world** ✓\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\n![[chart.png]]\n",
        )
        .unwrap();
        let out = v.path().join("out/Plan.pdf");
        let r = export_note_pdf(v.path(), &note, &out, None).await.unwrap();
        assert!(r.size_bytes > 500);
        assert!(std::fs::read(&out).unwrap().starts_with(b"%PDF"));
    }
}
