//! Outbound conversion — Markdown → any format via pandoc.
//!
//! `pandoc` must be available in PATH. If it is not found, the error
//! message includes platform-specific install instructions.

use std::path::Path;
use tokio::process::Command;
use tracing::{debug, warn};
use which::which;

use crate::brand::BrandKit;
use crate::types::{ConvertError, ExportFormat};

/// Converts Markdown content to other formats by calling `pandoc`.
pub struct OutboundConverter;

/// Reference-doc arg for docx/pptx: brand's own template if set, else (pptx
/// only) the legacy global `~/.hq/templates/reference.pptx` if present.
/// docx never had a legacy fallback — there was nothing to preserve there.
fn reference_doc_arg(format: &ExportFormat, brand: Option<&BrandKit>) -> Option<String> {
    let brand_template = brand.and_then(|kit| match format {
        ExportFormat::Docx => kit.reference_docx.as_ref(),
        ExportFormat::Pptx => kit.reference_pptx.as_ref(),
        _ => None,
    });
    if let Some(path) = brand_template {
        return Some(format!("--reference-doc={}", path.display()));
    }
    if *format == ExportFormat::Pptx
        && let Some(home) = dirs::home_dir()
    {
        let ref_doc = home.join(".hq").join("templates").join("reference.pptx");
        if ref_doc.exists() {
            return Some(format!("--reference-doc={}", ref_doc.to_string_lossy()));
        }
    }
    None
}

/// How the Markdown is read. Raw TeX and raw attributes are off: pandoc would otherwise pass
/// `\input{/some/file}` or `` `\input{...}`{=latex} `` through to LaTeX, which reads the named file
/// into the PDF. A document must not be able to pull in files or run commands.
const READER: &str = "markdown-raw_tex-raw_attribute";

/// The full pandoc command line. The format is always the enum's fixed name, so nothing a caller
/// types reaches `-t`.
fn pandoc_args(
    input: &Path,
    format: &ExportFormat,
    dest: &Path,
    brand: Option<&BrandKit>,
) -> Vec<String> {
    let mut args: Vec<String> = vec![
        input.to_string_lossy().into_owned(),
        "-f".to_string(),
        READER.to_string(),
        "-t".to_string(),
        format.pandoc_format().to_string(),
        "-o".to_string(),
        dest.to_string_lossy().into_owned(),
    ];

    // For PDF, use xelatex to handle Unicode properly, and never let it run shell commands.
    if *format == ExportFormat::Pdf {
        args.push("--pdf-engine=xelatex".to_string());
        args.push("--pdf-engine-opt=-no-shell-escape".to_string());
    }

    if let Some(reference_arg) = reference_doc_arg(format, brand) {
        args.push(reference_arg);
    }
    args
}

impl OutboundConverter {
    /// Check whether pandoc is available in PATH.
    /// Returns `Ok(path)` or `Err(ConvertError::PandocNotFound)`.
    pub fn check_pandoc() -> Result<std::path::PathBuf, ConvertError> {
        which("pandoc").map_err(|_| ConvertError::PandocNotFound)
    }

