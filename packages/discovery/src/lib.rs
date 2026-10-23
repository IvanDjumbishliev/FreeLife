//! Find other FreeLife devices on the local network via mDNS / DNS-SD.
//!
//! Each device registers a `_freelife._tcp` service whose TXT record carries
//! its [`DeviceInfo`]; peers browse for that service type.

use std::collections::HashMap;
use std::net::IpAddr;
use std::time::{Duration, Instant};

use freelife_core::{DeviceId, DeviceInfo, Platform};
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};

pub const SERVICE_TYPE: &str = "_freelife._tcp.local.";

const TXT_ID: &str = "id";
const TXT_NAME: &str = "name";
const TXT_PLATFORM: &str = "platform";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("mDNS error: {0}")]
    Mdns(#[from] mdns_sd::Error),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

/// A peer seen on the network.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredDevice {
    pub info: DeviceInfo,
    pub addresses: Vec<IpAddr>,
}

/// TXT record properties describing `info`.
pub fn txt_records(info: &DeviceInfo) -> HashMap<String, String> {
    HashMap::from([
        (TXT_ID.into(), info.id.to_string()),
        (TXT_NAME.into(), info.name.clone()),
        (TXT_PLATFORM.into(), info.platform.as_str().into()),
    ])
}

/// Rebuild a [`DeviceInfo`] from TXT properties. Returns `None` if required
/// fields are missing or invalid (i.e. it's not a FreeLife peer we understand).
pub fn device_from_txt<'a>(get: impl Fn(&str) -> Option<&'a str>, port: u16) -> Option<DeviceInfo> {
    Some(DeviceInfo {
        id: get(TXT_ID)?.parse::<DeviceId>().ok()?,
        name: get(TXT_NAME)?.to_owned(),
        platform: Platform::parse(get(TXT_PLATFORM).unwrap_or_default()),
        port,
    })
}

pub struct Discovery {
    daemon: ServiceDaemon,
    local: DeviceInfo,
}

impl Discovery {
    pub fn new(local: DeviceInfo) -> Result<Self> {
        Ok(Self {
            daemon: ServiceDaemon::new()?,
            local,
        })
    }

    /// Start announcing this device on the network.
    pub fn advertise(&self) -> Result<()> {
        let instance = self.local.id.to_string();
        let host = format!("{instance}.local.");
        let service = ServiceInfo::new(
            SERVICE_TYPE,
            &instance,
            &host,
            "",
            self.local.port,
            txt_records(&self.local),
        )?
        .enable_addr_auto();
        self.daemon.register(service)?;
        tracing::info!(name = %self.local.name, "advertising FreeLife service");
        Ok(())
    }

    /// Browse for peers for `timeout`, returning every device that resolved.
    /// Our own advertisement is filtered out.
    pub fn scan(&self, timeout: Duration) -> Result<Vec<DiscoveredDevice>> {
        let rx = self.daemon.browse(SERVICE_TYPE)?;
        let deadline = Instant::now() + timeout;
        let mut found: HashMap<DeviceId, DiscoveredDevice> = HashMap::new();

        while let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
            let Ok(event) = rx.recv_timeout(remaining) else {
                break;
            };
            if let ServiceEvent::ServiceResolved(svc) = event {
                let info = device_from_txt(|k| svc.get_property_val_str(k), svc.get_port());
                if let Some(info) = info.filter(|i| i.id != self.local.id) {
                    let addresses = svc.get_addresses().iter().copied().collect();
                    found.insert(info.id, DiscoveredDevice { info, addresses });
                }
            }
        }

        self.daemon.stop_browse(SERVICE_TYPE)?;
        Ok(found.into_values().collect())
    }

    pub fn shutdown(self) -> Result<()> {
        self.daemon.shutdown()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn txt_round_trip() {
        let info = DeviceInfo::new("Pixel", Platform::Android, 4000);
        let txt = txt_records(&info);
        let parsed = device_from_txt(|k| txt.get(k).map(String::as_str), info.port).unwrap();
        assert_eq!(parsed, info);
    }

    #[test]
    fn txt_without_id_is_rejected() {
        let txt: HashMap<String, String> = HashMap::from([(TXT_NAME.into(), "x".into())]);
        assert!(device_from_txt(|k| txt.get(k).map(String::as_str), 1).is_none());
    }
}
