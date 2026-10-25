//! An in-memory [`ClipboardBackend`] for tests.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::{BackendError, ClipboardBackend, ClipboardContent};

#[derive(Default)]
struct State {
    content: Option<ClipboardContent>,
    sensitive: bool,
    writes: Vec<(Instant, ClipboardContent)>,
}

/// A fake system clipboard. Clones share state, so a test keeps one handle
/// while the watcher owns another.
#[derive(Clone, Default)]
pub struct MemoryClipboard {
    state: Arc<Mutex<State>>,
}

impl MemoryClipboard {
    /// What a user copying something looks like.
    pub fn copy(&self, content: ClipboardContent) {
        let mut s = self.state.lock().unwrap();
        s.content = Some(content);
        s.sensitive = false;
    }

    /// A password manager copying something with a "sensitive" hint.
    pub fn copy_marked_sensitive(&self, content: ClipboardContent) {
        let mut s = self.state.lock().unwrap();
        s.content = Some(content);
        s.sensitive = true;
    }

    pub fn current(&self) -> Option<ClipboardContent> {
        self.state.lock().unwrap().content.clone()
    }

    /// Everything written by the sync engine, with timestamps.
    pub fn writes(&self) -> Vec<(Instant, ClipboardContent)> {
        self.state.lock().unwrap().writes.clone()
    }
}

impl ClipboardBackend for MemoryClipboard {
    fn read(&mut self, include_image: bool) -> Result<Option<ClipboardContent>, BackendError> {
        let s = self.state.lock().unwrap();
        Ok(match &s.content {
            Some(ClipboardContent::Image(_)) if !include_image => None,
            other => other.clone(),
        })
    }

    fn write(&mut self, content: &ClipboardContent) -> Result<(), BackendError> {
        let mut s = self.state.lock().unwrap();
        s.content = Some(content.clone());
        s.sensitive = false;
        s.writes.push((Instant::now(), content.clone()));
        Ok(())
    }

    fn is_marked_sensitive(&mut self) -> bool {
        self.state.lock().unwrap().sensitive
    }
}
