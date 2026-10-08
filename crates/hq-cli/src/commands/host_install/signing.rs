//! A stable code signature for `hq` on macOS. An ad-hoc signature changes with
//! every build, and macOS ties its folder-access approvals (Documents, Desktop,
//! Downloads) to the signature, so each update would ask again and a headless
//! agent would hang waiting for an answer nobody sees. Signing every build with
//! the same locally made certificate keeps one approval valid across updates.
//!
//! The certificate lives in its own keychain with an empty password, so signing
//! never prompts. It holds nothing but this self-signed key.

use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub(super) const CERT_NAME: &str = "HQ Local Code Signing";
const SIGN_IDENTIFIER: &str = "com.agent-hq.hq";
const KEYCHAIN_FILE: &str = "hq-signing.keychain-db";
const CERT_DAYS: &str = "9999";
const P12_PASSWORD: &str = "hq-p12-import";
/// How long `codesign` may take before it is assumed to be waiting on a dialog.
const SIGN_TIMEOUT: Duration = Duration::from_secs(60);
const POLL: Duration = Duration::from_millis(100);

pub(super) fn keychain_path(home: &Path) -> PathBuf {
    home.join(".hq").join(KEYCHAIN_FILE)
}

/// Whether `find-identity` output lists the HQ signing certificate.
pub(super) fn lists_identity(output: &str) -> bool {
    output.lines().any(|l| l.contains(CERT_NAME))
}

/// Whether `codesign -dr -` output says the binary is signed by a certificate
/// (anything but an ad-hoc cdhash requirement) for our identifier.
pub(super) fn is_stably_signed(requirement: &str) -> bool {
    requirement.contains(SIGN_IDENTIFIER) && requirement.contains("certificate") && !requirement.contains("cdhash")
}

fn tool(program: &str, args: &[&str]) -> Result<std::process::Output> {
    let out = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("running {program}"))?;
    if !out.status.success() {
        bail!("{program} {} failed: {}", args.first().unwrap_or(&""), String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(out)
}

fn identity_ready(keychain: &Path) -> bool {
    keychain.exists()
        && Command::new("security")
            .args(["find-identity", "-p", "codesigning"])
            .arg(keychain)
            .stdin(Stdio::null())
            .output()
            .is_ok_and(|o| lists_identity(&String::from_utf8_lossy(&o.stdout)))
}

/// Makes the certificate and its keychain if they are not there yet.
fn ensure_identity(home: &Path) -> Result<PathBuf> {
    let keychain = keychain_path(home);
    let kc = keychain.to_string_lossy().into_owned();
    if !identity_ready(&keychain) {
        let _ = std::fs::remove_file(&keychain);
        let work = std::env::temp_dir().join(format!("hq-sign-{}", std::process::id()));
        std::fs::create_dir_all(&work)?;
        std::fs::set_permissions(&work, std::os::unix::fs::PermissionsExt::from_mode(0o700))?;
        let result = create_identity(&work, &kc);
        let _ = std::fs::remove_dir_all(&work);
        result?;
    }
    tool("security", &["unlock-keychain", "-p", "", &kc])?;
    Ok(keychain)
}

fn create_identity(work: &Path, kc: &str) -> Result<()> {
    let key = work.join("key.pem");
    let cert = work.join("cert.pem");
    let p12 = work.join("id.p12");
    let (key, cert, p12) = (key.to_string_lossy(), cert.to_string_lossy(), p12.to_string_lossy());
    let subject = format!("/CN={CERT_NAME}/O=Agent HQ");
    tool(
        "openssl",
        &[
            "req", "-x509", "-newkey", "rsa:2048", "-keyout", &key, "-out", &cert, "-days", CERT_DAYS, "-nodes", "-subj",
            &subject, "-addext", "keyUsage=critical,digitalSignature", "-addext", "extendedKeyUsage=critical,codeSigning",
            "-addext", "basicConstraints=critical,CA:FALSE",
        ],
    )?;
    // macOS's `security` only reads the legacy PKCS12 ciphers.
    let pass = format!("pass:{P12_PASSWORD}");
    tool(
        "openssl",
        &[
            "pkcs12", "-export", "-out", &p12, "-inkey", &key, "-in", &cert, "-passout", &pass, "-certpbe",
            "PBE-SHA1-3DES", "-keypbe", "PBE-SHA1-3DES", "-macalg", "SHA1",
        ],
    )?;
    tool("security", &["create-keychain", "-p", "", kc])?;
    tool("security", &["set-keychain-settings", kc])?;
    tool("security", &["unlock-keychain", "-p", "", kc])?;
    tool("security", &["import", &p12, "-k", kc, "-P", P12_PASSWORD, "-T", "/usr/bin/codesign"])?;
    // Lets codesign use the key without asking.
    tool("security", &["set-key-partition-list", "-S", "apple-tool:,apple:,codesign:", "-s", "-k", "", kc])?;
    // codesign finds the identity through the keychain search list.
    let listed = tool("security", &["list-keychains", "-d", "user"])?;
    let mut all: Vec<String> = String::from_utf8_lossy(&listed.stdout)
        .lines()
        .map(|l| l.trim().trim_matches('"').to_string())
        .filter(|l| !l.is_empty())
        .collect();
    if !all.iter().any(|l| l == kc) {
        all.insert(0, kc.to_string());
        let mut args = vec!["list-keychains", "-d", "user", "-s"];
        args.extend(all.iter().map(String::as_str));
        tool("security", &args)?;
    }
    Ok(())
}

fn wait_with_deadline(child: &mut std::process::Child) -> Result<bool> {
    let deadline = Instant::now() + SIGN_TIMEOUT;
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status.success());
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            bail!("codesign did not finish in {}s; it is probably waiting on a keychain dialog", SIGN_TIMEOUT.as_secs());
        }
        std::thread::sleep(POLL);
    }
}

