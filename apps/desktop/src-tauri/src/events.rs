//! Push node and clipboard activity to the frontend as Tauri events, and
//! raise native notifications for received files.

use std::sync::Arc;
use std::sync::Mutex;

use base64::Engine;
use freelife_clipboard::{ClipboardEvent, ClipboardSync};
use freelife_core::prompts::PromptEvent;
use freelife_core::{DeviceId, FreeLifeMessage};
use freelife_transfer::node::Node;
use freelife_transfer::{Direction, TransferEvent};
use serde::Serialize;
use tauri::{AppHandle, Emitter};
#[cfg(not(target_os = "linux"))]
use tauri_plugin_notification::NotificationExt;
use tokio::sync::broadcast::error::RecvError;

use crate::commands::DeviceView;
use crate::NotificationSettings;

pub const TRANSFER_EVENT: &str = "transfer-event";
pub const PAIRING_PROMPT: &str = "pairing-prompt";
pub const PAIRING_DISMISS: &str = "pairing-dismiss";
pub const CLIPBOARD_EVENT: &str = "clipboard-event";
pub const NOTIFICATION_EVENT: &str = "notification-event";
pub const HANDOFF_EVENT: &str = "handoff-event";

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct NotificationView {
    id: String,
    device_id: String,
    app_package: String,
    app_name: String,
    title: String,
    body: String,
    timestamp: u64,
    icon_data: Option<String>,
    has_reply_action: bool,
    reply_token: Option<String>,
    removed: bool,
    blocked: bool,
}

