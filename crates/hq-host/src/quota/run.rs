//! The two outside calls readers make, each bounded by a timeout.

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde_json::Value;

/// How often a running child is checked for having exited.
const POLL: Duration = Duration::from_millis(25);

/// What a finished process printed, each stream cut at the cap.
pub(crate) struct Captured {
    pub stdout: String,
    pub stderr: String,
}

enum Stream {
    Out(Vec<u8>),
    Err(Vec<u8>),
}

fn pump<R: Read + Send + 'static>(
    pipe: R,
    cap: usize,
    tx: mpsc::Sender<Stream>,
    wrap: fn(Vec<u8>) -> Stream,
) {
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        // One byte past the cap tells "exactly full" from "too long".
        let _ = pipe.take(cap as u64 + 1).read_to_end(&mut buf);
        let _ = tx.send(wrap(buf));
    });
}

/// Runs `argv` with no shell and no stdin, killing it at `timeout`. A stream longer than `cap`
/// is an error, not a truncation, so a half-read answer is never parsed.
pub(crate) fn run_capture(
    argv: &[String],
    cwd: Option<&Path>,
    timeout: Duration,
    cap: usize,
) -> Result<Captured, String> {
    let (program, args) = argv.split_first().ok_or("empty command")?;
    let mut cmd = Command::new(program);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    let mut child = cmd.spawn().map_err(|e| format!("cannot start: {e}"))?;
    let (tx, rx) = mpsc::channel();
    if let Some(out) = child.stdout.take() {
        pump(out, cap, tx.clone(), Stream::Out);
    }
    if let Some(err) = child.stderr.take() {
        pump(err, cap, tx.clone(), Stream::Err);
    }
    drop(tx);

    let deadline = Instant::now() + timeout;
    let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
    for _ in 0..2 {
        let left = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(left) {
            Ok(Stream::Out(b)) => stdout = b,
            Ok(Stream::Err(b)) => stderr = b,
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("timed out after {}s", timeout.as_secs()));
            }
        }
    }
    // Both pipes are closed, so the process is ending; give it the rest of the time to do so.
    while child.try_wait().map_err(|e| e.to_string())?.is_none() {
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("timed out after {}s", timeout.as_secs()));
        }
        std::thread::sleep(POLL);
    }
    if stdout.len() > cap || stderr.len() > cap {
        return Err(format!("output longer than {cap} bytes"));
    }
    Ok(Captured {
        stdout: String::from_utf8_lossy(&stdout).into_owned(),
        stderr: String::from_utf8_lossy(&stderr).into_owned(),
    })
}

/// GET `url` and read the body as JSON. The error text never contains a header value.
pub(crate) fn http_get_json(
    url: &str,
    headers: &[(&str, String)],
    timeout: Duration,
) -> Result<Value, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(timeout)
        .build()
        .map_err(|_| "cannot build the HTTP client".to_string())?;
    let mut req = client.get(url).header("Accept", "application/json");
    for (name, value) in headers {
        req = req.header(*name, value);
    }
    let resp = req.send().map_err(|e| {
        if e.is_timeout() {
            "request timed out".to_string()
        } else {
            "request failed".to_string()
        }
    })?;
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("HTTP {}", status.as_u16()));
    }
    resp.json::<Value>()
        .map_err(|_| "answer was not JSON".to_string())
}
