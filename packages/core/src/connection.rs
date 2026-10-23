//! Keeps encrypted sessions to paired devices and routes messages.
//!
//! One TCP port serves both pairing and sessions. The first byte the dialer
//! sends picks the mode: [`MODE_PAIR`] runs the plaintext
//! [pairing handshake](crate::pairing), and [`MODE_SESSION`] starts TLS 1.3
//! with pinned identity keys (see [`crate::tls`]).
//!
//! Each session runs as one task. It sends a [`FreeLifeMessage::Ping`] every
//! `keepalive_interval`, answers pings automatically, and drops the
//! connection if nothing arrives for `idle_timeout`. Pings and pongs never
//! reach the incoming-message receiver.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio::task::{AbortHandle, JoinHandle};
use tokio::time::{timeout, MissedTickBehavior};
use tokio_rustls::{TlsAcceptor, TlsConnector};

use crate::pairing::{pair_as_initiator, pair_as_responder, PairingOutcome, PinConfirmer};
use crate::protocol::{read_frame, write_frame};
use crate::{tls, DeviceId, DeviceInfo, Error, FreeLifeMessage, Identity, PairingError, Result};
use crate::{TrustStore, DEFAULT_PORT};

/// First byte on a connection: run the pairing handshake.
pub const MODE_PAIR: u8 = b'P';
/// First byte on a connection: start a TLS session.
pub const MODE_SESSION: u8 = b'S';

const OUTGOING_QUEUE: usize = 64;
const INCOMING_QUEUE: usize = 256;

#[derive(Debug, Clone)]
pub struct ConnectionConfig {
    pub keepalive_interval: Duration,
    /// Drop a session after this long without receiving anything.
    pub idle_timeout: Duration,
    /// Limit for TCP connect, the mode byte and the TLS handshake.
    pub handshake_timeout: Duration,
    /// Limit for a whole pairing exchange, including the user confirming.
    pub pairing_timeout: Duration,
}

impl Default for ConnectionConfig {
    fn default() -> Self {
        Self {
            keepalive_interval: Duration::from_secs(30),
            idle_timeout: Duration::from_secs(90),
            handshake_timeout: Duration::from_secs(10),
            pairing_timeout: Duration::from_secs(120),
        }
    }
}

/// Messages from connected devices.
pub type Incoming = mpsc::Receiver<(DeviceId, FreeLifeMessage)>;

struct ActiveConnection {
    conn_id: u64,
    outgoing: mpsc::Sender<FreeLifeMessage>,
    task: AbortHandle,
}

struct Inner {
    identity: Arc<Identity>,
    local: DeviceInfo,
    trust: Arc<TrustStore>,
    config: ConnectionConfig,
    acceptor: TlsAcceptor,
    connections: Mutex<HashMap<DeviceId, ActiveConnection>>,
    incoming: mpsc::Sender<(DeviceId, FreeLifeMessage)>,
    next_conn_id: AtomicU64,
    listener: Mutex<Option<JoinHandle<()>>>,
}

/// Cheap to clone; all clones share the same connections.
#[derive(Clone)]
pub struct ConnectionManager {
    inner: Arc<Inner>,
}

impl ConnectionManager {
    pub fn new(
        identity: Arc<Identity>,
        local: DeviceInfo,
        trust: Arc<TrustStore>,
        config: ConnectionConfig,
    ) -> Result<(Self, Incoming)> {
        if local.public_key != identity.public_key() {
            return Err(Error::InvalidDeviceInfo(
                "local device info does not match identity",
            ));
        }
        let acceptor = TlsAcceptor::from(tls::server_config(&identity, trust.clone())?);
        let (incoming, rx) = mpsc::channel(INCOMING_QUEUE);
        let manager = Self {
            inner: Arc::new(Inner {
                identity,
                local,
                trust,
                config,
                acceptor,
                connections: Mutex::new(HashMap::new()),
                incoming,
                next_conn_id: AtomicU64::new(0),
                listener: Mutex::new(None),
            }),
        };
        Ok((manager, rx))
    }

