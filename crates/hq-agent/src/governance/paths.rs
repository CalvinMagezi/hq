use std::path::PathBuf;

/// Tools that interact with the filesystem and need path checks.
pub(super) const PATH_TOOLS: &[&str] = &[
    "read_file",
    "write_file",
    "edit_file",
    "find_files",
    "list_dir",
    "bash",
];

/// Expand each path to include both its literal form and its canonicalized
/// (symlink-resolved) form, deduplicated.
///
/// `is_path_allowed` resolves symlinks in the *target* before comparing
/// (falling back to the raw target when it doesn't exist yet), so an
/// allowlist entry that is itself a symlink must carry both forms or the
/// check fails asymmetrically: a read of a file that doesn't exist yet
/// compares raw-vs-raw and passes, but a read of a file that *does* exist
/// compares resolved-vs-raw and fails, because the resolved target now
/// points through the symlink while the allowlist entry still names the
/// symlink itself. `.vault` is exactly this case (it's a symlink to
/// `~/Library/Application Support/agent-hq/vault`), and `/tmp` (a symlink
/// to `/private/tmp` on macOS) is where this was first found.
pub(super) fn expand_with_canonical_forms(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::with_capacity(paths.len() * 2);
    for p in paths {
        if let Ok(resolved) = p.canonicalize()
            && !out.contains(&resolved)
        {
            out.push(resolved);
        }
        if !out.contains(&p) {
            out.push(p);
        }
    }
    out
}

/// System temp directories, always allowed alongside the vault/cwd sandbox.
///
/// `bash` has no path check at all (see `path_args_for_tool` below), so an
/// agent can already write anywhere under `/tmp` with a shell redirect;
/// without this, only the dedicated `read_file`/`write_file`/`edit_file`
/// tools were blocked from the same scratch space, which is exactly where
/// multi-step file generation (unpacking a template, building an
/// intermediate asset) wants to work.
pub fn system_temp_paths() -> Vec<PathBuf> {
    expand_with_canonical_forms(vec![PathBuf::from("/tmp"), std::env::temp_dir()])
}

/// Credential and key material a raw filesystem tool must never reach, even
/// once `allowed_paths` covers all of `$HOME` (see `ToolGuardian::new`).
/// hq's own code reaches what lives inside these through dedicated, audited
/// paths (hq-crypto's wallet ops, LLM provider construction from config) —
/// never generic `read_file`/`find_files` — so denying them here costs no
/// legitimate functionality. It does close an exfiltration route: a prompt
/// injection riding in through an untrusted vault note or fetched web page
/// could otherwise talk an agent into reading one of these back into a
/// reply. Checked in `is_path_allowed` before the allowlist, so it wins
/// regardless of how broad `allowed_paths` is.
pub(super) fn sensitive_denied_paths() -> Vec<PathBuf> {
    super::secrets::always_denied_roots()
}

/// Argument keys that name files, checked on every tool (not only
/// `PATH_TOOLS`) against the secret classifier, at any nesting depth so
/// batch edits and converters are covered too.
pub(super) const SECRET_CHECKED_ARG_KEYS: &[&str] =
    &["file_path", "path", "paths", "input_path", "file", "files"];

/// Which argument keys contain file paths for each tool.
pub(super) fn path_args_for_tool(tool_name: &str) -> &[&str] {
    match tool_name {
        "read_file" | "write_file" | "edit_file" => &["file_path"],
        "find_files" | "list_dir" => &["path"],
        _ => &[],
    }
}
