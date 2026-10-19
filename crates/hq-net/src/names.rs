//! Names that end up in printed root commands, file paths and device configs. Each is checked
//! with the same rule wherever it enters: at `init`, when `net.toml` is loaded, and by the applier.

const MAX_INTERFACE_NAME: usize = 15;
const MIN_PORT: u16 = 1;
const MAX_HOST_LEN: usize = 253;
const MAX_DNS_LABEL_LEN: usize = 63;

/// A Linux interface name the applier will take and a shell will not treat specially: a lowercase
/// letter, then lowercase letters, digits or `_`, at most 15 characters (the kernel's limit).
pub fn valid_interface(name: &str) -> bool {
    let mut chars = name.chars();
    name.len() <= MAX_INTERFACE_NAME
        && chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// Host and port, where the host is a DNS name or an IPv4 address (an IPv6 address in brackets).
/// It goes into device configs and QR codes, so nothing else is let through.
pub fn valid_endpoint(text: &str) -> bool {
    let Some((host, port)) = text.rsplit_once(':') else {
        return false;
    };
    let port_ok = port.parse::<u16>().is_ok_and(|p| p >= MIN_PORT)
        && !port.starts_with('0')
        && !port.starts_with('+');
    let host_ok = if let Some(inner) = host.strip_prefix('[').and_then(|h| h.strip_suffix(']')) {
        inner.parse::<std::net::Ipv6Addr>().is_ok()
    } else {
        !host.is_empty()
            && host.len() <= MAX_HOST_LEN
            && host.split('.').all(|label| {
                !label.is_empty()
                    && label.len() <= MAX_DNS_LABEL_LEN
                    && !label.starts_with('-')
                    && !label.ends_with('-')
                    && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
            })
    };
    port_ok && host_ok
}

/// `text` as one word for a POSIX shell. Plain paths and names pass through unchanged; anything
/// else is wrapped in single quotes, with each single quote written as `'\''`.
pub fn shell_quote(text: &str) -> String {
    let plain = !text.is_empty()
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "/._-+=:@%,".contains(c));
    if plain {
        text.to_string()
    } else {
        format!("'{}'", text.replace('\'', "'\\''"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_quote_leaves_plain_paths_and_wraps_everything_else() {
        assert_eq!(
            shell_quote("/srv/u/.hq/net/hub.key"),
            "/srv/u/.hq/net/hub.key"
        );
        assert_eq!(shell_quote("/srv/my dir/hub.key"), "'/srv/my dir/hub.key'");
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
        assert_eq!(shell_quote(""), "''");
        for hostile in [
            "a;id", "$(id)", "`id`", "a\nb", "a&b", "a|b", "*", "~", "a\"b", "a\\b", "#x",
        ] {
            let quoted = shell_quote(hostile);
            assert!(
                quoted.starts_with('\'') && quoted.ends_with('\''),
                "{hostile:?} -> {quoted}"
            );
        }
    }

    #[test]
    fn interface_names_match_the_kernels_limit_and_a_safe_alphabet() {
        for ok in ["hq0", "a", "wg_hq", "abcdefghijklmno"] {
            assert!(valid_interface(ok), "{ok}");
        }
        for bad in [
            "",
            "Hq0",
            "1hq",
            "hq.0",
            "hq 0",
            "hq/0",
            "hq0\n",
            "hq0\nEOF",
            "abcdefghijklmnop",
            "hq-0",
            "é",
            "hq0;id",
            "$(id)",
            "../x",
        ] {
            assert!(!valid_interface(bad), "{bad:?}");
        }
    }

    #[test]
    fn endpoints_are_host_and_port_with_nothing_that_could_break_a_config() {
        for ok in [
            "example.com:51820",
            "vpn.example.com:1",
            "203.0.113.7:51820",
            "[2001:db8::1]:51820",
            "a-b.example:65535",
        ] {
            assert!(valid_endpoint(ok), "{ok}");
        }
        for bad in [
            "",
            "example.com",
            "example.com:",
            "example.com:0",
            "example.com:65536",
            "example.com:+1",
            "exa mple.com:1",
            "example.com:1\nPostUp = x",
            "-a.example:1",
            "a..b:1",
            ":51820",
            "ex_ample.com:1",
            "2001:db8::1:51820",
            "example.com:1;id",
            "$(id).example.com:1",
        ] {
            assert!(!valid_endpoint(bad), "{bad:?}");
        }
    }
}
