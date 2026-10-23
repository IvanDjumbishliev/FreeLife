//! The FreeLife node, as seen from Dart.
//!
//! One [`Node`] per process, started by [`start_node`]. Everything async runs
//! on a dedicated tokio runtime, so it doesn't depend on which executor
//! flutter_rust_bridge uses for async calls.

use std::future::Future;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock, OnceLock};
use std::time::Duration;

use anyhow::{anyhow, Context};
use freelife_core::identity::FileKeyStore;
use freelife_core::prompts::PromptEvent;
use freelife_core::{DeviceId, DeviceInfo, OsType};
use freelife_transfer::node::{Node, NodeConfig};
use freelife_transfer::{Direction, TransferEvent};
use tokio::sync::broadcast::error::RecvError;

use crate::frb_generated::StreamSink;

static RUNTIME: LazyLock<tokio::runtime::Runtime> = LazyLock::new(|| {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_name("freelife")
        .enable_all()
        .build()
        .expect("failed to start tokio runtime")
});

static NODE: OnceLock<Arc<Node>> = OnceLock::new();

fn node() -> anyhow::Result<&'static Arc<Node>> {
    NODE.get()
        .ok_or_else(|| anyhow!("FreeLife has not been started"))
}

/// Run `fut` on our runtime and wait for it from whatever executor called us.
async fn on_runtime<F>(fut: F) -> F::Output
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    RUNTIME.spawn(fut).await.expect("FreeLife task panicked")
}

fn parse_id(id: &str) -> anyhow::Result<DeviceId> {
    id.parse().map_err(|_| anyhow!("invalid device id {id:?}"))
}

pub struct DeviceSummary {
    pub id: String,
    pub name: String,
    pub os: String,
    /// Short hash of the identity key, for users to compare.
    pub fingerprint: String,
}

impl From<&DeviceInfo> for DeviceSummary {
    fn from(info: &DeviceInfo) -> Self {
        Self {
            id: info.id.to_string(),
            name: info.name.clone(),
            os: format!("{:?}", info.os),
            fingerprint: info.fingerprint(),
        }
    }
}

pub struct PairedDevice {
    pub device: DeviceSummary,
    pub connected: bool,
}

pub struct NearbyDevice {
    pub device: DeviceSummary,
    pub paired: bool,
}

// Numbers are i64 (not u64) because flutter_rust_bridge maps i64 to a Dart
// `int` but u64 to `BigInt`.

/// A pairing code to show (`code` set) or a prompt to close (`code` unset).
pub struct PairingUpdate {
    pub request_id: i64,
    pub device: Option<DeviceSummary>,
    pub code: Option<String>,
}

pub enum TransferKind {
    Progress,
    Complete,
    Failed,
}

pub enum TransferDirection {
    Send,
    Receive,
}

pub struct TransferUpdate {
    pub id: String,
    pub kind: TransferKind,
    pub direction: TransferDirection,
    pub peer_id: String,
    pub peer_name: String,
    pub filename: String,
    pub bytes_sent: i64,
    pub total_bytes: i64,
    pub speed_bps: i64,
    pub eta_secs: Option<i64>,
    /// Where a received file was saved.
    pub saved_path: Option<String>,
    pub error: Option<String>,
}

fn peer_name(node: &Node, peer: DeviceId) -> String {
    node.paired_devices()
        .into_iter()
        .find(|d| d.id == peer)
        .map(|d| d.name)
        .unwrap_or_else(|| "Unknown device".into())
}

fn direction(d: Direction) -> TransferDirection {
    match d {
        Direction::Send => TransferDirection::Send,
        Direction::Receive => TransferDirection::Receive,
    }
}

fn int(n: u64) -> i64 {
    i64::try_from(n).unwrap_or(i64::MAX)
}

fn to_update(node: &Node, event: TransferEvent) -> TransferUpdate {
    match event {
        TransferEvent::Progress(p) => TransferUpdate {
            id: p.id.to_string(),
            kind: TransferKind::Progress,
            direction: direction(p.direction),
            peer_id: p.peer.to_string(),
            peer_name: peer_name(node, p.peer),
            filename: p.filename,
            bytes_sent: int(p.bytes_sent),
            total_bytes: int(p.total_bytes),
            speed_bps: int(p.speed_bps),
            eta_secs: p.eta_secs.map(int),
            saved_path: None,
            error: None,
        },
        TransferEvent::TransferComplete {
            id,
            peer,
            direction: d,
            filename,
            total_bytes,
            path,
        } => TransferUpdate {
            id: id.to_string(),
            kind: TransferKind::Complete,
            direction: direction(d),
            peer_id: peer.to_string(),
            peer_name: peer_name(node, peer),
            filename,
            bytes_sent: int(total_bytes),
            total_bytes: int(total_bytes),
            speed_bps: 0,
            eta_secs: Some(0),
            saved_path: path.map(|p| p.to_string_lossy().into_owned()),
            error: None,
        },
        TransferEvent::TransferError {
            id,
            peer,
            direction: d,
            filename,
            error,
        } => TransferUpdate {
            id: id.to_string(),
            kind: TransferKind::Failed,
            direction: direction(d),
            peer_id: peer.to_string(),
            peer_name: peer_name(node, peer),
            filename,
            bytes_sent: 0,
            total_bytes: 0,
            speed_bps: 0,
            eta_secs: None,
            saved_path: None,
            error: Some(error),
        },
    }
}