/// Signs `exe` with the stable identity, replacing the file atomically so a running
/// copy keeps its old inode (changing a running binary's signature in place kills
/// it). Does nothing when it is already signed that way. Returns whether it signed.
pub(super) fn sign(home: &Path, exe: &Path) -> Result<bool> {
    let current = Command::new("codesign").args(["-dr", "-"]).arg(exe).stdin(Stdio::null()).output()?;
    let text = format!("{}{}", String::from_utf8_lossy(&current.stdout), String::from_utf8_lossy(&current.stderr));
    if is_stably_signed(&text) {
        return Ok(false);
    }
    let keychain = ensure_identity(home)?;
    let staged = exe.with_extension("signing");
    std::fs::copy(exe, &staged).with_context(|| format!("copying {}", exe.display()))?;
    let mut child = Command::new("codesign")
        .args(["-f", "-s", CERT_NAME, "--keychain"])
        .arg(&keychain)
        .args(["--identifier", SIGN_IDENTIFIER])
        .arg(&staged)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("running codesign")?;
    let signed = wait_with_deadline(&mut child);
    match signed {
        Ok(true) => {
            std::fs::rename(&staged, exe)?;
            Ok(true)
        }
        Ok(false) => {
            let _ = std::fs::remove_file(&staged);
            bail!("codesign could not sign with the {CERT_NAME} identity")
        }
        Err(e) => {
            let _ = std::fs::remove_file(&staged);
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_identity_is_recognised_in_find_identity_output() {
        let listed = "  1) 7D1C2C98 \"HQ Local Code Signing\" (CSSMERR_TP_NOT_TRUSTED)\n     1 identities found\n";
        assert!(lists_identity(listed));
        assert!(!lists_identity("     0 identities found\n"));
    }

    #[test]
    fn only_a_certificate_requirement_counts_as_stable() {
        assert!(is_stably_signed("designated => identifier \"com.agent-hq.hq\" and certificate root = H\"7d1c\""));
        assert!(!is_stably_signed("designated => cdhash H\"a6fd\""));
        assert!(!is_stably_signed("designated => identifier \"other\" and certificate root = H\"7d1c\""));
        assert!(!is_stably_signed("code object is not signed at all"));
    }
}
