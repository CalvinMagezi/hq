use crate::error::HostError;

/// Longest logical key name accepted (`ctrl+shift+pagedown` is 19).
const MAX_KEY_LEN: usize = 32;

/// Bytes a terminal sends for a logical key name such as `enter`, `esc`,
/// `down`, `ctrl+c` or `f5`. Names are lowercase letters, digits and `+`, `_`,
/// `-`, so a key can never be mistaken for a flag by a caller that forwards it.
pub fn encode_key(name: &str) -> Result<Vec<u8>, HostError> {
    let bad = || HostError::InvalidKey(name.to_string());
    let mut chars = name.chars();
    let shape_ok = chars.next().is_some_and(|c| c.is_ascii_alphanumeric())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '_' | '-'))
        && name.len() <= MAX_KEY_LEN;
    if !shape_ok {
        return Err(bad());
    }
    let lower = name.to_ascii_lowercase();
    if let Some(letter) = lower.strip_prefix("ctrl+") {
        return match letter.as_bytes() {
            [c @ b'a'..=b'z'] => Ok(vec![c - b'a' + 1]),
            _ => Err(bad()),
        };
    }
    let seq: &[u8] = match lower.as_str() {
        "enter" | "return" => b"\r",
        "esc" | "escape" => b"\x1b",
        "tab" => b"\t",
        "backspace" => b"\x7f",
        "space" => b" ",
        "up" => b"\x1b[A",
        "down" => b"\x1b[B",
        "right" => b"\x1b[C",
        "left" => b"\x1b[D",
        "home" => b"\x1b[H",
        "end" => b"\x1b[F",
        "pageup" => b"\x1b[5~",
        "pagedown" => b"\x1b[6~",
        "delete" => b"\x1b[3~",
        "shift+tab" => b"\x1b[Z",
        single if single.len() == 1 => return Ok(single.as_bytes().to_vec()),
        _ => return Err(bad()),
    };
    Ok(seq.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn common_keys_encode() {
        assert_eq!(encode_key("enter").unwrap(), b"\r");
        assert_eq!(encode_key("ctrl+c").unwrap(), [3]);
        assert_eq!(encode_key("down").unwrap(), b"\x1b[B");
        assert_eq!(encode_key("0").unwrap(), b"0");
    }

    #[test]
    fn flags_and_junk_are_refused() {
        for bad in [
            "--help",
            "-x",
            "",
            "ctrl+",
            "ctrl+1",
            "enter now",
            "nokey",
            "a".repeat(40).as_str(),
        ] {
            assert!(encode_key(bad).is_err(), "{bad:?} should be refused");
        }
    }
}
