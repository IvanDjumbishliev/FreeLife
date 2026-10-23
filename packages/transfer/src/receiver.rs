use std::collections::HashMap;
use std::io::SeekFrom;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use freelife_core::{DeviceId, FreeLifeMessage};
use tokio::io::{AsyncSeekExt, AsyncWriteExt};
use tokio::sync::{broadcast, mpsc};
use uuid::Uuid;

use crate::progress::{Direction, ProgressTracker, TransferEvent};
use crate::{chunk_count, hash_file, sanitize_filename, Result, TransferError, CHUNK_SIZE};

#[derive(Debug, Clone)]
pub struct ReceiverConfig {
    /// Where finished files are saved. Partial files live here too, as
    /// hidden `.<id>.freelife-part` files.
    pub download_dir: PathBuf,
    /// Requests for bigger files are refused.
    pub max_file_size: u64,
    /// Abandon a transfer after this long without a chunk.
    pub stall_timeout: Duration,
}

impl Default for ReceiverConfig {
    fn default() -> Self {
        Self {
            download_dir: default_download_dir(),
            max_file_size: 64 * 1024 * 1024 * 1024,
            stall_timeout: Duration::from_secs(60),
        }
    }
}

/// `~/Downloads/FreeLife` (the OS's Downloads folder, wherever it is).
pub fn default_download_dir() -> PathBuf {
    dirs::download_dir()
        .or_else(|| dirs::home_dir().map(|h| h.join("Downloads")))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("FreeLife")
}

struct Incoming {
    peer: DeviceId,
    part_path: PathBuf,
    file: tokio::fs::File,
    size: u64,
    checksum: [u8; 32],
    received: Vec<bool>,
    chunks_left: u64,
    bytes: u64,
    tracker: ProgressTracker,
    last_activity: Instant,
}

type Key = (DeviceId, Uuid);

/// Reassembles incoming files and reports on them via `TransferEvent`s.
///
/// Chunks are written straight to their offset in a preallocated part file,
/// so they may arrive in any order. Once every chunk is in, the file is
/// hashed and moved into place, or deleted if the checksum doesn't match.
pub struct FileReceiver {
    config: ReceiverConfig,
    events: broadcast::Sender<TransferEvent>,
    active: HashMap<Key, Incoming>,
}

impl FileReceiver {
    pub fn new(config: ReceiverConfig, events: broadcast::Sender<TransferEvent>) -> Self {
        Self {
            config,
            events,
            active: HashMap::new(),
        }
    }

    pub fn active_transfers(&self) -> usize {
        self.active.len()
    }

    /// Process one message from `peer`. File-transfer messages are consumed
    /// (`None`); anything else is handed back for the caller to route.
    pub async fn handle(
        &mut self,
        peer: DeviceId,
        msg: FreeLifeMessage,
    ) -> Option<FreeLifeMessage> {
        match msg {
            FreeLifeMessage::FileTransferRequest {
                id,
                filename,
                size,
                checksum,
            } => {
                self.on_request(peer, id, filename, size, checksum).await;
                None
            }
            FreeLifeMessage::FileChunk { id, index, data } => {
                self.on_chunk((peer, id), index, &data).await;
                None
            }
            other => Some(other),
        }
    }

    /// Run until `incoming` closes, forwarding non-transfer messages to
    /// `other` and expiring stalled transfers along the way. If nobody is
    /// reading `other`, those messages are dropped rather than holding up
    /// file transfers.
    pub async fn run(
        mut self,
        mut incoming: mpsc::Receiver<(DeviceId, FreeLifeMessage)>,
        other: mpsc::Sender<(DeviceId, FreeLifeMessage)>,
    ) {
        let period = (self.config.stall_timeout / 4).max(Duration::from_millis(10));
        let mut sweep = tokio::time::interval(period);
        loop {
            tokio::select! {
                msg = incoming.recv() => {
                    let Some((peer, msg)) = msg else { break };
                    if let Some(rest) = self.handle(peer, msg).await {
                        if other.try_send((peer, rest)).is_err() {
                            tracing::debug!(%peer, "dropping unhandled message");
                        }
                    }
                }
                _ = sweep.tick() => self.expire_stalled().await,
            }
        }
        let keys: Vec<Key> = self.active.keys().copied().collect();
        for key in keys {
            self.fail(key, TransferError::Cancelled).await;
        }
    }

