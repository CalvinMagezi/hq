//! Old binary Word, PowerPoint and Excel files (`.doc`, `.ppt`, `.xls`).
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
        Kind::Xls => Some(("xlsx", Kind::Xlsx)),
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

/// The private profile's settings. The file being opened is anyone's, so LibreOffice is told
/// to run none of its macros, open no embedded active content or DDE, follow no link out of it
/// and refresh no linked data while it loads (Writer counts "never" as 2, Calc as 1).
const PROFILE_SETTINGS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<oor:items xmlns:oor="http://openoffice.org/2001/registry" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
<item oor:path="/org.openoffice.Office.Common/Security/Scripting"><prop oor:name="DisableMacrosExecution" oor:op="fuse"><value>true</value></prop></item>
<item oor:path="/org.openoffice.Office.Common/Security/Scripting"><prop oor:name="MacroSecurityLevel" oor:op="fuse"><value>3</value></prop></item>
<item oor:path="/org.openoffice.Office.Common/Security/Scripting"><prop oor:name="DisableActiveContent" oor:op="fuse"><value>true</value></prop></item>
<item oor:path="/org.openoffice.Office.Common/Security/Scripting"><prop oor:name="BlockUntrustedRefererLinks" oor:op="fuse"><value>true</value></prop></item>
<item oor:path="/org.openoffice.Office.Writer/Content/Update"><prop oor:name="Link" oor:op="fuse"><value>2</value></prop></item>
<item oor:path="/org.openoffice.Office.Calc/Content/Update"><prop oor:name="Link" oor:op="fuse"><value>1</value></prop></item>
</oor:items>
"#;

/// What LibreOffice is given of the daemon's environment: where programs are and the
/// language. Not the rest, which holds provider keys and tokens.
fn passed_env(name: &str) -> bool {
    name == "PATH" || name == "LANG" || name == "LANGUAGE" || name.starts_with("LC_")
}

/// A modern copy of a legacy file, and its kind.
pub async fn upgrade(bytes: &[u8], kind: Kind) -> Result<(Vec<u8>, Kind), ConvertError> {
    let Some(office) = find_office() else {
        return Err(unavailable(kind));
    };
    upgrade_with(&office, bytes, kind).await
}

/// `upgrade` with the LibreOffice program given.
async fn upgrade_with(
    office: &Path,
    bytes: &[u8],
    kind: Kind,
) -> Result<(Vec<u8>, Kind), ConvertError> {
    let Some((ext, new_kind)) = target(kind) else {
        return Err(unavailable(kind));
    };
    let dir = tempfile::tempdir().map_err(ConvertError::Io)?;
    let input = dir.path().join(format!("input.{}", kind.label()));
    tokio::fs::write(&input, bytes).await?;
    let out_dir = dir.path().join("out");
    tokio::fs::create_dir_all(&out_dir).await?;
    let profile = dir.path().join("profile");
    tokio::fs::create_dir_all(profile.join("user")).await?;
    tokio::fs::write(
        profile.join("user").join("registrymodifications.xcu"),
        PROFILE_SETTINGS,
    )
    .await?;

    let mut cmd = tokio::process::Command::new(office);
    cmd.kill_on_drop(true)
        .env_clear()
        .envs(std::env::vars_os().filter(|(k, _)| k.to_str().is_some_and(passed_env)))
        .env("HOME", dir.path())
        .env("TMPDIR", dir.path())
        // A private profile keeps this run from touching, or waiting on, the user's own LibreOffice.
        .arg(format!(
            "-env:UserInstallation=file://{}",
            profile.display()
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
        .arg(&input);
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

    /// A stand-in for LibreOffice that "converts" by writing out its environment and the
    /// profile settings it was started with.
    #[cfg(unix)]
    fn fake_office(dir: &Path) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let script = dir.join("soffice");
        std::fs::write(
            &script,
            "#!/bin/sh\nout=''\nprev=''\nfor a in \"$@\"; do\n  [ \"$prev\" = --outdir ] && out=\"$a\"\n  prev=\"$a\"\ndone\n\
             { env; echo '--- profile'; cat \"$HOME/profile/user/registrymodifications.xcu\"; } > \"$out/input.docx\"\n",
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        script
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn libreoffice_gets_no_secrets_from_the_environment_and_runs_no_macros() {
        let tools = tempfile::tempdir().unwrap();
        let office = fake_office(tools.path());
        let (out, kind) = upgrade_with(&office, b"not really a doc", Kind::Doc)
            .await
            .unwrap();
        assert_eq!(kind, Kind::Docx);
        let text = String::from_utf8(out).unwrap();
        let (env, profile) = text.split_once("--- profile").unwrap();
        // The shell adds its own few; nothing else of this process's environment arrives.
        let shell_own = ["PWD", "OLDPWD", "SHLVL", "_"];
        for line in env.lines().filter(|l| !l.is_empty()) {
            let name = line.split('=').next().unwrap();
            assert!(
                passed_env(name) || ["HOME", "TMPDIR"].contains(&name) || shell_own.contains(&name),
                "{name} reached LibreOffice"
            );
        }
        assert!(env.lines().any(|l| l.starts_with("PATH=")), "{env}");
        for setting in [
            "DisableMacrosExecution",
            "DisableActiveContent",
            "BlockUntrustedRefererLinks",
            "Office.Writer/Content/Update",
            "Office.Calc/Content/Update",
        ] {
            assert!(profile.contains(setting), "{setting}: {profile}");
        }
    }

    #[test]
    fn word_powerpoint_and_excel_are_upgraded() {
        assert!(target(Kind::Doc).is_some() && target(Kind::Ppt).is_some());
        assert_eq!(target(Kind::Xls), Some(("xlsx", Kind::Xlsx)));
        assert!(target(Kind::Pdf).is_none());
    }
}
