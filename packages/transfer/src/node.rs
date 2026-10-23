//! A running FreeLife device: identity, trust store, listener, mDNS,
//! pairing prompts and AirShare, wired together for the apps.

use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use freelife_core::connection::Incoming;
use freelife_core::discovery::{DiscoveredDevice, Discovery};
use freelife_core::identity::KeyStore;
use freelife_core::pairing::PinConfirmer;
use freelife_core::prompts::{PairingPrompts, PromptEvent};
use freelife_core::trust::{TrustedDevice, TRUSTED_DEVICES_FILE};
use freelife_core::{
    ActiveConnection, ConnectionConfig, ConnectionManager, DeviceId, DeviceInfo, FreeLifeMessage,
    Identity, OsType, TrustStore, DEFAULT_PORT,
};
use tokio::sync::{broadcast, mpsc};
use tokio_stream::StreamExt;
use uuid::Uuid;

use crate::progress::{Direction, TransferEvent};
use crate::{send_file_with_id, FileReceiver, ReceiverConfig, Result, TransferError};

/// How long [`Node::connection_to`] browses mDNS for a device it has no
/// address for.
const LOOKUP_TIMEOUT: Duration = Duration::from_secs(3);

pub struct NodeConfig {
    pub device_name: String,
    pub os: OsType,
    pub key_store: Arc<dyn KeyStore>,
    /// App data directory; `trusted_devices.json` is kept here.
    pub data_dir: PathBuf,
    pub receiver: ReceiverConfig,
    /// TCP port to listen on. If it's taken, an ephemeral port is used (and
    /// advertised) instead. Use 0 to always pick an ephemeral port.
    pub port: u16,
    /// Advertise and browse via mDNS.
    pub enable_mdns: bool,
    pub connection: ConnectionConfig,
    /// How long a pairing code waits for the user before counting as rejected.
    pub pairing_prompt_timeout: Duration,
}

impl NodeConfig {
    pub fn new(
        device_name: impl Into<String>,
        os: OsType,
        key_store: Arc<dyn KeyStore>,
        data_dir: impl Into<PathBuf>,
    ) -> Self {
        Self {
            device_name: device_name.into(),
            os,
            key_store,
            data_dir: data_dir.into(),
            receiver: ReceiverConfig::default(),
            port: DEFAULT_PORT,
            enable_mdns: true,
            connection: ConnectionConfig::default(),
            pairing_prompt_timeout: Duration::from_secs(90),
        }
    }
}

pub struct Node {
    device: DeviceInfo,
    port: u16,
    manager: ConnectionManager,
    discovery: Option<Discovery>,
    prompts: Arc<PairingPrompts>,
    events: broadcast::Sender<TransferEvent>,
    /// Last known addresses of devices, from discovery or pairing.
    addresses: Mutex<HashMap<DeviceId, Vec<SocketAddr>>>,
    other_messages: Mutex<Option<mpsc::Receiver<(DeviceId, FreeLifeMessage)>>>,
}

impl Node {
    pub async fn start(config: NodeConfig) -> Result<Arc<Self>> {
        let identity = Arc::new(Identity::load_or_generate(config.key_store).await?);
        let device = identity.device_info(config.device_name, config.os);
        device.validate()?;
        let trust = Arc::new(TrustStore::open(config.data_dir.join(TRUSTED_DEVICES_FILE)).await?);
        let (manager, incoming) =
            ConnectionManager::new(identity, device.clone(), trust, config.connection)?;

        let prompts = Arc::new(PairingPrompts::new(config.pairing_prompt_timeout));
        let port = listen(&manager, config.port, prompts.clone()).await?;

        let discovery = if config.enable_mdns {
            Discovery::new()
                .and_then(|d| d.advertise(&device, port).map(|()| d))
                .inspect_err(|e| tracing::warn!(error = %e, "mDNS unavailable"))
                .ok()
        } else {
            None
        };

        let events = broadcast::channel(512).0;
        let other_messages = spawn_receiver(incoming, config.receiver, events.clone());

        Ok(Arc::new(Self {
            device,
            port,
            manager,
            discovery,
            prompts,
            events,
            addresses: Mutex::new(HashMap::new()),
            other_messages: Mutex::new(Some(other_messages)),
        }))
    }

    pub fn device(&self) -> &DeviceInfo {
        &self.device
    }

    /// The port we're listening on.
    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn manager(&self) -> &ConnectionManager {
        &self.manager
    }

    pub fn mdns_available(&self) -> bool {
        self.discovery.is_some()
    }

