//! Devices we've paired with, persisted to `trusted_devices.json`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::RwLock;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::{DeviceId, DeviceInfo, OsType, Result};

pub const TRUSTED_DEVICES_FILE: &str = "trusted_devices.json";

const FILE_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrustedDevice {
    pub id: DeviceId,
    pub name: String,
    pub os: OsType,
    #[serde(with = "hex_bytes")]
    pub public_key: Vec<u8>,
    /// Unix timestamp (seconds) of when pairing completed.
    pub paired_at: u64,
}

impl TrustedDevice {
    pub fn info(&self) -> DeviceInfo {
        DeviceInfo {
            id: self.id,
            name: self.name.clone(),
            os: self.os,
            public_key: self.public_key.clone(),
        }
    }
}

#[derive(Serialize, Deserialize)]
struct TrustFile {
    version: u32,
    devices: Vec<TrustedDevice>,
}

/// Thread-safe set of trusted devices. Lookups are synchronous (the TLS
/// verifiers need that); writes persist to disk.
#[derive(Debug)]
pub struct TrustStore {
    path: Option<PathBuf>,
    devices: RwLock<HashMap<DeviceId, TrustedDevice>>,
    // Serializes file writes so concurrent updates can't interleave.
    write_lock: tokio::sync::Mutex<()>,
}

impl TrustStore {
    /// Open the store at `path`, starting empty if the file doesn't exist yet.
    pub async fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let devices = match tokio::fs::read(&path).await {
            Ok(bytes) => {
                let file: TrustFile = serde_json::from_slice(&bytes)?;
                file.devices.into_iter().map(|d| (d.id, d)).collect()
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => HashMap::new(),
            Err(e) => return Err(e.into()),
        };
        Ok(Self {
            path: Some(path),
            devices: RwLock::new(devices),
            write_lock: tokio::sync::Mutex::new(()),
        })
    }

    /// A store that never touches disk.
    pub fn in_memory() -> Self {
        Self {
            path: None,
            devices: RwLock::new(HashMap::new()),
            write_lock: tokio::sync::Mutex::new(()),
        }
    }

    pub fn get(&self, id: DeviceId) -> Option<TrustedDevice> {
        self.devices
            .read()
            .expect("lock poisoned")
            .get(&id)
            .cloned()
    }

    pub fn find_by_public_key(&self, public_key: &[u8]) -> Option<TrustedDevice> {
        self.get(DeviceId::from_public_key(public_key))
            .filter(|d| d.public_key == public_key)
    }

    pub fn is_trusted(&self, id: DeviceId) -> bool {
        self.devices
            .read()
            .expect("lock poisoned")
            .contains_key(&id)
    }

    pub fn list(&self) -> Vec<TrustedDevice> {
        let mut devices: Vec<_> = self
            .devices
            .read()
            .expect("lock poisoned")
            .values()
            .cloned()
            .collect();
        devices.sort_by_key(|d| d.paired_at);
        devices
    }

    /// Trust `info` (replacing any previous entry for the same device) and persist.
    pub async fn trust(&self, info: &DeviceInfo) -> Result<()> {
        info.validate()?;
        let paired_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let device = TrustedDevice {
            id: info.id,
            name: info.name.clone(),
            os: info.os,
            public_key: info.public_key.clone(),
            paired_at,
        };
        self.devices
            .write()
            .expect("lock poisoned")
            .insert(device.id, device);
        self.persist().await
    }

    /// Forget a device. Returns whether it was trusted.
    pub async fn revoke(&self, id: DeviceId) -> Result<bool> {
        let removed = self
            .devices
            .write()
            .expect("lock poisoned")
            .remove(&id)
            .is_some();
        if removed {
            self.persist().await?;
        }
        Ok(removed)
    }

    async fn persist(&self) -> Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        let _guard = self.write_lock.lock().await;
        let json = serde_json::to_vec_pretty(&TrustFile {
            version: FILE_VERSION,
            devices: self.list(),
        })?;
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            tokio::fs::create_dir_all(dir).await?;
        }
        // Write-then-rename so a crash can't leave a half-written file.
        let tmp = path.with_extension("json.tmp");
        tokio::fs::write(&tmp, json).await?;
        tokio::fs::rename(&tmp, path).await?;
        Ok(())
    }
}

mod hex_bytes {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&hex::encode(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let s = String::deserialize(d)?;
        hex::decode(s).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Identity;

    fn device(name: &str) -> DeviceInfo {
        Identity::generate()
            .unwrap()
            .device_info(name, OsType::Windows)
    }

    #[tokio::test]
    async fn trusted_devices_survive_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join(TRUSTED_DEVICES_FILE);
        let (a, b) = (device("a"), device("b"));

        let store = TrustStore::open(&path).await.unwrap();
        store.trust(&a).await.unwrap();
        store.trust(&b).await.unwrap();
        assert!(store.revoke(b.id).await.unwrap());

        let reopened = TrustStore::open(&path).await.unwrap();
        assert_eq!(reopened.get(a.id).unwrap().info(), a);
        assert!(!reopened.is_trusted(b.id));
        assert!(!path.with_extension("json.tmp").exists());
    }

    #[tokio::test]
    async fn file_is_readable_json_with_hex_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(TRUSTED_DEVICES_FILE);
        let a = device("a");
        TrustStore::open(&path)
            .await
            .unwrap()
            .trust(&a)
            .await
            .unwrap();

        let json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(json["version"], 1);
        assert_eq!(json["devices"][0]["public_key"], hex::encode(&a.public_key));
        assert_eq!(json["devices"][0]["os"], "Windows");
    }

    #[tokio::test]
    async fn lookup_by_public_key() {
        let store = TrustStore::in_memory();
        let a = device("a");
        store.trust(&a).await.unwrap();
        assert_eq!(store.find_by_public_key(&a.public_key).unwrap().id, a.id);
        assert!(store.find_by_public_key(&[0; 32]).is_none());
    }

    #[tokio::test]
    async fn refuses_invalid_device_info() {
        let store = TrustStore::in_memory();
        let mut bad = device("a");
        bad.public_key[0] ^= 1;
        assert!(store.trust(&bad).await.is_err());
        assert!(store.list().is_empty());
    }

    #[tokio::test]
    async fn missing_file_is_empty_store() {
        let dir = tempfile::tempdir().unwrap();
        let store = TrustStore::open(dir.path().join("nope.json"))
            .await
            .unwrap();
        assert!(store.list().is_empty());
    }
}