    /// Abandon transfers that haven't received a chunk within the stall timeout.
    pub async fn expire_stalled(&mut self) {
        let timeout = self.config.stall_timeout;
        let stalled: Vec<Key> = self
            .active
            .iter()
            .filter(|(_, t)| t.last_activity.elapsed() >= timeout)
            .map(|(k, _)| *k)
            .collect();
        for key in stalled {
            self.fail(key, TransferError::Stalled).await;
        }
    }

    /// Abandon every transfer from `peer` (e.g. when it disconnects).
    pub async fn cancel_from(&mut self, peer: DeviceId) {
        let keys: Vec<Key> = self
            .active
            .keys()
            .filter(|k| k.0 == peer)
            .copied()
            .collect();
        for key in keys {
            self.fail(key, TransferError::Cancelled).await;
        }
    }

    async fn on_request(
        &mut self,
        peer: DeviceId,
        id: Uuid,
        raw_name: String,
        size: u64,
        checksum: [u8; 32],
    ) {
        if self.active.contains_key(&(peer, id)) {
            self.emit_error(peer, id, &raw_name, &TransferError::Duplicate(id));
            return;
        }
        let Some(filename) = sanitize_filename(&raw_name) else {
            self.emit_error(peer, id, &raw_name, &TransferError::InvalidFilename);
            return;
        };
        if size > self.config.max_file_size {
            let err = TransferError::TooLarge {
                size,
                max: self.config.max_file_size,
            };
            self.emit_error(peer, id, &filename, &err);
            return;
        }

        let part_path = self
            .config
            .download_dir
            .join(format!(".{id}.freelife-part"));
        let file = match create_part_file(&self.config.download_dir, &part_path, size).await {
            Ok(f) => f,
            Err(e) => {
                let _ = tokio::fs::remove_file(&part_path).await;
                self.emit_error(peer, id, &filename, &e);
                return;
            }
        };
        tracing::info!(%peer, %id, %filename, size, "receiving file");

        let chunks = chunk_count(size);
        let mut tracker = ProgressTracker::new(id, peer, Direction::Receive, filename, size);
        if let Some(p) = tracker.update(0) {
            let _ = self.events.send(TransferEvent::Progress(p));
        }
        self.active.insert(
            (peer, id),
            Incoming {
                peer,
                part_path,
                file,
                size,
                checksum,
                received: vec![false; chunks as usize],
                chunks_left: chunks,
                bytes: 0,
                tracker,
                last_activity: Instant::now(),
            },
        );
        if chunks == 0 {
            self.finish((peer, id)).await;
        }
    }

    async fn on_chunk(&mut self, key: Key, index: u64, data: &[u8]) {
        let Some(t) = self.active.get_mut(&key) else {
            // Unknown, or already failed: drop silently rather than
            // reporting one error per remaining chunk.
            tracing::trace!(id = %key.1, index, "chunk for inactive transfer");
            return;
        };
        let Some(slot) = t.received.get(index as usize).copied() else {
            self.fail(key, TransferError::ChunkOutOfRange { index })
                .await;
            return;
        };
        let offset = index * CHUNK_SIZE as u64;
        let expected = (t.size - offset).min(CHUNK_SIZE as u64);
        if data.len() as u64 != expected {
            let err = TransferError::ChunkSize {
                index,
                got: data.len(),
                expected,
            };
            self.fail(key, err).await;
            return;
        }
        t.last_activity = Instant::now();
        if slot {
            return; // duplicate
        }

        let write = async {
            t.file.seek(SeekFrom::Start(offset)).await?;
            t.file.write_all(data).await
        };
        if let Err(e) = write.await {
            self.fail(key, e.into()).await;
            return;
        }
        t.received[index as usize] = true;
        t.chunks_left -= 1;
        t.bytes += data.len() as u64;
        if let Some(p) = t.tracker.update(t.bytes) {
            let _ = self.events.send(TransferEvent::Progress(p));
        }
        if t.chunks_left == 0 {
            self.finish(key).await;
        }
    }

