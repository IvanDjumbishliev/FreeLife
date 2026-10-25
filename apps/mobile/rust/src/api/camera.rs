//! ContinuityCamera on the phone: forward encoded camera frames to the
//! desktops that subscribed. Kotlin does the Camera2 capture and H.264
//! encoding (MediaCodec) and passes each access unit here.

use std::sync::Arc;

use anyhow::anyhow;
use freelife_camera::CameraSource;

use super::node::{on_runtime, CAMERA};

fn source() -> anyhow::Result<&'static Arc<CameraSource>> {
    CAMERA
        .get()
        .ok_or_else(|| anyhow!("FreeLife has not been started"))
}

/// Start answering camera subscriptions from paired devices. Call once after
/// the node has started.
pub async fn camera_source_start() -> anyhow::Result<()> {
    let source = source()?.clone();
    on_runtime(async move {
        // The task lives as long as the process.
        std::mem::forget(source.spawn_subscriptions());
        Ok(())
    })
    .await
}

/// The encoder's output format. Sent to subscribers so they can prepare.
#[flutter_rust_bridge::frb(sync)]
pub fn camera_set_stream_info(width: u32, height: u32, fps: u32) -> anyhow::Result<()> {
    source()?.set_stream_info(width, height, fps);
    Ok(())
}

/// Whether any paired desktop is watching. The app can skip encoding when not.
#[flutter_rust_bridge::frb(sync)]
pub fn camera_has_subscribers() -> anyhow::Result<bool> {
    Ok(source()?.has_subscribers())
}

/// One H.264 access unit (Annex B). `timestamp_us` is the capture time in
/// Unix epoch microseconds. Never blocks: frames a slow link can't take are
/// dropped.
#[flutter_rust_bridge::frb(sync)]
pub fn camera_push_frame(data: Vec<u8>, keyframe: bool, timestamp_us: i64) -> anyhow::Result<()> {
    let timestamp_us = u64::try_from(timestamp_us).unwrap_or(0);
    source()?.push_frame(data, keyframe, timestamp_us);
    Ok(())
}

/// The camera stopped; subscribers are told and their subscriptions end.
#[flutter_rust_bridge::frb(sync)]
pub fn camera_stop() -> anyhow::Result<()> {
    source()?.stop();
    Ok(())
}
