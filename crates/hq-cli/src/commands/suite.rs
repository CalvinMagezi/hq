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
        /// A picture encoder to try, such as jpeg:75, png or webp-lossless. Repeat for several.
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
