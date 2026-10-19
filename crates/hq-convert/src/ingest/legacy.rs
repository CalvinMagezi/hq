//! Old binary Word and PowerPoint files (`.doc`, `.ppt`).
//!
//! No Rust reader exists for these layouts, so an installed LibreOffice is
//! asked to save a modern copy, which the native reader then converts. The
//! original file is never modified. Without LibreOffice the file is refused
//! with a message that says what to install or how to re-save it.

use std::path::{Path, PathBuf};
use std::time::Duration;

use super::sniff::Kind;
use crate::types::ConvertError;

const TIMEOUT: Duration = Duration::from_secs(120);

fn find_office() -> Option<PathBuf> {
    for name in ["soffice", "libreoffice"] {
        if let Ok(p) = which::which(name) {
            return Some(p);
        }
    }
    [
        "/Applications/LibreOffice.app/Contents/MacOS/soffice",
        "/usr/lib/libreoffice/program/soffice",
        "/opt/libreoffice/program/soffice",
    ]
    .iter()
    .map(PathBuf::from)
    .find(|p| p.exists())
}

/// The modern format a legacy one is saved as, and its extension.
fn target(kind: Kind) -> Option<(&'static str, Kind)> {
    match kind {
        Kind::Doc => Some(("docx", Kind::Docx)),
        Kind::Ppt => Some(("pptx", Kind::Pptx)),
        _ => None,
    }
}

/// The hint shown when a legacy file cannot be opened here.
pub fn unavailable(kind: Kind) -> ConvertError {
    ConvertError::UnsupportedFormat(format!(
        "{} (an old binary Office format): install LibreOffice so HQ can open it, or save the file as {} and import that",
        kind.label(),
        match kind {
            Kind::Ppt => "pptx",
            _ => "docx",
        }
    ))
}

pub fn available() -> bool {
    find_office().is_some()
}

/// A modern copy of a legacy file, and its kind.
pub async fn upgrade(bytes: &[u8], kind: Kind) -> Result<(Vec<u8>, Kind), ConvertError> {
    let Some((ext, new_kind)) = target(kind) else {
        return Err(unavailable(kind));
    };
    let Some(office) = find_office() else {
        return Err(unavailable(kind));
    };
    let dir = tempfile::tempdir().map_err(ConvertError::Io)?;
    let input = dir.path().join(format!("input.{}", kind.label()));
    tokio::fs::write(&input, bytes).await?;
    let out_dir = dir.path().join("out");
    tokio::fs::create_dir_all(&out_dir).await?;

    let mut cmd = tokio::process::Command::new(&office);
    cmd.kill_on_drop(true)
        // A private profile keeps this run from touching, or waiting on, the user's own LibreOffice.
        .arg(format!(
            "-env:UserInstallation=file://{}",
            dir.path().join("profile").display()
        ))
        .args([
            "--headless",
            "--norestore",
            "--nolockcheck",
            "--convert-to",
            ext,
            "--outdir",
        ])
        .arg(&out_dir)
        .arg(&input)
        .env("HOME", dir.path());
    let run = tokio::time::timeout(TIMEOUT, crate::run_tool(cmd, "LibreOffice")).await;
    match run {
        Err(_) => {
            return Err(ConvertError::InboundFailed(
                "LibreOffice took too long to open this file".into(),
            ));
        }
        Ok(Err(msg)) => return Err(ConvertError::InboundFailed(msg)),
        Ok(Ok(_)) => {}
    }
    let produced: PathBuf = out_dir.join(format!("input.{ext}"));
    read_produced(&produced).await.map(|b| (b, new_kind))
}

async fn read_produced(path: &Path) -> Result<Vec<u8>, ConvertError> {
    tokio::fs::read(path).await.map_err(|_| {
        ConvertError::InboundFailed(
            "LibreOffice could not open this file (it may be damaged or password protected)".into(),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_refusal_names_what_to_do() {
        let e = unavailable(Kind::Doc).to_string();
        assert!(e.contains("LibreOffice") && e.contains("docx"), "{e}");
    }

    #[test]
    fn only_word_and_powerpoint_are_upgraded() {
        assert!(target(Kind::Doc).is_some() && target(Kind::Ppt).is_some());
        assert!(target(Kind::Xls).is_none());
    }
}
