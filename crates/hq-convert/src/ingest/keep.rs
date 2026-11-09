//! One note from a Google Keep export. Takeout writes each note as JSON, which is the only form
//! that keeps its link and labels.

use serde::Deserialize;

use super::SavedMeta;
use super::ir::{Block, Parsed};
use super::report::Losses;
use crate::types::ConvertError;

const MAX_URL_LEN: usize = 2048;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct KeepJson {
    #[serde(default)]
    title: String,
    #[serde(default)]
    text_content: String,
    #[serde(default)]
    list_content: Vec<ListEntry>,
    #[serde(default)]
    annotations: Vec<Annotation>,
    #[serde(default)]
    labels: Vec<Label>,
    #[serde(default)]
    is_trashed: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListEntry {
    #[serde(default)]
    text: String,
    #[serde(default)]
    is_checked: bool,
}

#[derive(Deserialize)]
struct Annotation {
    #[serde(default)]
    url: String,
    #[serde(default)]
    title: String,
}

#[derive(Deserialize)]
struct Label {
    #[serde(default)]
    name: String,
}

pub(super) struct KeepNote {
    pub parsed: Parsed,
    pub saved: SavedMeta,
}

/// A plain web address with no credentials in it.
fn web_address(raw: &str) -> Option<&str> {
    let raw = raw.trim();
    let rest = raw
        .strip_prefix("https://")
        .or_else(|| raw.strip_prefix("http://"))?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let plain = !authority.is_empty()
        && !authority.contains('@')
        && raw.len() <= MAX_URL_LEN
        && !raw.chars().any(char::is_whitespace);
    plain.then_some(raw)
}

/// The note's link: Keep's own annotation first, otherwise the first address in its text.
fn first_link(note: &KeepJson) -> Option<String> {
    let annotated = note.annotations.iter().find_map(|a| web_address(&a.url));
    let in_text = || note.text_content.split_whitespace().find_map(web_address);
    annotated.or_else(in_text).map(str::to_string)
}

fn body(note: &KeepJson) -> String {
    let mut parts: Vec<String> = Vec::new();
    if !note.text_content.trim().is_empty() {
        parts.push(note.text_content.trim().replace("\r\n", "\n"));
    }
    if !note.list_content.is_empty() {
        let items = note.list_content.iter().map(|e| {
            let mark = if e.is_checked { "x" } else { " " };
            format!("- [{mark}] {}", e.text.trim())
        });
        parts.push(items.collect::<Vec<_>>().join("\n"));
    }
    let links = note.annotations.iter().filter_map(|a| {
        let url = web_address(&a.url)?;
        let title = a.title.trim();
        Some(if title.is_empty() {
            format!("- {url}")
        } else {
            format!("- {title}: {url}")
        })
    });
    let links = links.collect::<Vec<_>>();
    if !links.is_empty() {
        parts.push(links.join("\n"));
    }
    parts.join("\n\n")
}

pub(super) fn read(bytes: &[u8]) -> Result<KeepNote, ConvertError> {
    let note: KeepJson = serde_json::from_slice(bytes)
        .map_err(|e| ConvertError::InboundFailed(format!("not a Keep note: {e}")))?;
    if note.is_trashed {
        return Err(ConvertError::UnsupportedFormat(
            "a note in Keep's trash".to_string(),
        ));
    }
    let title = Some(note.title.trim().to_string()).filter(|t| !t.is_empty());
    let saved = SavedMeta {
        source_url: first_link(&note),
        labels: note.labels.iter().map(|l| l.name.clone()).collect(),
    };
    Ok(KeepNote {
        parsed: Parsed {
            blocks: vec![Block::Raw(body(&note))],
            title,
            losses: Losses::default(),
        },
        saved,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingest::{ConvertOptions, Kind, convert_bytes, sniff};

    const NOTE: &str = r#"{"color":"DEFAULT","isTrashed":false,"isPinned":false,"isArchived":false,
        "textContent":"read later","title":"A tool","labels":[{"name":"To try"}],
        "annotations":[{"source":"WEBLINK","url":"https://example.com/tool","title":"The tool"}],
        "userEditedTimestampUsec":1}"#;

    #[tokio::test]
    async fn a_keep_note_keeps_its_title_link_and_labels() {
        assert_eq!(sniff(NOTE.as_bytes(), "json").kind, Kind::KeepNote);
        let out = convert_bytes(NOTE.into(), "json", None, &ConvertOptions::default())
            .await
            .unwrap();
        assert_eq!(out.title.as_deref(), Some("A tool"));
        assert_eq!(
            out.saved.source_url.as_deref(),
            Some("https://example.com/tool")
        );
        assert_eq!(out.saved.labels, vec!["To try"]);
        assert!(out.markdown.contains("read later"));
        assert!(out.markdown.contains("The tool: https://example.com/tool"));
    }

    #[test]
    fn a_note_in_keeps_trash_is_not_imported() {
        let trashed = NOTE.replace("\"isTrashed\":false", "\"isTrashed\":true");
        assert!(read(trashed.as_bytes()).is_err());
    }

    #[test]
    fn a_checklist_reads_as_one_and_a_bad_address_is_not_the_link() {
        let list = r#"{"isTrashed":false,"isPinned":false,"listContent":[{"text":"milk","isChecked":true}],
            "annotations":[{"url":"https://user:pw@example.com/","title":"x"},{"url":"file:///etc/passwd"}],
            "textContent":"http://plain.example/a"}"#;
        let note = read(list.as_bytes()).unwrap();
        assert_eq!(
            note.saved.source_url.as_deref(),
            Some("http://plain.example/a")
        );
        let Block::Raw(text) = &note.parsed.blocks[0] else {
            panic!("raw text");
        };
        assert!(text.contains("- [x] milk"));
        assert!(!text.contains("user:pw"));
        assert!(!text.contains("/etc/passwd"));
    }

    #[test]
    fn other_json_is_not_taken_for_a_keep_note() {
        assert_ne!(sniff(br#"{"a":1}"#, "json").kind, Kind::KeepNote);
    }
}
