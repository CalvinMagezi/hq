//! Safe access to the zip container behind docx, pptx, xlsx and OpenDocument
//! files, and to the XML inside it.
//!
//! Everything is read into memory with hard caps, so a crafted file that
//! expands to gigabytes (a zip bomb) or holds millions of entries is refused
//! rather than followed. Nothing is ever extracted to disk.

use std::collections::HashMap;
use std::io::{Cursor, Read};

use roxmltree::{Document, Node, ParsingOptions};

use crate::types::ConvertError;

/// Caps applied to one conversion.
#[derive(Debug, Clone)]
pub struct Limits {
    /// Largest input file accepted.
    pub max_input_bytes: u64,
    /// Most entries a container may hold.
    pub max_entries: usize,
    /// Largest single part, after decompression.
    pub max_entry_bytes: u64,
    /// Most bytes read from one container, after decompression.
    pub max_total_bytes: u64,
    /// Most nodes an XML document may hold.
    pub max_xml_nodes: u32,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_input_bytes: 64 * 1024 * 1024,
            max_entries: 20_000,
            max_entry_bytes: 96 * 1024 * 1024,
            max_total_bytes: 256 * 1024 * 1024,
            max_xml_nodes: 6_000_000,
        }
    }
}

pub struct Package<'a> {
    zip: zip::ZipArchive<Cursor<&'a [u8]>>,
    names: Vec<String>,
    read_total: u64,
    limits: Limits,
}

impl<'a> Package<'a> {
    pub fn open(bytes: &'a [u8], limits: &Limits) -> Result<Self, ConvertError> {
        let zip = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| {
            ConvertError::InboundFailed(format!("not a readable zip container: {e}"))
        })?;
        if zip.len() > limits.max_entries {
            return Err(ConvertError::TooLarge(format!(
                "container holds {} entries (limit {})",
                zip.len(),
                limits.max_entries
            )));
        }
        let names = zip
            .file_names()
            .filter_map(Result::ok)
            .map(|n| n.into_owned())
            .collect();
        Ok(Self {
            zip,
            names,
            read_total: 0,
            limits: limits.clone(),
        })
    }

    pub fn names(&self) -> &[String] {
        &self.names
    }

    pub fn has(&self, name: &str) -> bool {
        self.names.iter().any(|n| n == name)
    }

    /// The named part, or `None` if the container has no such part.
    pub fn read(&mut self, name: &str) -> Result<Option<Vec<u8>>, ConvertError> {
        let actual = match self.names.iter().find(|n| n.as_str() == name) {
            Some(n) => n.clone(),
            // Some producers vary the case of part names.
            None => match self.names.iter().find(|n| n.eq_ignore_ascii_case(name)) {
                Some(n) => n.clone(),
                None => return Ok(None),
            },
        };
        let remaining = self.limits.max_total_bytes.saturating_sub(self.read_total);
        let cap = self.limits.max_entry_bytes.min(remaining);
        let mut file = self
            .zip
            .by_name(&actual)
            .map_err(|e| ConvertError::InboundFailed(format!("cannot read part {actual}: {e}")))?;
        if file.encrypted() {
            return Err(ConvertError::Encrypted(format!(
                "part {actual} is encrypted"
            )));
        }
        if file.size() > cap {
            return Err(ConvertError::TooLarge(format!(
                "part {actual} claims {} bytes (limit {cap})",
                file.size()
            )));
        }
        let mut buf = Vec::with_capacity(file.size().min(cap) as usize);
        (&mut file)
            .take(cap + 1)
            .read_to_end(&mut buf)
            .map_err(|e| ConvertError::InboundFailed(format!("cannot read part {actual}: {e}")))?;
        if buf.len() as u64 > cap {
            return Err(ConvertError::TooLarge(format!(
                "part {actual} expands past the {cap} byte limit"
            )));
        }
        self.read_total += buf.len() as u64;
        Ok(Some(buf))
    }

    pub fn read_text(&mut self, name: &str) -> Result<Option<String>, ConvertError> {
        Ok(self.read(name)?.map(text_of))
    }

    pub fn nodes_limit(&self) -> u32 {
        self.limits.max_xml_nodes
    }
}

/// XML parts as text: UTF-8, or UTF-16 when a byte order mark says so.
pub fn text_of(bytes: Vec<u8>) -> String {
    if bytes.starts_with(&[0xFF, 0xFE]) {
        return encoding_rs::UTF_16LE.decode(&bytes[2..]).0.into_owned();
    }
    if bytes.starts_with(&[0xFE, 0xFF]) {
        return encoding_rs::UTF_16BE.decode(&bytes[2..]).0.into_owned();
    }
    match String::from_utf8(bytes) {
        Ok(s) => s.strip_prefix('\u{feff}').map(str::to_string).unwrap_or(s),
        Err(e) => String::from_utf8_lossy(e.as_bytes()).into_owned(),
    }
}

