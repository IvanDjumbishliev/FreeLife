use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{Error, Result};

/// Length of an Ed25519 public key.
pub const PUBLIC_KEY_LEN: usize = 32;

/// Longest device name we accept, in bytes. Keeps names well inside an mDNS
/// TXT record entry (255 bytes).
pub const MAX_NAME_LEN: usize = 64;

/// Identifies a device. Derived from its identity public key, so a peer can't
/// claim another device's id without also holding that device's private key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DeviceId(Uuid);

impl DeviceId {
    pub fn from_public_key(public_key: &[u8]) -> Self {
        let digest = Sha256::digest(public_key);
        let mut bytes = [0u8; 16];
        bytes.copy_from_slice(&digest[..16]);
        Self(Uuid::new_v8(bytes))
    }

    pub fn as_uuid(&self) -> &Uuid {
        &self.0
    }
}

impl fmt::Display for DeviceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl FromStr for DeviceId {
    type Err = uuid::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(s).map(Self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum OsType {
    Android,
    Linux,
    Windows,
    MacOS,
}

impl OsType {
    /// The OS this binary was compiled for, if FreeLife supports it.
    pub fn current() -> Option<Self> {
        if cfg!(target_os = "android") {
            Some(Self::Android)
        } else if cfg!(target_os = "linux") {
            Some(Self::Linux)
        } else if cfg!(target_os = "windows") {
            Some(Self::Windows)
        } else if cfg!(target_os = "macos") {
            Some(Self::MacOS)
        } else {
            None
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Android => "android",
            Self::Linux => "linux",
            Self::Windows => "windows",
            Self::MacOS => "macos",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "android" => Self::Android,
            "linux" => Self::Linux,
            "windows" => Self::Windows,
            "macos" => Self::MacOS,
            _ => return None,
        })
    }
}

/// What a device tells its peers about itself.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub id: DeviceId,
    pub name: String,
    pub os: OsType,
    /// Ed25519 identity public key.
    pub public_key: Vec<u8>,
}

impl DeviceInfo {
    /// Build a `DeviceInfo`, deriving the id from `public_key`.
    pub fn new(name: impl Into<String>, os: OsType, public_key: Vec<u8>) -> Self {
        Self {
            id: DeviceId::from_public_key(&public_key),
            name: name.into(),
            os,
            public_key,
        }
    }

    /// Check that info received from the network is well formed and that the
    /// id really belongs to the public key.
    pub fn validate(&self) -> Result<()> {
        if self.public_key.len() != PUBLIC_KEY_LEN {
            return Err(Error::InvalidDeviceInfo("public key must be 32 bytes"));
        }
        if self.id != DeviceId::from_public_key(&self.public_key) {
            return Err(Error::InvalidDeviceInfo("id does not match public key"));
        }
        if self.name.trim().is_empty() {
            return Err(Error::InvalidDeviceInfo("name is empty"));
        }
        if self.name.len() > MAX_NAME_LEN {
            return Err(Error::InvalidDeviceInfo("name is too long"));
        }
        Ok(())
    }

    /// Short, human-comparable fingerprint of the public key, e.g. `3f2a-91c0-77de-b415`.
    pub fn fingerprint(&self) -> String {
        let digest = Sha256::digest(&self.public_key);
        digest[..8]
            .chunks(2)
            .map(hex::encode)
            .collect::<Vec<_>>()
            .join("-")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info() -> DeviceInfo {
        DeviceInfo::new("Pixel 9", OsType::Android, vec![7; PUBLIC_KEY_LEN])
    }

    #[test]
    fn id_is_derived_from_public_key() {
        let a = info();
        assert_eq!(a.id, DeviceId::from_public_key(&a.public_key));
        assert_ne!(a.id, DeviceId::from_public_key(&[8; PUBLIC_KEY_LEN]));
        assert!(a.validate().is_ok());
    }

    #[test]
    fn validate_rejects_spoofed_id() {
        let mut spoofed = info();
        spoofed.id = DeviceId::from_public_key(&[1; PUBLIC_KEY_LEN]);
        assert!(matches!(
            spoofed.validate(),
            Err(Error::InvalidDeviceInfo(_))
        ));
    }

    #[test]
    fn validate_rejects_bad_keys_and_names() {
        let short = DeviceInfo::new("x", OsType::Linux, vec![1; 31]);
        assert!(short.validate().is_err());

        let empty = DeviceInfo::new("  ", OsType::Linux, vec![1; PUBLIC_KEY_LEN]);
        assert!(empty.validate().is_err());

        let long = DeviceInfo::new("x".repeat(MAX_NAME_LEN + 1), OsType::Linux, vec![1; 32]);
        assert!(long.validate().is_err());
    }

    #[test]
    fn os_type_round_trips_through_str() {
        for os in [
            OsType::Android,
            OsType::Linux,
            OsType::Windows,
            OsType::MacOS,
        ] {
            assert_eq!(OsType::parse(os.as_str()), Some(os));
        }
        assert_eq!(OsType::parse("plan9"), None);
    }

    #[test]
    fn device_id_round_trips_through_string() {
        let id = info().id;
        assert_eq!(id.to_string().parse::<DeviceId>().unwrap(), id);
    }

    #[test]
    fn fingerprint_format() {
        let fp = info().fingerprint();
        assert_eq!(fp.len(), 19);
        assert_eq!(fp.matches('-').count(), 3);
    }
}
