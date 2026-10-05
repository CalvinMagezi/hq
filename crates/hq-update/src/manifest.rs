//! Release manifest and channel pointer formats (see docs/UPDATE_SYSTEM.md).

use crate::error::{Result, UpdateError};
use semver::Version;
use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: u32 = 1;
pub const MANIFEST_NAME: &str = "manifest.json";
pub const SIGNATURE_SUFFIX: &str = ".minisig";

/// The platform every release must carry. Updaters released before
/// multi-platform artifacts only look for this name, so it never changes.
pub const LEGACY_PLATFORM: &str = "linux-x86_64";

/// Maps `std::env::consts` OS and ARCH to the `<os>-<arch>` names used in
/// artifact file names. Unknown values pass through and simply will not
/// match any artifact. A musl target gets a `-musl` suffix for the same
/// reason: the published Linux binaries link glibc and must not be selected.
pub fn platform_name(os: &str, arch: &str, env: &str) -> String {
    let os = match os {
        "macos" => "darwin",
        other => other,
    };
    match env {
        "musl" => format!("{os}-{arch}-musl"),
        _ => format!("{os}-{arch}"),
    }
}

/// The platform this binary was compiled for.
pub fn host_platform() -> String {
    let env = if cfg!(target_env = "musl") {
        "musl"
    } else {
        ""
    };
    platform_name(std::env::consts::OS, std::env::consts::ARCH, env)
}

/// Whether the updater has no stop step for this platform's service manager.
pub fn is_darwin(platform: &str) -> bool {
    platform.starts_with("darwin-")
}

pub fn binary_artifact_name_for(version: &str, platform: &str) -> String {
    format!("hq-{version}-{platform}.tar.gz")
}

/// The linux-x86_64 artifact name, required in every manifest.
pub fn binary_artifact_name(version: &str) -> String {
    binary_artifact_name_for(version, LEGACY_PLATFORM)
}

pub fn web_artifact_name(version: &str) -> String {
    format!("hq-web-{version}.tar.gz")
}

pub fn channel_file_name(channel: &str) -> String {
    format!("channel-{channel}.json")
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Artifact {
    pub name: String,
    pub sha256: String,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub schema: u32,
    pub version: String,
    pub git_sha: String,
    pub channel: String,
    pub built_at: String,
    pub min_updater_version: String,
    #[serde(default)]
    pub requires_db_snapshot: bool,
    pub artifacts: Vec<Artifact>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelPointer {
    pub schema: u32,
    /// The channel this pointer is signed for; stops a pointer for one
    /// channel being served as another.
    pub channel: String,
    pub version: String,
    pub manifest_url: String,
    pub manifest_sha256: String,
    /// ISO 8601 UTC time the pointer was signed. Optional in schema 1.
    #[serde(default)]
    pub issued_at: Option<String>,
    /// Strictly increasing per channel. Optional in schema 1; pointers
    /// without it are accepted but cannot be ordered against a replay.
    #[serde(default)]
    pub seq: Option<u64>,
}

fn invalid(what: &'static str, reason: impl Into<String>) -> UpdateError {
    UpdateError::Invalid {
        what,
        reason: reason.into(),
    }
}

/// Checks `schema` before anything else, so a future format reports as
/// unsupported instead of as a confusing missing-field error.
fn check_schema(what: &'static str, value: &serde_json::Value) -> Result<()> {
    let found = value
        .get("schema")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| invalid(what, "missing numeric `schema`"))?;
    if found != u64::from(SCHEMA_VERSION) {
        return Err(UpdateError::UnsupportedSchema {
            what,
            found,
            supported: SCHEMA_VERSION,
        });
    }
    Ok(())
}

pub fn parse_version(what: &'static str, raw: &str) -> Result<Version> {
    if raw.contains('+') {
        return Err(invalid(what, "version must not carry build metadata (`+`)"));
    }
    Version::parse(raw.strip_prefix('v').unwrap_or(raw))
        .map_err(|e| invalid(what, format!("version `{raw}` is not semver: {e}")))
}

pub fn is_hex(s: &str, len: usize) -> bool {
    s.len() == len && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// A bare file name: no separators, no traversal, nothing a shell or URL
/// would treat specially.
pub fn is_safe_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name != "."
        && name != ".."
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-' | b'+'))
}

/// The part of a release URL after the trusted prefix must be a plain path:
/// no traversal (also percent-encoded), empty segments, backslashes or
/// userinfo/query/fragment tricks.
pub fn is_clean_url_tail(tail: &str) -> bool {
    let lower = tail.to_ascii_lowercase();
    !tail.is_empty()
        && !tail.contains("//")
        && !tail.contains(['\\', '?', '#', '@', ' '])
        && !tail.split('/').any(|seg| seg == "." || seg == "..")
        && !lower.contains("..")
        && !lower.contains("%2e")
        && !lower.contains("%2f")
        && !lower.contains("%5c")
        && !lower.contains("%00")
}

impl Manifest {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let value: serde_json::Value = serde_json::from_slice(bytes)
            .map_err(|e| invalid("manifest", format!("not JSON: {e}")))?;
        check_schema("manifest", &value)?;
        let manifest: Manifest =
            serde_json::from_value(value).map_err(|e| invalid("manifest", e.to_string()))?;
        manifest.validate()?;
        Ok(manifest)
    }

    fn validate(&self) -> Result<()> {
        parse_version("manifest", &self.version)?;
        parse_version("manifest min_updater_version", &self.min_updater_version)?;
        if !(7..=40).contains(&self.git_sha.len())
            || !self.git_sha.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err(invalid(
                "manifest",
                "git_sha must be 7 to 40 hex characters",
            ));
        }
        if !is_safe_name(&self.channel) {
            return Err(invalid("manifest", "channel is not a plain name"));
        }
        for artifact in &self.artifacts {
            if !is_safe_name(&artifact.name) {
                return Err(invalid(
                    "manifest",
                    format!("unsafe artifact name `{}`", artifact.name),
                ));
            }
            if !is_hex(&artifact.sha256, 64) {
                return Err(invalid(
                    "manifest",
                    format!(
                        "{}: sha256 must be 64 lowercase hex characters",
                        artifact.name
                    ),
                ));
            }
            if artifact.size == 0 {
                return Err(invalid(
                    "manifest",
                    format!("{}: size must be positive", artifact.name),
                ));
            }
        }
        if self
            .artifact(&binary_artifact_name(&self.version))
            .is_none()
        {
            return Err(invalid(
                "manifest",
                format!(
                    "artifact {} is required",
                    binary_artifact_name(&self.version)
                ),
            ));
        }
        Ok(())
    }

    pub fn artifact(&self, name: &str) -> Option<&Artifact> {
        self.artifacts.iter().find(|a| a.name == name)
    }

    pub fn semver(&self) -> Version {
        parse_version("manifest", &self.version).expect("validated at parse time")
    }
}