    pub fn local_device(&self) -> &DeviceInfo {
        &self.inner.local
    }

    pub fn trust_store(&self) -> &Arc<TrustStore> {
        &self.inner.trust
    }

    /// Accept connections on `addr` (use port 0 for an ephemeral port, or
    /// [`DEFAULT_PORT`]). Incoming pairing requests go to `pairing`; pass
    /// `None` to refuse them. Returns the bound address.
    pub async fn listen(
        &self,
        addr: SocketAddr,
        pairing: Option<Arc<dyn PinConfirmer>>,
    ) -> Result<SocketAddr> {
        let listener = TcpListener::bind(addr).await?;
        let bound = listener.local_addr()?;
        let manager = self.clone();
        let task = tokio::spawn(async move {
            loop {
                let (stream, peer) = match listener.accept().await {
                    Ok(conn) => conn,
                    Err(e) => {
                        tracing::warn!(error = %e, "accept failed");
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        continue;
                    }
                };
                let manager = manager.clone();
                let pairing = pairing.clone();
                tokio::spawn(async move {
                    if let Err(e) = manager.handle_inbound(stream, pairing).await {
                        tracing::debug!(%peer, error = %e, "inbound connection failed");
                    }
                });
            }
        });
        if let Some(old) = self
            .inner
            .listener
            .lock()
            .expect("lock poisoned")
            .replace(task)
        {
            old.abort();
        }
        tracing::info!(addr = %bound, "listening");
        Ok(bound)
    }

    /// Listen on all interfaces on [`DEFAULT_PORT`].
    pub async fn listen_default(
        &self,
        pairing: Option<Arc<dyn PinConfirmer>>,
    ) -> Result<SocketAddr> {
        self.listen(SocketAddr::from(([0, 0, 0, 0], DEFAULT_PORT)), pairing)
            .await
    }

    /// Pair with the device listening at `addr`. On success it's in the trust
    /// store and [`connect`](Self::connect) will work.
    pub async fn pair(
        &self,
        addr: SocketAddr,
        confirmer: &dyn PinConfirmer,
    ) -> Result<PairingOutcome> {
        let config = &self.inner.config;
        let mut stream = timeout(config.handshake_timeout, TcpStream::connect(addr))
            .await
            .map_err(|_| Error::Timeout)??;
        stream.set_nodelay(true)?;
        stream.write_all(&[MODE_PAIR]).await?;
        let inner = &self.inner;
        timeout(
            config.pairing_timeout,
            pair_as_initiator(
                &mut stream,
                &inner.identity,
                &inner.local,
                &inner.trust,
                confirmer,
            ),
        )
        .await
        .map_err(|_| Error::Timeout)?
    }

    /// Open an encrypted session to a paired device at `addr`, replacing any
    /// existing session with it.
    pub async fn connect(&self, device_id: DeviceId, addr: SocketAddr) -> Result<()> {
        let peer = self
            .inner
            .trust
            .get(device_id)
            .ok_or(Error::NotPaired(device_id))?;
        let limit = self.inner.config.handshake_timeout;
        let connector =
            TlsConnector::from(tls::client_config(&self.inner.identity, &peer.public_key)?);

        let handshake = async {
            let mut tcp = TcpStream::connect(addr).await?;
            tcp.set_nodelay(true)?;
            tcp.write_all(&[MODE_SESSION]).await?;
            connector.connect(tls::server_name(), tcp).await
        };
        let stream = timeout(limit, handshake)
            .await
            .map_err(|_| Error::Timeout)??;
        self.register(device_id, stream);
        tracing::info!(device = %device_id, %addr, "connected");
        Ok(())
    }

    /// Queue `message` for `device_id`. Waits if the connection's send
    /// queue is full.
    pub async fn send(&self, device_id: DeviceId, message: FreeLifeMessage) -> Result<()> {
        let sender = self
            .inner
            .connections
            .lock()
            .expect("lock poisoned")
            .get(&device_id)
            .map(|c| c.outgoing.clone())
            .ok_or(Error::NotConnected(device_id))?;
        sender
            .send(message)
            .await
            .map_err(|_| Error::NotConnected(device_id))
    }