    /// Progress, completion and errors for transfers in both directions.
    /// Slow subscribers may see `RecvError::Lagged`; just keep receiving.
    pub fn subscribe_transfers(&self) -> broadcast::Receiver<TransferEvent> {
        self.events.subscribe()
    }

    /// Pairing codes to show the user, for both incoming and outgoing pairing.
    pub fn subscribe_pairing(&self) -> broadcast::Receiver<PromptEvent> {
        self.prompts.subscribe()
    }

    pub fn respond_pairing(&self, request_id: u64, accept: bool) -> bool {
        self.prompts.respond(request_id, accept)
    }

    /// Non-transfer messages (clipboard, notifications, handoff URLs).
    /// Available once; later calls return `None`.
    pub fn take_other_messages(&self) -> Option<mpsc::Receiver<(DeviceId, FreeLifeMessage)>> {
        self.other_messages.lock().expect("lock poisoned").take()
    }

    pub fn paired_devices(&self) -> Vec<TrustedDevice> {
        self.manager.trust_store().list()
    }

    pub fn is_paired(&self, device_id: DeviceId) -> bool {
        self.manager.trust_store().is_trusted(device_id)
    }

    pub fn is_connected(&self, device_id: DeviceId) -> bool {
        self.manager.is_connected(device_id)
    }

    pub async fn unpair(&self, device_id: DeviceId) -> Result<bool> {
        self.manager.disconnect(device_id);
        Ok(self.manager.trust_store().revoke(device_id).await?)
    }

    pub fn remember_address(&self, device_id: DeviceId, addr: SocketAddr) {
        let mut addresses = self.addresses.lock().expect("lock poisoned");
        let known = addresses.entry(device_id).or_default();
        known.retain(|a| *a != addr);
        known.insert(0, addr);
    }

    /// Browse mDNS for `timeout` and return every device seen.
    pub async fn discover(&self, timeout: Duration) -> Result<Vec<DiscoveredDevice>> {
        let Some(discovery) = &self.discovery else {
            return Ok(Vec::new());
        };
        let mut rx = discovery.discover_devices().await?;
        let mut found: HashMap<DeviceId, DiscoveredDevice> = HashMap::new();
        let deadline = tokio::time::Instant::now() + timeout;
        while let Ok(Some(device)) = tokio::time::timeout_at(deadline, rx.recv()).await {
            found.insert(device.info.id, device);
        }
        let mut found: Vec<_> = found.into_values().collect();
        found.sort_by(|a, b| a.info.name.cmp(&b.info.name));
        self.record(&found);
        Ok(found)
    }

    /// Pair with a device found by [`discover`](Self::discover).
    pub async fn pair(&self, device_id: DeviceId) -> Result<DeviceInfo> {
        let addrs = self.known_addresses(device_id);
        if addrs.is_empty() {
            return Err(TransferError::Unreachable(device_id));
        }
        let mut last_err = None;
        for addr in addrs {
            match self.pair_at(addr).await {
                Ok(peer) if peer.id == device_id => return Ok(peer),
                Ok(peer) => {
                    // Someone else answered at that address. The user saw
                    // and approved `peer`'s details, so the pairing stands,
                    // but it isn't the device that was asked for.
                    tracing::warn!(expected = %device_id, got = %peer.id, "paired with a different device");
                    return Ok(peer);
                }
                Err(e) => last_err = Some(e),
            }
        }
        Err(last_err.unwrap_or(TransferError::Unreachable(device_id)))
    }

    /// Pair with whatever device is listening at `addr`, for when mDNS
    /// isn't available.
    pub async fn pair_at(&self, addr: SocketAddr) -> Result<DeviceInfo> {
        let outcome = self.manager.pair(addr, self.prompts.as_ref()).await?;
        self.remember_address(outcome.peer.id, addr);
        Ok(outcome.peer)
    }

    /// The session with a paired device, connecting first if needed. Tries
    /// known addresses, then browses mDNS for the device.
    pub async fn connection_to(&self, device_id: DeviceId) -> Result<ActiveConnection> {
        if let Some(conn) = self
            .manager
            .connection(device_id)
            .filter(|c| !c.is_closed())
        {
            return Ok(conn);
        }
        if !self.is_paired(device_id) {
            return Err(TransferError::NotPaired(device_id));
        }
        if self
            .try_connect(device_id, self.known_addresses(device_id))
            .await
        {
            return self.current_connection(device_id);
        }
        self.discover(LOOKUP_TIMEOUT).await?;
        if self
            .try_connect(device_id, self.known_addresses(device_id))
            .await
        {
            return self.current_connection(device_id);
        }
        Err(TransferError::Unreachable(device_id))
    }

