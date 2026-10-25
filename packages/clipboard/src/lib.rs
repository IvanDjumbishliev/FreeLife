//! Universal Clipboard: keep clipboards in sync across paired devices.
//!
//! - [`ClipboardSync`] is the engine. It deduplicates, filters secrets,
//!   applies settings, sends local changes to peers and accepts theirs.
//! - [`ClipboardWatcher`] polls a [`ClipboardBackend`] on a background
//!   thread (desktop) and applies received content on the same thread.
//! - `ArboardBackend` (desktop) is the real system clipboard. Android reads
//!   the clipboard in Kotlin and calls [`ClipboardSync::local_changed`]
//!   itself.
//!
//! Clipboard contents are never logged. Logs carry an 8-character hash
//! prefix ([`ContentHash`]) at most.

mod content;
mod dedup;
mod sensitive;
mod settings;
mod sync;
pub mod testing;
mod watcher;

#[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]
mod desktop;
#[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]
mod probe;

pub use content::{
    decode_png, encode_png, ClipboardContent, ContentError, ContentHash, ContentKind, Image,
    MAX_IMAGE_PIXELS, MAX_PNG_BYTES, MAX_TEXT_BYTES,
};
#[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]
pub use desktop::ArboardBackend;
pub use sensitive::{detect_secret, SensitiveReason};
pub use settings::{ClipboardSettings, SETTINGS_FILE};
pub use sync::{ClipboardEvent, ClipboardSync, LocalDecision, RemoteClip};
pub use watcher::{ClipboardWatcher, WatcherConfig};

#[derive(Debug, thiserror::Error)]
#[error("clipboard error: {0}")]
pub struct BackendError(pub String);

/// Access to one system clipboard. Used from a single thread (the
/// watcher's), so implementations needn't be `Send` or `Sync`.
pub trait ClipboardBackend {
    /// Current contents, or `None` if empty, busy, or holding something we
    /// don't sync. Images are only read when `include_image` is set, since
    /// fetching one can mean copying tens of megabytes.
    fn read(&mut self, include_image: bool) -> Result<Option<ClipboardContent>, BackendError>;

    fn write(&mut self, content: &ClipboardContent) -> Result<(), BackendError>;

    /// Whether the current contents carry a "don't sync me" hint from a
    /// password manager. Called only when the contents have changed.
    fn is_marked_sensitive(&mut self) -> bool {
        false
    }
}
