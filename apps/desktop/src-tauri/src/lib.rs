mod commands;

use std::sync::{Arc, Mutex};

use freelife_core::discovery::{DiscoveredDevice, Discovery};
use freelife_core::identity::{Identity, KeyStore, KeyringStore};
use freelife_core::{DeviceInfo, OsType};

/// State shared by all commands.
pub struct AppState {
    pub identity: Arc<Identity>,
    pub device: DeviceInfo,
    /// `None` if the mDNS daemon couldn't start (e.g. no usable network interface).
    pub discovery: Option<Discovery>,
    /// Peers from the most recent scan, used to resolve `peer_id` arguments.
    pub peers: Mutex<Vec<DiscoveredDevice>>,
}

fn device_name() -> String {
    std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .unwrap_or_else(|_| "FreeLife Desktop".into())
}

fn load_identity() -> Identity {
    let store: Arc<dyn KeyStore> = Arc::new(KeyringStore::default());
    tauri::async_runtime::block_on(Identity::load_or_generate(store)).unwrap_or_else(|e| {
        eprintln!("warning: OS keychain unavailable ({e}); using a temporary identity");
        Identity::generate().expect("system RNG unavailable")
    })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let identity = Arc::new(load_identity());
    // Every desktop target is covered; other Unixes are closest to Linux.
    let os = OsType::current().unwrap_or(OsType::Linux);
    let device = identity.device_info(device_name(), os);
    let discovery = Discovery::new()
        .inspect_err(|e| eprintln!("warning: mDNS unavailable: {e}"))
        .ok();
    // TODO: start a ConnectionManager listener and `discovery.advertise()` it
    // once the pairing UI exists. Until then we only browse.

    let state = AppState {
        identity,
        device,
        discovery,
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
