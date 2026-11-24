use super::*;

const NOTE: &str = "# Quarterly report\n\nWe saw **three** sites, see [the plan](https://example.com/plan).\n\n## Figures\n\n| Item | Qty |\n|---|---|\n| Sugar | 40 |\n| Salt | 25 |\n\n- first point\n- second point\n\n1. open\n2. close\n\nA closing paragraph.\n";

#[tokio::test]
async fn a_record_rendered_as_word_reads_back_as_the_same_record() {
    let docx = render_docx("Quarterly report", NOTE, None).unwrap();
    assert!(docx.starts_with(b"PK"), "a Word file is a zip");
    let r = regenerate_check(&docx).await.unwrap();
    assert!(r.equal, "{:?}", r.differences);
    assert!(r.blocks >= 7, "{} blocks", r.blocks);
}

#[test]
fn a_difference_in_text_a_table_or_a_link_is_reported() {
    let a = fingerprint(NOTE);
    assert!(compare(a.clone(), fingerprint(NOTE)).equal);
    for (changed, what) in [
        (NOTE.replace("three", "four"), "text"),
        (
            NOTE.replace("| Salt | 25 |", "| Salt | 26 |"),
            "a table cell",
        ),
        (
            NOTE.replace("example.com/plan", "example.com/other"),
            "a link",
        ),
        (NOTE.replace("- second point\n", ""), "a list item"),
        (format!("{NOTE}\nExtra.\n"), "a paragraph"),
    ] {
        let r = compare(a.clone(), fingerprint(&changed));
        assert!(!r.equal, "{what} went unnoticed");
        assert!(!r.differences.is_empty());
    }
}

#[test]
fn spelling_that_does_not_change_meaning_does_not_count() {
    let a = fingerprint("A  line\nbroken here.\n\n![Plan](../../_media/cas/ab/x.webp)\n");
    let b = fingerprint("A line broken here.\n\n![Plan](other/place.png)\n");
    assert!(compare(a, b).equal);
    let c = fingerprint("A line broken here.\n\n![Other](other/place.png)\n");
    assert!(!compare(fingerprint("A line broken here.\n\n![Plan](x.png)\n"), c).equal);
}

#[tokio::test]
async fn something_that_is_not_a_word_file_is_refused_with_a_reason() {
    let err = regenerate_check(b"just some text").await.unwrap_err();
    assert!(matches!(err, DocsError::Unreadable(_)), "{err}");
}

#[test]
fn picture_links_are_rewritten_from_the_vaults_root_and_never_out_of_it() {
    let md = "Text ![A](../../_media/cas/ab/x.webp) and ![B](pics/y.png)\n\n![C](../../../../outside.png) ![D](https://example.com/z.png)\n";
    let out = rebase_pictures(md, "Notebooks/Inbox");
    assert!(out.contains("![A](_media/cas/ab/x.webp)"), "{out}");
    assert!(out.contains("![B](Notebooks/Inbox/pics/y.png)"), "{out}");
    assert!(
        out.contains("![C](../../../../outside.png)"),
        "left as it was: {out}"
    );
    assert!(out.contains("![D](https://example.com/z.png)"), "{out}");
    assert_eq!(rebase_pictures("no pictures", "a"), "no pictures");
    assert_eq!(rebase_pictures("broken ![x](", "a"), "broken ![x](");
}

#[test]
fn frontmatter_asks_for_a_toc_and_a_header_and_a_bad_value_asks_for_nothing() {
    let on = DocxOptions::from_frontmatter("---\nsuite_toc: true\nsuite_header: \"Draft\"\n---\n");
    assert_eq!(
        on,
        DocxOptions {
            toc: true,
            header: "Draft".into()
        }
    );
    assert_eq!(
        DocxOptions::from_frontmatter("---\nsuite_toc: maybe\n---\n"),
        DocxOptions::default()
    );
    assert_eq!(DocxOptions::from_frontmatter(""), DocxOptions::default());
}

#[tokio::test]
async fn comments_are_cut_from_the_word_file_and_a_contents_page_still_reads_back() {
    let note = "# Plan\n\nBody text.\n\n```comment on=0123456789ab\nsecret review note\n```\n\n## Costs\n\nMore.\n";
    let opts = DocxOptions {
        toc: true,
        header: String::new(),
    };
    let docx = render_docx_with("Plan", note, None, &opts).unwrap();
    let read = hq_suite_ingest::read_document(docx.clone(), "docx", &ConvertOptions::default())
        .await
        .unwrap();
    assert!(!read.markdown.contains("secret review note"));
    assert!(read.markdown.contains("Body text.") && read.markdown.contains("Costs"));
    // And the regenerate check still holds for a file that carries a contents field.
    let r = regenerate_check(&docx).await.unwrap();
    assert!(r.equal, "{:?}", r.differences);
}
