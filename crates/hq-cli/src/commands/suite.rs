//! `hq suite`: the HQ Suite from the command line. Argument handling only; the work is in
//! `hq-suite-adapter` and the crates under `crates/suite/`. See `docs/suite/README.md`.

use std::path::PathBuf;

use anyhow::Result;
use clap::Subcommand;

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
}

#[cfg(feature = "suite")]
pub async fn run(command: SuiteCommand) -> Result<()> {
    use hq_suite_adapter::measure::{Request, run as measure};
    match command {
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
pub async fn run(_command: SuiteCommand) -> Result<()> {
    anyhow::bail!("this build of hq has no HQ Suite; build with the `suite` feature")
}
