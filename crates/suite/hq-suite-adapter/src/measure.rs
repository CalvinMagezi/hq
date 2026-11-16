//! `hq suite measure`: run the measurement and show the result.

use std::io::{IsTerminal, Write};
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use hq_suite_shrink::{Encoder, Settings, measure};

/// What the command line asked for.
#[derive(Debug, Clone)]
pub struct Request {
    pub folder: PathBuf,
    /// Encoders as typed (`jpeg:75`); empty means the ones this build has.
    pub encoders: Vec<String>,
    pub max_long_edge: Option<u32>,
    pub max_pixels: Option<u64>,
    pub max_files: Option<usize>,
    /// Write the JSON report here (refusing to replace a file) as well as showing the table.
    pub out: Option<PathBuf>,
    /// Print the JSON report instead of the table.
    pub json: bool,
}

/// The settings a request describes.
pub fn settings_for(req: &Request) -> Result<Settings> {
    let mut s = Settings::default();
    if !req.encoders.is_empty() {
        s.encoders = req
            .encoders
            .iter()
            .map(|e| Encoder::parse(e))
            .collect::<Result<_, _>>()?;
    }
    if let Some(n) = req.max_long_edge {
        if n == 0 {
            bail!("--max-long-edge must be more than zero");
        }
        s.max_long_edge = n;
    }
    if let Some(n) = req.max_pixels {
        s.max_pixels = n;
    }
    if let Some(n) = req.max_files {
        s.max_files = n;
    }
    Ok(s)
}

/// Runs the measurement. Progress goes to stderr when it is a terminal, the report to stdout.
pub async fn run(req: Request) -> Result<()> {
    let settings = settings_for(&req)?;
    // Refuse before the long run, not after it.
    if let Some(out) = &req.out
        && out.exists()
    {
        bail!("{} already exists; choose another name", out.display());
    }
    let tty = std::io::stderr().is_terminal();
    let report = measure(&req.folder, &settings, |done, total| {
        if tty && (done % 10 == 0 || done == total) {
            eprint!("\rmeasured {done} of {total}");
            let _ = std::io::stderr().flush();
        }
    })
    .await
    .with_context(|| "measuring failed")?;
    if tty {
        eprintln!();
    }
    let json = report.to_json();
    if let Some(out) = &req.out {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(out)
            .with_context(|| format!("cannot create {}", out.display()))?;
        file.write_all(json.as_bytes())?;
        file.write_all(b"\n")?;
    }
    if req.json {
        println!("{json}");
    } else {
        print!("{}", report.summary());
        for note in &report.notes {
            println!("note: {note}");
        }
        if let Some(out) = &req.out {
            println!("report written to {}", out.display());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(folder: PathBuf) -> Request {
        Request {
            folder,
            encoders: vec![],
            max_long_edge: None,
            max_pixels: None,
            max_files: None,
            out: None,
            json: false,
        }
    }

    #[test]
    fn settings_come_from_the_request() {
        let mut r = req(PathBuf::from("."));
        r.encoders = vec!["jpeg:60".into(), "png".into()];
        r.max_long_edge = Some(800);
        let s = settings_for(&r).unwrap();
        assert_eq!(s.encoders.len(), 2);
        assert_eq!(s.max_long_edge, 800);
        r.encoders = vec!["nope".into()];
        assert!(settings_for(&r).is_err());
        r.encoders = vec![];
        r.max_long_edge = Some(0);
        assert!(settings_for(&r).is_err());
    }

    #[tokio::test]
    async fn writes_the_report_once_and_refuses_to_replace_it() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.csv"), "a,b\n1,2\n").unwrap();
        let out = tempfile::tempdir().unwrap().keep().join("report.json");
        let mut r = req(dir.path().to_path_buf());
        r.out = Some(out.clone());
        r.json = true;
        run(r.clone()).await.unwrap();
        let written: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&out).unwrap()).unwrap();
        assert_eq!(written["schema"], 1);
        assert!(
            run(r).await.is_err(),
            "a second run must not overwrite the first report"
        );
        std::fs::remove_dir_all(out.parent().unwrap()).unwrap();
    }
}
