//! Push node activity to the frontend as Tauri events, and raise native
//! notifications for received files.

use std::sync::Arc;

use freelife_core::prompts::PromptEvent;
use freelife_core::{ClipboardContentType, FreeLifeMessage};
use freelife_transfer::node::Node;
use freelife_transfer::{Direction, TransferEvent};
use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tauri_plugin_notification::NotificationExt;
use tokio::sync::broadcast::error::RecvError;

use crate::commands::DeviceView;

pub const TRANSFER_EVENT: &str = "transfer-event";
pub const PAIRING_PROMPT: &str = "pairing-prompt";
pub const PAIRING_DISMISS: &str = "pairing-dismiss";
pub const CLIPBOARD_RECEIVED: &str = "clipboard-received";

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
struct ClipboardView {
    from: String,
    text: String,
}

pub fn forward(app: AppHandle, node: Arc<Node>) {
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

    if let Some(mut other) = node.take_other_messages() {
        tauri::async_runtime::spawn(async move {
            while let Some((from, msg)) = other.recv().await {
                match msg {
                    FreeLifeMessage::ClipboardUpdate {
                        content_type: ClipboardContentType::Text,
                        payload,
                    } => {
                        let view = ClipboardView {
                            from: from.to_string(),
                            text: String::from_utf8_lossy(&payload).into_owned(),
                        };
                        let _ = app.emit(CLIPBOARD_RECEIVED, view);
                    }
                    other => tracing::debug!(%from, ?other, "unhandled message"),
                }
            }
        });
    }
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
    if let Err(e) = app
        .notification()
        .builder()
        .title(format!("File received from {device}"))
        .body(body)
        .show()
    {
        tracing::warn!(error = %e, "could not show notification");
    }
}
