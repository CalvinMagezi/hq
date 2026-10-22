//! Copies bytes both ways between a client and a device, under an idle timeout, an optional shared
//! rate limit and a cancel signal. Generic over the streams so it is tested without a network.

use std::{
    future::Future,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    time::Instant,
};

use crate::limits::TokenBucket;

const CHUNK: usize = 16 * 1024;

pub struct PipeLimits {
    /// Close when no bytes have moved in either direction for this long.
    pub idle: Duration,
    /// A budget shared with the other connections of the same device. `None` is unlimited.
    pub bucket: Option<Arc<TokenBucket>>,
}

/// Why a pipe ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PipeEnd {
    /// Both sides finished or one failed.
    Closed,
    /// No bytes moved for the idle timeout.
    Idle,
    /// The cancel future completed: the device was revoked or the edge is stopping.
    Cancelled,
}

/// Copies `client` to `device` and back until both directions end, the connection goes idle, or
/// `cancel` completes. Dropping the streams afterwards closes them. A side that reaches end of
/// input is shut down (its peer sees end of input) while the other direction carries on.
pub async fn pipe<CR, CW, DR, DW>(
    (client_read, client_write): (CR, CW),
    (device_read, device_write): (DR, DW),
    limits: PipeLimits,
    cancel: impl Future<Output = ()>,
) -> PipeEnd
where
    CR: AsyncRead + Unpin,
    CW: AsyncWrite + Unpin,
    DR: AsyncRead + Unpin,
    DW: AsyncWrite + Unpin,
{
    let started = Instant::now();
    let last_activity = AtomicU64::new(0);
    let bucket = limits.bucket.as_deref();
    let to_device = copy(client_read, device_write, bucket, started, &last_activity);
    let to_client = copy(device_read, client_write, bucket, started, &last_activity);
    tokio::select! {
        _ = async { tokio::join!(to_device, to_client) } => PipeEnd::Closed,
        () = idle_watch(started, &last_activity, limits.idle) => PipeEnd::Idle,
        () = cancel => PipeEnd::Cancelled,
    }
}

async fn copy<R: AsyncRead + Unpin, W: AsyncWrite + Unpin>(
    mut from: R,
    mut to: W,
    bucket: Option<&TokenBucket>,
    started: Instant,
    last_activity: &AtomicU64,
) {
    let chunk = bucket.map_or(CHUNK, |b| b.burst().min(CHUNK));
    let mut buf = vec![0u8; chunk];
    loop {
        let n = match from.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        if let Some(bucket) = bucket {
            bucket.take(n).await;
        }
        if to.write_all(&buf[..n]).await.is_err() {
            return;
        }
        touch(started, last_activity);
    }
    let _ = to.shutdown().await;
}

fn touch(started: Instant, last_activity: &AtomicU64) {
    let ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    last_activity.store(ms, Ordering::Relaxed);
}

/// Completes once `idle` has passed with no change to `last_activity`.
async fn idle_watch(started: Instant, last_activity: &AtomicU64, idle: Duration) {
    loop {
        let seen = last_activity.load(Ordering::Relaxed);
        tokio::time::sleep_until(started + Duration::from_millis(seen) + idle).await;
        if last_activity.load(Ordering::Relaxed) == seen {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{duplex, split};

    const IDLE: Duration = Duration::from_secs(30);

    fn limits(bucket: Option<Arc<TokenBucket>>) -> PipeLimits {
        PipeLimits { idle: IDLE, bucket }
    }

    /// Wires two duplex pairs through `pipe`: returns the client's and the device's ends.
    fn wired(
        limits: PipeLimits,
        cancel: impl Future<Output = ()> + Send + 'static,
    ) -> (
        tokio::io::DuplexStream,
        tokio::io::DuplexStream,
        tokio::task::JoinHandle<PipeEnd>,
    ) {
        let (client_end, client_inner) = duplex(64 * 1024);
        let (device_end, device_inner) = duplex(64 * 1024);
        let handle = tokio::spawn(async move {
            pipe(split(client_inner), split(device_inner), limits, cancel).await
        });
        (client_end, device_end, handle)
    }

    #[tokio::test]
    async fn bytes_flow_both_ways_and_a_half_close_passes_through() {
        let (mut client, mut device, handle) = wired(limits(None), std::future::pending());
        client.write_all(b"ping").await.unwrap();
        let mut buf = [0u8; 4];
        device.read_exact(&mut buf).await.unwrap();
        assert_eq!(&buf, b"ping");
        // The client finishes sending; the device can still answer.
        client.shutdown().await.unwrap();
        let mut rest = Vec::new();
        device.read_to_end(&mut rest).await.unwrap();
        assert!(rest.is_empty());
        device.write_all(b"pong").await.unwrap();
        device.shutdown().await.unwrap();
        let mut answer = Vec::new();
        client.read_to_end(&mut answer).await.unwrap();
        assert_eq!(answer, b"pong");
        assert_eq!(handle.await.unwrap(), PipeEnd::Closed);
    }

    #[tokio::test(start_paused = true)]
    async fn silence_closes_the_pipe_and_traffic_keeps_it_open() {
        let (mut client, mut device, handle) = wired(limits(None), std::future::pending());
        for _ in 0..4 {
            tokio::time::sleep(IDLE * 3 / 4).await;
            client.write_all(b"x").await.unwrap();
            let mut b = [0u8; 1];
            device.read_exact(&mut b).await.unwrap();
        }
        assert!(!handle.is_finished(), "traffic must keep the pipe alive");
        tokio::time::sleep(IDLE + Duration::from_secs(1)).await;
        assert_eq!(handle.await.unwrap(), PipeEnd::Idle);
    }

    #[tokio::test]
    async fn cancel_ends_the_pipe_and_closes_both_sides() {
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        let (mut client, mut device, handle) = wired(limits(None), async {
            let _ = rx.await;
        });
        tx.send(()).unwrap();
        assert_eq!(handle.await.unwrap(), PipeEnd::Cancelled);
        let mut b = Vec::new();
        assert_eq!(client.read_to_end(&mut b).await.unwrap(), 0);
        assert_eq!(device.read_to_end(&mut b).await.unwrap(), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn the_shared_bucket_slows_the_copy() {
        // 10 KB/s with a 1 KB burst: 21 KB takes about two seconds, not an instant.
        let bucket = Arc::new(TokenBucket::new(10_000, 1_000));
        let (mut client, mut device, _handle) = wired(limits(Some(bucket)), std::future::pending());
        let data = vec![7u8; 21_000];
        let sender = tokio::spawn(async move {
            client.write_all(&data).await.unwrap();
            client.shutdown().await.unwrap();
            client
        });
        let started = Instant::now();
        let mut got = Vec::new();
        device.read_to_end(&mut got).await.unwrap();
        assert_eq!(got.len(), 21_000);
        assert!(
            started.elapsed() >= Duration::from_millis(1900),
            "copied 21 KB in {:?}",
            started.elapsed()
        );
        let _client = sender.await.unwrap();
    }
}
