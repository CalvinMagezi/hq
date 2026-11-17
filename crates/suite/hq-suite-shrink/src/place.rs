//! Puts each stored picture where it sat in the document.
//!
//! The importer, asked for markers, writes `![alt](hqmedia:<package part>)` where a Word picture
//! was. Once the pictures are in the media store each marker becomes an ordinary relative link to
//! its object. A marker whose picture was not stored (a format the suite does not recompress or
//! keep, a part over the limits) reads as the plain placeholder the importer always wrote.

use std::collections::{HashMap, HashSet};

use hq_suite_store::CasRef;

const OPEN: &str = "![";
const SPLIT: &str = "](hqmedia:";

/// What placing did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Placement {
    /// Pictures linked from their place in the text.
    pub placed: HashSet<CasRef>,
    /// Markers left as the plain placeholder.
    pub unresolved: u32,
}

/// `markdown` with every marker turned into a link (or a placeholder), and what happened.
pub fn place(
    markdown: &str,
    note_rel: &str,
    by_part: &HashMap<String, CasRef>,
) -> (String, Placement) {
    let mut out = String::with_capacity(markdown.len());
    let mut placement = Placement::default();
    let mut rest = markdown;
    while let Some(start) = rest.find(OPEN) {
        let after_open = &rest[start + OPEN.len()..];
        let marker = after_open.find(SPLIT).and_then(|alt_end| {
            let alt = &after_open[..alt_end];
            let tail = &after_open[alt_end + SPLIT.len()..];
            let part_end = tail.find(')')?;
            // An alt that spans a line or holds another image is not one of ours.
            (!alt.contains(['\n', '[', ']'])).then_some((
                alt,
                &tail[..part_end],
                alt_end + SPLIT.len() + part_end + 1,
            ))
        });
        let Some((alt, part, len)) = marker else {
            out.push_str(&rest[..start + OPEN.len()]);
            rest = &rest[start + OPEN.len()..];
            continue;
        };
        out.push_str(&rest[..start]);
        let after = &rest[start + OPEN.len() + len..];
        match by_part.get(part) {
            Some(r) => {
                let words = if alt.trim().is_empty() {
                    "Picture"
                } else {
                    alt
                };
                // A picture gets a paragraph of its own: the readers draw an image only when it is
                // alone on its line, and text beside it would show as the raw link.
                if !out
                    .rsplit('\n')
                    .next()
                    .unwrap_or_default()
                    .trim()
                    .is_empty()
                {
                    out.push_str("\n\n");
                }
                out.push_str(&r.markdown_image(note_rel, words));
                placement.placed.insert(r.clone());
                if !after
                    .split('\n')
                    .next()
                    .unwrap_or_default()
                    .trim()
                    .is_empty()
                {
                    out.push_str("\n\n");
                    rest = after.trim_start_matches([' ', '\t']);
                    continue;
                }
            }
            None => {
                placement.unresolved += 1;
                if alt.trim().is_empty() {
                    out.push_str("*[image]*");
                } else {
                    out.push_str(&format!("*[image: {alt}]*"));
                }
            }
        }
        rest = &rest[start + OPEN.len() + len..];
    }
    out.push_str(rest);
    (out, placement)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(n: u8) -> CasRef {
        CasRef::new(&format!("{n:02x}").repeat(32), "webp").unwrap()
    }

    #[test]
    fn markers_become_links_where_they_sat() {
        let by_part = HashMap::from([("word/media/image1.png".to_string(), r(1))]);
        let md = "Before\n\n![A plan](hqmedia:word/media/image1.png)\n\nAfter";
        let (out, p) = place(md, "Notebooks/a.md", &by_part);
        assert!(
            out.starts_with("Before\n\n![A plan](../_media/cas/01/"),
            "{out}"
        );
        assert!(out.ends_with(".webp)\n\nAfter"), "{out}");
        assert_eq!(p.placed, HashSet::from([r(1)]));
        assert_eq!(p.unresolved, 0);
    }

    #[test]
    fn a_picture_inside_a_line_of_text_gets_its_own_paragraph() {
        let by_part = HashMap::from([("word/media/image1.png".to_string(), r(1))]);
        let (out, _) = place(
            "**Signed** ![Seal](hqmedia:word/media/image1.png) by us",
            "Notebooks/a.md",
            &by_part,
        );
        let parts: Vec<&str> = out.split("\n\n").collect();
        assert_eq!(parts.len(), 3, "{out:?}");
        assert_eq!(parts[0].trim(), "**Signed**");
        assert!(
            parts[1].starts_with("![Seal](") && parts[1].ends_with(')'),
            "{out:?}"
        );
        assert_eq!(parts[2], "by us");
    }

    #[test]
    fn an_unstored_picture_keeps_the_plain_placeholder() {
        let (out, p) = place(
            "x ![](hqmedia:word/media/image9.emf) y ![Chart](hqmedia:word/media/image8.emf)",
            "a.md",
            &HashMap::new(),
        );
        assert_eq!(out, "x *[image]* y *[image: Chart]*");
        assert_eq!(p.unresolved, 2);
        assert!(p.placed.is_empty());
    }

    #[test]
    fn ordinary_markdown_is_returned_untouched() {
        let md = "Text with ![a web picture](https://example.com/x.png) and [a link](hqmedia-not) ![broken";
        let (out, p) = place(md, "a.md", &HashMap::new());
        assert_eq!(out, md);
        assert_eq!(p, Placement::default());
    }

    #[test]
    fn the_same_picture_can_sit_in_two_places() {
        let by_part = HashMap::from([
            ("word/media/a.png".to_string(), r(2)),
            ("word/media/b.png".to_string(), r(2)),
        ]);
        let (out, p) = place(
            "![](hqmedia:word/media/a.png) ![](hqmedia:word/media/b.png)",
            "a.md",
            &by_part,
        );
        assert_eq!(out.matches("](").count(), 2);
        assert_eq!(p.placed.len(), 1);
    }
}
