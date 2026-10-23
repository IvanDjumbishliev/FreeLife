//! AirShare: file transfer between paired FreeLife devices.
//!
//! A file goes out as one [`FreeLifeMessage::FileTransferRequest`] (name,
//! size, SHA-256 of the whole file) followed by [`CHUNK_SIZE`]
//! [`FreeLifeMessage::FileChunk`]s.
//!
//! - [`send_file`] streams a file over an [`ActiveConnection`] and yields
//!   [`TransferProgress`].
//! - [`FileReceiver`] reassembles incoming chunks in any order, verifies the
//!   checksum, and saves the file to the download directory.
//! - [`node::Node`] wires both into a running device (identity, pairing,
//!   discovery, connections). This is what the apps use.
//!
//! There is no acknowledgement message yet, so "sent" means handed to the
//! encrypted connection, not confirmed by the receiver. Completion on the
//! receiving side is reported by its own [`TransferEvent::TransferComplete`].
//!
//! [`ActiveConnection`]: freelife_core::ActiveConnection

mod filename;
pub mod node;
mod progress;
mod receiver;
mod sender;

use std::path::PathBuf;

use freelife_core::{ClipboardContentType, DeviceId, FreeLifeMessage};
use uuid::Uuid;

pub use filename::sanitize_filename;
pub use progress::{Direction, TransferEvent, TransferProgress};
pub use receiver::{default_download_dir, FileReceiver, ReceiverConfig};
pub use sender::{send_file, send_file_with_id};

pub const CHUNK_SIZE: usize = 256 * 1024;

pub fn chunk_count(size: u64) -> u64 {
    size.div_ceil(CHUNK_SIZE as u64)
}

/// A fresh id for [`send_file_with_id`] or
/// [`node::Node::send_file_as`].
pub fn new_transfer_id() -> Uuid {
    Uuid::new_v4()
}

pub fn clipboard_text(text: impl Into<String>) -> FreeLifeMessage {
    FreeLifeMessage::ClipboardUpdate {
        content_type: ClipboardContentType::Text,
        payload: text.into().into_bytes(),
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TransferError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Core(#[from] freelife_core::Error),

    #[error("{} is not a regular file", .0.display())]
    NotAFile(PathBuf),

    #[error("the file changed while it was being sent")]
    FileChanged,

    #[error("invalid file name")]
    InvalidFilename,

    #[error("file is {size} bytes, over the {max}-byte limit")]
    TooLarge { size: u64, max: u64 },

    #[error("transfer {0} is already in progress")]
    Duplicate(Uuid),

    #[error("chunk {index} is out of range")]
    ChunkOutOfRange { index: u64 },

    #[error("chunk {index} has {got} bytes, expected {expected}")]
    ChunkSize {
        index: u64,
        got: usize,
        expected: u64,
    },

    #[error("checksum mismatch: the file was corrupted in transit")]
    ChecksumMismatch,

    #[error("transfer stalled: no data received for too long")]
    Stalled,

    #[error("transfer cancelled")]
    Cancelled,

    #[error("device {0} is not paired")]
    NotPaired(DeviceId),

    #[error("device {0} could not be reached")]
    Unreachable(DeviceId),
}

pub type Result<T, E = TransferError> = std::result::Result<T, E>;

/// SHA-256 of a file, computed on a blocking thread.
pub(crate) async fn hash_file(path: PathBuf) -> Result<[u8; 32]> {
    tokio::task::spawn_blocking(move || {
        use sha2::{Digest, Sha256};
        use std::io::Read;

        let mut file = std::fs::File::open(path)?;
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; 1024 * 1024];
        loop {
            let n = file.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
        Ok(hasher.finalize().into())
    })
    .await
    .map_err(|e| TransferError::Io(std::io::Error::other(e)))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_count_rounds_up() {
        assert_eq!(chunk_count(0), 0);
        assert_eq!(chunk_count(1), 1);
        assert_eq!(chunk_count(CHUNK_SIZE as u64), 1);
        assert_eq!(chunk_count(CHUNK_SIZE as u64 + 1), 2);
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

    #[tokio::test]
    async fn hash_file_matches_in_memory_hash() {
        use sha2::{Digest, Sha256};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("f");
        let data: Vec<u8> = (0..3_000_000u32).map(|i| (i % 251) as u8).collect();
        std::fs::write(&path, &data).unwrap();
        let expected: [u8; 32] = Sha256::digest(&data).into();
        assert_eq!(hash_file(path).await.unwrap(), expected);
    }
}