    pub fn is_connected(&self, device_id: DeviceId) -> bool {
        self.inner
            .connections
            .lock()
            .expect("lock poisoned")
            .contains_key(&device_id)
    }

    pub fn connected_devices(&self) -> Vec<DeviceId> {
        self.inner
            .connections
            .lock()
            .expect("lock poisoned")
            .keys()
            .copied()
            .collect()
    }

    /// Close the session with `device_id`. Returns whether one existed.
    pub fn disconnect(&self, device_id: DeviceId) -> bool {
        let removed = self
            .inner
            .connections
            .lock()
            .expect("lock poisoned")
            .remove(&device_id);
        if let Some(conn) = &removed {
            conn.task.abort();
        }
        removed.is_some()
    }

    /// Stop listening and close every session.
    pub fn shutdown(&self) {
        if let Some(task) = self.inner.listener.lock().expect("lock poisoned").take() {
            task.abort();
        }
        for (_, conn) in self
            .inner
            .connections
            .lock()
            .expect("lock poisoned")
            .drain()
        {
            conn.task.abort();
        }
    }

    async fn handle_inbound(
        &self,
        mut stream: TcpStream,
        pairing: Option<Arc<dyn PinConfirmer>>,
    ) -> Result<()> {
        stream.set_nodelay(true)?;
        let config = &self.inner.config;
        let mode = timeout(config.handshake_timeout, stream.read_u8())
            .await
            .map_err(|_| Error::Timeout)??;

        match mode {
            MODE_PAIR => {
                let confirmer = pairing.ok_or(PairingError::Disabled)?;
                let inner = &self.inner;
                let outcome = timeout(
                    config.pairing_timeout,
                    pair_as_responder(
                        &mut stream,
                        &inner.identity,
                        &inner.local,
                        &inner.trust,
                        confirmer.as_ref(),
                    ),
                )
                .await
                .map_err(|_| Error::Timeout)??;
                tracing::info!(device = %outcome.peer.id, name = %outcome.peer.name, "paired");
                Ok(())
            }
            MODE_SESSION => {
                let tls = timeout(config.handshake_timeout, self.inner.acceptor.accept(stream))
                    .await
                    .map_err(|_| Error::Timeout)??;
                // The verifier only admits trusted keys, so this always resolves.
                let peer = tls::peer_device_id(tls.get_ref().1.peer_certificates())
                    .filter(|id| self.inner.trust.is_trusted(*id))
                    .ok_or(Error::UntrustedPeer)?;
                self.register(peer, tls);
                tracing::info!(device = %peer, "accepted session");
                Ok(())
            }
            _ => Err(Error::Protocol("unknown connection mode")),
        }
    }

    fn register<S>(&self, peer: DeviceId, stream: S)
    where
        S: AsyncRead + AsyncWrite + Send + Unpin + 'static,
    {
        let conn_id = self.inner.next_conn_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = mpsc::channel(OUTGOING_QUEUE);

        // Spawn while holding the lock so the task can't finish and try to
        // deregister before it's been inserted.
        let mut connections = self.inner.connections.lock().expect("lock poisoned");
        let task = tokio::spawn(run_session(
            self.inner.clone(),
            peer,
            conn_id,
            stream,
            tx.clone(),
            rx,
        ));
        let replaced = connections.insert(
            peer,
            ActiveConnection {
                conn_id,
                outgoing: tx,
                task: task.abort_handle(),
            },
        );
        if let Some(old) = replaced {
            old.task.abort();
        }
    }
}

