//! File and clipboard transfer.
//!
//! A file goes out as a [`FreeLifeMessage::FileTransferRequest`] followed by
//! fixed-size [`FreeLifeMessage::FileChunk`]s. The receiver reassembles them
//! with [`IncomingFile`] and verifies the SHA-256 once all bytes arrive.
//!
//! This layer only produces and consumes messages; sending them is the job of
//! [`freelife_core::ConnectionManager`].

use freelife_core::{ClipboardContentType, FreeLifeMessage};
use sha2::{Digest, Sha256};
use uuid::Uuid;

pub const CHUNK_SIZE: usize = 256 * 1024;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error("chunk for transfer {got} does not belong to transfer {expected}")]
    WrongTransfer { expected: Uuid, got: Uuid },
    #[error("chunk {got} arrived out of order (expected {expected})")]
    OutOfOrder { expected: u64, got: u64 },
    #[error("received more data than the {expected} bytes announced")]
    TooMuchData { expected: u64 },
    #[error("transfer incomplete: {got} of {expected} bytes")]
    Incomplete { expected: u64, got: u64 },
    #[error("checksum mismatch")]
    ChecksumMismatch,
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

pub fn chunk_count(size: u64) -> u64 {
    size.div_ceil(CHUNK_SIZE as u64)
}

pub fn sha256(data: &[u8]) -> [u8; 32] {
    Sha256::digest(data).into()
}

pub fn clipboard_text(text: impl Into<String>) -> FreeLifeMessage {
    FreeLifeMessage::ClipboardUpdate {
        content_type: ClipboardContentType::Text,
        payload: text.into().into_bytes(),
    }
}

/// A file held in memory, ready to be sent.
///
/// TODO: stream from disk instead of buffering the whole file.
pub struct OutgoingFile {
    pub id: Uuid,
    pub filename: String,
    data: Vec<u8>,
}

impl OutgoingFile {
    pub fn new(filename: impl Into<String>, data: Vec<u8>) -> Self {
        Self {
            id: Uuid::new_v4(),
            filename: filename.into(),
            data,
        }
    }

    pub fn size(&self) -> u64 {
        self.data.len() as u64
    }

    pub fn request(&self) -> FreeLifeMessage {
        FreeLifeMessage::FileTransferRequest {
            id: self.id,
            filename: self.filename.clone(),
            size: self.size(),
            checksum: sha256(&self.data),
        }
    }

    pub fn chunks(&self) -> impl Iterator<Item = FreeLifeMessage> + '_ {
        self.data
            .chunks(CHUNK_SIZE)
            .enumerate()
            .map(|(i, chunk)| FreeLifeMessage::FileChunk {
                id: self.id,
                index: i as u64,
                data: chunk.to_vec(),
            })
    }
}

/// Reassembles a file from in-order chunks.
pub struct IncomingFile {
    pub id: Uuid,
    pub filename: String,
    size: u64,
    checksum: [u8; 32],
    next_index: u64,
    buf: Vec<u8>,
}

impl IncomingFile {
    pub fn new(id: Uuid, filename: String, size: u64, checksum: [u8; 32]) -> Self {
        Self {
            id,
            filename,
            size,
            checksum,
            next_index: 0,
            buf: Vec::new(),
        }
    }

    pub fn push_chunk(&mut self, id: Uuid, index: u64, data: &[u8]) -> Result<()> {
        if id != self.id {
            return Err(Error::WrongTransfer {
                expected: self.id,
                got: id,
            });
        }
        if index != self.next_index {
            return Err(Error::OutOfOrder {
                expected: self.next_index,
                got: index,
            });
        }
        if self.buf.len() as u64 + data.len() as u64 > self.size {
            return Err(Error::TooMuchData {
                expected: self.size,
            });
        }
        self.buf.extend_from_slice(data);
        self.next_index += 1;
        Ok(())
    }

    pub fn is_complete(&self) -> bool {
        self.buf.len() as u64 == self.size
    }

    pub fn progress(&self) -> f64 {
        if self.size == 0 {
            1.0
        } else {
            self.buf.len() as f64 / self.size as f64
        }
    }

    pub fn finish(self) -> Result<Vec<u8>> {
        if !self.is_complete() {
            return Err(Error::Incomplete {
                expected: self.size,
                got: self.buf.len() as u64,
            });
        }
        if sha256(&self.buf) != self.checksum {
            return Err(Error::ChecksumMismatch);
        }
        Ok(self.buf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn receive(out: &OutgoingFile) -> Result<Vec<u8>> {
        let FreeLifeMessage::FileTransferRequest {
            id,
            filename,
            size,
            checksum,
        } = out.request()
        else {
            unreachable!()
        };
        let mut incoming = IncomingFile::new(id, filename, size, checksum);
        for msg in out.chunks() {
            if let FreeLifeMessage::FileChunk { id, index, data } = msg {
                incoming.push_chunk(id, index, &data)?;
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
        assert_eq!(out.chunks().count() as u64, chunk_count(out.size()));
        assert_eq!(receive(&out).unwrap(), data);
    }

    #[test]
    fn empty_file_round_trip() {
        assert_eq!(receive(&OutgoingFile::new("empty", vec![])).unwrap(), b"");
    }

    #[test]
    fn rejects_out_of_order_and_foreign_chunks() {
        let id = Uuid::new_v4();
        let mut f = IncomingFile::new(id, "f".into(), 2, sha256(b"ab"));
        assert_eq!(
            f.push_chunk(id, 1, b"b"),
            Err(Error::OutOfOrder {
                expected: 0,
                got: 1
            })
        );
        assert!(matches!(
            f.push_chunk(Uuid::new_v4(), 0, b"a"),
            Err(Error::WrongTransfer { .. })
        ));
    }

    #[test]
    fn rejects_overflow_and_corruption() {
        let id = Uuid::new_v4();
        let mut f = IncomingFile::new(id, "f".into(), 2, sha256(b"ab"));
        assert_eq!(
            f.push_chunk(id, 0, b"abc"),
            Err(Error::TooMuchData { expected: 2 })
        );
        f.push_chunk(id, 0, b"xx").unwrap();
        assert!(f.is_complete());
        assert_eq!(f.finish(), Err(Error::ChecksumMismatch));
    }

    #[test]
    fn clipboard_text_message() {
        assert_eq!(
            clipboard_text("hi"),
            FreeLifeMessage::ClipboardUpdate {
                content_type: ClipboardContentType::Text,
                payload: b"hi".to_vec(),
            }
        );
    }
}
