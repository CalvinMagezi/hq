//! Captures everything `tracing` emits on the current thread, at every level.
//! Pair with `#[tokio::test]` (single-threaded runtime) so awaits stay on the thread.

use std::io;
use std::sync::{Arc, Mutex};
use tracing::subscriber::DefaultGuard;
use tracing_subscriber::fmt::MakeWriter;

#[derive(Clone, Default)]
pub struct LogCapture(Arc<Mutex<Vec<u8>>>);

impl LogCapture {
    pub fn contents(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
    }
}

pub struct CaptureWriter(Arc<Mutex<Vec<u8>>>);

impl io::Write for CaptureWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for LogCapture {
    type Writer = CaptureWriter;
    fn make_writer(&'a self) -> Self::Writer {
        CaptureWriter(self.0.clone())
    }
}

/// Keep the guard alive for as long as logs should be captured.
pub fn capture_logs() -> (LogCapture, DefaultGuard) {
    let capture = LogCapture::default();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .with_writer(capture.clone())
        .finish();
    (capture, tracing::subscriber::set_default(subscriber))
}
