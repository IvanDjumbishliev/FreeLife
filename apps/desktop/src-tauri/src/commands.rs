//! Tauri commands exposed to the React frontend (see `src/lib/api.ts`).
//!
//! Errors are returned as `String` so they serialize straight to the JS side.

use std::path::PathBuf;
use std::time::Duration;

use freelife_clipboard::ClipboardSettings;
use freelife_core::{DeviceId, DeviceInfo, OsType};
use serde::Serialize;
use tauri::State;

use crate::AppState;

#[derive(Clone, Serialize)]
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
pub struct NearbyDevice {
    device: DeviceView,
    addresses: Vec<String>,
    paired: bool,
}

#[derive(Serialize)]
pub struct PairedDevice {
    device: DeviceView,
    connected: bool,
}

fn parse_id(id: &str) -> Result<DeviceId, String> {
    id.parse().map_err(|_| format!("invalid device id {id:?}"))
}

#[tauri::command]
pub fn get_device_info(state: State<'_, AppState>) -> DeviceView {
    DeviceView::from(state.node.device())
}

/// Browse the LAN for `timeout_ms` and return every FreeLife device seen.
#[tauri::command]
pub async fn discover_devices(
    state: State<'_, AppState>,
    timeout_ms: Option<u64>,
) -> Result<Vec<NearbyDevice>, String> {
    let node = &state.node;
    if !node.mdns_available() {
        return Err("mDNS discovery is unavailable on this system".into());
    }
    let found = node
        .discover(Duration::from_millis(timeout_ms.unwrap_or(2000)))
        .await
        .map_err(|e| e.to_string())?;
    let views = found
        .iter()
        .map(|d| NearbyDevice {
            device: DeviceView::from(&d.info),
            addresses: d.socket_addrs().map(|a| a.to_string()).collect(),
            paired: node.is_paired(d.info.id),
        })
        .collect();
    *state.nearby.lock().map_err(|e| e.to_string())? = found;
    Ok(views)
}

#[tauri::command]
pub fn list_paired_devices(state: State<'_, AppState>) -> Vec<PairedDevice> {
    state
        .node
        .paired_devices()
        .iter()
        .map(|d| PairedDevice {
            device: DeviceView::from(&d.info()),
            connected: state.node.is_connected(d.id),
        })
        .collect()
}

/// Pair with a device from the last scan. Both sides show a code via the
/// `pairing-prompt` event; this resolves once both users have answered.
#[tauri::command]
pub async fn pair_device(
    state: State<'_, AppState>,
    device_id: String,
) -> Result<DeviceView, String> {
    let id = parse_id(&device_id)?;
    let peer = state.node.pair(id).await.map_err(|e| e.to_string())?;
    Ok(DeviceView::from(&peer))
}

#[tauri::command]
pub fn respond_pairing(state: State<'_, AppState>, request_id: u64, accept: bool) -> bool {
    state.node.respond_pairing(request_id, accept)
}

#[tauri::command]
pub async fn unpair_device(state: State<'_, AppState>, device_id: String) -> Result<bool, String> {
    let id = parse_id(&device_id)?;
    state.node.unpair(id).await.map_err(|e| e.to_string())
}

/// Send a file to a paired device. Progress arrives as `transfer-event`s;
/// this resolves when the whole file has been sent.
#[tauri::command]
pub async fn send_file(
    state: State<'_, AppState>,
    device_id: String,
    file_path: String,
) -> Result<(), String> {
    let id = parse_id(&device_id)?;
    state
        .node
        .send_file(id, &PathBuf::from(file_path))
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_clipboard_settings(state: State<'_, AppState>) -> ClipboardSettings {
    state.clipboard.settings()
}

#[tauri::command]
pub async fn set_clipboard_settings(
    state: State<'_, AppState>,
    settings: ClipboardSettings,
) -> Result<(), String> {
    state
        .clipboard
        .set_settings(settings)
        .await
        .map_err(|e| e.to_string())
}

/// Start using a paired phone's camera as a webcam. Stats and previews
/// arrive as `camera-event`s.
#[tauri::command]
pub async fn phone_webcam_start(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    device_id: String,
) -> Result<(), String> {
    let id = parse_id(&device_id)?;
    let mut webcam = state.webcam.lock().await;
    if webcam.is_some() {
        return Err("the phone camera is already in use".into());
    }
    let session = crate::webcam::start(app, state.node.clone(), id).await?;
    *webcam = Some(session);
    Ok(())
}

#[tauri::command]
pub async fn phone_webcam_stop(state: State<'_, AppState>) -> Result<(), String> {
    let session = state.webcam.lock().await.take();
    if let Some(session) = session {
        crate::webcam::stop(&state.node, session).await;
    }
    Ok(())
}

/// The device whose camera is in use, if any.
#[tauri::command]
pub async fn phone_webcam_status(state: State<'_, AppState>) -> Result<Option<String>, String> {
    Ok(state
        .webcam
        .lock()
        .await
        .as_ref()
        .map(|s| s.device.to_string()))
}
