//! `hq suite`: the HQ Suite from the command line. Argument handling only; the work is in
//! `hq-suite-adapter` and the crates under `crates/suite/`. See `docs/suite/README.md`.

use std::path::PathBuf;

use anyhow::Result;
use clap::Subcommand;
use hq_core::config::HqConfig;

#[derive(Subcommand)]
pub enum SuiteCommand {
    /// Measure what shrinking a folder of documents would save, and what it would cost.
    ///
    /// Reads the folder and writes nothing except the report you ask for. The report holds
    /// numbers and format classes only (no file names, paths or content), so you can read it
    /// before anyone else does. See docs/suite/MEASURE.md.
    Measure {
        /// The folder to look at (its sub-folders are included; links are not followed)
        folder: PathBuf,
        /// A picture encoder to try, such as jpeg:75, png, webp-lossless or webp:80. Repeat for several.
        /// Without it, every encoder this build has is tried.
        #[arg(long = "encoder", value_name = "NAME[:QUALITY]")]
        encoders: Vec<String>,
        /// Scale pictures down to this long edge before encoding, as shrinking would (default 1600)
        #[arg(long, value_name = "PIXELS")]
        max_long_edge: Option<u32>,
        /// Do not decode pictures with more pixels than this (default 50 million)
        #[arg(long, value_name = "PIXELS")]
        max_pixels: Option<u64>,
        /// Look at no more than this many files (default 5000)
        #[arg(long, value_name = "N")]
        max_files: Option<usize>,
        /// Also write the JSON report to this new file (an existing file is never replaced)
        #[arg(long, value_name = "FILE")]
        out: Option<PathBuf>,
        /// Print the JSON report instead of the table
        #[arg(long)]
        json: bool,
    },

    /// Make a document smaller: keep its text and pictures as a note, set the original aside
    ///
    /// Shows what would be saved and what is left out, and moves nothing until you agree. The
    /// original goes to the vault's trash, where it stays for 30 days.
    Shrink {
        /// The file, in the vault (a vault-relative path, or one inside the vault folder)
        file: PathBuf,
        /// Agree to move the original to the trash without asking
        #[arg(long)]
        yes: bool,
    },
    /// Agree to a prepared shrink: move its original to the trash
    Confirm { job: String },
    /// Leave a prepared shrink undone: removes the new note, the original is not touched
    Abandon { job: String },
    /// Bring an original back from the trash and mark the note as having it again
    Undo { job: String },
    /// List shrink jobs and where each stands
    Jobs,
    /// Finish or resume jobs a crash or a restart interrupted
    Recover,
    /// Render a note as a Word file, pictures included
    Docx {
        /// The note, in the vault
        note: PathBuf,
        /// Where to write it (default: beside the note; an existing file is never replaced)
        #[arg(long, value_name = "FILE")]
        out: Option<PathBuf>,
    },
    /// Check that a Word file survives the suite: read it, render it, read the render, compare
    Check { file: PathBuf },
    /// What the suite holds, in words; with --tidy also removes what nothing needs
    ///
    /// Tidying removes pictures no note links to, marks notes whose original the trash has
    /// cleaned up, and, when `suite.budget_mb` is set and passed, clears temporary files and then
    /// originals older than `suite.cold_window_days`.
    Storage {
        /// Run the governor's pass now
        #[arg(long)]
        tidy: bool,
    },
}

#[cfg(feature = "suite")]
pub async fn run(config: &HqConfig, command: SuiteCommand) -> Result<()> {
    use hq_suite_adapter::measure::{Request, run as measure};
    use hq_suite_adapter::shrink;
    let vault = &config.vault_path;
    match command {
        SuiteCommand::Shrink { file, yes } => shrink::shrink(vault, &file, yes).await,
        SuiteCommand::Confirm { job } => shrink::confirm(vault, &job),
        SuiteCommand::Abandon { job } => shrink::abandon(vault, &job),
        SuiteCommand::Undo { job } => shrink::undo(vault, &job),
        SuiteCommand::Jobs => shrink::jobs(vault),
        SuiteCommand::Recover => shrink::recover(vault),
        SuiteCommand::Docx { note, out } => {
            let target = hq_suite_adapter::docs::render(vault, &note, out.as_deref())?;
            println!("Wrote {}", target.display());
            Ok(())
        }
        SuiteCommand::Check { file } => {
            let r = hq_suite_adapter::docs::check(&file).await?;
            print!("{}", hq_suite_adapter::docs::check_text(&r));
            if r.equal {
                Ok(())
            } else {
                anyhow::bail!("the render differs")
            }
        }
        SuiteCommand::Storage { tidy } => {
            use hq_suite_adapter::governor;
            let c = &config.suite;
            if tidy {
                let settings = governor::settings(c.budget_mb, c.cold_window_days);
                let text = governor::outcome_text(&governor::run(vault, &settings)?);
                println!(
                    "{}",
                    if text.is_empty() {
                        "Nothing to tidy."
                    } else {
                        &text
                    }
                );
            }
            println!("{}", governor::storage_text(vault, c.budget_mb));
            Ok(())
        }
        SuiteCommand::Measure {
            folder,
            encoders,
            max_long_edge,
            max_pixels,
            max_files,
            out,
            json,
        } => {
            measure(Request {
                folder,
                encoders,
                max_long_edge,
                max_pixels,
                max_files,
                out,
                json,
            })
            .await
        }
    }
}

#[cfg(not(feature = "suite"))]
pub async fn run(_config: &HqConfig, _command: SuiteCommand) -> Result<()> {
    anyhow::bail!("this build of hq has no HQ Suite; build with the `suite` feature")
}
