//! The per-device Ed25519 identity key.
//!
//! Generated on first run and persisted through a [`KeyStore`]. On desktop
//! that's the OS keychain ([`KeyringStore`]); mobile apps plug in their own
//! (Android Keystore / iOS Keychain).

use std::fmt;
use std::sync::{Arc, Mutex};

use ring::rand::{SecureRandom, SystemRandom};
use ring::signature::{self, Ed25519KeyPair, KeyPair};

use crate::{DeviceId, DeviceInfo, Error, OsType, Result};

const SEED_LEN: usize = 32;

/// Persists the identity key's 32-byte seed. Methods are blocking; callers
/// should run them off the async executor (as [`Identity::load_or_generate`] does).
pub trait KeyStore: Send + Sync {
    fn load(&self) -> Result<Option<Vec<u8>>>;
    fn save(&self, secret: &[u8]) -> Result<()>;
}

/// Non-persistent store, for tests and ephemeral sessions.
#[derive(Default)]
pub struct MemoryKeyStore(Mutex<Option<Vec<u8>>>);

impl KeyStore for MemoryKeyStore {
    fn load(&self) -> Result<Option<Vec<u8>>> {
        Ok(self.0.lock().expect("lock poisoned").clone())
    }

    fn save(&self, secret: &[u8]) -> Result<()> {
        *self.0.lock().expect("lock poisoned") = Some(secret.to_vec());
        Ok(())
    }
}

/// OS keychain: macOS Keychain, Windows Credential Manager, or the Secret
/// Service (GNOME Keyring / KWallet) on Linux.
#[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]
pub struct KeyringStore {
    service: String,
    user: String,
}

#[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]
impl KeyringStore {
    pub const DEFAULT_SERVICE: &'static str = "dev.freelife";
    pub const DEFAULT_USER: &'static str = "device-identity-ed25519";

    pub fn new(service: impl Into<String>, user: impl Into<String>) -> Self {
        Self {
            service: service.into(),
            user: user.into(),
        }
    }

    fn entry(&self) -> Result<keyring::Entry> {
        keyring::Entry::new(&self.service, &self.user).map_err(|e| Error::KeyStore(e.to_string()))
    }
}

#[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]
impl Default for KeyringStore {
    fn default() -> Self {
        Self::new(Self::DEFAULT_SERVICE, Self::DEFAULT_USER)
    }
}

#[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]
impl KeyStore for KeyringStore {
    fn load(&self) -> Result<Option<Vec<u8>>> {
        match self.entry()?.get_secret() {
            Ok(secret) => Ok(Some(secret)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(Error::KeyStore(e.to_string())),
        }
    }

    fn save(&self, secret: &[u8]) -> Result<()> {
        self.entry()?
            .set_secret(secret)
            .map_err(|e| Error::KeyStore(e.to_string()))
    }
}

pub struct Identity {
    seed: [u8; SEED_LEN],
    keypair: Ed25519KeyPair,
}

impl Identity {
    pub fn generate() -> Result<Self> {
        let mut seed = [0u8; SEED_LEN];
        SystemRandom::new()
            .fill(&mut seed)
            .map_err(|_| Error::Crypto("system RNG failed"))?;
        Self::from_seed(&seed)
    }

    pub fn from_seed(seed: &[u8]) -> Result<Self> {
        let seed: [u8; SEED_LEN] = seed
            .try_into()
            .map_err(|_| Error::KeyStore("stored identity key has the wrong length".into()))?;
        let keypair = Ed25519KeyPair::from_seed_unchecked(&seed)
            .map_err(|_| Error::Crypto("invalid Ed25519 seed"))?;
        Ok(Self { seed, keypair })
    }

    /// Load the identity from `store`, generating and saving a new one on first run.
    pub async fn load_or_generate(store: Arc<dyn KeyStore>) -> Result<Self> {
        tokio::task::spawn_blocking(move || match store.load()? {
            Some(seed) => Self::from_seed(&seed),
            None => {
                let identity = Self::generate()?;
                store.save(&identity.seed)?;
                Ok(identity)
            }
        })
        .await
        .map_err(|e| Error::KeyStore(format!("key store task failed: {e}")))?
    }

    pub fn public_key(&self) -> &[u8] {
        self.keypair.public_key().as_ref()
    }

    pub fn device_id(&self) -> DeviceId {
        DeviceId::from_public_key(self.public_key())
    }

    pub fn device_info(&self, name: impl Into<String>, os: OsType) -> DeviceInfo {
        DeviceInfo::new(name, os, self.public_key().to_vec())
    }

    pub fn sign(&self, message: &[u8]) -> Vec<u8> {
        self.keypair.sign(message).as_ref().to_vec()
    }

    /// The key as an unencrypted PKCS#8 v1 document, for handing to rustls.
    pub(crate) fn pkcs8_der(&self) -> Vec<u8> {
        // SEQUENCE { INTEGER 0, SEQUENCE { OID 1.3.101.112 }, OCTET STRING { OCTET STRING seed } }
        const PREFIX: [u8; 16] = [
            0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22,
            0x04, 0x20,
        ];
        [&PREFIX[..], &self.seed].concat()
    }
}

impl fmt::Debug for Identity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Identity")
            .field("device_id", &self.device_id())
            .finish_non_exhaustive()
    }
}

/// Verify an Ed25519 signature.
pub fn verify_signature(public_key: &[u8], message: &[u8], signature: &[u8]) -> bool {
    signature::UnparsedPublicKey::new(&signature::ED25519, public_key)
        .verify(message, signature)
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn load_or_generate_persists_and_reuses_key() {
        let store: Arc<dyn KeyStore> = Arc::new(MemoryKeyStore::default());
        let first = Identity::load_or_generate(store.clone()).await.unwrap();
        let second = Identity::load_or_generate(store.clone()).await.unwrap();
        assert_eq!(first.public_key(), second.public_key());
        assert_eq!(store.load().unwrap().unwrap().len(), SEED_LEN);
    }

    #[test]
    fn distinct_identities_have_distinct_keys() {
        let a = Identity::generate().unwrap();
        let b = Identity::generate().unwrap();
        assert_ne!(a.public_key(), b.public_key());
        assert_ne!(a.device_id(), b.device_id());
    }

    #[test]
    fn signatures_verify_only_with_matching_key_and_message() {
        let a = Identity::generate().unwrap();
        let b = Identity::generate().unwrap();
        let sig = a.sign(b"hello");
        assert!(verify_signature(a.public_key(), b"hello", &sig));
        assert!(!verify_signature(a.public_key(), b"hellO", &sig));
        assert!(!verify_signature(b.public_key(), b"hello", &sig));
    }

    #[test]
    fn rejects_wrong_length_seed() {
        assert!(Identity::from_seed(&[1; 31]).is_err());
    }

    #[test]
    fn device_info_is_valid() {
        let id = Identity::generate().unwrap();
        let info = id.device_info("laptop", OsType::Linux);
        info.validate().unwrap();
        assert_eq!(info.id, id.device_id());
    }

    #[test]
    fn pkcs8_matches_public_key() {
        let id = Identity::generate().unwrap();
        let reparsed = Ed25519KeyPair::from_pkcs8_maybe_unchecked(&id.pkcs8_der()).unwrap();
        assert_eq!(reparsed.public_key().as_ref(), id.public_key());
    }

    #[test]
    fn debug_does_not_leak_seed() {
        let id = Identity::from_seed(&[0x42; 32]).unwrap();
        let dbg = format!("{id:?}");
        assert!(!dbg.contains("42, 42"));
        assert!(!dbg.to_lowercase().contains("4242"));
    }
}
