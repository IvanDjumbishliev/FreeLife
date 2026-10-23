use std::path::{Path, PathBuf};

use freelife_core::{ActiveConnection, FreeLifeMessage};
use tokio::io::AsyncReadExt;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::Stream;
use uuid::Uuid;

use crate::progress::{Direction, ProgressTracker, TransferProgress};
use crate::{hash_file, Result, TransferError, CHUNK_SIZE};

/// Send the file at `path` over `conn`.
///
/// Hashes the file, sends a `FileTransferRequest` carrying its SHA-256, then
/// streams `CHUNK_SIZE` chunks. The stream yields progress (throttled to
/// about 10 updates/s, always including the first and the 100% update),
/// then ends. If anything fails, the last item is an `Err`.
///
/// The transfer runs on a background task. **Dropping the stream cancels
/// it**; the receiver then discards the partial file once it stalls.
pub fn send_file(
    conn: &ActiveConnection,
    path: &Path,
) -> impl Stream<Item = Result<TransferProgress>> + Send + 'static {
    send_file_with_id(conn, path, Uuid::new_v4())
}

/// [`send_file`] with a caller-chosen transfer id, so a UI can track the
/// transfer before the first progress update arrives.
pub fn send_file_with_id(
    conn: &ActiveConnection,
    path: &Path,
    id: Uuid,
) -> impl Stream<Item = Result<TransferProgress>> + Send + 'static {
    let (tx, rx) = mpsc::channel(8);
    let conn = conn.clone();
    let path = path.to_path_buf();
    tokio::spawn(async move {
        if let Err(e) = run(&conn, path, id, &tx).await {
            let _ = tx.send(Err(e)).await;
        }
    });
    ReceiverStream::new(rx)
}

async fn run(
    conn: &ActiveConnection,
    path: PathBuf,
    id: Uuid,
    progress: &mpsc::Sender<Result<TransferProgress>>,
) -> Result<()> {
    let meta = tokio::fs::metadata(&path).await?;
    if !meta.is_file() {
        return Err(TransferError::NotAFile(path));
    }
    let filename = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or(TransferError::InvalidFilename)?;
    let size = meta.len();
    let checksum = hash_file(path.clone()).await?;

    let mut tracker = ProgressTracker::new(
        id,
        conn.device_id(),
        Direction::Send,
        filename.clone(),
        size,
    );
    report(progress, tracker.update(0)).await?;

    conn.send(FreeLifeMessage::FileTransferRequest {
        id,
        filename,
        size,
        checksum,
    })
    .await?;

    let mut file = tokio::fs::File::open(&path).await?;
    let mut sent = 0u64;
    let mut index = 0u64;
    loop {
        if progress.is_closed() {
            return Err(TransferError::Cancelled);
        }
        let mut buf = vec![0u8; CHUNK_SIZE];
        let n = read_full(&mut file, &mut buf).await?;
        if n == 0 {
            break;
        }
        buf.truncate(n);
        sent += n as u64;
        if sent > size {
            return Err(TransferError::FileChanged);
        }
        conn.send(FreeLifeMessage::FileChunk {
            id,
            index,
            data: buf,
        })
        .await?;
        index += 1;
        if sent < size {
            report(progress, tracker.update(sent)).await?;
        }
    }
    if sent != size {
        return Err(TransferError::FileChanged);
    }
    // Empty files reported 100% as their first update already.
    if size > 0 {
        report(progress, tracker.update(size)).await?;
    }
    Ok(())
}

async fn report(
    progress: &mpsc::Sender<Result<TransferProgress>>,
    update: Option<TransferProgress>,
) -> Result<()> {
    match update {
        Some(p) => progress
            .send(Ok(p))
            .await
            .map_err(|_| TransferError::Cancelled),
        None => Ok(()),
    }
}

/// Fill `buf` unless EOF comes first; returns the bytes read.
async fn read_full(file: &mut tokio::fs::File, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        let n = file.read(&mut buf[filled..]).await?;
        if n == 0 {
            break;
        }
        filled += n;
    }
    Ok(filled)
}
