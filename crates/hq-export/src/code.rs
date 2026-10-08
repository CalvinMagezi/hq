//! Pull the code blocks out of a note as files.

use crate::doc::{Block, Document, plain_text};
use crate::error::ExportError;
use crate::util::{slug, zip_files};

pub(crate) struct Packed {
    pub bytes: Vec<u8>,
    pub extension: String,
    pub mime: &'static str,
}

/// File extension for a fence language, `txt` when it is unknown or absent.
pub(crate) fn extension_for(lang: Option<&str>) -> &'static str {
    match lang.map(str::to_ascii_lowercase).as_deref() {
        Some("rust" | "rs") => "rs",
        Some("python" | "python3" | "py" | "ipython") => "py",
        Some("javascript" | "js" | "node") => "js",
        Some("typescript" | "ts") => "ts",
        Some("tsx") => "tsx",
        Some("jsx") => "jsx",
        Some("bash" | "sh" | "shell" | "zsh" | "console") => "sh",
        Some("json") => "json",
        Some("yaml" | "yml") => "yml",
        Some("toml") => "toml",
        Some("html") => "html",
        Some("css") => "css",
        Some("sql") => "sql",
        Some("go" | "golang") => "go",
        Some("java") => "java",
        Some("kotlin" | "kt") => "kt",
        Some("swift") => "swift",
        Some("c") => "c",
        Some("cpp" | "c++" | "cc") => "cpp",
        Some("csharp" | "cs" | "c#") => "cs",
        Some("ruby" | "rb") => "rb",
        Some("php") => "php",
        Some("lua") => "lua",
        Some("xml") => "xml",
        Some("markdown" | "md") => "md",
        Some("dockerfile") => "dockerfile",
        Some("diff" | "patch") => "diff",
        _ => "txt",
    }
}

struct Found {
    lang: Option<String>,
    text: String,
    heading: String,
}

fn collect(blocks: &[Block], heading: &mut String, out: &mut Vec<Found>) {
    for b in blocks {
        match b {
            Block::Heading { content, .. } => *heading = plain_text(content),
            Block::Code { lang, text } => out.push(Found {
                lang: lang.clone(),
                text: text.clone(),
                heading: heading.clone(),
            }),
            Block::Quote(body) | Block::Callout { body, .. } => collect(body, heading, out),
            Block::List { items, .. } => {
                for item in items {
                    collect(&item.blocks, heading, out);
                }
            }
            _ => {}
        }
    }
}

/// With `languages` non-empty, only blocks in those languages are kept. One
/// block is returned as a plain file, several as a zip.
pub(crate) fn extract(doc: &Document, languages: &[String]) -> Result<Packed, ExportError> {
    let mut found = Vec::new();
    collect(&doc.blocks, &mut String::new(), &mut found);
    if !languages.is_empty() {
        let wanted: Vec<String> = languages.iter().map(|l| l.to_ascii_lowercase()).collect();
        found.retain(|f| {
            f.lang
                .as_deref()
                .is_some_and(|l| wanted.contains(&l.to_ascii_lowercase()))
        });
    }
    if found.is_empty() {
        return Err(ExportError::Unsupported(if languages.is_empty() {
            "this note has no code blocks to export".into()
        } else {
            format!("this note has no code blocks in: {}", languages.join(", "))
        }));
    }
    let ext_of = |f: &Found| extension_for(f.lang.as_deref());
    if let [only] = found.as_slice() {
        return Ok(Packed {
            bytes: format!("{}\n", only.text).into_bytes(),
            extension: ext_of(only).to_owned(),
            mime: "text/plain",
        });
    }
    let files = found
        .iter()
        .enumerate()
        .map(|(i, f)| {
            let name = format!("{:02}-{}.{}", i + 1, slug(&f.heading, "snippet"), ext_of(f));
            (name, format!("{}\n", f.text).into_bytes())
        })
        .collect();
    Ok(Packed {
        bytes: zip_files(files)?,
        extension: "zip".into(),
        mime: "application/zip",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(md: &str) -> Document {
        Document::from_markdown("t", md)
    }

    #[test]
    fn no_code_is_an_error() {
        assert!(extract(&doc("text"), &[]).is_err());
    }

    #[test]
    fn a_single_block_is_a_plain_file_with_the_right_extension() {
        let p = extract(&doc("```rust\nfn main() {}\n```\n"), &[]).unwrap();
        assert_eq!(p.extension, "rs");
        assert_eq!(p.bytes, b"fn main() {}\n");
    }

    #[test]
    fn several_blocks_become_a_zip_named_after_their_headings() {
        let p = extract(
            &doc("## Setup\n\n```bash\nls\n```\n\n## Run it\n\n```python\nprint(1)\n```\n"),
            &[],
        )
        .unwrap();
        assert_eq!(p.extension, "zip");
        let mut z = zip::ZipArchive::new(std::io::Cursor::new(p.bytes)).unwrap();
        let names: Vec<String> = (0..z.len()).map(|i| z.by_index(i).unwrap().name().to_owned()).collect();
        assert_eq!(names, ["01-setup.sh", "02-run-it.py"]);
    }

    #[test]
    fn language_filter_keeps_only_matching_blocks() {
        let md = "```py\na\n```\n\n```sh\nb\n```\n";
        let p = extract(&doc(md), &["sh".into()]).unwrap();
        assert_eq!(p.extension, "sh");
        assert!(extract(&doc(md), &["go".into()]).is_err());
    }

    #[test]
    fn file_names_never_contain_path_separators() {
        let p = extract(
            &doc("## ../../etc/passwd\n\n```sh\na\n```\n\n```sh\nb\n```\n"),
            &[],
        )
        .unwrap();
        let mut z = zip::ZipArchive::new(std::io::Cursor::new(p.bytes)).unwrap();
        for i in 0..z.len() {
            let name = z.by_index(i).unwrap().name().to_owned();
            assert!(!name.contains('/') && !name.contains(".."), "{name}");
        }
    }
}
