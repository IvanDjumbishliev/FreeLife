//! Tauri commands exposed to the React frontend (see `src/lib/api.ts`).
//!
//! Errors are returned as `String` so they serialize straight to the JS side.

use std::time::Duration;

use freelife_core::discovery::DiscoveredDevice;
use freelife_core::{DeviceInfo, OsType};
use freelife_transfer::{chunk_count, OutgoingFile};
use serde::Serialize;
use tauri::State;

use crate::AppState;

#[derive(Serialize)]
pub struct DeviceView {
    id: String,
    name: String,
    os: OsType,
    fingerprint: String,
}

impl From<&DeviceInfo> for DeviceView {
    fn from(info: &DeviceInfo) -> Self {
        Self {
            id: info.id.to_string(),
            name: info.name.clone(),
            os: info.os,
            fingerprint: info.fingerprint(),
        }
    }
}

#[derive(Serialize)]
pub struct Peer {
    device: DeviceView,
    addresses: Vec<String>,
    port: u16,
}

impl From<&DiscoveredDevice> for Peer {
    fn from(d: &DiscoveredDevice) -> Self {
        Self {
            device: DeviceView::from(&d.info),
            addresses: d.addresses.iter().map(ToString::to_string).collect(),
            port: d.port,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferReceipt {
    transfer_id: String,
    chunks: u64,
}

fn find_peer(state: &AppState, peer_id: &str) -> Result<DiscoveredDevice, String> {
    state
        .peers
        .lock()
        .map_err(|e| e.to_string())?
        .iter()
        .find(|p| p.info.id.to_string() == peer_id)
        .cloned()
        .ok_or_else(|| format!("unknown peer {peer_id}; scan again"))
}

#[tauri::command]
pub fn get_device_info(state: State<'_, AppState>) -> DeviceView {
    DeviceView::from(&state.device)
}

/// Browse the LAN for `timeout_ms` and return every FreeLife device seen.
#[tauri::command]
pub async fn discover_devices(
    state: State<'_, AppState>,
    timeout_ms: Option<u64>,
) -> Result<Vec<Peer>, String> {
    let discovery = state
        .discovery
        .as_ref()
        .ok_or("mDNS discovery is unavailable on this system")?;
    let mut rx = discovery
        .discover_devices()
        .await
        .map_err(|e| e.to_string())?;

    let mut found: Vec<DiscoveredDevice> = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_millis(timeout_ms.unwrap_or(2000));
    while let Ok(Some(device)) = tokio::time::timeout_at(deadline, rx.recv()).await {
        found.retain(|d| d.info.id != device.info.id);
        found.push(device);
    }
    // Dropping `rx` stops the browse.

    let peers = found.iter().map(Peer::from).collect();
    *state.peers.lock().map_err(|e| e.to_string())? = found;
    Ok(peers)
}

/// STUB: builds the clipboard message but doesn't send it yet.
#[tauri::command]
pub fn send_clipboard(
    state: State<'_, AppState>,
    peer_id: String,
    text: String,
) -> Result<(), String> {
    let peer = find_peer(&state, &peer_id)?;
    let _msg = freelife_transfer::clipboard_text(text);
    // TODO: pair, then `ConnectionManager::send(peer.info.id, _msg)`.
    eprintln!("[stub] would send clipboard to {}", peer.info.name);
    Ok(())
}

/// STUB: reads the file and prepares the chunked transfer, but doesn't send it.
#[tauri::command]
pub fn send_file(
    state: State<'_, AppState>,
    peer_id: String,
    path: String,
) -> Result<TransferReceipt, String> {
    let peer = find_peer(&state, &peer_id)?;
    let data = std::fs::read(&path).map_err(|e| format!("{path}: {e}"))?;
    let name = std::path::Path::new(&path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".into());

    let file = OutgoingFile::new(name, data);
    let chunks = chunk_count(file.size());
    // TODO: send `file.request()` then `file.chunks()` via the ConnectionManager.
    eprintln!(
        "[stub] would send {} ({chunks} chunks) to {}",
        file.filename, peer.info.name
    );
    Ok(TransferReceipt {
        transfer_id: file.id.to_string(),
        chunks,
    })
}
