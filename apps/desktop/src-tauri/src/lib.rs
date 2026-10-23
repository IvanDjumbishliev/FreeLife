mod commands;

use std::sync::Mutex;

use freelife_core::{DeviceInfo, Platform, DEFAULT_PORT};

/// State shared by all commands.
pub struct AppState {
    pub device: DeviceInfo,
    /// Peers from the most recent scan, used to resolve `peer_id` arguments.
    pub peers: Mutex<Vec<freelife_discovery::DiscoveredDevice>>,
}

fn device_name() -> String {
    std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .unwrap_or_else(|_| "FreeLife Desktop".into())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let state = AppState {
        device: DeviceInfo::new(device_name(), Platform::current(), DEFAULT_PORT),
        peers: Mutex::new(Vec::new()),
    };

    tauri::Builder::default()
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            commands::get_device_info,
            commands::discover_devices,
            commands::send_clipboard,
            commands::send_file,
        ])
        .run(tauri::generate_context!())
        .expect("error while running FreeLife desktop");
}
