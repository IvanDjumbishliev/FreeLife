//! End-to-end AirShare tests: real files, real `send_file` and
//! `FileReceiver`, joined by in-process mock connections (or, for `Node`,
//! real TLS sessions on localhost).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use freelife_core::identity::MemoryKeyStore;
use freelife_core::prompts::PromptEvent;
use freelife_core::{ActiveConnection, DeviceId, FreeLifeMessage, Identity, OsType};
use freelife_transfer::node::{Node, NodeConfig};
use freelife_transfer::{
    send_file, Direction, FileReceiver, ReceiverConfig, TransferError, TransferEvent,
    TransferProgress, CHUNK_SIZE,
};
use sha2::{Digest, Sha256};
use tokio::sync::{broadcast, mpsc};
use tokio_stream::StreamExt;

const FIFTY_MB: u64 = 50 * 1024 * 1024;

fn device_id() -> DeviceId {
    Identity::generate().unwrap().device_id()
}

/// Deterministic pseudo-random file contents (xorshift), so a dropped or
/// misplaced chunk can't go unnoticed the way it could with repeating data.
fn write_test_file(dir: &Path, name: &str, size: u64) -> PathBuf {
    use std::io::Write;
    let path = dir.join(name);
    let mut file = std::io::BufWriter::new(std::fs::File::create(&path).unwrap());
    let mut state: u64 = 0x9E37_79B9_7F4A_7C15 ^ size;
    let mut written = 0;
    let mut block = vec![0u8; 1024 * 1024];
    while written < size {
        for b in block.chunks_mut(8) {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            b.copy_from_slice(&state.to_le_bytes()[..b.len()]);
        }
        let n = (size - written).min(block.len() as u64) as usize;
        file.write_all(&block[..n]).unwrap();
        written += n as u64;
    }
    file.flush().unwrap();
    path
}

fn sha256_file(path: &Path) -> [u8; 32] {
    use std::io::Read;
    let mut file = std::fs::File::open(path).unwrap();
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1024 * 1024];
    loop {
        let n = file.read(&mut buf).unwrap();
        if n == 0 {
            return hasher.finalize().into();
        }
        hasher.update(&buf[..n]);
    }
}

fn dir_entries(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

/// How the mock link mangles traffic between sender and receiver.
#[derive(Clone, Copy)]
enum Link {
    InOrder,
    /// Deliver the request, then every chunk in a scrambled order.
    Shuffled,
    /// Flip one bit in the given chunk.
    Corrupt(u64),
    /// Never deliver the given chunk.
    Drop(u64),
}

struct Harness {
    conn: ActiveConnection,
    events: broadcast::Receiver<TransferEvent>,
    sender: DeviceId,
    download_dir: tempfile::TempDir,
}

/// A mock connection whose far end is a `FileReceiver` driven by `run()`,
/// exactly as a real node drives it.
fn harness(link: Link, config: impl FnOnce(&mut ReceiverConfig)) -> Harness {
    let download_dir = tempfile::tempdir().unwrap();
    let mut cfg = ReceiverConfig {
        download_dir: download_dir.path().to_path_buf(),
        ..ReceiverConfig::default()
    };
    config(&mut cfg);

    let sender = device_id();
    let receiver_id = device_id();
    let (wire_tx, mut wire_rx) = mpsc::channel::<FreeLifeMessage>(64);
    let (deliver_tx, deliver_rx) = mpsc::channel(64);
    let (events_tx, events) = broadcast::channel(4096);
    let (other_tx, _other_rx) = mpsc::channel(8);
    tokio::spawn(FileReceiver::new(cfg, events_tx).run(deliver_rx, other_tx));

    tokio::spawn(async move {
        let mut held = Vec::new();
        let mut expected_chunks = 0;
        while let Some(mut msg) = wire_rx.recv().await {
            match &mut msg {
                FreeLifeMessage::FileTransferRequest { size, .. } => {
                    expected_chunks = freelife_transfer::chunk_count(*size) as usize;
                }
                FreeLifeMessage::FileChunk { index, data, .. } => match link {
                    Link::Shuffled => {
                        held.push(msg);
                        if held.len() == expected_chunks {
                            release_scrambled(&mut held, sender, &deliver_tx).await;
                        }
                        continue;
                    }
                    Link::Corrupt(i) if *index == i => {
                        let mid = data.len() / 2;
                        data[mid] ^= 0x01;
                    }
                    Link::Drop(i) if *index == i => continue,
                    _ => {}
                },
                _ => {}
            }
            if deliver_tx.send((sender, msg)).await.is_err() {
                return;
            }
        }
    });

    Harness {
        conn: ActiveConnection::from_sender(receiver_id, wire_tx),
        events,
        sender,
        download_dir,
    }
}

/// Deliver `held` in a fixed scrambled order: index `(k * 7919 + 13) % n`
/// visits every chunk once since 7919 is prime (and far above any `n` here).
async fn release_scrambled(
    held: &mut Vec<FreeLifeMessage>,
    sender: DeviceId,
    deliver: &mpsc::Sender<(DeviceId, FreeLifeMessage)>,
) {
    let n = held.len();
    let mut slots: Vec<Option<FreeLifeMessage>> = held.drain(..).map(Some).collect();
    for k in 0..n {
        let msg = slots[(k * 7919 + 13) % n].take().expect("visited twice");
        let _ = deliver.send((sender, msg)).await;
    }
}

async fn collect_progress(
    conn: &ActiveConnection,
    path: &Path,
) -> Result<Vec<TransferProgress>, TransferError> {
    let mut stream = std::pin::pin!(send_file(conn, path));
    let mut updates = Vec::new();
    while let Some(update) = stream.next().await {
        updates.push(update?);
    }
    Ok(updates)
}

/// Wait for the terminal event (complete or error) of the receive side.
async fn final_event(events: &mut broadcast::Receiver<TransferEvent>) -> TransferEvent {
    tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            match events.recv().await {
                Ok(TransferEvent::Progress(_)) => continue,
                Ok(event) => return event,
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(e) => panic!("event channel closed: {e}"),
            }
        }
    })
    .await
    .expect("no completion event")
}