    /// Send a file to a paired device, reporting progress on
    /// [`subscribe_transfers`](Self::subscribe_transfers). Resolves when the
    /// last chunk has been handed to the connection.
    pub async fn send_file(&self, device_id: DeviceId, path: &Path) -> Result<Uuid> {
        let id = Uuid::new_v4();
        self.send_file_as(id, device_id, path).await.map(|()| id)
    }

    /// [`send_file`](Self::send_file) with a caller-chosen transfer id.
    pub async fn send_file_as(&self, id: Uuid, device_id: DeviceId, path: &Path) -> Result<()> {
        let filename = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let result: Result<u64> = async {
            let conn = self.connection_to(device_id).await?;
            let mut progress = std::pin::pin!(send_file_with_id(&conn, path, id));
            let mut total = 0;
            while let Some(update) = progress.next().await {
                let update = update?;
                total = update.total_bytes;
                let _ = self.events.send(TransferEvent::Progress(update));
            }
            Ok(total)
        }
        .await;

        let event = match &result {
            Ok(total_bytes) => TransferEvent::TransferComplete {
                id,
                peer: device_id,
                direction: Direction::Send,
                filename,
                total_bytes: *total_bytes,
                path: None,
            },
            Err(e) => TransferEvent::TransferError {
                id,
                peer: device_id,
                direction: Direction::Send,
                filename,
                error: e.to_string(),
            },
        };
        let _ = self.events.send(event);
        result.map(|_| ())
    }

    /// Stop listening, close sessions, and stop advertising.
    pub fn shutdown(&self) {
        self.manager.shutdown();
    }

    fn record(&self, devices: &[DiscoveredDevice]) {
        let mut addresses = self.addresses.lock().expect("lock poisoned");
        for d in devices {
            // IPv4 first: we only listen on IPv4, and link-local IPv6
            // addresses need scope ids we don't track.
            let mut addrs: Vec<SocketAddr> = d.socket_addrs().filter(SocketAddr::is_ipv4).collect();
            addrs.extend(d.socket_addrs().filter(SocketAddr::is_ipv6));
            addresses.insert(d.info.id, addrs);
        }
    }

    fn known_addresses(&self, device_id: DeviceId) -> Vec<SocketAddr> {
        self.addresses
            .lock()
            .expect("lock poisoned")
            .get(&device_id)
            .cloned()
            .unwrap_or_default()
    }

    async fn try_connect(&self, device_id: DeviceId, addrs: Vec<SocketAddr>) -> bool {
        for addr in addrs {
            match self.manager.connect(device_id, addr).await {
                Ok(()) => {
                    self.remember_address(device_id, addr);
                    return true;
                }
                Err(e) => tracing::debug!(%addr, error = %e, "connect failed"),
            }
        }
        false
    }

    fn current_connection(&self, device_id: DeviceId) -> Result<ActiveConnection> {
        self.manager
            .connection(device_id)
            .ok_or(TransferError::Unreachable(device_id))
    }
}

impl Drop for Node {
    fn drop(&mut self) {
        self.manager.shutdown();
        if let Some(d) = self.discovery.take() {
            let _ = d.shutdown();
        }
    }
}

async fn listen(
    manager: &ConnectionManager,
    port: u16,
    prompts: Arc<PairingPrompts>,
) -> Result<u16> {
    let confirmer: Arc<dyn PinConfirmer> = prompts;
    let any = |port| SocketAddr::from((Ipv4Addr::UNSPECIFIED, port));
    let addr = match manager.listen(any(port), Some(confirmer.clone())).await {
        Ok(addr) => addr,
        Err(e) if port != 0 => {
            tracing::warn!(port, error = %e, "port unavailable, using an ephemeral port");
            manager.listen(any(0), Some(confirmer)).await?
        }
        Err(e) => return Err(e.into()),
    };
    Ok(addr.port())
}

fn spawn_receiver(
    incoming: Incoming,
    config: ReceiverConfig,
    events: broadcast::Sender<TransferEvent>,
) -> mpsc::Receiver<(DeviceId, FreeLifeMessage)> {
    let (other_tx, other_rx) = mpsc::channel(64);
    tokio::spawn(FileReceiver::new(config, events).run(incoming, other_tx));
    other_rx
}
