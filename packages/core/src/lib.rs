//! FreeLife core protocol.
//!
//! - [`discovery`]: advertise and find devices on the LAN via mDNS.
//! - [`identity`]: the per-device Ed25519 identity key, kept in the OS keychain.
//! - [`pairing`]: PIN-confirmed handshake that establishes mutual trust.
//! - [`trust`]: the `trusted_devices.json` store of paired devices.
//! - [`protocol`]: [`FreeLifeMessage`] and length-prefixed bincode framing.
//! - [`tls`]: TLS 1.3 configs authenticated by pinned identity keys.
//! - [`connection`]: [`ConnectionManager`], which keeps encrypted sessions
//!   to paired devices alive and routes messages.

pub mod connection;
mod device;
pub mod discovery;
mod error;
pub mod identity;
pub mod pairing;
pub mod protocol;
pub mod tls;
pub mod trust;

pub use connection::{ConnectionConfig, ConnectionManager};
pub use device::{DeviceId, DeviceInfo, OsType, PUBLIC_KEY_LEN};
pub use error::{Error, PairingError, Result};
pub use identity::Identity;
pub use protocol::{ClipboardContentType, FreeLifeMessage};
pub use trust::TrustStore;

/// Protocol version, checked during pairing and advertised via TLS ALPN.
/// Bump on breaking wire changes.
pub const PROTOCOL_VERSION: u16 = 1;

/// TCP port FreeLife devices listen on, for both pairing and sessions.
pub const DEFAULT_PORT: u16 = 7432;