fn assert_progress_is_sane(updates: &[TransferProgress], total: u64) {
    assert!(!updates.is_empty());
    assert_eq!(updates[0].bytes_sent, 0);
    assert!(updates
        .windows(2)
        .all(|w| w[0].bytes_sent <= w[1].bytes_sent));
    let last = updates.last().unwrap();
    assert_eq!(last.bytes_sent, total);
    assert_eq!(last.total_bytes, total);
    assert_eq!(last.eta_secs, Some(0));
    assert!(last.speed_bps > 0);
    assert!(updates.iter().all(|u| u.direction == Direction::Send));
}

async fn assert_received(h: &mut Harness, source: &Path, expected_name: &str) {
    match final_event(&mut h.events).await {
        TransferEvent::TransferComplete {
            peer,
            direction,
            filename,
            total_bytes,
            path,
            ..
        } => {
            let path = path.expect("receiver reports where it saved the file");
            assert_eq!(peer, h.sender);
            assert_eq!(direction, Direction::Receive);
            assert_eq!(filename, expected_name);
            assert_eq!(total_bytes, std::fs::metadata(source).unwrap().len());
            assert_eq!(path, h.download_dir.path().join(expected_name));
            assert_eq!(sha256_file(&path), sha256_file(source), "checksum mismatch");
        }
        other => panic!("expected completion, got {other:?}"),
    }
    assert_eq!(
        dir_entries(h.download_dir.path()),
        vec![expected_name.to_string()]
    );
}

#[tokio::test]
async fn fifty_mb_file_arrives_intact() {
    let src = tempfile::tempdir().unwrap();
    let path = write_test_file(src.path(), "video.mp4", FIFTY_MB);
    let mut h = harness(Link::InOrder, |_| {});

    let updates = collect_progress(&h.conn, &path).await.unwrap();
    assert_progress_is_sane(&updates, FIFTY_MB);
    assert_received(&mut h, &path, "video.mp4").await;
}

#[tokio::test]
async fn fifty_mb_file_survives_out_of_order_delivery() {
    let src = tempfile::tempdir().unwrap();
    let path = write_test_file(src.path(), "shuffled.bin", FIFTY_MB);
    let mut h = harness(Link::Shuffled, |_| {});

    collect_progress(&h.conn, &path).await.unwrap();
    assert_received(&mut h, &path, "shuffled.bin").await;
}

#[tokio::test]
async fn corrupted_chunk_is_detected_and_discarded() {
    let src = tempfile::tempdir().unwrap();
    let path = write_test_file(src.path(), "c.bin", 3 * CHUNK_SIZE as u64 + 5);
    let mut h = harness(Link::Corrupt(1), |_| {});

    collect_progress(&h.conn, &path).await.unwrap();
    match final_event(&mut h.events).await {
        TransferEvent::TransferError { error, .. } => {
            assert!(error.contains("checksum"), "{error}")
        }
        other => panic!("expected error, got {other:?}"),
    }
    assert!(
        dir_entries(h.download_dir.path()).is_empty(),
        "nothing saved, no part file left"
    );
}

