//! Which programs this machine can start. A pane execs its agent directly with the host's own
//! PATH (`env::pane_env`), so that PATH, and not a shell's, decides what can really run.

use std::ffi::OsStr;
use std::path::Path;

const MAX_NAMES: usize = 64;

/// Which of `names` are on `path`. A name with a slash is not a program to search for.
fn found_on(names: &[String], path: &OsStr) -> Vec<String> {
    names
        .iter()
        .take(MAX_NAMES)
        .filter(|name| !name.contains('/'))
        .filter(|name| which::which_in(name.as_str(), Some(path), Path::new("/")).is_ok())
        .cloned()
        .collect()
}

/// Which of `names` are installed; `None` when this host has no PATH to look along.
pub(crate) fn installed(names: &[String]) -> Option<Vec<String>> {
    Some(found_on(names, &std::env::var_os("PATH")?))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn only_an_executable_on_the_path_counts() {
        let dir = tempfile::tempdir().unwrap();
        let program = dir.path().join("agent");
        std::fs::write(&program, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::write(dir.path().join("readme"), "not a program").unwrap();
        let names = ["agent", "readme", "missing", "../agent"].map(String::from);
        assert_eq!(found_on(&names, dir.path().as_os_str()), ["agent"]);
    }
}
