use std::path::PathBuf;
use std::time::{Duration, Instant};

use freelife_core::DeviceId;
use serde::Serialize;
use uuid::Uuid;

/// Minimum gap between progress updates, so UIs aren't flooded.
const EMIT_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    Send,
    Receive,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferProgress {
    pub id: Uuid,
    /// The other device.
    pub peer: DeviceId,
    pub direction: Direction,
    pub filename: String,
    /// Bytes moved so far: sent, or received for incoming transfers.
    pub bytes_sent: u64,
    pub total_bytes: u64,
    /// Average throughput since the transfer started, in bytes per second.
    pub speed_bps: u64,
    /// Estimated seconds remaining; `None` until there's data to estimate from.
    pub eta_secs: Option<u64>,
}

impl TransferProgress {
    pub fn fraction(&self) -> f64 {
        if self.total_bytes == 0 {
            1.0
        } else {
            self.bytes_sent as f64 / self.total_bytes as f64
        }
    }

    pub fn is_complete(&self) -> bool {
        self.bytes_sent == self.total_bytes
    }
}

/// Everything AirShare reports, on a `tokio::sync::broadcast` channel.
///
/// Serializes as `{"type": "progress" | "transferComplete" | "transferError", ...}`
/// for UIs.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum TransferEvent {
    Progress(TransferProgress),
    TransferComplete {
        id: Uuid,
        peer: DeviceId,
        direction: Direction,
        filename: String,
        total_bytes: u64,
        /// Where the file was saved (incoming transfers only).
        path: Option<PathBuf>,
    },
    TransferError {
        id: Uuid,
        peer: DeviceId,
        direction: Direction,
        filename: String,
        error: String,
    },
}

impl TransferEvent {
    pub fn id(&self) -> Uuid {
        match self {
            Self::Progress(p) => p.id,
            Self::TransferComplete { id, .. } | Self::TransferError { id, .. } => *id,
        }
    }
}

pub(crate) struct ProgressTracker {
    id: Uuid,
    peer: DeviceId,
    direction: Direction,
    pub(crate) filename: String,
    total: u64,
    started: Instant,
    last_emit: Option<Instant>,
}

impl ProgressTracker {
    pub(crate) fn new(
        id: Uuid,
        peer: DeviceId,
        direction: Direction,
        filename: String,
        total: u64,
    ) -> Self {
        Self {
            id,
            peer,
            direction,
            filename,
            total,
            started: Instant::now(),
            last_emit: None,
        }
    }

    /// Progress at `done` bytes, unless throttled. The first and final
    /// updates are never throttled.
    pub(crate) fn update(&mut self, done: u64) -> Option<TransferProgress> {
        let now = Instant::now();
        let due = self
            .last_emit
            .is_none_or(|last| now.duration_since(last) >= EMIT_INTERVAL);
        if !due && done < self.total {
            return None;
        }
        self.last_emit = Some(now);
        Some(self.snapshot(done))
    }

    pub(crate) fn snapshot(&self, done: u64) -> TransferProgress {
        let elapsed = self.started.elapsed().as_secs_f64();
        let speed = if elapsed > 0.0 {
            (done as f64 / elapsed) as u64
        } else {
            0
        };
        let remaining = self.total.saturating_sub(done);
        let eta_secs = if remaining == 0 {
            Some(0)
        } else if speed > 0 {
            Some(remaining.div_ceil(speed))
        } else {
            None
        };
        TransferProgress {
            id: self.id,
            peer: self.peer,
            direction: self.direction,
            filename: self.filename.clone(),
            bytes_sent: done,
            total_bytes: self.total,
            speed_bps: speed,
            eta_secs,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tracker(total: u64) -> ProgressTracker {
        ProgressTracker::new(
            Uuid::new_v4(),
            freelife_core::Identity::generate().unwrap().device_id(),
            Direction::Send,
            "f".into(),
            total,
        )
    }

    #[test]
    fn throttles_but_always_reports_first_and_last() {
        let mut t = tracker(1000);
        assert!(t.update(0).is_some());
        assert!(t.update(10).is_none());
        assert!(t.update(500).is_none());
        let last = t.update(1000).unwrap();
        assert!(last.is_complete());
        assert_eq!(last.eta_secs, Some(0));
    }

    #[test]
    fn speed_and_eta() {
        let mut t = tracker(1_000_000);
        t.started -= Duration::from_secs(2);
        let p = t.snapshot(250_000);
        assert!(
            (120_000..=125_000).contains(&p.speed_bps),
            "{}",
            p.speed_bps
        );
        assert!((6..=7).contains(&p.eta_secs.unwrap()));
        assert!((p.fraction() - 0.25).abs() < 1e-9);
        assert_eq!(t.snapshot(0).eta_secs, None);
    }

    #[test]
    fn events_serialize_for_ui() {
        let p = tracker(10).snapshot(5);
        let json = serde_json::to_value(TransferEvent::Progress(p.clone())).unwrap();
        assert_eq!(json["type"], "progress");
        assert_eq!(json["bytesSent"], 5);
        assert_eq!(json["direction"], "send");

        let done = TransferEvent::TransferComplete {
            id: p.id,
            peer: p.peer,
            direction: Direction::Receive,
            filename: "f".into(),
            total_bytes: 10,
            path: Some("/tmp/f".into()),
        };
        let json = serde_json::to_value(done).unwrap();
        assert_eq!(json["type"], "transferComplete");
        assert_eq!(json["totalBytes"], 10);
        assert_eq!(json["peer"], p.peer.to_string());
    }
}
