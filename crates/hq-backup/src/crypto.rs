//! The container: a short clear header, then the payload sealed in 64 KiB chunks.
//!
//! Every chunk is ChaCha20-Poly1305. Its nonce is a random per-file base with the chunk counter
//! mixed in, and its associated data is the whole header plus a flag saying whether this is the
//! last chunk. So a changed header, a reordered or dropped chunk, a duplicated chunk, and a file
//! cut short at a chunk boundary all fail to open, and nothing past the first failure is used.
//!
//! The header holds no secret: a format version, how the key was made (a key file, or Argon2id
//! with its cost and salt) and the nonce base.

use std::cell::Cell;
use std::io::{self, Read, Write};
use std::rc::Rc;

use ring::aead::{Aad, CHACHA20_POLY1305, LessSafeKey, Nonce, UnboundKey};
use ring::rand::{SecureRandom, SystemRandom};

use crate::error::{BackupError, Result, corrupt, refused};
use crate::key::{Argon2Cost, KEY_LEN};

const MAGIC: [u8; 8] = *b"HQBACKUP";
pub(crate) const FORMAT: u8 = 1;
const KDF_NONE: u8 = 0;
const KDF_ARGON2ID: u8 = 1;
/// Plaintext bytes per chunk. Fixed, so a header cannot ask a reader for a huge buffer.
pub(crate) const CHUNK: usize = 64 * 1024;
const TAG: usize = 16;
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 12;
pub(crate) const HEADER_LEN: usize = 8 + 1 + 1 + 4 + 4 + 4 + SALT_LEN + NONCE_LEN + 4;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum KdfParams {
    /// The key is the bytes of a key file.
    None,
    Argon2id {
        cost: Argon2Cost,
        salt: [u8; SALT_LEN],
    },
}

#[derive(Debug, Clone)]
pub(crate) struct Header {
    pub kdf: KdfParams,
    nonce_base: [u8; NONCE_LEN],
}

pub(crate) fn random<const N: usize>() -> Result<[u8; N]> {
    let mut out = [0u8; N];
    SystemRandom::new()
        .fill(&mut out)
        .map_err(|_| BackupError::Refused("the system has no source of randomness".into()))?;
    Ok(out)
}

impl Header {
    pub fn new(passphrase: bool, cost: Argon2Cost) -> Result<Self> {
        let kdf = if passphrase {
            cost.check()?;
            KdfParams::Argon2id {
                cost,
                salt: random()?,
            }
        } else {
            KdfParams::None
        };
        Ok(Self {
            kdf,
            nonce_base: random()?,
        })
    }

    pub fn to_bytes(&self) -> [u8; HEADER_LEN] {
        let mut out = [0u8; HEADER_LEN];
        let mut at = 0;
        let mut put = |bytes: &[u8]| {
            out[at..at + bytes.len()].copy_from_slice(bytes);
            at += bytes.len();
        };
        put(&MAGIC);
        put(&[FORMAT]);
        let (kind, cost, salt) = match &self.kdf {
            KdfParams::None => (
                KDF_NONE,
                Argon2Cost {
                    memory_kib: 0,
                    passes: 0,
                    lanes: 0,
                },
                [0u8; SALT_LEN],
            ),
            KdfParams::Argon2id { cost, salt } => (KDF_ARGON2ID, *cost, *salt),
        };
        put(&[kind]);
        put(&cost.memory_kib.to_le_bytes());
        put(&cost.passes.to_le_bytes());
        put(&cost.lanes.to_le_bytes());
        put(&salt);
        put(&self.nonce_base);
        put(&(CHUNK as u32).to_le_bytes());
        debug_assert_eq!(at, HEADER_LEN);
        out
    }

    /// Reads and checks a header. Anything that is not exactly what this build writes is refused
    /// before a key is derived or a byte of payload is read.
    pub fn read(input: &mut impl Read) -> Result<(Self, [u8; HEADER_LEN])> {
        let mut raw = [0u8; HEADER_LEN];
        input.read_exact(&mut raw).map_err(|e| {
            if e.kind() == io::ErrorKind::UnexpectedEof {
                BackupError::Refused("this file is too short to be an HQ backup".into())
            } else {
                BackupError::Io(e)
            }
        })?;
        if raw[..8] != MAGIC {
            return refused("this is not an HQ backup file");
        }
        match raw[8] {
            FORMAT => {}
            newer if newer > FORMAT => {
                return Err(BackupError::Newer(format!(
                    "this backup uses format {newer}, which is newer than this HQ understands; \
                     update HQ to open it"
                )));
            }
            other => return corrupt(format!("unknown backup format {other}")),
        }
        let u32_at = |at: usize| u32::from_le_bytes(raw[at..at + 4].try_into().expect("4 bytes"));
        let kdf = match raw[9] {
            KDF_NONE => KdfParams::None,
            KDF_ARGON2ID => {
                let cost = Argon2Cost {
                    memory_kib: u32_at(10),
                    passes: u32_at(14),
                    lanes: u32_at(18),
                };
                cost.check()?;
                KdfParams::Argon2id {
                    cost,
                    salt: raw[22..22 + SALT_LEN].try_into().expect("salt"),
                }
            }
            other => return corrupt(format!("unknown key type {other}")),
        };
        if u32_at(HEADER_LEN - 4) as usize != CHUNK {
            return corrupt("unexpected chunk size");
        }
        let nonce_base = raw[22 + SALT_LEN..22 + SALT_LEN + NONCE_LEN]
            .try_into()
            .expect("nonce");
        Ok((Self { kdf, nonce_base }, raw))
    }
}

