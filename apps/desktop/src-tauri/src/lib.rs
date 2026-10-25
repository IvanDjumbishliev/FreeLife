mod commands;
mod events;
mod webcam;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use freelife_clipboard::{
    ArboardBackend, ClipboardSync, ClipboardWatcher, WatcherConfig, SETTINGS_FILE,
};
use freelife_core::discovery::DiscoveredDevice;
use freelife_core::identity::{FileKeyStore, KeyStore, KeyringStore};
use freelife_core::OsType;
use freelife_transfer::node::{Node, NodeConfig};
use tauri::Manager;

/// State shared by all commands.
pub struct AppState {
    pub node: Arc<Node>,
    pub clipboard: Arc<ClipboardSync>,
    /// Kept alive for the app's lifetime; dropping it stops clipboard polling.
    pub clipboard_watcher: Mutex<Option<ClipboardWatcher>>,
    /// Devices from the most recent scan.
    pub nearby: Mutex<Vec<DiscoveredDevice>>,
    /// The phone whose camera is currently being used as a webcam, if any.
    pub webcam: tokio::sync::Mutex<Option<webcam::WebcamSession>>,
    pub notification_settings: Arc<Mutex<NotificationSettings>>,
    pub notification_settings_path: PathBuf,
}

#[derive(Clone, Debug, Default, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationSettings {
    pub blocked_apps: Vec<String>,
}

fn load_notification_settings(
    path: &Path,
) -> Result<NotificationSettings, Box<dyn std::error::Error>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(NotificationSettings::default())
        }
        Err(error) => Err(error.into()),
    }
}

fn device_name() -> String {
    std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .unwrap_or_else(|_| "FreeLife Desktop".into())
}

/// The OS keychain if it works here, else a private file in the app data
/// directory. A keychain can be missing on Linux without a Secret Service.
fn key_store(data_dir: &Path) -> Arc<dyn KeyStore> {
    let keyring = KeyringStore::default();
    match keyring.load() {
        Ok(_) => Arc::new(keyring),
        Err(e) => {
            tracing::warn!(error = %e, "OS keychain unavailable; storing the identity key in the app data directory");
            Arc::new(FileKeyStore::new(data_dir.join(FileKeyStore::DEFAULT_FILE)))
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,mdns_sd=warn".into()),
        )
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            let notification_settings_path = data_dir.join("notifications.json");
            let notification_settings = Arc::new(Mutex::new(load_notification_settings(
                &notification_settings_path,
            )?));
            // Every desktop target is covered; other Unixes are closest to Linux.
            let os = OsType::current().unwrap_or(OsType::Linux);
            let config = NodeConfig::new(device_name(), os, key_store(&data_dir), data_dir.clone());
            let node = tauri::async_runtime::block_on(Node::start(config))?;
            tracing::info!(
                port = node.port(),
                mdns = node.mdns_available(),
                "FreeLife node started"
            );

            // Clipboard sync starts with the app: it watches the clipboard and
            // writes what paired devices send.
            let clipboard = tauri::async_runtime::block_on(ClipboardSync::new(
                node.clone(),
                Some(data_dir.join(SETTINGS_FILE)),
            ));
            let watcher = ClipboardWatcher::spawn(
                clipboard.clone(),
                ArboardBackend::new,
                WatcherConfig::default(),
            )?;

            events::forward(
                app.handle().clone(),
                node.clone(),
                clipboard.clone(),
                notification_settings.clone(),
            );
            app.manage(AppState {
                node,
                clipboard,
                clipboard_watcher: Mutex::new(Some(watcher)),
                nearby: Mutex::new(Vec::new()),
                webcam: tokio::sync::Mutex::new(None),
                notification_settings,
                notification_settings_path,
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_device_info,
            commands::discover_devices,
            commands::list_paired_devices,
            commands::pair_device,
            commands::respond_pairing,
            commands::unpair_device,
            commands::send_file,
            commands::get_clipboard_settings,
            commands::set_clipboard_settings,
            commands::get_notification_settings,
            commands::set_notification_settings,
            commands::send_notification_reply,
            commands::dismiss_notification,
            commands::phone_webcam_start,
            commands::phone_webcam_stop,
            commands::phone_webcam_status,
        ])
        .run(tauri::generate_context!())
        .expect("error while running FreeLife desktop");
}
