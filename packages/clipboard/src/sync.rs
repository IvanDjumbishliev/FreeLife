use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use freelife_core::{DeviceId, FreeLifeMessage};
use freelife_transfer::node::Node;
use serde::Serialize;
use tokio::sync::broadcast;
use tokio::sync::broadcast::error::RecvError;

use crate::dedup::{Dedup, Observation};
use crate::{
    detect_secret, ClipboardContent, ClipboardSettings, ContentHash, ContentKind, SensitiveReason,
};

/// Give up on a device that hasn't accepted the update by then. Clipboard
/// content goes stale fast; there's no point delivering it later.
const SEND_TIMEOUT: Duration = Duration::from_secs(3);

/// What happened to a local clipboard change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalDecision {
    /// Sent to every allowed paired device (in the background).
    Broadcast,
    /// Not actually a change.
    Unchanged,
    /// The value a peer just sent us; not sent back.
    EchoOfRemote,
    Disabled,
    ImagesDisabled,
    Sensitive(SensitiveReason),
    TooLarge,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ClipboardEvent {
    Sent {
        hash: String,
        kind: ContentKind,
        /// How many devices accepted it.
        devices: usize,
    },
    Received {
        from: DeviceId,
        device_name: String,
        kind: ContentKind,
        hash: String,
    },
    Skipped {
        hash: String,
        kind: ContentKind,
        reason: SensitiveReason,
    },
    SettingsChanged {
        settings: ClipboardSettings,
    },
}

/// Content from a peer, accepted and ready to write to the local clipboard.
#[derive(Debug, Clone)]
pub struct RemoteClip {
    pub from: DeviceId,
    pub device_name: String,
    pub content: ClipboardContent,
    pub hash: ContentHash,
}

pub struct ClipboardSync {
    node: Arc<Node>,
    runtime: tokio::runtime::Handle,
    settings: RwLock<ClipboardSettings>,
    settings_path: Option<PathBuf>,
    dedup: Mutex<Dedup>,
    events: broadcast::Sender<ClipboardEvent>,
}

impl ClipboardSync {
    /// Must be called inside a tokio runtime; background sends run on it.
    /// Settings persist to `settings_path` if given.
    pub async fn new(node: Arc<Node>, settings_path: Option<PathBuf>) -> Arc<Self> {
        let settings = match &settings_path {
            Some(path) => ClipboardSettings::load(path).await,
            None => ClipboardSettings::default(),
        };
        Arc::new(Self {
            node,
            runtime: tokio::runtime::Handle::current(),
            settings: RwLock::new(settings),
            settings_path,
            dedup: Mutex::new(Dedup::default()),
            events: broadcast::channel(64).0,
        })
    }

    pub fn node(&self) -> &Arc<Node> {
        &self.node
    }

    pub fn settings(&self) -> ClipboardSettings {
        self.settings.read().expect("lock poisoned").clone()
    }

