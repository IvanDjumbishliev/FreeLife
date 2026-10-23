//! Find FreeLife devices on the local network via mDNS / DNS-SD.
//!
//! Each device registers a `_freelife._tcp` service whose TXT record carries
//! its [`DeviceInfo`]. Discovery is unauthenticated, so anyone on the LAN can
//! advertise anything. Treat results as hints for *where* to connect; trust
//! comes from pairing and the key pinning in [`crate::tls`].

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Mutex;

use mdns_sd::{ResolvedService, ServiceDaemon, ServiceEvent, ServiceInfo};
use tokio::sync::mpsc;

use crate::{DeviceId, DeviceInfo, OsType, Result, PROTOCOL_VERSION};

pub const SERVICE_TYPE: &str = "_freelife._tcp.local.";

const TXT_VERSION: &str = "v";
const TXT_ID: &str = "id";
const TXT_NAME: &str = "name";
const TXT_OS: &str = "os";
const TXT_PUBLIC_KEY: &str = "pk";

/// A device seen on the network, with where to reach it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredDevice {
    pub info: DeviceInfo,
    pub addresses: Vec<IpAddr>,
    pub port: u16,
}

impl DiscoveredDevice {
    pub fn socket_addrs(&self) -> impl Iterator<Item = SocketAddr> + '_ {
        self.addresses
            .iter()
            .map(|ip| SocketAddr::new(*ip, self.port))
    }
}

/// TXT record properties describing `info`.
pub fn txt_records(info: &DeviceInfo) -> HashMap<String, String> {
    HashMap::from([
        (TXT_VERSION.into(), PROTOCOL_VERSION.to_string()),
        (TXT_ID.into(), info.id.to_string()),
        (TXT_NAME.into(), info.name.clone()),
        (TXT_OS.into(), info.os.as_str().into()),
        (TXT_PUBLIC_KEY.into(), hex::encode(&info.public_key)),
    ])
}

/// Rebuild a [`DeviceInfo`] from TXT properties. `None` if fields are
/// missing or inconsistent, or the device speaks another protocol version.
pub fn device_from_txt<'a>(get: impl Fn(&str) -> Option<&'a str>) -> Option<DeviceInfo> {
    if get(TXT_VERSION)?.parse::<u16>().ok()? != PROTOCOL_VERSION {
        return None;
    }
    let info = DeviceInfo {
        id: get(TXT_ID)?.parse::<DeviceId>().ok()?,
        name: get(TXT_NAME)?.to_owned(),
        os: OsType::parse(get(TXT_OS)?)?,
        public_key: hex::decode(get(TXT_PUBLIC_KEY)?).ok()?,
    };
    info.validate().ok()?;
    Some(info)
}

fn from_resolved(svc: &ResolvedService) -> Option<DiscoveredDevice> {
    let info = device_from_txt(|k| svc.txt_properties.get_property_val_str(k))?;
    let mut addresses: Vec<IpAddr> = svc.addresses.iter().map(|a| a.to_ip_addr()).collect();
    addresses.sort();
    Some(DiscoveredDevice {
        info,
        addresses,
        port: svc.port,
    })
}

pub struct Discovery {
    daemon: ServiceDaemon,
    local_id: Mutex<Option<DeviceId>>,
}

impl Discovery {
    pub fn new() -> Result<Self> {
        Ok(Self {
            daemon: ServiceDaemon::new()?,
            local_id: Mutex::new(None),
        })
    }

    /// Announce this device on `port` (normally [`crate::DEFAULT_PORT`]).
    /// Our own advertisement is then filtered out of discovery results.
    pub fn advertise(&self, info: &DeviceInfo, port: u16) -> Result<()> {
        let instance = info.id.to_string();
        let host = format!("{instance}.local.");
        let service =
            ServiceInfo::new(SERVICE_TYPE, &instance, &host, "", port, txt_records(info))?
                .enable_addr_auto();
        self.daemon.register(service)?;
        *self.local_id.lock().expect("lock poisoned") = Some(info.id);
        tracing::info!(name = %info.name, port, "advertising on mDNS");
        Ok(())
    }

