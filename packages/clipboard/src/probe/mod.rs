//! Detect "don't sync me" hints that password managers attach to the
//! clipboard. Each platform has its own convention:
//!
//! - Windows: the `ExcludeClipboardContentFromMonitorProcessing` format
//!   (KeePass, 1Password, Bitwarden, Windows itself) and the older
//!   `Clipboard Viewer Ignore`.
//! - macOS: `org.nspasteboard.ConcealedType` / `TransientType`
//!   (nspasteboard.org; 1Password, KeePassXC, Bitwarden).
//! - Linux: the `x-kde-passwordManagerHint` target (KeePassXC, KDE).
//!
//! A probe that can't tell reports "not marked"; the content heuristics in
//! `sensitive.rs` still apply.

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
pub(crate) use linux::Probe;
#[cfg(target_os = "macos")]
pub(crate) use macos::Probe;
#[cfg(target_os = "windows")]
pub(crate) use windows::Probe;