async fn run_session<S>(
    inner: Arc<Inner>,
    peer: DeviceId,
    conn_id: u64,
    stream: S,
    outgoing_tx: mpsc::Sender<FreeLifeMessage>,
    outgoing_rx: mpsc::Receiver<FreeLifeMessage>,
) where
    S: AsyncRead + AsyncWrite + Send + Unpin + 'static,
{
    let (reader, writer) = tokio::io::split(stream);
    let result = tokio::select! {
        r = read_loop(&inner, peer, reader, outgoing_tx) => r,
        r = write_loop(&inner.config, writer, outgoing_rx) => r,
    };
    match result {
        Ok(()) | Err(Error::ConnectionClosed) => tracing::info!(device = %peer, "session closed"),
        Err(e) => tracing::warn!(device = %peer, error = %e, "session ended"),
    }

    let mut connections = inner.connections.lock().expect("lock poisoned");
    // Only remove ourselves; a newer connection may have replaced us.
    if connections.get(&peer).is_some_and(|c| c.conn_id == conn_id) {
        connections.remove(&peer);
    }
}

async fn read_loop<R>(
    inner: &Inner,
    peer: DeviceId,
    mut reader: R,
    outgoing: mpsc::Sender<FreeLifeMessage>,
) -> Result<()>
where
    R: AsyncRead + Unpin,
{
    loop {
        let msg: FreeLifeMessage = timeout(inner.config.idle_timeout, read_frame(&mut reader))
            .await
            .map_err(|_| Error::Timeout)??;
        match msg {
            FreeLifeMessage::Ping => {
                if outgoing.send(FreeLifeMessage::Pong).await.is_err() {
                    return Ok(());
                }
            }
            FreeLifeMessage::Pong => {}
            other => {
                if inner.incoming.send((peer, other)).await.is_err() {
                    // Nobody is listening for messages any more.
                    return Ok(());
                }
            }
        }
    }
}

