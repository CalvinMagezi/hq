//! End-to-end renders through the embedded Typst engine.

use hq_export::{Document, RenderOptions, render};

/// A valid 1x1 RGBA PNG.
const PNG_1X1: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4,
    0x89, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0xF8, 0xCF, 0xC0, 0xF0,
    0x1F, 0x00, 0x05, 0x00, 0x01, 0xFF, 0x89, 0x99, 0x3D, 0x1D, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45,
    0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
];

const KITCHEN_SINK: &str = r#"Some **bold**, *italic*, `inline code`, ~~gone~~ and [a link](https://example.com).
A second line in the same paragraph, then a hard break:\
after the break. Unicode: café, Naïve, Ssebo, Nnyabo, ŋ, ɛ.

> [!note] Plain note
> Body of a note.

> [!tip] Remember
> Bring the signed forms.
>
> - and a charger
> - and water

> [!warning]-
> No title here.

> [!danger] Careful
> [!bug] is not a nested callout.

> An ordinary quote.

## Table

| Site | Farmers | Status |
|:-----|--------:|:------:|
| Mukono | 42 | done |
| Wakiso | 17 |
| Kampala | 9 | pending | extra |

## Lists

1. one
2. two
   - nested **bold**
   - nested two
10. ten

- [x] done
- [ ] todo

3. starts at three

## Code

```rust
fn main() { println!("hello # $ * _ ` backtick"); }
```

````
a fence with ``` inside
````

```
a very long line that keeps going and going and going well past the width of an A4 page so that it has to be wrapped somewhere
```

---

[bad scheme](javascript:alert(1)) and [relative](notes/other.md).

![missing](nope.png) ![remote](https://example.com/a.png)
"#;

fn doc(md: &str) -> Document {
    Document::from_markdown("Weekly planning: Kampala field visits", md)
}

#[test]
fn kitchen_sink_renders_to_every_format() {
    let d = doc(KITCHEN_SINK);
    let o = RenderOptions::default();

    let pdf = render::pdf(&d, &o).expect("pdf");
    assert!(pdf.starts_with(b"%PDF-"), "not a PDF");

    let png = render::png(&d, &o).expect("png");
    assert!(png.starts_with(&[0x89, b'P', b'N', b'G']), "not a PNG");

    let svg = render::svg(&d, &o).expect("svg");
    assert!(svg.contains("<svg"), "not an SVG");

    assert!(!render::svg_pages(&d, &o).expect("svg pages").is_empty());
    assert!(!render::png_pages(&d, &o).expect("png pages").is_empty());
}

#[test]
fn an_empty_note_still_renders_its_title() {
    let d = doc("");
    assert!(render::pdf(&d, &RenderOptions::default()).is_ok());
}

#[test]
fn text_that_looks_like_typst_code_is_inert() {
    // Any of these would fail the compile or read a file if interpreted.
    let hostile = r##"
#panic("boom")

#import "@preview/anything:0.1.0": *

#read("/etc/passwd")

$ x + y $ and #{ 1 + } and @label and <label> and \ and // not a comment

`#panic("in code")`

```
#panic("in a fence")
```

[#panic("link text")](https://example.com/"#panic("url"))

| #panic("a") | $b$ |
|---|---|
| #read("/etc/passwd") | = not a heading |

= not a heading
- not a list item?
+ nor this
/ term: nor this
"##;
    let d = Document::from_markdown("Title with \"quotes\", #hash and $dollar", hostile);
    render::pdf(&d, &RenderOptions::default()).expect("hostile text must compile as plain text");
}

#[test]
fn long_notes_paginate() {
    let md: String = (0..400)
        .map(|i| format!("Paragraph number {i} with a little text in it.\n\n"))
        .collect();
    let d = doc(&md);
    let o = RenderOptions::default();
    assert!(render::svg_pages(&d, &o).unwrap().len() > 1);
    // The single-image form is still one image.
    assert!(render::png(&d, &o).unwrap().starts_with(&[0x89, b'P']));
}

#[test]
fn images_inside_the_asset_root_are_embedded() {
    let root = tempfile::tempdir().unwrap();
    let img = root.path().join("pic.png");
    std::fs::write(&img, PNG_1X1).unwrap();
    let md = format!("![a picture]({})\n", img.display());
    let d = doc(&md);

    let with_root = RenderOptions {
        asset_root: Some(root.path().to_path_buf()),
        ..Default::default()
    };
    // SVG keeps images as <image> elements, so it is a direct oracle for
    // "was this embedded", which a PDF's byte size is not.
    let embedded = render::svg(&d, &with_root).expect("svg with image");
    let refused = render::svg(&d, &RenderOptions::default()).expect("svg without root");
    assert!(embedded.contains("<image"), "image should be embedded");
    assert!(!refused.contains("<image"), "no root means no image");
    // The PDF path takes the same route.
    assert!(render::pdf(&d, &with_root).is_ok());
}

#[test]
fn a_broken_image_file_degrades_instead_of_failing_the_export() {
    let root = tempfile::tempdir().unwrap();
    let img = root.path().join("bad.png");
    std::fs::write(&img, b"this is not a png").unwrap();
    let d = doc(&format!(
        "before\n\n![broken]({})\n\nafter\n",
        img.display()
    ));
    let o = RenderOptions {
        asset_root: Some(root.path().to_path_buf()),
        ..Default::default()
    };
    // The engine rejects undecodable images; the exporter must not lose the
    // whole note over one bad attachment.
    render::pdf(&d, &o).expect("a bad image must not sink the export");
}
