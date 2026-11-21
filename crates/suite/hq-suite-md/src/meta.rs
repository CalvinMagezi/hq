//! The flat frontmatter keys the suite adds to a note, and what their values may be.
//! Existing import keys (`source`, `source_sha256`, `noteType`, ...) are not repeated here.

use chrono::{DateTime, NaiveDate};

use crate::frontmatter;

pub const KEY_KIND: &str = "suite_kind";
pub const KEY_FIDELITY: &str = "suite_fidelity";
pub const KEY_ORIGINAL_STATE: &str = "suite_original_state";
pub const KEY_ORIGINAL_SINK: &str = "suite_original_sink";
pub const KEY_ORIGINAL_BYTES: &str = "suite_original_bytes";
pub const KEY_SHRUNK_AT: &str = "suite_shrunk_at";
pub const KEY_SAVED_BYTES: &str = "suite_saved_bytes";
pub const KEY_TOC: &str = "suite_toc";
pub const KEY_HEADER: &str = "suite_header";

/// Every key above, for tools that list or strip them.
pub const KEYS: &[&str] = &[
    KEY_KIND,
    KEY_FIDELITY,
    KEY_ORIGINAL_STATE,
    KEY_ORIGINAL_SINK,
    KEY_ORIGINAL_BYTES,
    KEY_SHRUNK_AT,
    KEY_SAVED_BYTES,
    KEY_TOC,
    KEY_HEADER,
];

macro_rules! vocabulary {
    ($(#[$doc:meta])* $name:ident { $($variant:ident => $text:literal),+ $(,)? }) => {
        $(#[$doc])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum $name { $($variant),+ }

        impl $name {
            pub const ALL: &'static [$name] = &[$($name::$variant),+];

            pub fn as_str(self) -> &'static str {
                match self { $($name::$variant => $text),+ }
            }

            pub fn parse(s: &str) -> Option<$name> {
                match s { $($text => Some($name::$variant),)+ _ => None }
            }
        }
    };
}

vocabulary! {
    /// Which tool owns the note.
    SuiteKind {
        Doc => "doc",
        Sheet => "sheet",
        Slides => "slides",
        Dashboard => "dashboard",
        DriveItem => "drive-item",
    }
}

vocabulary! {
    /// How faithfully the note stands in for the file it came from (see `docs/suite/HQMD.md`).
    Fidelity {
        Native => "native",
        Safe => "safe",
        Lossy => "lossy",
        View => "view",
    }
}

vocabulary! {
    /// Where the original file is.
    OriginalState {
        Kept => "kept",
        Offloaded => "offloaded",
        Trash => "trash",
        Gone => "gone",
    }
}

impl Fidelity {
    /// Whether a file of this tier may be shrunk (its original moved away). `View` never is;
    /// `Lossy` only with consent that names the losses, which the caller must ask for.
    pub fn shrinkable(self) -> bool {
        matches!(self, Fidelity::Safe | Fidelity::Lossy)
    }

    /// Whether the note may be edited as hqmd.
    pub fn editable(self) -> bool {
        !matches!(self, Fidelity::View)
    }
}

/// What the suite's keys say about one note. Every field is optional because most notes have
/// none of them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SuiteMeta {
    pub kind: Option<SuiteKind>,
    pub fidelity: Option<Fidelity>,
    pub original_state: Option<OriginalState>,
    pub original_sink: Option<String>,
    pub original_bytes: Option<u64>,
    pub shrunk_at: Option<String>,
    pub saved_bytes: Option<i64>,
    /// A Word export gets a table of contents (`suite_toc: true`).
    pub toc: Option<bool>,
    /// Plain text for the page header of a Word export (`suite_header`).
    pub header: Option<String>,
}

/// A problem with the suite's keys on one note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetaProblem {
    pub key: &'static str,
    pub message: String,
}

impl SuiteMeta {
    /// Reads the suite keys from a raw frontmatter block. Values that do not fit their
    /// vocabulary are left unset and listed in the second part, never guessed.
    pub fn read(raw: &str) -> (SuiteMeta, Vec<MetaProblem>) {
        let mut meta = SuiteMeta::default();
        let mut problems = Vec::new();
        let mut bad =
            |key: &'static str, message: String| problems.push(MetaProblem { key, message });

        if let Some(v) = frontmatter::get(raw, KEY_KIND) {
            match SuiteKind::parse(&v) {
                Some(k) => meta.kind = Some(k),
                None => bad(
                    KEY_KIND,
                    format!("`{v}` is not one of doc, sheet, slides, dashboard, drive-item"),
                ),
            }
        }
        if let Some(v) = frontmatter::get(raw, KEY_FIDELITY) {
            match Fidelity::parse(&v) {
                Some(k) => meta.fidelity = Some(k),
                None => bad(
                    KEY_FIDELITY,
                    format!("`{v}` is not one of native, safe, lossy, view"),
                ),
            }
        }
        if let Some(v) = frontmatter::get(raw, KEY_ORIGINAL_STATE) {
            match OriginalState::parse(&v) {
                Some(k) => meta.original_state = Some(k),
                None => bad(
                    KEY_ORIGINAL_STATE,
                    format!("`{v}` is not one of kept, offloaded, trash, gone"),
                ),
            }
        }
        meta.original_sink = frontmatter::get(raw, KEY_ORIGINAL_SINK).filter(|s| !s.is_empty());
        if let Some(v) = frontmatter::get(raw, KEY_ORIGINAL_BYTES) {
            match v.parse::<u64>() {
                Ok(n) => meta.original_bytes = Some(n),
                Err(_) => bad(
                    KEY_ORIGINAL_BYTES,
                    format!("`{v}` is not a whole number of bytes"),
                ),
            }
        }
        if let Some(v) = frontmatter::get(raw, KEY_SHRUNK_AT) {
            if is_iso_date_or_time(&v) {
                meta.shrunk_at = Some(v);
            } else {
                bad(KEY_SHRUNK_AT, format!("`{v}` is not an ISO 8601 date"));
            }
        }
        if let Some(v) = frontmatter::get(raw, KEY_SAVED_BYTES) {
            match v.parse::<i64>() {
                Ok(n) => meta.saved_bytes = Some(n),
                Err(_) => bad(
                    KEY_SAVED_BYTES,
                    format!("`{v}` is not a whole number of bytes"),
                ),
            }
        }

        if let Some(v) = frontmatter::get(raw, KEY_TOC) {
            match v.as_str() {
                "true" => meta.toc = Some(true),
                "false" => meta.toc = Some(false),
                _ => bad(KEY_TOC, format!("`{v}` is not true or false")),
            }
        }
        meta.header = frontmatter::get(raw, KEY_HEADER).filter(|s| !s.trim().is_empty());

        // Keys that only make sense together.
        if meta.original_state == Some(OriginalState::Offloaded) && meta.original_sink.is_none() {
            bad(
                KEY_ORIGINAL_SINK,
                "an offloaded original needs the sink it went to".into(),
            );
        }
        if meta.original_state != Some(OriginalState::Offloaded) && meta.original_sink.is_some() {
            bad(
                KEY_ORIGINAL_SINK,
                "a sink is only named while the original is offloaded".into(),
            );
        }
        if meta.fidelity == Some(Fidelity::View)
            && meta
                .original_state
                .is_some_and(|s| s != OriginalState::Kept)
        {
            bad(
                KEY_FIDELITY,
                "a view-only note must keep its original".into(),
            );
        }
        (meta, problems)
    }
}