    /// Convert `markdown` to `format` at `dest`. `brand` only styles docx and
    /// pptx, the formats with a pandoc `--reference-doc`; PDF needs `xelatex`.
    pub async fn convert(
        markdown: &str,
        format: &ExportFormat,
        dest: &Path,
        brand: Option<&BrandKit>,
    ) -> Result<(), ConvertError> {
        let pandoc = Self::check_pandoc()?;

        // A file input keeps large documents off the child's stdin pipe.
        let tmp_dir = tempfile::tempdir().map_err(ConvertError::Io)?;
        let input_path = tmp_dir.path().join("input.md");
        std::fs::write(&input_path, markdown.as_bytes()).map_err(ConvertError::Io)?;

        let args = pandoc_args(&input_path, format, dest, brand);

        debug!(
            pandoc = %pandoc.display(),
            format = %format,
            dest = %dest.display(),
            "running pandoc"
        );

        let mut cmd = Command::new(&pandoc);
        cmd.kill_on_drop(true).args(&args);
        crate::run_tool(cmd, "pandoc").await.map_err(|msg| {
            warn!(error = %msg, "pandoc conversion failed");
            ConvertError::OutboundFailed(msg)
        })?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args_for(format: ExportFormat) -> Vec<String> {
        pandoc_args(
            Path::new("/tmp/in/input.md"),
            &format,
            Path::new("/tmp/out/doc"),
            None,
        )
    }

    fn after<'a>(args: &'a [String], flag: &str) -> &'a str {
        let i = args.iter().position(|a| a == flag).unwrap();
        &args[i + 1]
    }

    #[test]
    fn the_target_is_always_a_fixed_name_and_the_reader_has_no_raw_tex() {
        for format in ExportFormat::ALL {
            let args = args_for(format);
            assert_eq!(after(&args, "-t"), format.pandoc_format());
            assert!(ExportFormat::names().contains(after(&args, "-t")));
            assert_eq!(after(&args, "-f"), "markdown-raw_tex-raw_attribute");
            assert_eq!(after(&args, "-o"), "/tmp/out/doc");
            // No filter, template or other file-loading option is ever added.
            for a in &args {
                assert!(
                    !a.starts_with("--lua-filter")
                        && !a.starts_with("--filter")
                        && !a.starts_with("--template")
                        && !a.starts_with("--include")
                        && !a.ends_with(".lua"),
                    "{a}"
                );
            }
        }
    }

    #[test]
    fn pdf_runs_xelatex_without_shell_escape_and_other_formats_do_not_use_it() {
        let pdf = args_for(ExportFormat::Pdf);
        assert!(pdf.contains(&"--pdf-engine=xelatex".to_string()));
        assert!(pdf.contains(&"--pdf-engine-opt=-no-shell-escape".to_string()));
        assert!(
            !args_for(ExportFormat::Docx)
                .iter()
                .any(|a| a.contains("pdf-engine"))
        );
    }

    /// Runs the installed pandoc with the real reader flags. Opt in:
    /// `cargo test -p hq-convert -- --ignored outbound::tests`
    #[tokio::test]
    #[ignore = "needs pandoc on PATH"]
    async fn raw_tex_is_not_passed_through_and_the_flags_are_valid_for_this_pandoc() {
        let dir = tempfile::tempdir().unwrap();
        let secret = dir.path().join("secret.txt");
        std::fs::write(&secret, "TOP-SECRET-MARKER").unwrap();
        let md = format!(
            "# Title\n\n\\input{{{p}}}\n\nText `\\input{{{p}}}`{{=latex}}\n",
            p = secret.display()
        );
        let out = dir.path().join("out.html");
        OutboundConverter::convert(&md, &ExportFormat::Html, &out, None)
            .await
            .unwrap();
        let html = std::fs::read_to_string(&out).unwrap();
        assert!(html.contains("<h1"), "{html}");
        assert!(!html.contains("TOP-SECRET-MARKER"), "{html}");

        // What xelatex would be given: the TeX is text, not a command. Read with plain
        // `markdown` it is passed through untouched, which is what let a document read files.
        let input = dir.path().join("in.md");
        std::fs::write(&input, &md).unwrap();
        let latex_with = |reader: &str| {
            let out = std::process::Command::new("pandoc")
                .args(["-f", reader, "-t", "latex"])
                .arg(&input)
                .output()
                .unwrap();
            String::from_utf8_lossy(&out.stdout).into_owned()
        };
        assert!(
            latex_with("markdown").contains("\\input{"),
            "control: the old reader"
        );
        let safe = latex_with(READER);
        assert!(!safe.contains("\\input{"), "{safe}");
        assert!(safe.contains("textbackslash"), "{safe}");
    }
}