    async fn finish(&mut self, key: Key) {
        let Some(mut t) = self.active.remove(&key) else {
            return;
        };
        let filename = t.tracker.filename.clone();
        match self.verify_and_save(&mut t).await {
            Ok(path) => {
                tracing::info!(peer = %t.peer, path = %path.display(), "file received");
                let _ = self.events.send(TransferEvent::TransferComplete {
                    id: key.1,
                    peer: t.peer,
                    direction: Direction::Receive,
                    filename,
                    total_bytes: t.size,
                    path: Some(path),
                });
            }
            Err(e) => {
                let _ = tokio::fs::remove_file(&t.part_path).await;
                self.emit_error(t.peer, key.1, &filename, &e);
            }
        }
    }

    async fn verify_and_save(&self, t: &mut Incoming) -> Result<PathBuf> {
        t.file.sync_all().await?;
        if hash_file(t.part_path.clone()).await? != t.checksum {
            return Err(TransferError::ChecksumMismatch);
        }
        let dest = unique_path(&self.config.download_dir, &t.tracker.filename).await;
        tokio::fs::rename(&t.part_path, &dest).await?;
        Ok(dest)
    }

    async fn fail(&mut self, key: Key, err: TransferError) {
        let Some(t) = self.active.remove(&key) else {
            return;
        };
        drop(t.file);
        let _ = tokio::fs::remove_file(&t.part_path).await;
        tracing::warn!(peer = %t.peer, id = %key.1, error = %err, "transfer failed");
        self.emit_error(t.peer, key.1, &t.tracker.filename, &err);
    }

    fn emit_error(&self, peer: DeviceId, id: Uuid, filename: &str, err: &TransferError) {
        let _ = self.events.send(TransferEvent::TransferError {
            id,
            peer,
            direction: Direction::Receive,
            filename: filename.to_owned(),
            error: err.to_string(),
        });
    }
}

async fn create_part_file(dir: &Path, part: &Path, size: u64) -> Result<tokio::fs::File> {
    tokio::fs::create_dir_all(dir).await?;
    let file = tokio::fs::File::create(part).await?;
    // Sparse preallocation lets chunks land at their offsets in any order.
    file.set_len(size).await?;
    Ok(file)
}

/// `dir/name`, or `dir/stem (n).ext` for the first free `n`.
async fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let candidate = dir.join(name);
    if !exists(&candidate).await {
        return candidate;
    }
    let (stem, ext) = match name.rfind('.') {
        Some(dot) if dot > 0 => name.split_at(dot),
        _ => (name, ""),
    };
    for n in 1.. {
        let candidate = dir.join(format!("{stem} ({n}){ext}"));
        if !exists(&candidate).await {
            return candidate;
        }
    }
    unreachable!()
}

async fn exists(path: &Path) -> bool {
    tokio::fs::try_exists(path).await.unwrap_or(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn unique_path_numbers_duplicates() {
        let dir = tempfile::tempdir().unwrap();
        let first = unique_path(dir.path(), "a.txt").await;
        assert_eq!(first, dir.path().join("a.txt"));
        std::fs::write(&first, b"").unwrap();
        let second = unique_path(dir.path(), "a.txt").await;
        assert_eq!(second, dir.path().join("a (1).txt"));
        std::fs::write(&second, b"").unwrap();
        assert_eq!(
            unique_path(dir.path(), "a.txt").await,
            dir.path().join("a (2).txt")
        );

        std::fs::write(dir.path().join("README"), b"").unwrap();
        assert_eq!(
            unique_path(dir.path(), "README").await,
            dir.path().join("README (1)")
        );
    }

    #[test]
    fn default_download_dir_ends_in_freelife() {
        assert!(default_download_dir().ends_with("FreeLife"));
    }
}