fn is_iso_date_or_time(s: &str) -> bool {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").is_ok() || DateTime::parse_from_rfc3339(s).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vocabularies_round_trip() {
        for k in SuiteKind::ALL {
            assert_eq!(SuiteKind::parse(k.as_str()), Some(*k));
        }
        for k in Fidelity::ALL {
            assert_eq!(Fidelity::parse(k.as_str()), Some(*k));
        }
        for k in OriginalState::ALL {
            assert_eq!(OriginalState::parse(k.as_str()), Some(*k));
        }
        assert_eq!(Fidelity::parse("Safe"), None);
    }

    #[test]
    fn tiers_say_what_they_allow() {
        assert!(Fidelity::Safe.shrinkable() && Fidelity::Lossy.shrinkable());
        assert!(!Fidelity::View.shrinkable() && !Fidelity::Native.shrinkable());
        assert!(!Fidelity::View.editable() && Fidelity::Lossy.editable());
    }

    #[test]
    fn reads_a_complete_block() {
        let raw = "---\nsuite_kind: doc\nsuite_fidelity: safe\nsuite_original_state: offloaded\nsuite_original_sink: usb-1\nsuite_original_bytes: 52000\nsuite_shrunk_at: \"2026-11-16\"\nsuite_saved_bytes: 40000\n---\n";
        let (m, problems) = SuiteMeta::read(raw);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(m.kind, Some(SuiteKind::Doc));
        assert_eq!(m.fidelity, Some(Fidelity::Safe));
        assert_eq!(m.original_state, Some(OriginalState::Offloaded));
        assert_eq!(m.original_sink.as_deref(), Some("usb-1"));
        assert_eq!(m.original_bytes, Some(52000));
        assert_eq!(m.saved_bytes, Some(40000));
    }

    #[test]
    fn a_note_with_none_of_the_keys_is_fine() {
        let (m, p) = SuiteMeta::read("---\ntitle: x\n---\n");
        assert_eq!(m, SuiteMeta::default());
        assert!(p.is_empty());
        assert_eq!(SuiteMeta::read("").0, SuiteMeta::default());
    }

    #[test]
    fn reports_values_it_cannot_use() {
        let raw = "---\nsuite_kind: spreadsheet\nsuite_original_bytes: lots\nsuite_shrunk_at: yesterday\n---\n";
        let (m, p) = SuiteMeta::read(raw);
        assert_eq!(m.kind, None);
        let keys: Vec<_> = p.iter().map(|x| x.key).collect();
        assert!(
            keys.contains(&KEY_KIND)
                && keys.contains(&KEY_ORIGINAL_BYTES)
                && keys.contains(&KEY_SHRUNK_AT)
        );
    }

    #[test]
    fn checks_keys_that_depend_on_each_other() {
        let (_, p) = SuiteMeta::read("---\nsuite_original_state: offloaded\n---\n");
        assert!(p.iter().any(|x| x.key == KEY_ORIGINAL_SINK));
        let (_, p) =
            SuiteMeta::read("---\nsuite_original_state: kept\nsuite_original_sink: usb-1\n---\n");
        assert!(p.iter().any(|x| x.key == KEY_ORIGINAL_SINK));
        let (_, p) =
            SuiteMeta::read("---\nsuite_fidelity: view\nsuite_original_state: trash\n---\n");
        assert!(p.iter().any(|x| x.key == KEY_FIDELITY));
    }

    #[test]
    fn toc_and_header_are_read_and_a_bad_toc_is_reported() {
        let (m, p) = SuiteMeta::read("---\nsuite_toc: true\nsuite_header: \"Draft v2\"\n---\n");
        assert!(p.is_empty(), "{p:?}");
        assert_eq!(m.toc, Some(true));
        assert_eq!(m.header.as_deref(), Some("Draft v2"));
        let (m, p) = SuiteMeta::read("---\nsuite_toc: maybe\n---\n");
        assert_eq!((m.toc, p.len()), (None, 1));
    }
}