#[tokio::test]
async fn missing_chunk_stalls_and_is_cleaned_up() {
    let src = tempfile::tempdir().unwrap();
    let path = write_test_file(src.path(), "s.bin", 4 * CHUNK_SIZE as u64);
    let mut h = harness(Link::Drop(2), |c| {
        c.stall_timeout = Duration::from_millis(200)
    });

    collect_progress(&h.conn, &path).await.unwrap();
    match final_event(&mut h.events).await {
        TransferEvent::TransferError { error, .. } => assert!(error.contains("stalled"), "{error}"),
        other => panic!("expected stall, got {other:?}"),
    }
    assert!(dir_entries(h.download_dir.path()).is_empty());
}

#[tokio::test]
async fn empty_file_round_trip() {
    let src = tempfile::tempdir().unwrap();
    let path = write_test_file(src.path(), "empty.txt", 0);
    let mut h = harness(Link::InOrder, |_| {});

    let updates = collect_progress(&h.conn, &path).await.unwrap();
    assert_eq!(updates.len(), 1);
    assert!(updates[0].is_complete());
    assert_received(&mut h, &path, "empty.txt").await;
}

#[tokio::test]
async fn same_name_twice_gets_numbered() {
    let src = tempfile::tempdir().unwrap();
    let path = write_test_file(src.path(), "notes.txt", 1000);
    let mut h = harness(Link::InOrder, |_| {});

    for _ in 0..2 {
        collect_progress(&h.conn, &path).await.unwrap();
        assert!(matches!(
            final_event(&mut h.events).await,
            TransferEvent::TransferComplete { .. }
        ));
    }
    assert_eq!(
        dir_entries(h.download_dir.path()),
        vec!["notes (1).txt".to_string(), "notes.txt".to_string()]
    );
}

#[tokio::test]
async fn oversized_files_are_refused() {
    let src = tempfile::tempdir().unwrap();
    let path = write_test_file(src.path(), "big.bin", 2048);
    let mut h = harness(Link::InOrder, |c| c.max_file_size = 1024);

    collect_progress(&h.conn, &path).await.unwrap();
    match final_event(&mut h.events).await {
        TransferEvent::TransferError { error, .. } => assert!(error.contains("limit"), "{error}"),
        other => panic!("expected refusal, got {other:?}"),
    }
    assert!(dir_entries(h.download_dir.path()).is_empty());
}

