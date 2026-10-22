//! Reads a TLS ClientHello far enough to learn the server name, without answering it.
//!
//! rustls' `Acceptor` does the parsing (it handles a hello split across records and packets and
//! derives no keys); this module adds what a public listener needs on top: a hard cap on the
//! bytes it will read and a verbatim copy of everything read, so the device can be handed the
//! hello as if the edge had never looked at it.

use rustls::server::Acceptor;
use tokio::io::{AsyncRead, AsyncReadExt};

/// A TLS record holds at most 16 KiB, and real hellos are about 2 KiB. The cap is a little over
/// one full record so a hello split in two still fits and nothing larger ever does.
pub const MAX_HELLO_BYTES: usize = 18 * 1024;
const READ_CHUNK: usize = 2048;

/// What the edge learned from the first bytes of a connection.
#[derive(Debug)]
pub struct ClientHello {
    /// The server name, lowercased. `None` when the hello carried none.
    pub name: Option<String>,
    /// Everything read from the client so far, to be sent to the device first.
    pub bytes: Vec<u8>,
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum HelloError {
    #[error("the client closed before a full ClientHello")]
    Closed,
    #[error("not a TLS ClientHello")]
    Malformed,
    #[error("the ClientHello is larger than {MAX_HELLO_BYTES} bytes")]
    TooLarge,
    #[error("reading from the client failed")]
    Io,
}

/// Reads from `client` until a complete ClientHello has arrived, and returns its server name and
/// the bytes consumed. It never writes, so a refused client gets no answer. The caller applies
/// the time limit around the whole call: a per-read timeout would let a client send one byte at a
/// time forever.
pub async fn read_client_hello<R: AsyncRead + Unpin>(
    client: &mut R,
    max_bytes: usize,
) -> Result<ClientHello, HelloError> {
    let mut acceptor = Acceptor::default();
    let mut all: Vec<u8> = Vec::new();
    let mut chunk = [0u8; READ_CHUNK];
    loop {
        let room = max_bytes.saturating_sub(all.len());
        if room == 0 {
            return Err(HelloError::TooLarge);
        }
        let want = room.min(READ_CHUNK);
        let n = client
            .read(&mut chunk[..want])
            .await
            .map_err(|_| HelloError::Io)?;
        if n == 0 {
            return Err(HelloError::Closed);
        }
        all.extend_from_slice(&chunk[..n]);
        let mut unread: &[u8] = &chunk[..n];
        while !unread.is_empty() {
            let taken = acceptor
                .read_tls(&mut unread)
                .map_err(|_| HelloError::Malformed)?;
            if taken == 0 {
                return Err(HelloError::TooLarge);
            }
            match acceptor.accept() {
                Ok(Some(accepted)) => {
                    let name = accepted
                        .client_hello()
                        .server_name()
                        .map(str::to_ascii_lowercase);
                    return Ok(ClientHello { name, bytes: all });
                }
                Ok(None) => {}
                Err(_) => return Err(HelloError::Malformed),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{sync::Arc, time::Duration};
    use tokio::io::{AsyncWriteExt, duplex};

    /// A real ClientHello from rustls, for the name and ALPN given.
    pub(crate) fn hello_bytes(name: &str, alpn: &[&[u8]]) -> Vec<u8> {
        let roots = rustls::RootCertStore::empty();
        let mut config = rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(roots)
        .with_no_client_auth();
        config.alpn_protocols = alpn.iter().map(|a| a.to_vec()).collect();
        let name = rustls::pki_types::ServerName::try_from(name.to_owned()).unwrap();
        let mut conn = rustls::ClientConnection::new(Arc::new(config), name).unwrap();
        let mut out = Vec::new();
        conn.write_tls(&mut out).unwrap();
        out
    }

    async fn parse(bytes: &[u8]) -> Result<ClientHello, HelloError> {
        let (mut client, mut server) = duplex(64 * 1024);
        client.write_all(bytes).await.unwrap();
        drop(client);
        read_client_hello(&mut server, MAX_HELLO_BYTES).await
    }

    #[tokio::test]
    async fn the_server_name_is_read_and_every_byte_is_kept() {
        let bytes = hello_bytes("AbCd.Edge.Example.com", &[]);
        let hello = parse(&bytes).await.unwrap();
        assert_eq!(hello.name.as_deref(), Some("abcd.edge.example.com"));
        assert_eq!(
            hello.bytes, bytes,
            "the device must get the hello unchanged"
        );
    }

    #[tokio::test]
    async fn an_acme_alpn_hello_is_read_like_any_other() {
        let bytes = hello_bytes("dev.edge.example.com", &[b"acme-tls/1"]);
        let hello = parse(&bytes).await.unwrap();
        assert_eq!(hello.name.as_deref(), Some("dev.edge.example.com"));
        assert_eq!(hello.bytes, bytes);
    }

    #[tokio::test]
    async fn a_hello_written_in_small_pieces_is_assembled() {
        let bytes = hello_bytes("dev.edge.example.com", &[]);
        let (mut client, mut server) = duplex(64 * 1024);
        let sent = bytes.clone();
        tokio::spawn(async move {
            for piece in sent.chunks(7) {
                client.write_all(piece).await.unwrap();
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        });
        let hello = read_client_hello(&mut server, MAX_HELLO_BYTES)
            .await
            .unwrap();
        assert_eq!(hello.name.as_deref(), Some("dev.edge.example.com"));
        assert_eq!(hello.bytes, bytes);
    }

    #[tokio::test]
    async fn a_hello_split_across_two_records_is_assembled() {
        let one = hello_bytes("dev.edge.example.com", &[]);
        // Re-frame the handshake message as two records.
        let (header, body) = one.split_at(5);
        let cut = body.len() / 2;
        let mut split = Vec::new();
        for part in [&body[..cut], &body[cut..]] {
            split.extend_from_slice(&[header[0], header[1], header[2]]);
            split.extend_from_slice(&(part.len() as u16).to_be_bytes());
            split.extend_from_slice(part);
        }
        let hello = parse(&split).await.unwrap();
        assert_eq!(hello.name.as_deref(), Some("dev.edge.example.com"));
    }

    #[tokio::test]
    async fn a_hello_with_no_server_name_has_none() {
        // An IP address as the name makes rustls send no server_name extension.
        let bytes = hello_bytes("192.0.2.7", &[]);
        let hello = parse(&bytes).await.unwrap();
        assert_eq!(hello.name, None);
    }

    #[tokio::test]
    async fn plaintext_and_garbage_are_malformed() {
        assert_eq!(
            parse(b"GET / HTTP/1.1\r\nHost: dev.edge.example.com\r\n\r\n")
                .await
                .unwrap_err(),
            HelloError::Malformed
        );
        assert_eq!(parse(&[0xff; 64]).await.unwrap_err(), HelloError::Malformed);
        assert_eq!(parse(&[0u8; 64]).await.unwrap_err(), HelloError::Malformed);
    }

    #[tokio::test]
    async fn a_record_that_announces_far_more_than_the_cap_is_refused_without_waiting() {
        // 0x16 handshake, TLS 1.2, length 0xFFFF, then only a few bytes: it must not wait for
        // 64 KiB that will never come.
        let started = std::time::Instant::now();
        let err = parse(&[0x16, 0x03, 0x03, 0xff, 0xff, 0x01, 0x00])
            .await
            .unwrap_err();
        assert!(matches!(err, HelloError::Malformed | HelloError::TooLarge));
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[tokio::test]
    async fn the_byte_cap_stops_a_client_that_keeps_sending() {
        let (mut client, mut server) = duplex(1024 * 1024);
        // A valid record header for a 16000 byte handshake, then filler: never complete.
        let mut data = vec![0x16, 0x03, 0x03, 0x3e, 0x80, 0x01, 0x00, 0x3e, 0x7c];
        data.extend(std::iter::repeat_n(0u8, 40 * 1024));
        client.write_all(&data).await.unwrap();
        let err = read_client_hello(&mut server, 4096).await.unwrap_err();
        assert_eq!(err, HelloError::TooLarge, "the cap must bound what is read");
    }

    #[tokio::test]
    async fn a_close_before_the_hello_is_complete_is_reported() {
        let bytes = hello_bytes("dev.edge.example.com", &[]);
        assert_eq!(
            parse(&bytes[..bytes.len() / 2]).await.unwrap_err(),
            HelloError::Closed
        );
    }

    #[tokio::test]
    async fn no_input_whatever_panics() {
        // Every prefix of a real hello, and some mutations of it.
        let bytes = hello_bytes("dev.edge.example.com", &[b"h2"]);
        for end in 0..bytes.len() {
            let _ = parse(&bytes[..end]).await;
        }
        for at in (0..bytes.len()).step_by(3) {
            let mut mutated = bytes.clone();
            mutated[at] ^= 0xa5;
            let _ = parse(&mutated).await;
        }
    }
}
