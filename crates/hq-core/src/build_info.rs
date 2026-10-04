//! Build identity of the running binary, registered once at startup by the
//! binary crate (which owns the build script) so library crates such as
//! `hq-web` can report it without a build script of their own.

use serde::Serialize;
use std::sync::OnceLock;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BuildInfo {
    pub version: String,
    pub git_sha: String,
    pub build_time: String,
}

static BUILD_INFO: OnceLock<BuildInfo> = OnceLock::new();

/// Records the binary's build identity. Later calls are ignored.
pub fn set(info: BuildInfo) {
    let _ = BUILD_INFO.set(info);
}

pub fn get() -> Option<&'static BuildInfo> {
    BUILD_INFO.get()
}

/// The git commit the binary was built from, or `"unknown"` before `set`.
pub fn git_sha() -> &'static str {
    get().map(|b| b.git_sha.as_str()).unwrap_or("unknown")
}

pub fn build_time() -> &'static str {
    get().map(|b| b.build_time.as_str()).unwrap_or("unknown")
}
