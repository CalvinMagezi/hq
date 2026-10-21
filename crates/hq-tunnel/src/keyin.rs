//! Taking the private key in. Never from the command line or the environment, where other
//! processes can read it: from the first line of a descriptor the app owns, or from a file only
//! the user can read. The copies made on the way are wiped.

use crate::error::{Result, invalid};
use crate::fsutil;
use crate::secret::Secret32;
use hq_net::key::KEY_TEXT_LEN;
use std::os::fd::RawFd;
use std::path::Path;
use zeroize::Zeroize;

/// A key is 44 characters, then the line ends. Anything longer is not a key.
pub const KEY_LINE_MAX: usize = KEY_TEXT_LEN + 2;

fn decode_line(line: &[u8]) -> Result<Secret32> {
    let line = line.strip_suffix(b"\r").unwrap_or(line);
    // The message never repeats the line: it may be a key with a typo.
    let Some(text) = std::str::from_utf8(line).ok() else {
        return invalid("the key is not 44 characters of base64");
    };
    let Some(mut bytes) = hq_net::key::decode(text) else {
        return invalid("the key is not 44 characters of base64");
    };
    let secret = Secret32::new(bytes);
    bytes.zeroize();
    Ok(secret)
}

/// Reads one line from `fd`, one byte at a time, and stops at the newline. Nothing past it is
/// consumed, so the lines the app sends next are still there for the control channel. A
/// buffered reader would swallow them.
pub fn read_key_from_fd(fd: RawFd) -> Result<Secret32> {
    // SAFETY: isatty only inspects the descriptor.
    if unsafe { libc::isatty(fd) } == 1 {
        return invalid(
            "refusing to read a key from a terminal, where it would be shown as you type; pipe it in or use --key-file",
        );
    }
    let mut line = [0u8; KEY_LINE_MAX];
    let mut len = 0;
    let result = loop {
        let mut byte = 0u8;
        // SAFETY: `byte` is valid for one byte; the descriptor is the caller's open pipe.
        let n = unsafe { libc::read(fd, std::ptr::addr_of_mut!(byte).cast(), 1) };
        if n < 0 {
            let e = std::io::Error::last_os_error();
            if e.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            break Err(e.into());
        }
        if n == 0 {
            break invalid("the input ended before the key line did");
        }
        if byte == b'\n' {
            break decode_line(&line[..len]);
        }
        if len == line.len() {
            break invalid("the first line is too long to be a key");
        }
        line[len] = byte;
        len += 1;
    };
    line.zeroize();
    result
}

/// Reads a key from a file. The file must be a regular file that belongs to this user and is
/// closed to everyone else, and a link is refused.
pub fn read_key_file(path: &Path) -> Result<Secret32> {
    let mut bytes = fsutil::read_private(path, KEY_LINE_MAX as u64 + 1, fsutil::Access::OwnerOnly)
        .map_err(|e| {
            crate::error::Error::Invalid(format!("cannot use the key file {}: {e}", path.display()))
        })?;
    let line = bytes.strip_suffix(b"\n").unwrap_or(&bytes);
    let result = decode_line(line);
    bytes.zeroize();
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::os::unix::fs::PermissionsExt;

    fn pipe() -> (OwnedFd, std::fs::File) {
        let mut fds = [0 as RawFd; 2];
        // SAFETY: `fds` has room for the two descriptors pipe(2) fills in.
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
        // SAFETY: both descriptors were just made and are owned by nobody else.
        unsafe {
            (
                OwnedFd::from_raw_fd(fds[0]),
                std::fs::File::from_raw_fd(fds[1]),
            )
        }
    }

    fn key_text(fill: u8) -> String {
        hq_net::key::encode(&[fill; 32])
    }

    #[test]
    fn the_key_is_the_first_line_and_what_follows_is_left_for_the_control_channel() {
        let (read, mut write) = pipe();
        write
            .write_all(format!("{}\nnetwork-changed\nquit\n", key_text(7)).as_bytes())
            .unwrap();
        let secret = read_key_from_fd(read.as_raw_fd()).unwrap();
        assert_eq!(secret.bytes(), [7u8; 32]);
        drop(write);
        let mut rest = String::new();
        std::fs::File::from(read).read_to_string(&mut rest).unwrap();
        assert_eq!(
            rest, "network-changed\nquit\n",
            "nothing past the newline was taken"
        );
    }

    #[test]
    fn a_line_with_a_carriage_return_is_read() {
        let (read, mut write) = pipe();
        write
            .write_all(format!("{}\r\n", key_text(9)).as_bytes())
            .unwrap();
        assert_eq!(
            read_key_from_fd(read.as_raw_fd()).unwrap().bytes(),
            [9u8; 32]
        );
    }

    #[test]
    fn what_is_not_a_key_is_refused_without_repeating_it() {
        let almost = format!("{}x", key_text(3));
        let long = "a".repeat(200);
        for input in [
            "".to_string(),
            "\n".to_string(),
            "not a key\n".to_string(),
            format!("{almost}\n"),
            format!("{long}\n"),
            // the end of input before the newline
            key_text(3),
        ] {
            let (read, mut write) = pipe();
            write.write_all(input.as_bytes()).unwrap();
            drop(write);
            let err = read_key_from_fd(read.as_raw_fd()).unwrap_err().to_string();
            assert!(
                !err.contains(&almost) && !err.contains("not a key"),
                "{err}"
            );
            assert!(!err.contains(&key_text(3)), "{err}");
        }
    }

    #[test]
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn a_terminal_is_refused() {
        let (mut master, mut slave) = (0 as RawFd, 0 as RawFd);
        // SAFETY: both out-pointers are valid; the name, termios and winsize arguments may be null.
        let ok = unsafe {
            libc::openpty(
                &mut master,
                &mut slave,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        assert_eq!(ok, 0);
        let err = read_key_from_fd(slave).unwrap_err().to_string();
        assert!(err.contains("terminal"), "{err}");
        // SAFETY: closing the two descriptors opened above.
        unsafe {
            libc::close(master);
            libc::close(slave);
        }
    }

    #[test]
    fn a_key_file_must_be_private_and_not_a_link() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("key");
        std::fs::write(&path, format!("{}\n", key_text(5))).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(read_key_file(&path).unwrap().bytes(), [5u8; 32]);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let err = read_key_file(&path).unwrap_err().to_string();
        assert!(
            err.contains("chmod 600") && !err.contains(&key_text(5)),
            "{err}"
        );
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        assert!(read_key_file(&link).is_err());
        std::fs::write(&path, "x".repeat(500)).unwrap();
        assert!(read_key_file(&path).is_err(), "a long file is not a key");
    }
}