fn sealing_key(key: &[u8; KEY_LEN]) -> LessSafeKey {
    LessSafeKey::new(UnboundKey::new(&CHACHA20_POLY1305, key).expect("a 32 byte key"))
}

fn chunk_nonce(base: &[u8; NONCE_LEN], counter: u64) -> Nonce {
    let mut bytes = *base;
    for (slot, byte) in bytes[NONCE_LEN - 8..].iter_mut().zip(counter.to_be_bytes()) {
        *slot ^= byte;
    }
    Nonce::assume_unique_for_key(bytes)
}

fn chunk_aad(header: &[u8; HEADER_LEN], last: bool) -> Aad<Vec<u8>> {
    let mut aad = Vec::with_capacity(HEADER_LEN + 1);
    aad.extend_from_slice(header);
    aad.push(u8::from(last));
    Aad::from(aad)
}

/// Why a reader stopped. It is also recorded where the caller can read it, because the layers
/// above (gzip, tar) are free to re-wrap an `io::Error` and a wrong key must not look like damage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StreamFailure {
    /// The first chunk did not open: almost always the wrong key.
    WrongKey,
    /// A later chunk did not open.
    Altered,
    /// The stream ended before its last chunk, or its last chunk was cut off.
    Truncated,
}

impl std::fmt::Display for StreamFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::WrongKey => "the first part of the backup did not open",
            Self::Altered => "a part of the backup was changed or is damaged",
            Self::Truncated => "the backup ends before it should",
        })
    }
}

impl std::error::Error for StreamFailure {}

/// Turns what a pipeline built on [`OpenReader`] returned into a [`BackupError`]. `failure` is
/// what the reader recorded, which wins over whatever the layers above made of the error.
pub(crate) fn classify(error: io::Error, failure: Option<StreamFailure>) -> BackupError {
    match failure {
        Some(StreamFailure::WrongKey) => BackupError::Decrypt,
        Some(other) => BackupError::Corrupt(other.to_string()),
        None if error.kind() == io::ErrorKind::InvalidData
            || error.kind() == io::ErrorKind::UnexpectedEof =>
        {
            BackupError::Corrupt(error.to_string())
        }
        None => BackupError::Io(error),
    }
}

/// Seals what is written to it, a chunk at a time, into `out`. Call [`SealWriter::finish`] to
/// write the last chunk; dropping it without that leaves a file that will not open.
pub(crate) struct SealWriter<W: Write> {
    out: W,
    key: LessSafeKey,
    header: [u8; HEADER_LEN],
    base: [u8; NONCE_LEN],
    counter: u64,
    buf: Vec<u8>,
}

impl<W: Write> SealWriter<W> {
    pub fn new(mut out: W, key: &[u8; KEY_LEN], header: &Header) -> io::Result<Self> {
        let raw = header.to_bytes();
        out.write_all(&raw)?;
        Ok(Self {
            out,
            key: sealing_key(key),
            header: raw,
            base: header.nonce_base,
            counter: 0,
            buf: Vec::with_capacity(CHUNK + TAG),
        })
    }

    fn seal(&mut self, len: usize, last: bool) -> io::Result<()> {
        let mut chunk: Vec<u8> = self.buf.drain(..len).collect();
        self.key
            .seal_in_place_append_tag(
                chunk_nonce(&self.base, self.counter),
                chunk_aad(&self.header, last),
                &mut chunk,
            )
            .map_err(|_| io::Error::other("could not seal a chunk"))?;
        self.counter += 1;
        self.out.write_all(&chunk)
    }

    pub fn finish(mut self) -> io::Result<W> {
        let rest = self.buf.len();
        self.seal(rest, true)?;
        self.out.flush()?;
        Ok(self.out)
    }
}

