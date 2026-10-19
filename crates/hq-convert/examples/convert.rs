//! Convert files to Markdown with the import pipeline and show the report.
//!
//! cargo run -p hq-convert --example convert -- [--md] [--json] <file>...
//!
//! Without flags it prints one summary line per file. `--md` prints the
//! Markdown, `--json` the report.

use hq_convert::ingest::{ConvertOptions, convert_file};

#[tokio::main]
async fn main() {
    let mut md = false;
    let mut json = false;
    let mut files = Vec::new();
    for a in std::env::args().skip(1) {
        match a.as_str() {
            "--md" => md = true,
            "--json" => json = true,
            _ => files.push(a),
        }
    }
    if files.is_empty() {
        eprintln!("usage: convert [--md] [--json] <file>...");
        std::process::exit(2);
    }
    let opts = ConvertOptions::default();
    for f in files {
        let name = std::path::Path::new(&f)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| f.clone());
        match convert_file(std::path::Path::new(&f), &opts).await {
            Ok(c) => {
                let r = &c.report;
                if json {
                    println!("{}", serde_json::to_string_pretty(r).unwrap());
                } else {
                    println!(
                        "OK  {name:34} {:5} {:?} {:20} chars={:6} h={} t={} li={} ln={} warn={}",
                        r.source_format,
                        r.outcome,
                        r.engine,
                        r.stats.chars,
                        r.stats.headings,
                        r.stats.tables,
                        r.stats.list_items,
                        r.stats.links,
                        r.warnings.len()
                    );
                }
                if md {
                    println!("{}", c.markdown);
                }
            }
            Err(e) => println!("ERR {name:34} {e}"),
        }
    }
}
