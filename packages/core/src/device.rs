use std::fmt;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Stable, randomly generated identifier for a device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DeviceId(Uuid);

impl DeviceId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for DeviceId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for DeviceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl std::str::FromStr for DeviceId {
    type Err = uuid::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(s).map(Self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    Android,
    Ios,
    Linux,
    Windows,
    Macos,
    Browser,
    Unknown,
}

impl Platform {
    /// The platform this binary was compiled for.
    pub fn current() -> Self {
        if cfg!(target_os = "android") {
            Self::Android
        } else if cfg!(target_os = "ios") {
            Self::Ios
        } else if cfg!(target_os = "linux") {
            Self::Linux
        } else if cfg!(target_os = "windows") {
            Self::Windows
        } else if cfg!(target_os = "macos") {
            Self::Macos
        } else {
            Self::Unknown
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Android => "android",
            Self::Ios => "ios",
            Self::Linux => "linux",
            Self::Windows => "windows",
            Self::Macos => "macos",
            Self::Browser => "browser",
            Self::Unknown => "unknown",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "android" => Self::Android,
            "ios" => Self::Ios,
            "linux" => Self::Linux,
            "windows" => Self::Windows,
            "macos" => Self::Macos,
            "browser" => Self::Browser,
            _ => Self::Unknown,
        }
    }
}

/// What a device tells its peers about itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub id: DeviceId,
    pub name: String,
    pub platform: Platform,
    pub port: u16,
}

impl DeviceInfo {
    pub fn new(name: impl Into<String>, platform: Platform, port: u16) -> Self {
        Self {
            id: DeviceId::new(),
            name: name.into(),
            platform,
            port,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_round_trips_through_str() {
        for p in [
            Platform::Android,
            Platform::Ios,
            Platform::Linux,
            Platform::Windows,
            Platform::Macos,
            Platform::Browser,
        ] {
            assert_eq!(Platform::parse(p.as_str()), p);
        }
        assert_eq!(Platform::parse("plan9"), Platform::Unknown);
    }

    #[test]
    fn device_id_round_trips_through_string() {
        let id = DeviceId::new();
        assert_eq!(id.to_string().parse::<DeviceId>().unwrap(), id);
    }
}
