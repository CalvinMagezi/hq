//! Render a Markdown file: `cargo run -p hq-export --example export -- note.md out.pdf`
//!
//! The format comes from the output extension (pdf, png, svg). Images resolve
//! below the input file's folder. Meant for looking at output while working on
//! the renderer; `hq vault export` is the real entry point.

use hq_export::{Document, RenderOptions, render};
use std::path::PathBuf;

fn main() {
    let mut args = std::env::args().skip(1);
    let (Some(input), Some(output)) = (args.next(), args.next()) else {
        eprintln!("usage: export <input.md> <output.pdf|png|svg>");
        std::process::exit(2);
    };
    let input = PathBuf::from(input);
    let markdown = std::fs::read_to_string(&input).expect("read input");
    let title = input
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let doc = Document::from_markdown(title, &markdown);
    let opts = RenderOptions {
        asset_root: input.parent().map(PathBuf::from),
        ..Default::default()
    };
    let output = PathBuf::from(output);
    let bytes: Vec<u8> = match output.extension().and_then(|e| e.to_str()) {
        Some("pdf") => render::pdf(&doc, &opts),
        Some("png") => render::png(&doc, &opts),
        Some("svg") => render::svg(&doc, &opts).map(String::into_bytes),
        other => {
            eprintln!("unsupported output extension: {other:?}");
            std::process::exit(2);
        }
    }
    .expect("render");
    std::fs::write(&output, bytes).expect("write output");
}
