//! Universal Clipboard for Android.
//!
//! Kotlin reads and writes the Android clipboard and passes everything
//! through here. The engine (dedup, secret filter, sending) is the same
//! `freelife_clipboard::ClipboardSync` the desktop app uses.

use std::sync::Arc;

use anyhow::anyhow;
use freelife_clipboard::{decode_png, ClipboardContent, ClipboardSync, LocalDecision};

use super::node::{node, on_runtime, CLIPBOARD};
use crate::frb_generated::StreamSink;

fn clipboard() -> anyhow::Result<&'static Arc<ClipboardSync>> {
    CLIPBOARD
        .get()
        .ok_or_else(|| anyhow!("FreeLife has not been started"))
}

/// What happened to a local copy. Mirrors `LocalDecision` without payloads.
pub enum LocalClipDecision {
    /// Sent to paired devices.
    Broadcast,
    /// Same content as before.
    Unchanged,
    /// The content a paired device just sent us; not sent back.
    EchoOfRemote,
    Disabled,
    ImagesDisabled,
    /// Held back: marked sensitive, or looks like a password or key.
    Sensitive,
    TooLarge,
}

impl From<LocalDecision> for LocalClipDecision {
    fn from(d: LocalDecision) -> Self {
        match d {
            LocalDecision::Broadcast => Self::Broadcast,
            LocalDecision::Unchanged => Self::Unchanged,
            LocalDecision::EchoOfRemote => Self::EchoOfRemote,
            LocalDecision::Disabled => Self::Disabled,
            LocalDecision::ImagesDisabled => Self::ImagesDisabled,
            LocalDecision::Sensitive(_) => Self::Sensitive,
            LocalDecision::TooLarge => Self::TooLarge,
        }
    }
}

pub enum ClipKind {
    Text,
    Url,
    Image,
}

/// Content a paired device copied, ready to write to the Android clipboard.
pub struct ClipboardReceived {
    pub device_name: String,
    pub kind: ClipKind,
    /// Set for text and URLs.
    pub text: Option<String>,
    /// PNG bytes, set for images.
    pub png: Option<Vec<u8>>,
}

/// A local copy of plain text (or a URL). `marked_sensitive` is the
/// platform's "don't sync me" hint (`EXTRA_IS_SENSITIVE`, Android 13+).
#[flutter_rust_bridge::frb(sync)]
pub fn clipboard_local_text(
    text: String,
    marked_sensitive: bool,
) -> anyhow::Result<LocalClipDecision> {
    let content = ClipboardContent::from_text(text);
    Ok(clipboard()?
        .local_changed(content, || marked_sensitive)
        .into())
}

/// A local copy of an image, as PNG bytes. Decoding is done off the Dart
/// thread.
pub async fn clipboard_local_png(
    png: Vec<u8>,
    marked_sensitive: bool,
) -> anyhow::Result<LocalClipDecision> {
    let sync = clipboard()?.clone();
    on_runtime(async move {
        let image = decode_png(&png)?;
        Ok(sync
            .local_changed(ClipboardContent::Image(image), || marked_sensitive)
            .into())
    })
    .await
}

/// Clipboard updates from paired devices. Each one should be written to
/// the Android clipboard.
pub fn clipboard_received_events(sink: StreamSink<ClipboardReceived>) -> anyhow::Result<()> {
    let sync = clipboard()?.clone();
    // Subscribed before returning, so nothing sent after this call is missed.
    let _ = node()?;
    let handle = sync.spawn_receiver(move |clip| {
        let received = match clip.content {
            ClipboardContent::Text(t) => ClipboardReceived {
                device_name: clip.device_name,
                kind: ClipKind::Text,
                text: Some(t),
                png: None,
            },
            ClipboardContent::Url(u) => ClipboardReceived {
                device_name: clip.device_name,
                kind: ClipKind::Url,
                text: Some(u),
                png: None,
            },
            ClipboardContent::Image(img) => match freelife_clipboard::encode_png(&img) {
                Ok(png) => ClipboardReceived {
                    device_name: clip.device_name,
                    kind: ClipKind::Image,
                    text: None,
                    png: Some(png),
                },
                Err(_) => return,
            },
        };
        let _ = sink.add(received);
    });
    std::mem::forget(handle); // lives as long as the process
    Ok(())
}

/// Record what Android now holds after we wrote a received clip, so the
/// platform's own rewrite of it isn't sent back.
pub async fn clipboard_applied_text(text: String) -> anyhow::Result<()> {
    let sync = clipboard()?.clone();
    on_runtime(async move {
        sync.applied(ClipboardContent::from_text(text).hash());
        Ok(())
    })
    .await
}

#[flutter_rust_bridge::frb(sync)]
pub fn clipboard_enabled() -> anyhow::Result<bool> {
    Ok(clipboard()?.settings().enabled)
}

pub async fn set_clipboard_enabled(enabled: bool) -> anyhow::Result<()> {
    let sync = clipboard()?.clone();
    on_runtime(async move {
        let mut settings = sync.settings();
        settings.enabled = enabled;
        sync.set_settings(settings).await?;
        Ok(())
    })
    .await
}
