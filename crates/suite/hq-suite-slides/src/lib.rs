//! Slides: a slides note as cards, written as a native `.pptx`.
//!
//! The note is ordinary markdown (`docs/suite/HQMD.md`): the level 1 heading is the deck title,
//! each level 2 heading a card. `hq-suite-adapter` reads the note and writes the file; this crate
//! only turns text into bytes.

pub mod deck;
pub mod pptx;

pub use deck::{Card, Deck, Item, Layout, Run};
pub use pptx::{PptxError, render_pptx};

/// A slides note's markdown (without its frontmatter) as the deck it describes and the bytes of
/// its `.pptx`. The deck says what the file leaves out (pictures, unknown layouts).
pub fn pptx_from_markdown(title: &str, markdown: &str) -> Result<(Deck, Vec<u8>), PptxError> {
    let deck = Deck::from_markdown(title, markdown);
    let bytes = render_pptx(&deck)?;
    Ok((deck, bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn parts(bytes: &[u8]) -> Vec<(String, String)> {
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        (0..zip.len())
            .map(|i| {
                let mut f = zip.by_index(i).unwrap();
                let mut text = String::new();
                f.read_to_string(&mut text).unwrap();
                (String::from_utf8_lossy(f.name_raw()).into_owned(), text)
            })
            .collect()
    }

    const NOTE: &str = "# Q4 review & plan\n\nFor the board.\n\n## Results\n\n- Revenue **up** 12%\n  - Uganda *strongest*\n- Costs \"flat\" & steady\n\n## Plan\n<!-- layout: two-column -->\n\n1. Hire\n\n---\n\n| item | qty |\n|---|---|\n| Sugar | 40 |\n";

    #[test]
    fn the_file_is_a_package_with_a_slide_for_the_title_and_each_card() {
        let (deck, bytes) = pptx_from_markdown("fallback", NOTE).unwrap();
        assert_eq!(deck.cards.len(), 2);
        let names: Vec<String> = parts(&bytes).into_iter().map(|(n, _)| n).collect();
        for wanted in [
            "[Content_Types].xml",
            "_rels/.rels",
            "ppt/presentation.xml",
            "ppt/slideMasters/slideMaster1.xml",
            "ppt/slideLayouts/slideLayout1.xml",
            "ppt/slideLayouts/slideLayout2.xml",
            "ppt/theme/theme1.xml",
            "ppt/slides/slide1.xml",
            "ppt/slides/slide2.xml",
            "ppt/slides/slide3.xml",
        ] {
            assert!(names.iter().any(|n| n == wanted), "{wanted} is missing");
        }
        assert!(!names.iter().any(|n| n == "ppt/slides/slide4.xml"));
    }

    #[test]
    fn every_part_is_well_formed_xml_and_special_characters_are_escaped() {
        let (_, bytes) = pptx_from_markdown("fallback", NOTE).unwrap();
        for (name, xml) in parts(&bytes) {
            roxmltree::Document::parse(&xml).unwrap_or_else(|e| panic!("{name}: {e}"));
        }
        let all: String = parts(&bytes).into_iter().map(|(_, x)| x).collect();
        assert!(all.contains("Q4 review &amp; plan"));
        assert!(all.contains("Costs &quot;flat&quot; &amp; steady"));
    }

    #[test]
    fn a_hostile_note_cannot_break_the_package() {
        let hostile = format!(
            "# T\u{1}\u{8}x\n\n## \u{0}A\n\n{}\n",
            "- <&>\"'\n".repeat(50)
        );
        let (_, bytes) = pptx_from_markdown("t", &hostile).unwrap();
        for (name, xml) in parts(&bytes) {
            roxmltree::Document::parse(&xml).unwrap_or_else(|e| panic!("{name}: {e}"));
        }
    }
}
