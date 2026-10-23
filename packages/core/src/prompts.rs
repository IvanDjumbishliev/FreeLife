//! Route pairing confirmations to a UI.
//!
//! [`PairingPrompts`] is a [`PinConfirmer`] for apps whose UI runs
//! elsewhere (a webview, Dart). Each confirmation becomes a
//! [`PromptEvent::Show`] on a broadcast channel; the UI calls
//! [`PairingPrompts::respond`] with the user's answer. When the pairing
//! finishes or is abandoned, a [`PromptEvent::Dismiss`] tells the UI to
//! close the dialog.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use tokio::sync::{broadcast, oneshot};

use crate::pairing::{BoxFuture, PairingCode, PinConfirmer};
use crate::DeviceInfo;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairingPrompt {
    pub request_id: u64,
    pub peer: DeviceInfo,
    pub code: PairingCode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptEvent {
    Show(PairingPrompt),
    /// The prompt was answered, timed out, or its pairing was abandoned.
    Dismiss {
        request_id: u64,
    },
}

pub struct PairingPrompts {
    next_id: AtomicU64,
    pending: Mutex<HashMap<u64, oneshot::Sender<bool>>>,
    events: broadcast::Sender<PromptEvent>,
    timeout: Duration,
}

impl PairingPrompts {
    /// Unanswered prompts count as "rejected" after `timeout`.
    pub fn new(timeout: Duration) -> Self {
        Self {
            next_id: AtomicU64::new(1),
            pending: Mutex::new(HashMap::new()),
            events: broadcast::channel(16).0,
            timeout,
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<PromptEvent> {
        self.events.subscribe()
    }

    /// Deliver the user's answer. Returns `false` if the prompt is no longer
    /// pending.
    pub fn respond(&self, request_id: u64, accept: bool) -> bool {
        let sender = self
            .pending
            .lock()
            .expect("lock poisoned")
            .remove(&request_id);
        sender.is_some_and(|tx| tx.send(accept).is_ok())
    }
}

/// Clears the pending entry and dismisses the UI prompt however `confirm`
/// ends, including when its future is dropped mid-wait.
struct PendingGuard<'a> {
    prompts: &'a PairingPrompts,
    request_id: u64,
}

impl Drop for PendingGuard<'_> {
    fn drop(&mut self) {
        self.prompts
            .pending
            .lock()
            .expect("lock poisoned")
            .remove(&self.request_id);
        let _ = self.prompts.events.send(PromptEvent::Dismiss {
            request_id: self.request_id,
        });
    }
}

impl PinConfirmer for PairingPrompts {
    fn confirm(&self, peer: DeviceInfo, code: PairingCode) -> BoxFuture<'_, bool> {
        Box::pin(async move {
            let request_id = self.next_id.fetch_add(1, Ordering::Relaxed);
            let (tx, rx) = oneshot::channel();
            self.pending
                .lock()
                .expect("lock poisoned")
                .insert(request_id, tx);
            let _guard = PendingGuard {
                prompts: self,
                request_id,
            };

            let prompt = PairingPrompt {
                request_id,
                peer,
                code,
            };
            if self.events.send(PromptEvent::Show(prompt)).is_err() {
                return false; // no UI is listening
            }
            matches!(tokio::time::timeout(self.timeout, rx).await, Ok(Ok(true)))
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::{Identity, OsType};

    fn peer() -> DeviceInfo {
        Identity::generate()
            .unwrap()
            .device_info("peer", OsType::Android)
    }

    async fn next_show(rx: &mut broadcast::Receiver<PromptEvent>) -> PairingPrompt {
        match rx.recv().await.unwrap() {
            PromptEvent::Show(p) => p,
            other => panic!("expected Show, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn ui_answer_is_returned() {
        let prompts = Arc::new(PairingPrompts::new(Duration::from_secs(5)));
        let mut rx = prompts.subscribe();
        let info = peer();

        for answer in [true, false] {
            let p = prompts.clone();
            let peer = info.clone();
            let confirm =
                tokio::spawn(async move { p.confirm(peer, PairingCode::for_test(123_456)).await });
            let shown = next_show(&mut rx).await;
            assert_eq!(shown.peer, info);
            assert_eq!(shown.code.to_string(), "123456");
            assert!(prompts.respond(shown.request_id, answer));
            assert_eq!(confirm.await.unwrap(), answer);
            assert_eq!(
                rx.recv().await.unwrap(),
                PromptEvent::Dismiss {
                    request_id: shown.request_id
                }
            );
            assert!(!prompts.respond(shown.request_id, true), "already answered");
        }
    }

    #[tokio::test]
    async fn no_listener_means_reject() {
        let prompts = PairingPrompts::new(Duration::from_secs(5));
        assert!(!prompts.confirm(peer(), PairingCode::for_test(1)).await);
    }

    #[tokio::test]
    async fn timeout_rejects_and_dismisses() {
        let prompts = PairingPrompts::new(Duration::from_millis(50));
        let mut rx = prompts.subscribe();
        assert!(!prompts.confirm(peer(), PairingCode::for_test(1)).await);
        let shown = next_show(&mut rx).await;
        assert_eq!(
            rx.recv().await.unwrap(),
            PromptEvent::Dismiss {
                request_id: shown.request_id
            }
        );
    }

    #[tokio::test]
    async fn abandoned_pairing_dismisses_prompt() {
        let prompts = PairingPrompts::new(Duration::from_secs(60));
        let mut rx = prompts.subscribe();
        {
            let fut = prompts.confirm(peer(), PairingCode::for_test(1));
            let _ = tokio::time::timeout(Duration::from_millis(20), fut).await;
        }
        let shown = next_show(&mut rx).await;
        assert!(matches!(
            rx.recv().await.unwrap(),
            PromptEvent::Dismiss { .. }
        ));
        assert!(!prompts.respond(shown.request_id, true));
    }
}
