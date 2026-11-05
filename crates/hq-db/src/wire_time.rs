//! Times on the wire as instants (web wave 2, rule 1): SQLite's `datetime('now')` writes UTC as
//! "YYYY-MM-DD HH:MM:SS" with no zone, which a browser reads as local time. Fields that hold such
//! text serialize as RFC 3339 with `Z`; a value that already names its zone passes through.

use serde::Serializer;

/// "2026-11-05 10:00:00" becomes "2026-11-05T10:00:00Z"; anything else is sent as it is.
pub fn to_instant(text: &str) -> String {
    let bare = text.len() >= DATE_TIME_LEN
        && text.as_bytes().get(DATE_LEN) == Some(&b' ')
        && !has_zone(text);
    if bare {
        format!("{}T{}Z", &text[..DATE_LEN], &text[DATE_LEN + 1..])
    } else {
        text.to_string()
    }
}

const DATE_LEN: usize = 10;
const DATE_TIME_LEN: usize = 19;

fn has_zone(text: &str) -> bool {
    let time = &text[DATE_LEN..];
    time.ends_with('Z') || time.contains('+') || time[1..].contains('-')
}

pub fn utc<S: Serializer>(value: &str, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&to_instant(value))
}

pub fn utc_opt<S: Serializer>(value: &Option<String>, s: S) -> Result<S::Ok, S::Error> {
    match value {
        Some(v) => s.serialize_str(&to_instant(v)),
        None => s.serialize_none(),
    }
}

#[cfg(test)]
mod tests {
    use super::to_instant;

    #[test]
    fn a_bare_sqlite_time_gains_its_zone_and_others_pass_through() {
        assert_eq!(to_instant("2026-11-05 10:00:00"), "2026-11-05T10:00:00Z");
        assert_eq!(
            to_instant("2026-11-05 10:00:00.123"),
            "2026-11-05T10:00:00.123Z"
        );
        assert_eq!(
            to_instant("2026-11-05T10:00:00+00:00"),
            "2026-11-05T10:00:00+00:00"
        );
        assert_eq!(to_instant("2026-11-05T10:00:00Z"), "2026-11-05T10:00:00Z");
        assert_eq!(to_instant("2026-11-05"), "2026-11-05");
        assert_eq!(to_instant(""), "");
    }
}
