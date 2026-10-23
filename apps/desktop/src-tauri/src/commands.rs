//! Tauri commands exposed to the React frontend (see `src/lib/api.ts`).
//!
//! Errors are returned as `String` so they serialize straight to the JS side.

use std::time::Duration;

use freelife_core::DeviceInfo;
use freelife_discovery::{DiscoveredDevice, Discovery};
use freelife_transfer::{chunk_count, OutgoingFile};
use serde::Serialize;
use tauri::State;

use crate::AppState;

#[derive(Serialize)]
pub struct Peer {
    info: DeviceInfo,
    addresses: Vec<String>,
}

impl From<&DiscoveredDevice> for Peer {
    fn from(d: &DiscoveredDevice) -> Self {
        Self {
            info: d.info.clone(),
            addresses: d.addresses.iter().map(ToString::to_string).collect(),
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
pub fn get_device_info(state: State<'_, AppState>) -> DeviceInfo {
    state.device.clone()
}

/// Browse the LAN for other FreeLife devices. Runs on a blocking thread
/// because mDNS scanning waits for `timeout_ms`.
#[tauri::command]
pub async fn discover_devices(
    state: State<'_, AppState>,
    timeout_ms: Option<u64>,
) -> Result<Vec<Peer>, String> {
    let local = state.device.clone();
    let timeout = Duration::from_millis(timeout_ms.unwrap_or(2000));

    let found = tauri::async_runtime::spawn_blocking(move || {
        let discovery = Discovery::new(local)?;
        let found = discovery.scan(timeout);
        discovery.shutdown()?;
        found
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())?;

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
    let _msg = freelife_transfer::clipboard_message(text);
    // TODO: open a connection to `peer` and write `encode_frame(&_msg)`.
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
    // TODO: send `file.offer()` then `file.messages()` to `peer`.
    eprintln!(
        "[stub] would send {} ({chunks} chunks) to {}",
        file.name, peer.info.name
    );
    Ok(TransferReceipt {
        transfer_id: file.transfer_id,
        chunks,
    })
}