/// Parses XML without ever resolving external entities. `allow_dtd` is off for
/// office parts, which have no business carrying one.
pub fn parse_xml(
    text: &str,
    allow_dtd: bool,
    max_nodes: u32,
) -> Result<Document<'_>, ConvertError> {
    Document::parse_with_options(
        text,
        ParsingOptions {
            allow_dtd,
            nodes_limit: max_nodes,
            ..ParsingOptions::default()
        },
    )
    .map_err(|e| ConvertError::InboundFailed(format!("malformed XML: {e}")))
}

/// Whether `n` is the element `{ns}name`.
pub fn is(n: Node<'_, '_>, ns: &str, name: &str) -> bool {
    n.is_element() && n.tag_name().name() == name && n.tag_name().namespace() == Some(ns)
}

/// The first child element `{ns}name`.
pub fn child<'a, 'i>(n: Node<'a, 'i>, ns: &str, name: &str) -> Option<Node<'a, 'i>> {
    n.children().find(|c| is(*c, ns, name))
}

/// The attribute `{ns}name`.
pub fn attr<'a>(n: Node<'a, '_>, ns: &str, name: &str) -> Option<&'a str> {
    n.attribute((ns, name))
}

/// An on/off property such as `w:b`: present, and not switched off.
pub fn flag(n: Option<Node<'_, '_>>, ns: &str) -> bool {
    match n {
        None => false,
        Some(n) => !matches!(
            attr(n, ns, "val").unwrap_or("true"),
            "0" | "false" | "off" | "none"
        ),
    }
}

/// The title in `docProps/core.xml`, if the file has one.
pub fn core_title(xml: &str, max_nodes: u32) -> Option<String> {
    const DC: &str = "http://purl.org/dc/elements/1.1/";
    let doc = parse_xml(xml, false, max_nodes).ok()?;
    let title = doc
        .descendants()
        .find(|n| is(*n, DC, "title"))?
        .text()?
        .trim()
        .to_string();
    (!title.is_empty()).then_some(title)
}

pub const REL_NS: &str = "http://schemas.openxmlformats.org/package/2006/relationships";

/// `_rels/*.rels`: relationship id to (target, is external).
pub fn read_rels(xml: &str, max_nodes: u32) -> HashMap<String, (String, bool)> {
    let mut out = HashMap::new();
    if let Ok(doc) = parse_xml(xml, false, max_nodes) {
        for n in doc.descendants().filter(|n| is(*n, REL_NS, "Relationship")) {
            if let (Some(id), Some(target)) = (n.attribute("Id"), n.attribute("Target")) {
                let external = n.attribute("TargetMode") == Some("External");
                out.insert(id.to_string(), (target.to_string(), external));
            }
        }
    }
    out
}

/// Resolves `target` against the part that holds the relationship.
pub fn resolve_part(base_dir: &str, target: &str) -> String {
    if let Some(abs) = target.strip_prefix('/') {
        return abs.to_string();
    }
    let mut parts: Vec<&str> = base_dir.split('/').filter(|p| !p.is_empty()).collect();
    for seg in target.split('/') {
        match seg {
            "." | "" => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    parts.join("/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingest::testutil::zip_of;

    #[test]
    fn reads_parts_and_reports_missing_ones() {
        let bytes = zip_of(&[("a.xml", b"<a/>".to_vec())]);
        let mut p = Package::open(&bytes, &Limits::default()).unwrap();
        assert_eq!(p.read_text("a.xml").unwrap().as_deref(), Some("<a/>"));
        assert!(p.read("missing.xml").unwrap().is_none());
    }

    #[test]
    fn a_part_that_expands_past_the_cap_is_refused() {
        let bytes = zip_of(&[("big.xml", vec![b'a'; 100_000])]);
        let limits = Limits {
            max_entry_bytes: 10_000,
            ..Limits::default()
        };
        let mut p = Package::open(&bytes, &limits).unwrap();
        assert!(matches!(p.read("big.xml"), Err(ConvertError::TooLarge(_))));
    }

    #[test]
    fn too_many_entries_are_refused_up_front() {
        let entries: Vec<(String, Vec<u8>)> =
            (0..30).map(|i| (format!("f{i}.txt"), vec![b'x'])).collect();
        let refs: Vec<(&str, Vec<u8>)> = entries
            .iter()
            .map(|(n, d)| (n.as_str(), d.clone()))
            .collect();
        let bytes = zip_of(&refs);
        let limits = Limits {
            max_entries: 10,
            ..Limits::default()
        };
        assert!(matches!(
            Package::open(&bytes, &limits),
            Err(ConvertError::TooLarge(_))
        ));
    }

    #[test]
    fn doctype_is_refused_in_office_parts() {
        let xml = "<?xml version=\"1.0\"?><!DOCTYPE a [<!ENTITY x \"y\">]><a>&x;</a>";
        assert!(parse_xml(xml, false, 1000).is_err());
    }

    #[test]
    fn relationship_targets_resolve_against_their_part() {
        assert_eq!(
            resolve_part("ppt/slides", "../notesSlides/notesSlide1.xml"),
            "ppt/notesSlides/notesSlide1.xml"
        );
        assert_eq!(resolve_part("word", "media/a.png"), "word/media/a.png");
        assert_eq!(resolve_part("word", "/xl/a.xml"), "xl/a.xml");
    }
}