impl ChannelPointer {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let value: serde_json::Value = serde_json::from_slice(bytes)
            .map_err(|e| invalid("channel pointer", format!("not JSON: {e}")))?;
        check_schema("channel pointer", &value)?;
        let pointer: ChannelPointer =
            serde_json::from_value(value).map_err(|e| invalid("channel pointer", e.to_string()))?;
        parse_version("channel pointer", &pointer.version)?;
        if !is_safe_name(&pointer.channel) {
            return Err(invalid("channel pointer", "channel is not a plain name"));
        }
        if let Some(at) = &pointer.issued_at
            && chrono::DateTime::parse_from_rfc3339(at).is_err()
        {
            return Err(invalid(
                "channel pointer",
                "issued_at is not an RFC 3339 time",
            ));
        }
        if !is_hex(&pointer.manifest_sha256, 64) {
            return Err(invalid(
                "channel pointer",
                "manifest_sha256 must be 64 lowercase hex characters",
            ));
        }
        if pointer.manifest_url.contains(['?', '#']) {
            return Err(invalid(
                "channel pointer",
                "manifest_url must not carry a query or fragment",
            ));
        }
        Ok(pointer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub fn sample(version: &str) -> serde_json::Value {
        serde_json::json!({
            "schema": 1, "version": version, "git_sha": "abcdef1234567",
            "channel": "main", "built_at": "2026-10-04T00:00:00Z",
            "min_updater_version": "0.9.0", "requires_db_snapshot": false,
            "artifacts": [{"name": binary_artifact_name(version), "sha256": "a".repeat(64), "size": 10}],
        })
    }

    #[test]
    fn parses_a_valid_manifest() {
        let m = Manifest::parse(sample("0.9.1").to_string().as_bytes()).unwrap();
        assert_eq!(m.version, "0.9.1");
        assert!(!m.requires_db_snapshot);
    }

    #[test]
    fn platform_names_map_rust_consts() {
        assert_eq!(platform_name("linux", "x86_64", ""), "linux-x86_64");
        assert_eq!(platform_name("linux", "aarch64", "gnu"), "linux-aarch64");
        assert_eq!(platform_name("macos", "aarch64", ""), "darwin-aarch64");
        assert_eq!(platform_name("macos", "x86_64", ""), "darwin-x86_64");
        assert_eq!(platform_name("freebsd", "x86_64", ""), "freebsd-x86_64");
        assert_eq!(
            platform_name("linux", "x86_64", "musl"),
            "linux-x86_64-musl"
        );
        assert!(is_darwin("darwin-aarch64") && !is_darwin("linux-aarch64"));
    }

    #[test]
    fn linux_x86_64_artifact_name_is_frozen() {
        assert_eq!(
            binary_artifact_name("0.9.1-main.6"),
            "hq-0.9.1-main.6-linux-x86_64.tar.gz"
        );
        assert_eq!(
            binary_artifact_name_for("0.9.1", LEGACY_PLATFORM),
            binary_artifact_name("0.9.1")
        );
        assert_eq!(
            binary_artifact_name_for("0.9.1", "darwin-aarch64"),
            "hq-0.9.1-darwin-aarch64.tar.gz"
        );
    }

    #[test]
    fn extra_platform_artifacts_are_accepted() {
        let mut v = sample("0.9.1");
        v["artifacts"].as_array_mut().unwrap().push(serde_json::json!(
            {"name": binary_artifact_name_for("0.9.1", "darwin-aarch64"), "sha256": "b".repeat(64), "size": 10}
        ));
        assert!(Manifest::parse(v.to_string().as_bytes()).is_ok());
    }

    #[test]
    fn rejects_unknown_schema_before_other_errors() {
        let mut v = sample("0.9.1");
        v["schema"] = 2.into();
        v.as_object_mut().unwrap().remove("artifacts");
        let err = Manifest::parse(v.to_string().as_bytes()).unwrap_err();
        assert!(
            matches!(err, UpdateError::UnsupportedSchema { found: 2, .. }),
            "{err}"
        );
    }

    #[test]
    fn rejects_bad_fields() {
        for (key, value) in [
            ("version", serde_json::json!("not-semver")),
            ("git_sha", serde_json::json!("xyz")),
            ("channel", serde_json::json!("../x")),
        ] {
            let mut v = sample("0.9.1");
            v[key] = value;
            assert!(Manifest::parse(v.to_string().as_bytes()).is_err(), "{key}");
        }
        let mut v = sample("0.9.1");
        v["artifacts"][0]["name"] = "../evil".into();
        assert!(Manifest::parse(v.to_string().as_bytes()).is_err());
        let mut v = sample("0.9.1");
        v["artifacts"][0]["sha256"] = "ABC".into();
        assert!(Manifest::parse(v.to_string().as_bytes()).is_err());
        let mut v = sample("0.9.1");
        v["artifacts"] = serde_json::json!([]);
        assert!(Manifest::parse(v.to_string().as_bytes()).is_err());
    }

    #[test]
    fn url_tail_rules() {
        assert!(is_clean_url_tail("v0.9.1/manifest.json"));
        for bad in [
            "../x",
            "v1/../x",
            "v1/%2e%2e/x",
            "v1/%2E./x",
            "a//b",
            "a\\b",
            "a?b",
            "a#b",
            "u@h/x",
            "%2fetc",
            "",
        ] {
            assert!(!is_clean_url_tail(bad), "{bad}");
        }
    }

    #[test]
    fn channel_pointer_roundtrip_and_schema() {
        let ok = serde_json::json!({"schema":1,"channel":"main","version":"v0.9.1","manifest_url":"https://x/y/manifest.json","manifest_sha256":"b".repeat(64)});
        assert!(ChannelPointer::parse(ok.to_string().as_bytes()).is_ok());
        let mut with = ok.clone();
        with["seq"] = 5007.into();
        with["issued_at"] = "2026-10-04T12:00:00Z".into();
        let parsed = ChannelPointer::parse(with.to_string().as_bytes()).unwrap();
        assert_eq!(parsed.seq, Some(5007));
        let mut bad = ok.clone();
        bad["issued_at"] = "yesterday".into();
        assert!(ChannelPointer::parse(bad.to_string().as_bytes()).is_err());
        let mut bad = ok.clone();
        bad["schema"] = 9.into();
        assert!(matches!(
            ChannelPointer::parse(bad.to_string().as_bytes()),
            Err(UpdateError::UnsupportedSchema { .. })
        ));
        let mut bad = ok.clone();
        bad["manifest_url"] = "https://x/y?z=1".into();
        assert!(ChannelPointer::parse(bad.to_string().as_bytes()).is_err());
        assert!(ChannelPointer::parse(b"not json").is_err());
        let mut bad = ok.clone();
        bad.as_object_mut().unwrap().remove("channel");
        assert!(ChannelPointer::parse(bad.to_string().as_bytes()).is_err());
        let mut bad = ok;
        bad["version"] = "0.9.1+abc".into();
        assert!(ChannelPointer::parse(bad.to_string().as_bytes()).is_err());
    }
}
