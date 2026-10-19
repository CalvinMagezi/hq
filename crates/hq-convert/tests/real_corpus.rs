//! A set of real public files through the import pipeline. Opt in:
//!
//!   ./scripts/fetch-conversion-corpus.sh
//!   cargo test -p hq-convert --test real_corpus -- --ignored --nocapture
//!
//! Set `HQ_CONVERT_CORPUS` to read another folder. The test prints one line per
//! file and fails only if a file panics, takes too long, or too many fail.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use hq_convert::ingest::{ConvertOptions, Outcome, convert_file};

/// Files the corpus holds on purpose to be refused: encrypted, damaged or empty of text.
const EXPECTED_REFUSALS: &[&str] = &[
    "testODTEncrypted.odt",
    "testPPT_masterText.pptx",
    "testPDF_Version.8.x.pdf",
];

fn corpus_dir() -> PathBuf {
    std::env::var_os("HQ_CONVERT_CORPUS")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/conversion-corpus")
        })
}

#[tokio::test]
#[ignore = "needs the corpus from scripts/fetch-conversion-corpus.sh (network), plus LibreOffice and tesseract for .doc/.ppt and images"]
async fn real_files_convert_or_are_refused_for_a_stated_reason() {
    let dir = corpus_dir();
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| {
            panic!(
                "{}: {e}; run scripts/fetch-conversion-corpus.sh",
                dir.display()
            )
        })
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_file())
        .collect();
    files.sort();
    assert!(!files.is_empty(), "no files in {}", dir.display());

    let (mut ok, mut degraded, mut refused, mut surprises) = (0, 0, 0, Vec::new());
    for f in &files {
        let name = f.file_name().unwrap().to_string_lossy().into_owned();
        let started = Instant::now();
        let result = convert_file(f, &ConvertOptions::default()).await;
        let took = started.elapsed();
        assert!(took < Duration::from_secs(120), "{name} took {took:?}");
        match result {
            Ok(c) => {
                match c.report.outcome {
                    Outcome::Ok => ok += 1,
                    Outcome::Degraded => degraded += 1,
                }
                println!(
                    "ok       {name:44} {:22} {:>7} chars",
                    c.report.engine, c.report.stats.chars
                );
            }
            Err(e) => {
                refused += 1;
                println!("refused  {name:44} {e}");
                if !EXPECTED_REFUSALS.contains(&name.as_str()) {
                    surprises.push(format!("{name}: {e}"));
                }
            }
        }
    }
    println!(
        "{ok} ok, {degraded} degraded, {refused} refused of {}",
        files.len()
    );
    assert!(
        surprises.len() * 10 <= files.len(),
        "more than a tenth of the files failed unexpectedly:\n{}",
        surprises.join("\n")
    );
}
