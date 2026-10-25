//! Phone side: forward encoded camera frames to the devices that asked for them.

use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use freelife_core::{DeviceId, FreeLifeMessage};
use freelife_transfer::node::Node;
use tokio::task::JoinHandle;

/// Sends H.264 frames to subscribed paired devices.
///
/// Subscriptions come from the desktop side ([`FreeLifeMessage::CameraSubscribe`]).
/// Frames are only sent to subscribers; [`CameraSource::has_subscribers`] lets
/// the app know whether anyone is watching. The encoder itself runs whenever the
/// webcam switch is on. Frames are dropped when a link can't keep up: stale video
/// is worse than missing video.
pub struct CameraSource {
    node: Arc<Node>,
    subscribers: Mutex<HashSet<DeviceId>>,
    stream_info: Mutex<Option<(u32, u32, u32)>>,
    seq: AtomicU64,
    dropped: AtomicU64,
}

impl CameraSource {
    pub fn new(node: Arc<Node>) -> Arc<Self> {
        Arc::new(Self {
            node,
            subscribers: Mutex::new(HashSet::new()),
            stream_info: Mutex::new(None),
            seq: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
        })
    }

    /// Track subscribe/unsubscribe requests until the task is aborted.
    pub fn spawn_subscriptions(self: &Arc<Self>) -> JoinHandle<()> {
        // Subscribed now, so requests made after this returns are seen.
        let mut messages = self.node.subscribe_messages();
        let source = self.clone();
        tokio::spawn(async move {
            while let Ok((from, msg)) = messages.recv().await {
                match msg {
                    FreeLifeMessage::CameraSubscribe => source.subscribe(from),
                    FreeLifeMessage::CameraUnsubscribe => {
                        source
                            .subscribers
                            .lock()
                            .expect("lock poisoned")
                            .remove(&from);
                    }
                    _ => {}
                }
            }
        })
    }

    fn subscribe(&self, device: DeviceId) {
        if !self.node.is_paired(device) {
            tracing::warn!(%device, "camera subscription from an unpaired device ignored");
            return;
        }
        self.subscribers
            .lock()
            .expect("lock poisoned")
            .insert(device);
        let info = *self.stream_info.lock().expect("lock poisoned");
        if let (Some((width, height, fps)), Some(conn)) =
            (info, self.node.manager().connection(device))
        {
            let _ = conn.try_send(FreeLifeMessage::CameraStreamInfo { width, height, fps });
        }
    }

    pub fn has_subscribers(&self) -> bool {
        !self.subscribers.lock().expect("lock poisoned").is_empty()
    }

    /// The camera is running at this format. Tells current subscribers.
    pub fn set_stream_info(&self, width: u32, height: u32, fps: u32) {
        *self.stream_info.lock().expect("lock poisoned") = Some((width, height, fps));
        self.broadcast(FreeLifeMessage::CameraStreamInfo { width, height, fps });
    }

    /// The camera stopped. Subscribers are told, and their subscriptions cleared.
    pub fn stop(&self) {
        *self.stream_info.lock().expect("lock poisoned") = None;
        let targets: Vec<DeviceId> = self
            .subscribers
            .lock()
            .expect("lock poisoned")
            .drain()
            .collect();
        for device in targets {
            if let Some(conn) = self.node.manager().connection(device) {
                let _ = conn.try_send(FreeLifeMessage::CameraEnded);
            }
        }
    }

    /// Send one encoded access unit (Annex B) to every subscriber. Never blocks.
    pub fn push_frame(&self, data: Vec<u8>, keyframe: bool, timestamp_us: u64) {
        let targets: Vec<DeviceId> = self
            .subscribers
            .lock()
            .expect("lock poisoned")
            .iter()
            .copied()
            .collect();
        if targets.is_empty() {
            return;
        }
        let seq = self.seq.fetch_add(1, Ordering::Relaxed);
        for device in targets {
            let Some(conn) = self.node.manager().connection(device) else {
                continue;
            };
            let frame = FreeLifeMessage::CameraFrame {
                seq,
                timestamp_us,
                keyframe,
                data: data.clone(),
            };
            if let Ok(false) = conn.try_send(frame) {
                self.dropped.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// Frames dropped because a link was congested.
    pub fn dropped_frames(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    fn broadcast(&self, message: FreeLifeMessage) {
        let targets: Vec<DeviceId> = self
            .subscribers
            .lock()
            .expect("lock poisoned")
            .iter()
            .copied()
            .collect();
        for device in targets {
            if let Some(conn) = self.node.manager().connection(device) {
                let _ = conn.try_send(message.clone());
            }
        }
    }
}