impl<W: Write> Write for SealWriter<W> {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.buf.extend_from_slice(data);
        // Keep the last bytes back: only `finish` knows which chunk is the last one.
        while self.buf.len() > CHUNK {
            self.seal(CHUNK, false)?;
        }
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Opens a sealed stream chunk by chunk. A chunk is handed on only after it has been checked.
pub(crate) struct OpenReader<R: Read> {
    input: R,
    key: LessSafeKey,
    header: [u8; HEADER_LEN],
    base: [u8; NONCE_LEN],
    counter: u64,
    pending: Vec<u8>,
    plain: Vec<u8>,
    at: usize,
    done: bool,
    failure: Rc<Cell<Option<StreamFailure>>>,
}

impl<R: Read> OpenReader<R> {
    pub fn new(input: R, key: &[u8; KEY_LEN], header: &Header, raw: [u8; HEADER_LEN]) -> Self {
        Self {
            input,
            key: sealing_key(key),
            header: raw,
            base: header.nonce_base,
            counter: 0,
            pending: Vec::with_capacity(CHUNK + TAG + 1),
            plain: Vec::new(),
            at: 0,
            done: false,
            failure: Rc::new(Cell::new(None)),
        }
    }

    /// Where this reader records why it stopped, for [`classify`].
    pub fn failure(&self) -> Rc<Cell<Option<StreamFailure>>> {
        Rc::clone(&self.failure)
    }

    fn fail(&self, kind: StreamFailure) -> io::Error {
        self.failure.set(Some(kind));
        io::Error::new(io::ErrorKind::InvalidData, kind)
    }

    /// Reads one more chunk into `plain`. One byte past a full chunk is read ahead: if it is
    /// there, this chunk is not the last; if the input ends first, it is.
    fn next_chunk(&mut self) -> io::Result<()> {
        let want = CHUNK + TAG + 1;
        let mut tmp = [0u8; 8192];
        while self.pending.len() < want {
            let room = (want - self.pending.len()).min(tmp.len());
            let n = self.input.read(&mut tmp[..room])?;
            if n == 0 {
                break;
            }
            self.pending.extend_from_slice(&tmp[..n]);
        }
        if self.pending.len() < TAG {
            return Err(self.fail(if self.counter == 0 {
                StreamFailure::WrongKey
            } else {
                StreamFailure::Truncated
            }));
        }
        let last = self.pending.len() <= CHUNK + TAG;
        let take = if last {
            self.pending.len()
        } else {
            CHUNK + TAG
        };
        let mut chunk: Vec<u8> = self.pending.drain(..take).collect();
        let opened = self
            .key
            .open_in_place(
                chunk_nonce(&self.base, self.counter),
                chunk_aad(&self.header, last),
                &mut chunk,
            )
            .map(|plain| plain.len());
        match opened {
            Ok(len) => {
                chunk.truncate(len);
                self.plain = chunk;
                self.at = 0;
                self.counter += 1;
                self.done = last;
                Ok(())
            }
            Err(_) if self.counter == 0 => Err(self.fail(StreamFailure::WrongKey)),
            Err(_) if !last => Err(self.fail(StreamFailure::Altered)),
            // The last chunk failed to open: either it was changed, or the file was cut at a
            // chunk boundary so that a chunk sealed as "not last" is now at the end.
            Err(_) => Err(self.fail(StreamFailure::Truncated)),
        }
    }
}

impl<R: Read> Read for OpenReader<R> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        while self.at == self.plain.len() {
            if self.done {
                return Ok(0);
            }
            self.next_chunk()?;
        }
        let n = out.len().min(self.plain.len() - self.at);
        out[..n].copy_from_slice(&self.plain[self.at..self.at + n]);
        self.at += n;
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: [u8; KEY_LEN] = [7; KEY_LEN];

    fn seal(payload: &[u8]) -> (Vec<u8>, Header) {
        let header = Header::new(false, Argon2Cost::default()).unwrap();
        let mut writer = SealWriter::new(Vec::new(), &KEY, &header).unwrap();
        writer.write_all(payload).unwrap();
        (writer.finish().unwrap(), header)
    }

    fn open(sealed: &[u8], key: &[u8; KEY_LEN]) -> std::result::Result<Vec<u8>, BackupError> {
        let mut input = sealed;
        let (header, raw) = Header::read(&mut input)?;
        let mut reader = OpenReader::new(input, key, &header, raw);
        let failure = reader.failure();
        let mut out = Vec::new();
        reader
            .read_to_end(&mut out)
            .map_err(|e| classify(e, failure.get()))?;
        Ok(out)
    }

    fn payload(len: usize) -> Vec<u8> {
        (0..len).map(|i| (i * 31 % 251) as u8).collect()
    }

