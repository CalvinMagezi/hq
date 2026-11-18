//! Slides: a slides note as cards, written as a native `.pptx`.
//!
//! The note is ordinary markdown (`docs/suite/HQMD.md`): the level 1 heading is the deck title,
//! each level 2 heading a card. `hq-suite-adapter` reads the note and writes the file; this crate
//! only turns text into bytes.

pub mod deck;
pub mod pptx;

pub use deck::{Card, Deck, Item, Kind, Layout, Look, Run, Speed, Transition};
pub use pptx::{Picture, PictureLoader, PptxError, PptxReport, render_pptx};

/// A slides note's markdown (without its frontmatter) as the deck it describes and the bytes of
/// its `.pptx`. `load` supplies the pictures the cards link; the report says how many the file
/// holds and how many it left out, and the deck names any layout it did not know.
pub fn pptx_from_markdown(
    title: &str,
    markdown: &str,
    load: PictureLoader<'_>,
) -> Result<(Deck, Vec<u8>, PptxReport), PptxError> {
    let deck = Deck::from_markdown(title, markdown);
    let (bytes, report) = render_pptx(&deck, load)?;
    Ok((deck, bytes, report))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn parts(bytes: &[u8]) -> Vec<(String, String)> {
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        (0..zip.len())
            .filter_map(|i| {
                let mut f = zip.by_index(i).unwrap();
                let name = String::from_utf8_lossy(f.name_raw()).into_owned();
                if name.starts_with("ppt/media/") {
                    return None;
                }
                let mut text = String::new();
                f.read_to_string(&mut text).unwrap();
                Some((name, text))
            })
            .collect()
    }

    const NOTE: &str = "# Q4 review & plan\n\nFor the board.\n\n## Results\n\n- Revenue **up** 12%\n  - Uganda *strongest*\n- Costs \"flat\" & steady\n\n## Plan\n<!-- layout: two-column -->\n\n1. Hire\n\n---\n\n| item | qty |\n|---|---|\n| Sugar | 40 |\n";

    #[test]
    fn the_file_is_a_package_with_a_slide_for_the_title_and_each_card() {
        let (deck, bytes, _) = pptx_from_markdown("fallback", NOTE, &|_| None).unwrap();
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
        let (_, bytes, _) = pptx_from_markdown("fallback", NOTE, &|_| None).unwrap();
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
        let (_, bytes, _) = pptx_from_markdown("t", &hostile, &|_| None).unwrap();
        for (name, xml) in parts(&bytes) {
            roxmltree::Document::parse(&xml).unwrap_or_else(|e| panic!("{name}: {e}"));
        }
    }

    fn tiny_png(w: u32, h: u32) -> Picture {
        // A valid PNG header and IHDR is enough for the package; the bytes are not decoded here.
        let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        png.extend_from_slice(&[0; 16]);
        Picture {
            png,
            width: w,
            height: h,
        }
    }

    #[test]
    fn a_loaded_picture_is_placed_once_per_file_and_a_missing_one_is_counted_as_left_out() {
        let md = "# T\n\n## One\n\n![a](a.png)\n\n![gone](gone.png)\n\n## Two\n\nText\n\n![a again](a.png)\n";
        let load = |src: &str| (src == "a.png").then(|| tiny_png(800, 400));
        let (_, bytes, report) = pptx_from_markdown("t", md, &load).unwrap();
        assert_eq!(
            report,
            PptxReport {
                pictures_placed: 2,
                pictures_left_out: 1
            }
        );
        let names: Vec<String> = parts_names(&bytes);
        assert_eq!(
            names.iter().filter(|n| n.starts_with("ppt/media/")).count(),
            1,
            "the same picture is stored once"
        );
        let all = parts(&bytes);
        let slide2 = &all
            .iter()
            .find(|(n, _)| n == "ppt/slides/slide2.xml")
            .unwrap()
            .1;
        assert!(slide2.contains("<p:pic>") && slide2.contains("r:embed=\"rId2\""));
        let rels2 = &all
            .iter()
            .find(|(n, _)| n == "ppt/slides/_rels/slide2.xml.rels")
            .unwrap()
            .1;
        assert!(rels2.contains("../media/image1.png"));
        for (name, xml) in &all {
            roxmltree::Document::parse(xml).unwrap_or_else(|e| panic!("{name}: {e}"));
        }
    }

    fn parts_names(bytes: &[u8]) -> Vec<String> {
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        (0..zip.len())
            .map(|i| String::from_utf8_lossy(zip.by_index(i).unwrap().name_raw()).into_owned())
            .collect()
    }

    #[test]
    fn speaker_notes_make_a_notes_master_and_a_notes_page_per_card_with_notes_and_well_formed_parts()
     {
        let md = "# T\n\n## One\n<!-- notes: Say <this> & that. -->\n\n- a\n\n## Two\n\n- b\n\n## Three\n<!-- notes: Last one. -->\n\n- c\n";
        let (_, bytes, _) = pptx_from_markdown("t", md, &|_| None).unwrap();
        let all = parts(&bytes);
        let names: Vec<&str> = all.iter().map(|(n, _)| n.as_str()).collect();
        for wanted in [
            "ppt/notesMasters/notesMaster1.xml",
            "ppt/theme/theme2.xml",
            "ppt/notesSlides/notesSlide2.xml",
            "ppt/notesSlides/notesSlide4.xml",
        ] {
            assert!(names.contains(&wanted), "{wanted} is missing");
        }
        assert!(
            !names.contains(&"ppt/notesSlides/notesSlide3.xml"),
            "a card with no notes has no notes page"
        );
        for (name, xml) in &all {
            roxmltree::Document::parse(xml).unwrap_or_else(|e| panic!("{name}: {e}"));
        }
        let page = &all
            .iter()
            .find(|(n, _)| n == "ppt/notesSlides/notesSlide2.xml")
            .unwrap()
            .1;
        assert!(page.contains("Say &lt;this&gt; &amp; that."));
        let slide_rels = &all
            .iter()
            .find(|(n, _)| n == "ppt/slides/_rels/slide2.xml.rels")
            .unwrap()
            .1;
        assert!(slide_rels.contains("../notesSlides/notesSlide2.xml"));
        let pres = &all
            .iter()
            .find(|(n, _)| n == "ppt/presentation.xml")
            .unwrap()
            .1;
        assert!(pres.contains("<p:notesMasterIdLst>"));
        let none = pptx_from_markdown("t", "# T\n\n## A\n\ntext\n", &|_| None)
            .unwrap()
            .1;
        assert!(
            !parts(&none).iter().any(|(n, _)| n.contains("notes")),
            "a deck without notes has no notes parts"
        );
    }

    fn slide(bytes: &[u8], n: usize) -> String {
        parts(bytes)
            .into_iter()
            .find(|(name, _)| *name == format!("ppt/slides/slide{n}.xml"))
            .unwrap()
            .1
    }

    const LOOK_NOTE: &str = "# Board\n<!-- transition: fade -->\n<!-- color: #222 -->\n\n## Dark\n<!-- background: #1F3A5F -->\n<!-- color: #FFFFFF -->\n<!-- transition: push slow -->\n\n- White on blue\n\n## Plain\n\nUses the deck look.\n\n## Still\n<!-- transition: none -->\n\nNo motion.\n";

    #[test]
    fn a_cards_look_comes_from_its_comments_over_the_deck_look() {
        let (deck, bytes, _) = pptx_from_markdown("x", LOOK_NOTE, &|_| None).unwrap();
        assert!(deck.look_problems.is_empty(), "{:?}", deck.look_problems);
        let (title, dark, plain, still) = (
            slide(&bytes, 1),
            slide(&bytes, 2),
            slide(&bytes, 3),
            slide(&bytes, 4),
        );
        assert!(title.contains("<p:transition spd=\"med\"><p:fade/></p:transition>"));
        assert!(title.contains("<a:srgbClr val=\"222222\"/>"), "{title}");
        assert!(dark.contains("<p:bg><p:bgPr><a:solidFill><a:srgbClr val=\"1F3A5F\"/>"));
        assert!(dark.contains("<p:transition spd=\"slow\"><p:push/></p:transition>"));
        assert!(dark.contains("<a:rPr lang=\"en-US\" sz=\"3600\" dirty=\"0\"><a:solidFill><a:srgbClr val=\"FFFFFF\"/>"), "{dark}");
        assert!(plain.contains("<p:fade/>") && !plain.contains("<p:bg>"));
        assert!(
            !still.contains("<p:transition"),
            "none cancels the deck transition"
        );
    }

    #[test]
    fn the_transition_follows_the_colour_map_and_the_fill_leads_the_run_properties() {
        let (_, bytes, _) = pptx_from_markdown("x", LOOK_NOTE, &|_| None).unwrap();
        for n in 1..=4 {
            let xml = slide(&bytes, n);
            roxmltree::Document::parse(&xml).unwrap();
            if let Some(t) = xml.find("<p:transition") {
                assert!(
                    xml.find("</p:clrMapOvr>").unwrap() < t,
                    "transition is after clrMapOvr"
                );
            }
            for run in xml.split("<a:rPr ").skip(1) {
                let body = &run[run.find('>').unwrap() + 1..];
                if let (Some(fill), Some(latin)) =
                    (body.find("<a:solidFill>"), body.find("<a:latin"))
                {
                    assert!(fill < latin, "fill before latin");
                }
            }
        }
    }

    #[test]
    fn an_unusable_transition_or_colour_is_reported_and_hard_to_read_colours_are_flagged() {
        let md = "# T\n\n## A\n<!-- transition: swirl -->\n<!-- background: red -->\n\nx\n\n## B\n<!-- background: #FFFFFF -->\n<!-- color: #F0F0F0 -->\n\ny\n\n## C\n<!-- transition: fade fast -->\n\nz\n";
        let (deck, bytes, _) = pptx_from_markdown("x", md, &|_| None).unwrap();
        let said = deck.look_problems.join("\n");
        assert!(said.contains("`swirl` is not a transition"), "{said}");
        assert!(
            said.contains("`red` is not a colour for the background"),
            "{said}"
        );
        assert!(
            said.contains("Card \"B\": the text (#F0F0F0) is hard to read"),
            "{said}"
        );
        assert_eq!(deck.look_problems.len(), 3, "{said}");
        assert!(slide(&bytes, 4).contains("<p:transition spd=\"fast\"><p:fade/>"));
        assert!(!slide(&bytes, 2).contains("<p:transition"));
    }

    #[test]
    fn a_card_that_sets_only_a_background_is_checked_against_the_themes_black_text() {
        let md = "# T\n\n## Night\n<!-- background: #000000 -->\n\nBlack on black\n";
        let (deck, _, _) = pptx_from_markdown("x", md, &|_| None).unwrap();
        assert_eq!(deck.look_problems.len(), 1, "{:?}", deck.look_problems);
    }

    #[test]
    fn colours_parse_from_three_or_six_digits_and_transitions_from_a_name_and_a_speed() {
        use crate::deck::{Kind, Speed, Transition, parse_color};
        assert_eq!(parse_color("#1f3a5f").as_deref(), Some("1F3A5F"));
        assert_eq!(parse_color("abc").as_deref(), Some("AABBCC"));
        for bad in ["", "#12", "#GGGGGG", "red", "#1234567"] {
            assert_eq!(parse_color(bad), None, "{bad}");
        }
        assert_eq!(
            Transition::parse("Wipe FAST"),
            Some(Transition {
                kind: Kind::Wipe,
                speed: Speed::Fast
            })
        );
        assert_eq!(Transition::parse("fade slowly"), None);
        assert_eq!(Transition::parse("fade slow extra"), None);
    }
}