#[flutter_rust_bridge::frb(init)]
pub fn init_app() {
    flutter_rust_bridge::setup_default_user_utils();
}

/// Start the node: load (or create) the identity in `data_dir`, listen for
/// peers, and advertise via mDNS. Received files go to `download_dir`.
/// Calling it again returns the running node's device.
pub async fn start_node(
    data_dir: String,
    download_dir: String,
    device_name: String,
) -> anyhow::Result<DeviceSummary> {
    if let Some(node) = NODE.get() {
        return Ok(DeviceSummary::from(node.device()));
    }
    on_runtime(async move {
        let data_dir = PathBuf::from(data_dir);
        let key_store = Arc::new(FileKeyStore::new(data_dir.join(FileKeyStore::DEFAULT_FILE)));
        let os = OsType::current().unwrap_or(OsType::Android);
        let mut config = NodeConfig::new(device_name, os, key_store, data_dir);
        config.receiver.download_dir = PathBuf::from(download_dir);
        let node = Node::start(config).await.context("starting FreeLife")?;
        let node = NODE.get_or_init(|| node);
        Ok(DeviceSummary::from(node.device()))
    })
    .await
}

#[flutter_rust_bridge::frb(sync)]
pub fn paired_devices() -> anyhow::Result<Vec<PairedDevice>> {
    let node = node()?;
    Ok(node
        .paired_devices()
        .iter()
        .map(|d| PairedDevice {
            device: DeviceSummary::from(&d.info()),
            connected: node.is_connected(d.id),
        })
        .collect())
}

pub async fn discover_devices(timeout_ms: u32) -> anyhow::Result<Vec<NearbyDevice>> {
    let node = node()?.clone();
    on_runtime(async move {
        let found = node
            .discover(Duration::from_millis(timeout_ms.into()))
            .await?;
        Ok(found
            .iter()
            .map(|d| NearbyDevice {
                device: DeviceSummary::from(&d.info),
                paired: node.is_paired(d.info.id),
            })
            .collect())
    })
    .await
}

/// Pair with a device from the last discovery. Codes arrive on
/// [`pairing_events`]; resolves once both users have answered.
pub async fn pair_device(device_id: String) -> anyhow::Result<DeviceSummary> {
    let node = node()?.clone();
    let id = parse_id(&device_id)?;
    on_runtime(async move { Ok(DeviceSummary::from(&node.pair(id).await?)) }).await
}

#[flutter_rust_bridge::frb(sync)]
pub fn respond_pairing(request_id: i64, accept: bool) -> anyhow::Result<bool> {
    let request_id = u64::try_from(request_id).context("negative request id")?;
    Ok(node()?.respond_pairing(request_id, accept))
}

pub async fn unpair_device(device_id: String) -> anyhow::Result<bool> {
    let node = node()?.clone();
    let id = parse_id(&device_id)?;
    on_runtime(async move { Ok(node.unpair(id).await?) }).await
}

/// Pairing prompts (incoming and outgoing) for the UI.
pub fn pairing_events(sink: StreamSink<PairingUpdate>) -> anyhow::Result<()> {
    let mut rx = node()?.subscribe_pairing();
    RUNTIME.spawn(async move {
        loop {
            let update = match rx.recv().await {
                Ok(PromptEvent::Show(p)) => PairingUpdate {
                    request_id: int(p.request_id),
                    device: Some(DeviceSummary::from(&p.peer)),
                    code: Some(p.code.to_string()),
                },
                Ok(PromptEvent::Dismiss { request_id }) => PairingUpdate {
                    request_id: int(request_id),
                    device: None,
                    code: None,
                },
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => break,
            };
            if sink.add(update).is_err() {
                break; // Dart cancelled the stream
            }
        }
    });
    Ok(())
}

/// Every transfer, in both directions.
pub fn transfer_events(sink: StreamSink<TransferUpdate>) -> anyhow::Result<()> {
    let node = node()?.clone();
    let mut rx = node.subscribe_transfers();
    RUNTIME.spawn(async move {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    if sink.add(to_update(&node, event)).is_err() {
                        break;
                    }
                }
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => break,
            }
        }
    });
    Ok(())
}

/// Send `path` to a paired device. The stream carries this transfer's
/// updates and closes after its `Complete` or `Failed` update.
pub fn send_file(
    device_id: String,
    path: String,
    sink: StreamSink<TransferUpdate>,
) -> anyhow::Result<()> {
    let node = node()?.clone();
    let id = parse_id(&device_id)?;
    let transfer_id = freelife_transfer::new_transfer_id();
    let mut rx = node.subscribe_transfers();

    let forwarder_node = node.clone();
    RUNTIME.spawn(async move {
        loop {
            match rx.recv().await {
                Ok(event) if event.id() == transfer_id => {
                    let done = !matches!(event, TransferEvent::Progress(_));
                    if sink.add(to_update(&forwarder_node, event)).is_err() || done {
                        break;
                    }
                }
                Ok(_) | Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => break,
            }
        }
    });
    RUNTIME.spawn(async move {
        // Failures are reported through the stream as a `Failed` update.
        let _ = node
            .send_file_as(transfer_id, id, &PathBuf::from(path))
            .await;
    });
    Ok(())
}
