//! Folder-name checking, shared by the workspace (Unix) and by callers on any platform.

use std::io::{Error, ErrorKind};

const MAX_NAME_CHARS: usize = 100;
const MAX_NAME_BYTES: usize = 200;

fn invalid(message: impl Into<String>) -> Error {
    Error::new(ErrorKind::InvalidInput, message.into())
}

/// A single folder name: no separators, not `.` or `..`, no control characters.
pub fn check_folder_name(name: &str) -> std::io::Result<&str> {
    let name = name.trim();
    let bad = name.is_empty()
        || name == "."
        || name == ".."
        || name.chars().count() > MAX_NAME_CHARS
        || name.len() > MAX_NAME_BYTES
        || name.chars().any(|c| c.is_control() || matches!(c, '/' | '\\' | ':' | '~' | '$' | '`'));
    if bad {
        return Err(invalid(
            "a folder name is 1 to 100 characters with no slashes, colons, '~', '$' or backticks",
        ));
    }
    Ok(name)
}
