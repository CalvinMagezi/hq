//! What a conversion says about itself: the engine used, what was kept, what
//! was flattened or dropped, and a few counts the caller can sanity-check.
//!
//! The report is data for the person or agent reading the result. Nothing in it
//! is taken from the document's own text except short, labelled details.

use std::collections::BTreeMap;

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LossKind {
    ImagesOmitted,
    TextBoxesInlined,
    NestedTablesFlattened,
    FootnotesAppended,
    ChartsOmitted,
    EmbeddedObjectsOmitted,
    HiddenContentSkipped,
    ScriptsRemoved,
    Truncated,
    OcrDerived,
    LayoutNotPreserved,
    EncodingGuessed,
    NotWellFormed,
    LegacyConverted,
    ExtensionMismatch,
    FormulasShownAsValues,
    Other,
}

impl LossKind {
    /// Whether this kind means the Markdown lacks something the source had.
    /// The others are notes for the reader and leave the outcome at `ok`.
    pub fn is_loss(self) -> bool {
        !matches!(
            self,
            LossKind::ImagesOmitted
                | LossKind::TextBoxesInlined
                | LossKind::FootnotesAppended
                | LossKind::HiddenContentSkipped
                | LossKind::ScriptsRemoved
                | LossKind::LegacyConverted
                | LossKind::ExtensionMismatch
                | LossKind::FormulasShownAsValues
        )
    }

    fn counted(self, n: usize) -> String {
        let s = if n == 1 { "" } else { "s" };
        match self {
            LossKind::ImagesOmitted => format!("{n} image{s} not carried into the note"),
            LossKind::TextBoxesInlined => format!(
                "{n} text box{} placed after its paragraph",
                if n == 1 { "" } else { "es" }
            ),
            LossKind::NestedTablesFlattened => format!("{n} nested table{s} flattened to text"),
            LossKind::FootnotesAppended => format!("{n} footnote{s} listed at the end"),
            LossKind::ChartsOmitted => format!("{n} chart{s} or diagram{s} omitted"),
            LossKind::EmbeddedObjectsOmitted => format!("{n} embedded object{s} omitted"),
            LossKind::HiddenContentSkipped => format!("{n} hidden or deleted item{s} skipped"),
            LossKind::ScriptsRemoved => format!("{n} script or style element{s} removed"),
            _ => format!("{n} occurrence{s}"),
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Warning {
    pub kind: LossKind,
    pub detail: String,
}

/// Collects what a reader had to leave out. Counted kinds become one line each.
#[derive(Debug, Default)]
pub struct Losses {
    counts: BTreeMap<LossKind, usize>,
    notes: Vec<Warning>,
}

impl Losses {
    pub fn hit(&mut self, kind: LossKind) {
        self.hit_n(kind, 1);
    }

    pub fn hit_n(&mut self, kind: LossKind, n: usize) {
        if n > 0 {
            *self.counts.entry(kind).or_default() += n;
        }
    }

    pub fn note(&mut self, kind: LossKind, detail: impl Into<String>) {
        self.notes.push(Warning {
            kind,
            detail: detail.into(),
        });
    }

    pub fn merge(&mut self, other: Losses) {
        for (kind, n) in other.counts {
            self.hit_n(kind, n);
        }
        self.notes.extend(other.notes);
    }

    pub fn into_warnings(self) -> Vec<Warning> {
        let mut out: Vec<Warning> = self
            .counts
            .into_iter()
            .map(|(kind, n)| Warning {
                kind,
                detail: kind.counted(n),
            })
            .collect();
        out.extend(self.notes);
        out
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// Everything the reader understands was carried over.
    Ok,
    /// Converted, but the Markdown lacks something; see the warnings.
    Degraded,
}

/// Counts taken from the finished Markdown, so every engine is measured the same way.
#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct Stats {
    pub chars: usize,
    pub words: usize,
    pub headings: usize,
    pub tables: usize,
    pub list_items: usize,
    pub links: usize,
    pub code_blocks: usize,
    pub image_placeholders: usize,
}

impl Stats {
    pub fn of(markdown: &str) -> Self {
        let mut s = Stats {
            chars: markdown.chars().count(),
            words: markdown.split_whitespace().count(),
            ..Stats::default()
        };
        let mut in_fence = false;
        for line in markdown.lines() {
            let t = line.trim_start();
            if t.starts_with("```") || t.starts_with("~~~") {
                if !in_fence {
                    s.code_blocks += 1;
                }
                in_fence = !in_fence;
                continue;
            }
            if in_fence {
                continue;
            }
            if t.starts_with('#')
                && t.trim_start_matches('#').starts_with(' ')
                && t.len() - t.trim_start_matches('#').len() <= 6
            {
                s.headings += 1;
            } else if is_table_separator(t) {
                s.tables += 1;
            } else if is_list_item(t) {
                s.list_items += 1;
            }
            s.links += line.matches("](").count();
            s.image_placeholders += line.matches("*[image").count();
        }
        s
    }
}

fn is_table_separator(line: &str) -> bool {
    line.starts_with('|')
        && line.ends_with('|')
        && line.contains("---")
        && line.chars().all(|c| matches!(c, '|' | '-' | ':' | ' '))
}

fn is_list_item(line: &str) -> bool {
    if let Some(rest) = line.strip_prefix(['-', '*', '+']) {
        return rest.starts_with(' ');
    }
    let digits = line.chars().take_while(char::is_ascii_digit).count();
    digits > 0 && line[digits..].starts_with(". ")
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ConversionReport {
    /// What the file is, from its content: `docx`, `pdf`, `rtf` and so on.
    pub source_format: String,
    /// `signature`, `extension`, `content` or `signature+extension`.
    pub detected_by: String,
    /// The engine that produced the Markdown: `native:docx`, `transmutation:pdf`, `ocr:tesseract`.
    pub engine: String,
    pub outcome: Outcome,
    pub warnings: Vec<Warning>,
    pub stats: Stats,
    pub source_bytes: u64,
    pub source_sha256: String,
}

impl ConversionReport {
    pub fn has_loss(&self) -> bool {
        self.warnings.iter().any(|w| w.kind.is_loss())
    }

    pub fn warning_lines(&self) -> Vec<String> {
        self.warnings.iter().map(|w| w.detail.clone()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counted_losses_read_as_one_line_each() {
        let mut l = Losses::default();
        l.hit(LossKind::ImagesOmitted);
        l.hit(LossKind::ImagesOmitted);
        l.hit(LossKind::TextBoxesInlined);
        let w = l.into_warnings();
        assert_eq!(w[0].detail, "2 images not carried into the note");
        assert_eq!(w[1].detail, "1 text box placed after its paragraph");
    }

    #[test]
    fn only_content_losses_degrade_the_outcome() {
        assert!(!LossKind::ImagesOmitted.is_loss());
        assert!(LossKind::OcrDerived.is_loss());
        assert!(LossKind::LayoutNotPreserved.is_loss());
    }

    #[test]
    fn stats_count_structure_outside_code_fences() {
        let md = "# T\n\n## A\n\n| a | b |\n| --- | --- |\n| 1 | 2 |\n\n- x\n- y\n1. z\n\nSee [l](u).\n\n```\n# not a heading\n- not an item\n```\n";
        let s = Stats::of(md);
        assert_eq!(
            (s.headings, s.tables, s.list_items, s.links, s.code_blocks),
            (2, 1, 3, 1, 1)
        );
    }
}