#[tokio::test]
async fn hostile_filename_stays_in_download_dir() {
    let download_dir = tempfile::tempdir().unwrap();
    let (events_tx, mut events) = broadcast::channel(64);
    let mut receiver = FileReceiver::new(
        ReceiverConfig {
            download_dir: download_dir.path().to_path_buf(),
            ..ReceiverConfig::default()
        },
        events_tx,
    );
    let data = b"#!/bin/sh\necho pwned\n".to_vec();
    let id = uuid::Uuid::new_v4();
    let peer = device_id();
    receiver
        .handle(
            peer,
            FreeLifeMessage::FileTransferRequest {
                id,
                filename: "../../../.bashrc".into(),
                size: data.len() as u64,
                checksum: Sha256::digest(&data).into(),
            },
        )
        .await;
    receiver
        .handle(peer, FreeLifeMessage::FileChunk { id, index: 0, data })
        .await;

    match final_event(&mut events).await {
        TransferEvent::TransferComplete { path, .. } => {
            assert_eq!(path.unwrap(), download_dir.path().join("bashrc"));
        }
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn other_messages_are_handed_back() {
    let (events_tx, _events) = broadcast::channel(8);
    let mut receiver = FileReceiver::new(ReceiverConfig::default(), events_tx);
    let msg = FreeLifeMessage::HandoffUrl {
        url: "https://x".into(),
    };
    assert_eq!(receiver.handle(device_id(), msg.clone()).await, Some(msg));
}

#[tokio::test]
async fn sending_a_directory_or_missing_file_fails() {
    let src = tempfile::tempdir().unwrap();
    let h = harness(Link::InOrder, |_| {});
    assert!(matches!(
        collect_progress(&h.conn, src.path()).await,
        Err(TransferError::NotAFile(_))
    ));
    assert!(matches!(
        collect_progress(&h.conn, &src.path().join("nope")).await,
        Err(TransferError::Io(_))
    ));
}

#[tokio::test]
async fn closed_connection_fails_the_send() {
    let src = tempfile::tempdir().unwrap();
    let path = write_test_file(src.path(), "f", 10);
    let (tx, rx) = mpsc::channel(1);
    drop(rx);
    let conn = ActiveConnection::from_sender(device_id(), tx);
    assert!(matches!(
        collect_progress(&conn, &path).await,
        Err(TransferError::Core(freelife_core::Error::NotConnected(_)))
    ));
}

// ---- Full nodes over real TLS on localhost ----

struct TestNode {
    node: Arc<Node>,
    _data: tempfile::TempDir,
    downloads: tempfile::TempDir,
}

async fn start_node(name: &str) -> TestNode {
    let data = tempfile::tempdir().unwrap();
    let downloads = tempfile::tempdir().unwrap();
    let mut config = NodeConfig::new(
        name,
        OsType::Linux,
        Arc::new(MemoryKeyStore::default()),
        data.path(),
    );
    config.port = 0;
    config.enable_mdns = false;
    config.receiver.download_dir = downloads.path().to_path_buf();
    let node = Node::start(config).await.unwrap();

    // Stand-in for the user tapping "codes match".
    let mut prompts = node.subscribe_pairing();
    let approver = node.clone();
    tokio::spawn(async move {
        while let Ok(event) = prompts.recv().await {
            if let PromptEvent::Show(p) = event {
                approver.respond_pairing(p.request_id, true);
            }
        }
    });
    TestNode {
        node,
        _data: data,
        downloads,
    }
}

#[tokio::test]
async fn fifty_mb_between_paired_nodes_over_tls() {
    let phone = start_node("phone").await;
    let laptop = start_node("laptop").await;

    let peer = phone
        .node
        .pair_at(([127, 0, 0, 1], laptop.node.port()).into())
        .await
        .unwrap();
    assert_eq!(&peer, laptop.node.device());
    assert!(laptop.node.is_paired(phone.node.device().id));

    let src = tempfile::tempdir().unwrap();
    let path = write_test_file(src.path(), "holiday.mov", FIFTY_MB);
    let mut sent_events = phone.node.subscribe_transfers();
    let mut received_events = laptop.node.subscribe_transfers();

    let id = phone.node.send_file(peer.id, &path).await.unwrap();

    match final_event(&mut sent_events).await {
        TransferEvent::TransferComplete {
            id: done,
            direction,
            ..
        } => {
            assert_eq!(done, id);
            assert_eq!(direction, Direction::Send);
        }
        other => panic!("{other:?}"),
    }
    match final_event(&mut received_events).await {
        TransferEvent::TransferComplete {
            id: done,
            peer,
            path,
            ..
        } => {
            assert_eq!(done, id);
            assert_eq!(peer, phone.node.device().id);
            let path = path.unwrap();
            assert_eq!(path, laptop.downloads.path().join("holiday.mov"));
            assert_eq!(
                sha256_file(&path),
                sha256_file(&src.path().join("holiday.mov"))
            );
        }
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn node_refuses_to_send_to_unpaired_device() {
    let a = start_node("a").await;
    let src = tempfile::tempdir().unwrap();
    let path = write_test_file(src.path(), "f", 10);
    assert!(matches!(
        a.node.send_file(device_id(), &path).await,
        Err(TransferError::NotPaired(_))
    ));
}

#[tokio::test]
async fn non_transfer_messages_reach_the_app() {
    let a = start_node("a").await;
    let b = start_node("b").await;
    let peer = a
        .node
        .pair_at(([127, 0, 0, 1], b.node.port()).into())
        .await
        .unwrap();
    // Every subscriber sees every message.
    let mut first = b.node.subscribe_messages();
    let mut second = b.node.subscribe_messages();

    let msg = freelife_transfer::clipboard_text("hello from a");
    let delivered = a
        .node
        .send_to_many([peer.id], msg.clone(), Duration::from_secs(5))
        .await;
    assert_eq!(delivered, 1);
    for rx in [&mut first, &mut second] {
        let (from, got) = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(from, a.node.device().id);
        assert_eq!(got, msg);
    }
}

#[tokio::test]
async fn send_to_many_skips_unreachable_devices() {
    let a = start_node("a").await;
    let b = start_node("b").await;
    let peer = a
        .node
        .pair_at(([127, 0, 0, 1], b.node.port()).into())
        .await
        .unwrap();
    let mut rx = b.node.subscribe_messages();
    let stranger = device_id(); // not paired: fails fast

    let msg = freelife_transfer::clipboard_text("x");
    let delivered = a
        .node
        .send_to_many([peer.id, stranger], msg, Duration::from_secs(5))
        .await;
    assert_eq!(delivered, 1);
    assert!(rx.recv().await.is_ok());
}