async fn write_loop<W>(
    config: &ConnectionConfig,
    mut writer: W,
    mut outgoing: mpsc::Receiver<FreeLifeMessage>,
) -> Result<()>
where
    W: AsyncWrite + Unpin,
{
    let mut keepalive = tokio::time::interval(config.keepalive_interval);
    keepalive.set_missed_tick_behavior(MissedTickBehavior::Delay);
    keepalive.tick().await; // the first tick fires immediately

    loop {
        tokio::select! {
            msg = outgoing.recv() => match msg {
                Some(msg) => {
                    write_frame(&mut writer, &msg).await?;
                    keepalive.reset();
                }
                None => return Ok(()),
            },
            _ = keepalive.tick() => write_frame(&mut writer, &FreeLifeMessage::Ping).await?,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;

    use super::*;
    use crate::pairing::PairingCode;
    use crate::{ClipboardContentType, OsType};

    struct Node {
        manager: ConnectionManager,
        incoming: Incoming,
        addr: SocketAddr,
        info: DeviceInfo,
        identity: Arc<Identity>,
    }

    fn fast_config() -> ConnectionConfig {
        ConnectionConfig {
            keepalive_interval: Duration::from_millis(50),
            idle_timeout: Duration::from_millis(300),
            handshake_timeout: Duration::from_secs(5),
            pairing_timeout: Duration::from_secs(5),
        }
    }

    fn accept_all() -> Arc<dyn PinConfirmer> {
        Arc::new(|_: DeviceInfo, _: PairingCode| async { true })
    }

    async fn node(name: &str, config: ConnectionConfig) -> Node {
        let identity = Arc::new(Identity::generate().unwrap());
        let info = identity.device_info(name, OsType::Linux);
        let trust = Arc::new(TrustStore::in_memory());
        let (manager, incoming) =
            ConnectionManager::new(identity.clone(), info.clone(), trust, config).unwrap();
        let addr = manager
            .listen((Ipv4Addr::LOCALHOST, 0).into(), Some(accept_all()))
            .await
            .unwrap();
        Node {
            manager,
            incoming,
            addr,
            info,
            identity,
        }
    }

    async fn trust_each_other(a: &Node, b: &Node) {
        a.manager.trust_store().trust(&b.info).await.unwrap();
        b.manager.trust_store().trust(&a.info).await.unwrap();
    }

    async fn eventually(mut cond: impl FnMut() -> bool) -> bool {
        for _ in 0..100 {
            if cond() {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        false
    }

    async fn recv(node: &mut Node) -> (DeviceId, FreeLifeMessage) {
        timeout(Duration::from_secs(5), node.incoming.recv())
            .await
            .expect("timed out waiting for message")
            .expect("incoming channel closed")
    }

    #[tokio::test]
    async fn messages_flow_both_ways() {
        let mut a = node("a", ConnectionConfig::default()).await;
        let mut b = node("b", ConnectionConfig::default()).await;
        trust_each_other(&a, &b).await;

        a.manager.connect(b.info.id, b.addr).await.unwrap();
        assert!(a.manager.is_connected(b.info.id));
        assert!(eventually(|| b.manager.is_connected(a.info.id)).await);

        let clip = FreeLifeMessage::ClipboardUpdate {
            content_type: ClipboardContentType::Text,
            payload: b"copied on a".to_vec(),
        };
        a.manager.send(b.info.id, clip.clone()).await.unwrap();
        assert_eq!(recv(&mut b).await, (a.info.id, clip));

        let url = FreeLifeMessage::HandoffUrl {
            url: "https://freelife.dev".into(),
        };
        b.manager.send(a.info.id, url.clone()).await.unwrap();
        assert_eq!(recv(&mut a).await, (b.info.id, url));
    }

    #[tokio::test]
    async fn keepalive_holds_idle_sessions_open() {
        let a = node("a", fast_config()).await;
        let b = node("b", fast_config()).await;
        trust_each_other(&a, &b).await;
        a.manager.connect(b.info.id, b.addr).await.unwrap();

        // Several idle timeouts pass; pings keep both ends alive.
        tokio::time::sleep(fast_config().idle_timeout * 4).await;
        assert!(a.manager.is_connected(b.info.id));
        assert!(b.manager.is_connected(a.info.id));
    }

    #[tokio::test]
    async fn pings_are_sent_and_answered_but_not_surfaced() {
        let mut a = node("a", fast_config()).await;
        let b = node("b", fast_config()).await;
        trust_each_other(&a, &b).await;

        // Dial `a` by hand so we can see the raw frames.
        let mut tcp = TcpStream::connect(a.addr).await.unwrap();
        tcp.write_all(&[MODE_SESSION]).await.unwrap();
        let connector =
            TlsConnector::from(tls::client_config(&b.identity, &a.info.public_key).unwrap());
        let mut tls = connector.connect(tls::server_name(), tcp).await.unwrap();

        let first: FreeLifeMessage = read_frame(&mut tls).await.unwrap();
        assert_eq!(first, FreeLifeMessage::Ping);

        write_frame(&mut tls, &FreeLifeMessage::Ping).await.unwrap();
        loop {
            match read_frame::<_, FreeLifeMessage>(&mut tls).await.unwrap() {
                FreeLifeMessage::Pong => break,
                FreeLifeMessage::Ping => continue,
                other => panic!("unexpected {other:?}"),
            }
        }
        assert!(
            a.incoming.try_recv().is_err(),
            "ping/pong must not be surfaced"
        );
    }

    #[tokio::test]
    async fn silent_peer_is_dropped_after_idle_timeout() {
        let a = node("a", fast_config()).await;
        let b = node("b", fast_config()).await;
        trust_each_other(&a, &b).await;

        let mut tcp = TcpStream::connect(a.addr).await.unwrap();
        tcp.write_all(&[MODE_SESSION]).await.unwrap();
        let connector =
            TlsConnector::from(tls::client_config(&b.identity, &a.info.public_key).unwrap());
        let _tls = connector.connect(tls::server_name(), tcp).await.unwrap();

        assert!(eventually(|| a.manager.is_connected(b.info.id)).await);
        // We never read or write, so `a` hears nothing and gives up.
        assert!(eventually(|| !a.manager.is_connected(b.info.id)).await);
    }

    #[tokio::test]
    async fn unpaired_devices_cannot_connect() {
        let a = node("a", fast_config()).await;
        let b = node("b", fast_config()).await;

        // a doesn't know b at all.
        assert!(matches!(
            a.manager.connect(b.info.id, b.addr).await,
            Err(Error::NotPaired(_))
        ));

        // a trusts b, but b doesn't trust a: b refuses the session.
        a.manager.trust_store().trust(&b.info).await.unwrap();
        let _ = a.manager.connect(b.info.id, b.addr).await;
        assert!(eventually(|| !a.manager.is_connected(b.info.id)).await);
        assert!(!b.manager.is_connected(a.info.id));
    }

    #[tokio::test]
    async fn connecting_to_wrong_device_fails() {
        let a = node("a", fast_config()).await;
        let b = node("b", fast_config()).await;
        let c = node("c", fast_config()).await;
        trust_each_other(&a, &b).await;
        trust_each_other(&a, &c).await;

        // a thinks it's dialing b, but c answers.
        assert!(a.manager.connect(b.info.id, c.addr).await.is_err());
        assert!(!a.manager.is_connected(b.info.id));
    }

    #[tokio::test]
    async fn pair_then_connect_end_to_end() {
        let a = node("a", fast_config()).await;
        let mut b = node("b", fast_config()).await;

        let confirmer = |_: DeviceInfo, _: PairingCode| async { true };
        let outcome = a.manager.pair(b.addr, &confirmer).await.unwrap();
        assert_eq!(outcome.peer, b.info);
        assert!(a.manager.trust_store().is_trusted(b.info.id));
        assert!(eventually(|| b.manager.trust_store().is_trusted(a.info.id)).await);

        a.manager.connect(b.info.id, b.addr).await.unwrap();
        let msg = FreeLifeMessage::NotificationEvent {
            app: "Messages".into(),
            title: "Bob".into(),
            body: "hi".into(),
            reply_token: None,
        };
        a.manager.send(b.info.id, msg.clone()).await.unwrap();
        assert_eq!(recv(&mut b).await, (a.info.id, msg));
    }

    #[tokio::test]
    async fn pairing_refused_when_disabled() {
        let a = node("a", fast_config()).await;
        let identity = Arc::new(Identity::generate().unwrap());
        let info = identity.device_info("b", OsType::Windows);
        let (b, _rx) = ConnectionManager::new(
            identity,
            info,
            Arc::new(TrustStore::in_memory()),
            fast_config(),
        )
        .unwrap();
        let b_addr = b
            .listen((Ipv4Addr::LOCALHOST, 0).into(), None)
            .await
            .unwrap();

        let confirmer = |_: DeviceInfo, _: PairingCode| async { true };
        assert!(a.manager.pair(b_addr, &confirmer).await.is_err());
        assert!(a.manager.trust_store().list().is_empty());
    }

    #[tokio::test]
    async fn send_to_unknown_device_fails() {
        let a = node("a", fast_config()).await;
        let stranger = Identity::generate().unwrap().device_id();
        assert!(matches!(
            a.manager.send(stranger, FreeLifeMessage::Ping).await,
            Err(Error::NotConnected(_))
        ));
    }

    #[tokio::test]
    async fn reconnect_replaces_old_session_and_disconnect_closes() {
        let a = node("a", fast_config()).await;
        let mut b = node("b", fast_config()).await;
        trust_each_other(&a, &b).await;

        a.manager.connect(b.info.id, b.addr).await.unwrap();
        a.manager.connect(b.info.id, b.addr).await.unwrap();
        assert_eq!(a.manager.connected_devices(), vec![b.info.id]);

        let msg = FreeLifeMessage::HandoffUrl { url: "x".into() };
        a.manager.send(b.info.id, msg.clone()).await.unwrap();
        assert_eq!(recv(&mut b).await.1, msg);

        assert!(a.manager.disconnect(b.info.id));
        assert!(!a.manager.is_connected(b.info.id));
        assert!(eventually(|| !b.manager.is_connected(a.info.id)).await);
    }
}
