mod commands;
mod events;
mod handoff_ipc;
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
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{Emitter, Manager};

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
    pub feature_settings: Arc<Mutex<FeatureSettings>>,
    pub feature_settings_path: PathBuf,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FeatureSettings {
    pub airshare_enabled: bool,
    pub webcam_enabled: bool,
    pub handoff_enabled: bool,
}

impl Default for FeatureSettings {
    fn default() -> Self {
        Self {
            airshare_enabled: true,
            webcam_enabled: true,
            handoff_enabled: true,
        }
    }
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationSettings {
    #[serde(default = "notifications_enabled_by_default")]
    pub enabled: bool,
    #[serde(default)]
    pub blocked_apps: Vec<String>,
}

fn load_feature_settings(path: &Path) -> Result<FeatureSettings, Box<dyn std::error::Error>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(FeatureSettings::default())
        }
        Err(error) => Err(error.into()),
    }
}

impl Default for NotificationSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            blocked_apps: Vec::new(),
        }
    }
}

fn notifications_enabled_by_default() -> bool {
    true
}

fn build_tray(
    app: &tauri::App,
    features: &FeatureSettings,
    clipboard_enabled: bool,
    notifications_enabled: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let open = MenuItem::with_id(app, "open", "Open FreeLife", true, None::<&str>)?;
    let devices = MenuItem::with_id(app, "devices", "Paired devices", true, None::<&str>)?;
    let feature_separator = PredefinedMenuItem::separator(app)?;
    let quit_separator = PredefinedMenuItem::separator(app)?;
    let airshare = CheckMenuItem::with_id(
        app,
        "airshare",
        "AirShare",
        true,
        features.airshare_enabled,
        None::<&str>,
    )?;
    let clipboard = CheckMenuItem::with_id(
        app,
        "clipboard",
        "Clipboard",
        true,
        clipboard_enabled,
        None::<&str>,
    )?;
    let webcam = CheckMenuItem::with_id(
        app,
        "webcam",
        "Webcam",
        true,
        features.webcam_enabled,
        None::<&str>,
    )?;
    let notifications = CheckMenuItem::with_id(
        app,
        "notifications",
        "Notifications",
        true,
        notifications_enabled,
        None::<&str>,
    )?;
    let handoff = CheckMenuItem::with_id(
        app,
        "handoff",
        "Handoff",
        true,
        features.handoff_enabled,
        None::<&str>,
    )?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &open,
            &devices,
            &feature_separator,
            &airshare,
            &clipboard,
            &webcam,
            &notifications,
            &handoff,
            &quit_separator,
            &quit,
        ],
    )?;
    let icon = app
        .default_window_icon()
        .cloned()
        .ok_or_else(|| std::io::Error::other("the FreeLife tray icon is missing"))?;
    TrayIconBuilder::with_id("main")
        .icon(icon)
        .tooltip("FreeLife · 0 connected")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" | "devices" => {
                if let Some(window) = app.get_webview_window("main") {
                    if let Err(error) = window.show() {
                        tracing::warn!(%error, "could not show the FreeLife window");
                    }
                    if let Err(error) = window.set_focus() {
                        tracing::warn!(%error, "could not focus the FreeLife window");
                    }
                    if event.id().as_ref() == "devices" {
                        if let Err(error) = app.emit("focus-paired-devices", ()) {
                            tracing::warn!(%error, "could not focus paired devices panel");
                        }
                    }
                }
            }
            "quit" => app.exit(0),
            "airshare" | "clipboard" | "webcam" | "notifications" | "handoff" => {
                if let Err(error) = app.emit("tray-toggle-feature", event.id().as_ref()) {
                    tracing::warn!(%error, "could not publish tray feature toggle");
                }
            }
            _ => {}
        })
        .build(app)?;
    Ok(())
}

fn start_tray_status_updates(app: tauri::AppHandle, node: Arc<Node>) {
    tauri::async_runtime::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(5));
        loop {
            tick.tick().await;
            let connected = node
                .paired_devices()
                .iter()
                .filter(|device| node.is_connected(device.id))
                .count();
            if let Some(tray) = app.tray_by_id("main") {
                let tooltip = format!("FreeLife · {connected} connected");
                if let Err(error) = tray.set_tooltip(Some(&tooltip)) {
                    tracing::warn!(%error, "could not update tray connection count");
                }
            }
        }
    });
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

#[cfg(test)]
mod settings_tests {
    use super::{FeatureSettings, NotificationSettings};

    #[test]
    fn feature_settings_default_to_enabled_for_existing_installations() {
        let settings: FeatureSettings = serde_json::from_str("{}").unwrap();
        assert!(settings.airshare_enabled);
        assert!(settings.webcam_enabled);
        assert!(settings.handoff_enabled);
    }

    #[test]
    fn legacy_notification_settings_keep_notifications_enabled() {
        let settings: NotificationSettings =
            serde_json::from_str(r#"{"blockedApps":["example.app"]}"#).unwrap();
        assert!(settings.enabled);
        assert_eq!(settings.blocked_apps, vec!["example.app".to_owned()]);
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
            let feature_settings_path = data_dir.join("features.json");
            let feature_settings = Arc::new(Mutex::new(load_feature_settings(
                &feature_settings_path,
            )?));
            // Every desktop target is covered; other Unixes are closest to Linux.
            let os = OsType::current().unwrap_or(OsType::Linux);
            let config = NodeConfig::new(device_name(), os, key_store(&data_dir), data_dir.clone());
            let node = tauri::async_runtime::block_on(Node::start(config))?;
            handoff_ipc::start(
                &data_dir,
                node.clone(),
                app.handle().clone(),
                feature_settings.clone(),
            )?;
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
            let clipboard_enabled = clipboard.settings().enabled;
            let notifications_enabled = notification_settings
                .lock()
                .map_err(|error| std::io::Error::other(error.to_string()))?
                .enabled;
            let feature_snapshot = feature_settings
                .lock()
                .map_err(|error| std::io::Error::other(error.to_string()))?
                .clone();

            events::forward(
                app.handle().clone(),
                node.clone(),
                clipboard.clone(),
                notification_settings.clone(),
                feature_settings.clone(),
            );
            app.manage(AppState {
                node: node.clone(),
                clipboard,
                clipboard_watcher: Mutex::new(Some(watcher)),
                nearby: Mutex::new(Vec::new()),
                webcam: tokio::sync::Mutex::new(None),
                notification_settings,
                notification_settings_path,
                feature_settings,
                feature_settings_path,
            });
            build_tray(
                app,
                &feature_snapshot,
                clipboard_enabled,
                notifications_enabled,
            )?;
            start_tray_status_updates(app.handle().clone(), node);
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
            commands::get_feature_settings,
            commands::set_feature_settings,
            commands::send_notification_reply,
            commands::dismiss_notification,
            commands::phone_webcam_start,
            commands::phone_webcam_stop,
            commands::phone_webcam_status,
        ])
        .run(tauri::generate_context!())
        .expect("error while running FreeLife desktop");
}
