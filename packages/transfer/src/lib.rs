//! File and clipboard transfer.
//!
//! Files are sent as a [`Message::FileOffer`] followed by fixed-size
//! [`Message::FileChunk`]s and a [`Message::FileComplete`]. The receiver
//! reassembles them with [`IncomingFile`] and verifies the SHA-256.
//!
//! This layer only produces and consumes [`Message`]s; the network transport
//! lives elsewhere.

use freelife_core::Message;
use sha2::{Digest, Sha256};

pub const CHUNK_SIZE: usize = 256 * 1024;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error("chunk {got} arrived out of order (expected {expected})")]
    OutOfOrder { expected: u64, got: u64 },
    #[error("received {got} bytes but the offer announced {expected}")]
    SizeMismatch { expected: u64, got: u64 },
    #[error("checksum mismatch")]
    ChecksumMismatch,
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

pub fn chunk_count(size: u64) -> u64 {
    size.div_ceil(CHUNK_SIZE as u64)
}

pub fn sha256_hex(data: &[u8]) -> String {
    Sha256::digest(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

pub fn clipboard_message(text: impl Into<String>) -> Message {
    Message::ClipboardText { text: text.into() }
}

/// A file held in memory, ready to be sent.
///
/// TODO: stream from disk instead of buffering the whole file.
pub struct OutgoingFile {
    pub transfer_id: String,
    pub name: String,
    data: Vec<u8>,
}

impl OutgoingFile {
    pub fn new(name: impl Into<String>, data: Vec<u8>) -> Self {
        Self {
            transfer_id: uuid::Uuid::new_v4().to_string(),
            name: name.into(),
            data,
        }
    }

    pub fn size(&self) -> u64 {
        self.data.len() as u64
    }

    pub fn offer(&self) -> Message {
        Message::FileOffer {
            transfer_id: self.transfer_id.clone(),
            name: self.name.clone(),
            size: self.size(),
            sha256: sha256_hex(&self.data),
        }
    }

    /// All chunk messages followed by the completion marker.
    pub fn messages(&self) -> impl Iterator<Item = Message> + '_ {
        self.data
            .chunks(CHUNK_SIZE)
            .enumerate()
            .map(|(i, chunk)| Message::FileChunk {
                transfer_id: self.transfer_id.clone(),
                index: i as u64,
                data: chunk.to_vec(),
            })
            .chain(std::iter::once(Message::FileComplete {
                transfer_id: self.transfer_id.clone(),
            }))
    }
}

/// Reassembles a file from in-order chunks.
pub struct IncomingFile {
    pub transfer_id: String,
    pub name: String,
    expected_size: u64,
    expected_sha256: String,
    next_index: u64,
    buf: Vec<u8>,
}

impl IncomingFile {
    pub fn from_offer(transfer_id: String, name: String, size: u64, sha256: String) -> Self {
        Self {
            transfer_id,
            name,
            expected_size: size,
            expected_sha256: sha256,
            next_index: 0,
            buf: Vec::new(),
        }
    }

    pub fn push_chunk(&mut self, index: u64, data: &[u8]) -> Result<()> {
        if index != self.next_index {
            return Err(Error::OutOfOrder {
                expected: self.next_index,
                got: index,
            });
        }
        self.buf.extend_from_slice(data);
        self.next_index += 1;
        Ok(())
    }

    pub fn progress(&self) -> f64 {
        if self.expected_size == 0 {
            1.0
        } else {
            self.buf.len() as f64 / self.expected_size as f64
        }
    }

    pub fn finish(self) -> Result<Vec<u8>> {
        let got = self.buf.len() as u64;
        if got != self.expected_size {
            return Err(Error::SizeMismatch {
                expected: self.expected_size,
                got,
            });
        }
        if sha256_hex(&self.buf) != self.expected_sha256 {
            return Err(Error::ChecksumMismatch);
        }
        Ok(self.buf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn receive(out: &OutgoingFile) -> Result<Vec<u8>> {
        let Message::FileOffer {
            transfer_id,
            name,
            size,
            sha256,
        } = out.offer()
        else {
            unreachable!()
        };
        let mut incoming = IncomingFile::from_offer(transfer_id, name, size, sha256);
        for msg in out.messages() {
            if let Message::FileChunk { index, data, .. } = msg {
                incoming.push_chunk(index, &data)?;
            }
        }
        incoming.finish()
    }

    #[test]
    fn chunk_count_rounds_up() {
        assert_eq!(chunk_count(0), 0);
        assert_eq!(chunk_count(1), 1);
        assert_eq!(chunk_count(CHUNK_SIZE as u64), 1);
        assert_eq!(chunk_count(CHUNK_SIZE as u64 + 1), 2);
    }

    #[test]
    fn multi_chunk_round_trip() {
        let data: Vec<u8> = (0..CHUNK_SIZE * 2 + 17).map(|i| i as u8).collect();
        let out = OutgoingFile::new("blob.bin", data.clone());
        assert_eq!(
            out.messages().count() as u64,
            chunk_count(data.len() as u64) + 1
        );
        assert_eq!(receive(&out).unwrap(), data);
    }

    #[test]
    fn rejects_out_of_order_chunks() {
        let mut f = IncomingFile::from_offer("t".into(), "f".into(), 2, sha256_hex(b"ab"));
        assert_eq!(
            f.push_chunk(1, b"b"),
            Err(Error::OutOfOrder {
                expected: 0,
                got: 1
            })
        );
    }

    #[test]
    fn detects_corruption() {
        let mut f = IncomingFile::from_offer("t".into(), "f".into(), 2, sha256_hex(b"ab"));
        f.push_chunk(0, b"xx").unwrap();
        assert_eq!(f.finish(), Err(Error::ChecksumMismatch));
    }
}