    pub async fn set_settings(&self, settings: ClipboardSettings) -> std::io::Result<()> {
        *self.settings.write().expect("lock poisoned") = settings.clone();
        let _ = self.events.send(ClipboardEvent::SettingsChanged {
            settings: settings.clone(),
        });
        match &self.settings_path {
            Some(path) => settings.save(path).await,
            None => Ok(()),
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<ClipboardEvent> {
        self.events.subscribe()
    }

    /// Record what the clipboard holds at startup, so it isn't sent as if
    /// the user had just copied it.
    pub fn set_baseline(&self, content: &ClipboardContent) {
        self.dedup
            .lock()
            .expect("lock poisoned")
            .set_current(content.hash());
    }

    /// The local clipboard now holds `content`. Cheap if nothing changed;
    /// `marked_sensitive` is only consulted for real changes. Sending
    /// happens in the background.
    pub fn local_changed(
        &self,
        content: ClipboardContent,
        marked_sensitive: impl FnOnce() -> bool,
    ) -> LocalDecision {
        let hash = content.hash();
        match self
            .dedup
            .lock()
            .expect("lock poisoned")
            .observe_local(hash)
        {
            Observation::Unchanged => return LocalDecision::Unchanged,
            Observation::EchoOfRemote => return LocalDecision::EchoOfRemote,
            Observation::Changed => {}
        }

        let settings = self.settings();
        let kind = content.kind();
        if !settings.enabled {
            return LocalDecision::Disabled;
        }
        if kind == ContentKind::Image && !settings.sync_images {
            return LocalDecision::ImagesDisabled;
        }
        let secret = if marked_sensitive() {
            Some(SensitiveReason::MarkedByApp)
        } else if settings.block_secrets {
            content.as_text().and_then(detect_secret)
        } else {
            None
        };
        if let Some(reason) = secret {
            tracing::warn!(hash = %hash, ?kind, reason = reason.describe(), "not syncing sensitive clipboard content");
            let _ = self.events.send(ClipboardEvent::Skipped {
                hash: hash.short(),
                kind,
                reason,
            });
            return LocalDecision::Sensitive(reason);
        }
        let message = match content.to_message() {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!(hash = %hash, ?kind, error = %e, "not syncing clipboard content");
                return LocalDecision::TooLarge;
            }
        };

        let targets: Vec<DeviceId> = self
            .node
            .paired_devices()
            .into_iter()
            .map(|d| d.id)
            .filter(|id| settings.allows(*id))
            .collect();
        tracing::debug!(hash = %hash, ?kind, devices = targets.len(), "sending clipboard");
        let (node, events) = (self.node.clone(), self.events.clone());
        self.runtime.spawn(async move {
            let devices = node.send_to_many(targets, message, SEND_TIMEOUT).await;
            let _ = events.send(ClipboardEvent::Sent {
                hash: hash.short(),
                kind,
                devices,
            });
        });
        LocalDecision::Broadcast
    }

    /// Check an incoming message. Returns the content to write locally if
    /// it's a clipboard update we should apply.
    pub fn accept_remote(&self, from: DeviceId, msg: &FreeLifeMessage) -> Option<RemoteClip> {
        let parsed = ClipboardContent::from_message(msg)?;
        let settings = self.settings();
        if !settings.allows(from) {
            tracing::debug!(%from, "ignoring clipboard from a device sync is off for");
            return None;
        }
        let content = match parsed {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(%from, error = %e, "invalid clipboard update");
                return None;
            }
        };
        if content.kind() == ContentKind::Image && !settings.sync_images {
            return None;
        }
        let hash = content.hash();
        self.dedup
            .lock()
            .expect("lock poisoned")
            .record_remote(hash);

        let device_name = self
            .node
            .paired_devices()
            .into_iter()
            .find(|d| d.id == from)
            .map(|d| d.name)
            .unwrap_or_else(|| "another device".into());
        tracing::debug!(%from, hash = %hash, kind = ?content.kind(), "received clipboard");
        let _ = self.events.send(ClipboardEvent::Received {
            from,
            device_name: device_name.clone(),
            kind: content.kind(),
            hash: hash.short(),
        });
        Some(RemoteClip {
            from,
            device_name,
            content,
            hash,
        })
    }

    /// After writing a remote clip, record what the clipboard actually holds
    /// now. Platforms may alter content on write (line endings, image color
    /// conversion); this keeps the altered version from looking like a new
    /// local copy and bouncing back.
    pub fn applied(&self, hash: ContentHash) {
        self.dedup.lock().expect("lock poisoned").set_current(hash);
    }

    /// Feed clipboard updates from paired devices to `apply`.
    pub fn spawn_receiver(
        self: &Arc<Self>,
        apply: impl Fn(RemoteClip) + Send + 'static,
    ) -> tokio::task::JoinHandle<()> {
        // Subscribe now, so nothing sent after this call is missed.
        let mut messages = self.node.subscribe_messages();
        let sync = self.clone();
        self.runtime.spawn(async move {
            loop {
                match messages.recv().await {
                    Ok((from, msg)) => {
                        if let Some(clip) = sync.accept_remote(from, &msg) {
                            apply(clip);
                        }
                    }
                    Err(RecvError::Lagged(n)) => {
                        tracing::debug!(skipped = n, "clipboard receiver lagged");
                    }
                    Err(RecvError::Closed) => break,
                }
            }
        })
    }
}
