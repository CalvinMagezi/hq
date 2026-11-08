use std::path::{Component, Path, PathBuf};

/// Tools that interact with the filesystem and need path checks.
pub(super) const PATH_TOOLS: &[&str] = &[
    "read_file",
    "write_file",
    "edit_file",
    "find_files",
    "list_dir",
    "grep",
    "file_edit_batch",
    "rollback_file",
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
        "read_file" | "write_file" | "edit_file" | "rollback_file" => &["file_path"],
        "find_files" | "list_dir" | "grep" => &["path"],
        _ => &[],
    }
}

/// Every path a call names: the tool's top-level path keys, plus each `edits[].file_path` of a
/// batch edit. A path the allowlist never sees is a path it never refuses.
pub(super) fn path_values<'a>(tool_name: &str, args: &'a serde_json::Value) -> Vec<&'a str> {
    let mut paths: Vec<&str> = path_args_for_tool(tool_name)
        .iter()
        .filter_map(|key| args.get(*key).and_then(|v| v.as_str()))
        .collect();
    if tool_name == "file_edit_batch"
        && let Some(edits) = args.get("edits").and_then(|v| v.as_array())
    {
        paths.extend(
            edits
                .iter()
                .filter_map(|edit| edit.get("file_path").and_then(|v| v.as_str())),
        );
    }
    paths
}

/// Tools that change a file's contents. The vault's instruction folders are closed to them.
pub(super) const WRITE_TOOLS: &[&str] = &[
    "write_file",
    "edit_file",
    "file_edit_batch",
    "rollback_file",
];

/// The vault folders HQ loads back into its own sessions (identity, threads, skills, slash
/// commands, agent definitions, agent mailboxes). Each has a managed write path with its own
/// approval and checks; a raw file tool writing here would plant instructions or forge a message.
pub(super) fn vault_loaded_folders() -> Vec<&'static str> {
    let mut folders: Vec<&'static str> = hq_vault::rel_path::PRIVATE_FOLDERS
        .iter()
        .chain(hq_vault::rel_path::INSTRUCTION_FOLDERS)
        .copied()
        .collect();
    // Spellings HQ also loads from: `Agents` is the live agents directory, `Skills` and
    // `_agents` are older layouts. The comparison folds case, so these only document intent.
    folders.extend(["_agents"]);
    folders
}

/// Where `path` really points, for a policy check: the deepest ancestor that exists is resolved
/// through every symlink, and the rest is appended with `.` and `..` applied lexically. Parts that
/// do not exist cannot be links, so this is exact, unlike falling back to the raw path, which let
/// `<allowed>/../secret` or a linked parent through for a file that does not exist yet.
pub(super) fn resolve_for_check(path: &Path) -> PathBuf {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("/"))
            .join(path)
    };
    let components: Vec<Component> = absolute.components().collect();
    for split in (0..=components.len()).rev() {
        let head: PathBuf = components[..split].iter().collect();
        let Ok(mut resolved) = head.canonicalize() else {
            continue;
        };
        for part in &components[split..] {
            match part {
                Component::ParentDir => {
                    resolved.pop();
                }
                Component::Normal(name) => resolved.push(name),
                _ => {}
            }
        }
        return resolved;
    }
    absolute
}

/// The top folder of `resolved` inside `vault_root`, folded for comparison, or None outside it.
pub(super) fn vault_top_folder(vault_root: &Path, resolved: &Path) -> Option<String> {
    let rel = resolved.strip_prefix(vault_root).ok()?;
    let rel = rel.to_string_lossy();
    // An unparseable remainder cannot name a folder the loader would read; treat it as none.
    hq_vault::rel_path::RelPath::parse(&rel).ok()?.top()
}

/// Every string under an argument key that names files, at any depth (batch edits nest them).
pub(super) fn nested_path_args(value: &serde_json::Value, out: &mut Vec<String>) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, v) in map {
                if SECRET_CHECKED_ARG_KEYS.contains(&key.as_str()) {
                    match v {
                        serde_json::Value::String(s) => out.push(s.clone()),
                        serde_json::Value::Array(items) => {
                            out.extend(items.iter().filter_map(|i| i.as_str().map(str::to_string)))
                        }
                        _ => {}
                    }
                }
                nested_path_args(v, out);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                nested_path_args(item, out);
            }
        }
        _ => {}
    }
}
