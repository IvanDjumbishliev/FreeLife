use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use freelife_core::DeviceId;
use serde::{Deserialize, Serialize};

pub const SETTINGS_FILE: &str = "clipboard_settings.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ClipboardSettings {
    /// Master switch, both directions.
    pub enabled: bool,
    /// Only sync with the devices in `trusted_devices`, rather than with
    /// every paired device.
    pub trusted_only: bool,
    pub trusted_devices: BTreeSet<DeviceId>,
    /// Skip content that looks like a password, key or token, not just
    /// content a password manager marked as sensitive.
    pub block_secrets: bool,
    pub sync_images: bool,
}

impl Default for ClipboardSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            trusted_only: false,
            trusted_devices: BTreeSet::new(),
            block_secrets: true,
            sync_images: true,
        }
    }
}

impl ClipboardSettings {
    pub fn allows(&self, device: DeviceId) -> bool {
        self.enabled && (!self.trusted_only || self.trusted_devices.contains(&device))
    }

    pub(crate) async fn load(path: &Path) -> Self {
        match tokio::fs::read(path).await {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|e| {
                tracing::warn!(error = %e, "unreadable clipboard settings; using defaults");
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }

    pub(crate) async fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            tokio::fs::create_dir_all(dir).await?;
        }
        let tmp: PathBuf = path.with_extension("json.tmp");
        tokio::fs::write(&tmp, serde_json::to_vec_pretty(self)?).await?;
        tokio::fs::rename(&tmp, path).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use freelife_core::Identity;

    #[test]
    fn allowlist() {
        let (a, b) = (
            Identity::generate().unwrap().device_id(),
            Identity::generate().unwrap().device_id(),
        );
        let mut s = ClipboardSettings::default();
        assert!(s.allows(a) && s.allows(b));
        s.trusted_only = true;
        s.trusted_devices.insert(a);
        assert!(s.allows(a) && !s.allows(b));
        s.enabled = false;
        assert!(!s.allows(a));
    }

    #[tokio::test]
    async fn persists_and_tolerates_missing_fields() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(SETTINGS_FILE);
        assert_eq!(
            ClipboardSettings::load(&path).await,
            ClipboardSettings::default()
        );

        let mut s = ClipboardSettings {
            trusted_only: true,
            ..ClipboardSettings::default()
        };
        s.trusted_devices
            .insert(Identity::generate().unwrap().device_id());
        s.save(&path).await.unwrap();
        assert_eq!(ClipboardSettings::load(&path).await, s);

        // Older files without newer fields still load.
        std::fs::write(&path, r#"{"enabled": false}"#).unwrap();
        let loaded = ClipboardSettings::load(&path).await;
        assert!(!loaded.enabled && loaded.block_secrets);
    }
}