    /// Start browsing. The receiver yields each device when it's first
    /// resolved and again whenever its name, addresses or port change.
    /// Browsing stops when the receiver is dropped.
    pub async fn discover_devices(&self) -> Result<mpsc::Receiver<DiscoveredDevice>> {
        let events = self.daemon.browse(SERVICE_TYPE)?;
        let daemon = self.daemon.clone();
        let local_id = *self.local_id.lock().expect("lock poisoned");
        let (tx, rx) = mpsc::channel(32);

        tokio::spawn(async move {
            let mut seen: HashMap<DeviceId, DiscoveredDevice> = HashMap::new();
            loop {
                let event = tokio::select! {
                    event = events.recv_async() => match event {
                        Ok(event) => event,
                        Err(_) => break, // daemon shut down
                    },
                    _ = tx.closed() => break,
                };
                let ServiceEvent::ServiceResolved(svc) = event else {
                    continue;
                };
                let Some(device) = from_resolved(&svc) else {
                    tracing::debug!(service = %svc.fullname, "ignoring invalid FreeLife advertisement");
                    continue;
                };
                if Some(device.info.id) == local_id || seen.get(&device.info.id) == Some(&device) {
                    continue;
                }
                seen.insert(device.info.id, device.clone());
                if tx.send(device).await.is_err() {
                    break;
                }
            }
            let _ = daemon.stop_browse(SERVICE_TYPE);
        });

        Ok(rx)
    }

    pub fn shutdown(self) -> Result<()> {
        self.daemon.shutdown()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::Identity;

    #[test]
    fn txt_round_trip() {
        let info = Identity::generate()
            .unwrap()
            .device_info("Pixel", OsType::Android);
        let txt = txt_records(&info);
        assert_eq!(
            device_from_txt(|k| txt.get(k).map(String::as_str)),
            Some(info)
        );
        assert!(txt.iter().all(|(k, v)| k.len() + v.len() < 250));
    }

    #[test]
    fn txt_with_mismatched_key_is_rejected() {
        let info = Identity::generate()
            .unwrap()
            .device_info("x", OsType::Linux);
        let other = Identity::generate().unwrap();
        let mut txt = txt_records(&info);
        txt.insert(TXT_PUBLIC_KEY.into(), hex::encode(other.public_key()));
        assert_eq!(device_from_txt(|k| txt.get(k).map(String::as_str)), None);
    }

    #[test]
    fn txt_with_missing_or_bad_fields_is_rejected() {
        let info = Identity::generate()
            .unwrap()
            .device_info("x", OsType::Linux);
        for (key, value) in [
            (TXT_VERSION, Some("999")),
            (TXT_OS, Some("beos")),
            (TXT_PUBLIC_KEY, Some("zz")),
            (TXT_ID, None),
        ] {
            let mut txt = txt_records(&info);
            match value {
                Some(v) => txt.insert(key.into(), v.into()),
                None => txt.remove(key),
            };
            assert_eq!(
                device_from_txt(|k| txt.get(k).map(String::as_str)),
                None,
                "{key}"
            );
        }
    }

    /// Uses real multicast on this machine's interfaces, so it's opt-in:
    /// `cargo test -p freelife-core -- --ignored mdns`.
    #[tokio::test]
    #[ignore = "needs a multicast-capable network"]
    async fn mdns_advertise_and_discover() {
        let advertiser = Discovery::new().unwrap();
        let info = Identity::generate()
            .unwrap()
            .device_info("test-device", OsType::Linux);
        advertiser.advertise(&info, 7499).unwrap();

        let browser = Discovery::new().unwrap();
        let mut rx = browser.discover_devices().await.unwrap();
        let found = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let d = rx.recv().await.unwrap();
                if d.info.id == info.id {
                    return d;
                }
            }
        })
        .await
        .expect("device not discovered");
        assert_eq!(found.info, info);
        assert_eq!(found.port, 7499);
        assert!(!found.addresses.is_empty());
    }
}