    #[test]
    fn payloads_round_trip_at_every_chunk_boundary() {
        for len in [
            0,
            1,
            CHUNK - 1,
            CHUNK,
            CHUNK + 1,
            2 * CHUNK,
            2 * CHUNK + 5,
            5 * CHUNK,
        ] {
            let data = payload(len);
            let (sealed, _) = seal(&data);
            assert_eq!(open(&sealed, &KEY).unwrap(), data, "length {len}");
        }
    }

    #[test]
    fn a_wrong_key_is_reported_as_one() {
        let (sealed, _) = seal(&payload(3 * CHUNK));
        assert!(matches!(
            open(&sealed, &[8; KEY_LEN]),
            Err(BackupError::Decrypt)
        ));
    }

    #[test]
    fn a_changed_byte_anywhere_fails_and_nothing_after_it_is_returned() {
        let data = payload(3 * CHUNK + 10);
        let (sealed, _) = seal(&data);
        for at in [
            0,
            8,
            9,
            30,
            HEADER_LEN,
            HEADER_LEN + 5,
            sealed.len() / 2,
            sealed.len() - 1,
        ] {
            let mut bad = sealed.clone();
            bad[at] ^= 0x01;
            assert!(open(&bad, &KEY).is_err(), "byte {at}");
        }
    }

    #[test]
    fn a_file_cut_short_never_opens_even_at_a_chunk_boundary() {
        let data = payload(3 * CHUNK + 10);
        let (sealed, _) = seal(&data);
        let boundary = HEADER_LEN + 2 * (CHUNK + TAG);
        for cut in [
            0,
            HEADER_LEN - 1,
            HEADER_LEN,
            HEADER_LEN + 10,
            boundary,
            boundary + 1,
            sealed.len() - 1,
        ] {
            assert!(open(&sealed[..cut], &KEY).is_err(), "cut at {cut}");
        }
    }

    #[test]
    fn reordered_or_repeated_chunks_do_not_open() {
        let (sealed, _) = seal(&payload(3 * CHUNK));
        let body = &sealed[HEADER_LEN..];
        let size = CHUNK + TAG;
        let mut swapped = sealed[..HEADER_LEN].to_vec();
        swapped.extend_from_slice(&body[size..2 * size]);
        swapped.extend_from_slice(&body[..size]);
        swapped.extend_from_slice(&body[2 * size..]);
        assert!(open(&swapped, &KEY).is_err());
        let mut repeated = sealed[..HEADER_LEN].to_vec();
        repeated.extend_from_slice(&body[..size]);
        repeated.extend_from_slice(&body[..size]);
        repeated.extend_from_slice(&body[2 * size..]);
        assert!(open(&repeated, &KEY).is_err());
    }

    #[test]
    fn trailing_bytes_after_the_last_chunk_are_refused() {
        let (mut sealed, _) = seal(&payload(100));
        sealed.extend_from_slice(&[1, 2, 3]);
        assert!(open(&sealed, &KEY).is_err());
    }

    #[test]
    fn the_header_is_checked_before_anything_else() {
        let (sealed, _) = seal(&payload(10));
        let mut not_ours = sealed.clone();
        not_ours[0] = b'X';
        assert!(matches!(
            open(&not_ours, &KEY),
            Err(BackupError::Refused(_))
        ));
        let mut newer = sealed.clone();
        newer[8] = FORMAT + 1;
        assert!(matches!(open(&newer, &KEY), Err(BackupError::Newer(_))));
        let mut chunk_size = sealed.clone();
        chunk_size[HEADER_LEN - 1] = 0xff;
        assert!(matches!(
            open(&chunk_size, &KEY),
            Err(BackupError::Corrupt(_))
        ));
    }

    #[test]
    fn a_header_cannot_demand_more_than_the_limits_allow() {
        let header = Header::new(true, Argon2Cost::default()).unwrap();
        let mut raw = header.to_bytes();
        // Memory cost field (bytes 10..14) set to 4 GiB.
        raw[10..14].copy_from_slice(&(4 * 1024 * 1024u32).to_le_bytes());
        assert!(matches!(
            Header::read(&mut &raw[..]),
            Err(BackupError::Corrupt(_))
        ));
        let mut raw = header.to_bytes();
        raw[14..18].copy_from_slice(&0u32.to_le_bytes());
        assert!(Header::read(&mut &raw[..]).is_err());
    }

    #[test]
    fn two_seals_of_the_same_payload_differ() {
        let data = payload(1000);
        let (a, _) = seal(&data);
        let (b, _) = seal(&data);
        assert_ne!(a, b, "the nonce base is random per file");
    }
}
