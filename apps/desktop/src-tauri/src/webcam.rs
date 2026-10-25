//! "Use Phone as Webcam": subscribe to a paired phone's camera and feed
//! the frames to the virtual camera, forwarding stats and previews to the UI.

use std::sync::Arc;

use freelife_camera::{platform_sink, CameraEvent, CameraReceiver, ReceiverConfig};
use freelife_core::{DeviceId, FreeLifeMessage};
use freelife_transfer::node::Node;
use tauri::{AppHandle, Emitter};
use tokio::sync::broadcast::error::RecvError;
use tokio::task::AbortHandle;

pub const CAMERA_EVENT: &str = "camera-event";

/// One running session. Dropping it stops the receiver.
pub struct WebcamSession {
    pub device: DeviceId,
    tasks: Vec<AbortHandle>,
}

impl Drop for WebcamSession {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

/// Subscribe to `device`'s camera and start writing frames to the virtual camera.
pub async fn start(
    app: AppHandle,
    node: Arc<Node>,
    device: DeviceId,
) -> Result<WebcamSession, String> {
    if !node.is_paired(device) {
        return Err("this device isn't paired".into());
    }
    let (receiver, mut events) = CameraReceiver::new(platform_sink(), ReceiverConfig::default())
        .map_err(|e| e.to_string())?;

    // Frames from the phone come in on the node's message stream.
    let mut messages = node.subscribe_messages();
    let conn = node
        .connection_to(device)
        .await
        .map_err(|e| e.to_string())?;
    conn.send(FreeLifeMessage::CameraSubscribe)
        .await
        .map_err(|e| e.to_string())?;

    let feed = tauri::async_runtime::spawn(async move {
        let mut receiver = receiver;
        loop {
            match messages.recv().await {
                Ok((from, msg)) if from == device => receiver.handle(&msg),
                Ok(_) => {}
                Err(RecvError::Lagged(n)) => {
                    tracing::warn!(skipped = n, "camera frames dropped locally")
                }
                Err(RecvError::Closed) => break,
            }
        }
    });

    let forward_app = app.clone();
    let forward = tauri::async_runtime::spawn(async move {
        loop {
            match events.recv().await {
                Ok(event) => {
                    log_event(&event);
                    let _ = forward_app.emit(CAMERA_EVENT, &event);
                }
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => break,
            }
        }
    });

    Ok(WebcamSession {
        device,
        tasks: vec![feed.abort_handle(), forward.abort_handle()],
    })
}

/// Tell the phone to stop sending frames to this device.
pub async fn stop(node: &Node, session: WebcamSession) {
    if let Ok(conn) = node.connection_to(session.device).await {
        let _ = conn.send(FreeLifeMessage::CameraUnsubscribe).await;
    }
    drop(session);
}

fn log_event(event: &CameraEvent) {
    match event {
        CameraEvent::Started { width, height } => {
            tracing::info!(width, height, "phone camera streaming");
        }
        CameraEvent::SinkError { message } => {
            tracing::warn!(error = %message, "virtual camera unavailable");
        }
        // Stats and previews are sent every second; keep them out of the log.
        _ => {}
    }
}