impl NotificationView {
    fn removed(device_id: DeviceId, id: String) -> Self {
        Self {
            id,
            device_id: device_id.to_string(),
            app_package: String::new(),
            app_name: String::new(),
            title: String::new(),
            body: String::new(),
            timestamp: 0,
            icon_data: None,
            has_reply_action: false,
            reply_token: None,
            removed: true,
            blocked: false,
        }
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PromptView {
    request_id: u64,
    device: DeviceView,
    code: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DismissView {
    request_id: u64,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct HandoffView {
    url: String,
    timestamp: u64,
    source_device_id: String,
    source_device_name: String,
}

pub fn record_local_handoff(app: &AppHandle, node: &Node, url: String) {
    let device = node.device();
    let view = HandoffView {
        url,
        timestamp: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .try_into()
            .unwrap_or(u64::MAX),
        source_device_id: device.id.to_string(),
        source_device_name: device.name.clone(),
    };
    if let Err(error) = app.emit(HANDOFF_EVENT, view) {
        tracing::warn!(%error, "could not publish handoff history event");
    }
}

pub fn forward(
    app: AppHandle,
    node: Arc<Node>,
    clipboard: Arc<ClipboardSync>,
    notification_settings: Arc<Mutex<NotificationSettings>>,
) {
    let mut transfers = node.subscribe_transfers();
    let (a, n) = (app.clone(), node.clone());
    tauri::async_runtime::spawn(async move {
        loop {
            match transfers.recv().await {
                Ok(event) => {
                    notify_if_received(&a, &n, &event);
                    let _ = a.emit(TRANSFER_EVENT, &event);
                }
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => break,
            }
        }
    });

    let mut prompts = node.subscribe_pairing();
    let a = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            match prompts.recv().await {
                Ok(PromptEvent::Show(p)) => {
                    let view = PromptView {
                        request_id: p.request_id,
                        device: DeviceView::from(&p.peer),
                        code: p.code.to_string(),
                    };
                    let _ = a.emit(PAIRING_PROMPT, view);
                }
                Ok(PromptEvent::Dismiss { request_id }) => {
                    let _ = a.emit(PAIRING_DISMISS, DismissView { request_id });
                }
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => break,
            }
        }
    });

    let mut clips = clipboard.subscribe();
    let a = app;
    tauri::async_runtime::spawn(async move {
        loop {
            match clips.recv().await {
                Ok(event) => {
                    // The frontend only needs the summary, never the content.
                    let _ = a.emit(CLIPBOARD_EVENT, &event);
                    if let ClipboardEvent::Skipped { kind, reason, .. } = &event {
                        tracing::info!(
                            ?kind,
                            reason = reason.describe(),
                            "clipboard content kept local"
                        );
                    }
                }
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => break,
            }
        }
    });

    let mut messages = node.subscribe_messages();
    let notification_node = node.clone();
    let a = app;
    tauri::async_runtime::spawn(async move {
        loop {
            match messages.recv().await {
                Ok((
                    device_id,
                    FreeLifeMessage::NotificationEvent {
                        id,
                        app_package,
                        app_name,
                        title,
                        body,
                        timestamp,
                        icon_bytes,
                        has_reply_action,
                        reply_token,
                        removed,
                    },
                )) => {
                    let blocked =
                        !removed && notification_is_blocked(&notification_settings, &app_package);
                    let view = NotificationView {
                        id,
                        device_id: device_id.to_string(),
                        app_package,
                        app_name,
                        title,
                        body,
                        timestamp,
                        icon_data: icon_bytes.map(|bytes| {
                            format!(
                                "data:image/png;base64,{}",
                                base64::engine::general_purpose::STANDARD.encode(bytes)
                            )
                        }),
                        has_reply_action,
                        reply_token,
                        removed,
                        blocked,
                    };
                    if !removed && !blocked {
                        show_mirrored_notification(&a, notification_node.clone(), device_id, &view);
                    }
                    let _ = a.emit(NOTIFICATION_EVENT, &view);
                }
                Ok((device_id, FreeLifeMessage::NotificationDismiss { id })) => {
                    let _ = a.emit(NOTIFICATION_EVENT, NotificationView::removed(device_id, id));
                }
                Ok((device_id, FreeLifeMessage::HandoffUrl { url })) => {
                    let parsed = match url::Url::parse(&url) {
                        Ok(parsed)
                            if matches!(parsed.scheme(), "http" | "https")
                                && parsed.host_str().is_some() =>
                        {
                            parsed
                        }
                        _ => {
                            tracing::warn!(%device_id, "ignored invalid handoff URL");
                            continue;
                        }
                    };
                    let source_device_name = notification_node
                        .paired_devices()
                        .into_iter()
                        .find(|device| device.id == device_id)
                        .map(|device| device.name)
                        .unwrap_or_else(|| "Unknown device".into());
                    let view = HandoffView {
                        url: url.clone(),
                        timestamp: std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_millis()
                            .try_into()
                            .unwrap_or(u64::MAX),
                        source_device_id: device_id.to_string(),
                        source_device_name: source_device_name.clone(),
                    };
                    notify_handoff(&a, &source_device_name, parsed.host_str().unwrap_or_default());
                    if let Err(error) = a.emit(HANDOFF_EVENT, view) {
                        tracing::warn!(%error, "could not publish handoff event");
                    }
                    std::thread::spawn(move || {
                        if let Err(error) = open::that(&url) {
                            tracing::warn!(%error, %url, "could not open handed-off URL");
                        }
                    });
                }
                Ok(_) | Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => break,
            }
        }
    });
}

fn notify_handoff(app: &AppHandle, source: &str, domain: &str) {
    let title = format!("Handoff from {source}: {domain}");
    let body = "Opening in browser.";
    #[cfg(target_os = "linux")]
    let result = notify_rust::Notification::new()
        .appname("FreeLife")
        .summary(&title)
        .body(&body)
        .show()
        .map(|_| ());
    #[cfg(not(target_os = "linux"))]
    let result = app.notification().builder().title(title).body(body).show();
    if let Err(error) = result {
        tracing::warn!(%error, "could not show handoff notification");
    }
    let _ = app;
}

fn notification_is_blocked(settings: &Mutex<NotificationSettings>, app: &str) -> bool {
    match settings.lock() {
        Ok(settings) => settings.blocked_apps.iter().any(|blocked| blocked == app),
        Err(error) => {
            tracing::error!(%error, "could not read notification settings");
            true
        }
    }
}

fn show_mirrored_notification(
    app: &AppHandle,
    node: Arc<Node>,
    device_id: DeviceId,
    event: &NotificationView,
) {
    #[cfg(target_os = "linux")]
    {
        match notify_rust::Notification::new()
            .appname("FreeLife")
            .summary(&format!("{}: {}", event.app_name, event.title))
            .body(&event.body)
            .show()
        {
            Ok(handle) => {
                let notification_id = event.id.clone();
                let notification_node = node.clone();
                let notification_app = app.clone();
                std::thread::spawn(move || {
                    handle.on_close(move |reason| {
                        if reason == notify_rust::CloseReason::Dismissed {
                            let app = notification_app.clone();
                            let node = notification_node.clone();
                            let notification_id = notification_id.clone();
                            let _ = app.emit(
                                NOTIFICATION_EVENT,
                                NotificationView::removed(device_id, notification_id.clone()),
                            );
                            tauri::async_runtime::spawn(async move {
                                match node.connection_to(device_id).await {
                                    Ok(connection) => {
                                        if let Err(error) = connection
                                            .send(FreeLifeMessage::NotificationDismiss {
                                                id: notification_id,
                                            })
                                            .await
                                        {
                                            tracing::warn!(
                                                %error,
                                                "could not mirror notification dismissal"
                                            );
                                        }
                                    }
                                    Err(error) => tracing::warn!(
                                        %error,
                                        "could not connect to mirror notification dismissal"
                                    ),
                                }
                            });
                        }
                    });
                });
            }
            Err(error) => tracing::warn!(%error, "could not show Linux notification"),
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        if let Err(error) = app
            .notification()
            .builder()
            .title(format!("{}: {}", event.app_name, event.title))
            .body(&event.body)
            .show()
        {
            tracing::warn!(%error, "could not show native notification");
        }
    }
    let _ = app;
    let _ = node;
    let _ = device_id;
}

fn notify_if_received(app: &AppHandle, node: &Node, event: &TransferEvent) {
    let TransferEvent::TransferComplete {
        direction: Direction::Receive,
        peer,
        filename,
        path,
        ..
    } = event
    else {
        return;
    };
    let device = node
        .paired_devices()
        .into_iter()
        .find(|d| d.id == *peer)
        .map(|d| d.name)
        .unwrap_or_else(|| "a paired device".into());
    let mut body = filename.clone();
    if let Some(dir) = path.as_ref().and_then(|p| p.parent()) {
        body.push_str(&format!("\nSaved to {}", dir.display()));
    }
    let title = format!("File received from {device}");
    #[cfg(target_os = "linux")]
    let result = notify_rust::Notification::new()
        .appname("FreeLife")
        .summary(&title)
        .body(&body)
        .show()
        .map(|_| ());
    #[cfg(not(target_os = "linux"))]
    let result = app.notification().builder().title(title).body(body).show();
    if let Err(error) = result {
        tracing::warn!(%error, "could not show notification");
    }
    let _ = app;
}
