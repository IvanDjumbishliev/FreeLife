//! Two real nodes over TLS on localhost, each with a `ClipboardWatcher`
//! polling an in-memory clipboard.

use std::sync::Arc;
use std::time::{Duration, Instant};

use freelife_clipboard::testing::MemoryClipboard;
use freelife_clipboard::{
    BackendError, ClipboardBackend, ClipboardContent, ClipboardEvent, ClipboardSettings,
    ClipboardSync, ClipboardWatcher, Image, LocalDecision, SensitiveReason, WatcherConfig,
};
use freelife_core::identity::MemoryKeyStore;
use freelife_core::prompts::PromptEvent;
use freelife_core::OsType;
use freelife_transfer::node::{Node, NodeConfig};
use tokio::sync::broadcast;

struct Device {
    node: Arc<Node>,
    sync: Arc<ClipboardSync>,
    clipboard: MemoryClipboard,
    watcher: Option<ClipboardWatcher>,
    _dirs: (tempfile::TempDir, tempfile::TempDir),
}

impl Device {
    async fn start(name: &str) -> Self {
        let data = tempfile::tempdir().unwrap();
        let downloads = tempfile::tempdir().unwrap();
        let mut config = NodeConfig::new(
            name,
            OsType::Linux,
            Arc::new(MemoryKeyStore::default()),
            data.path(),
        );
        config.port = 0;
        config.enable_mdns = false;
        config.receiver.download_dir = downloads.path().to_path_buf();
        let node = Node::start(config).await.unwrap();

        // Stand-in for the user confirming pairing codes.
        let mut prompts = node.subscribe_pairing();
        let approver = node.clone();
        tokio::spawn(async move {
            while let Ok(event) = prompts.recv().await {
                if let PromptEvent::Show(p) = event {
                    approver.respond_pairing(p.request_id, true);
                }
            }
        });

        let sync = ClipboardSync::new(node.clone(), None).await;
        Self {
            node,
            sync,
            clipboard: MemoryClipboard::default(),
            watcher: None,
            _dirs: (data, downloads),
        }
    }

    fn watch(&mut self, poll: Duration) {
        self.watch_with(poll, self.clipboard.clone());
    }

    fn watch_with<B: ClipboardBackend + Send + 'static>(&mut self, poll: Duration, backend: B) {
        let config = WatcherConfig {
            poll_interval: poll,
            image_poll_interval: poll,
        };
        self.watcher =
            Some(ClipboardWatcher::spawn(self.sync.clone(), move || Ok(backend), config).unwrap());
    }
}

/// Two paired devices with an open session between them.
async fn paired_pair() -> (Device, Device) {
    let a = Device::start("laptop").await;
    let b = Device::start("phone").await;
    let peer = a
        .node
        .pair_at(([127, 0, 0, 1], b.node.port()).into())
        .await
        .unwrap();
    a.node.connection_to(peer.id).await.unwrap();
    // Let b register the inbound session.
    tokio::time::sleep(Duration::from_millis(50)).await;
    (a, b)
}

/// Wait until `clipboard` has been written `n` times; returns the time of
/// the n-th write.
async fn nth_write(clipboard: &MemoryClipboard, n: usize, within: Duration) -> Option<Instant> {
    let deadline = Instant::now() + within;
    while Instant::now() < deadline {
        if let Some((at, _)) = clipboard.writes().get(n - 1) {
            return Some(*at);
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    None
}

fn text(s: &str) -> ClipboardContent {
    ClipboardContent::from_text(s.into())
}

fn image(w: u32, h: u32, seed: u8) -> ClipboardContent {
    ClipboardContent::Image(Image {
        width: w,
        height: h,
        rgba: (0..w * h * 4)
            .map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed))
            .collect(),
    })
}

async fn next_event(rx: &mut broadcast::Receiver<ClipboardEvent>) -> ClipboardEvent {
    tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("no clipboard event")
        .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn network_latency_is_well_under_100ms() {
    let (a, mut b) = paired_pair().await;
    b.watch(Duration::from_millis(200));

    // Measure from "change detected on a" to "written on b": everything
    // except the detection delay, which depends only on the poll interval.
    let mut worst = Duration::ZERO;
    for i in 1..=20 {
        let start = Instant::now();
        let decision = a
            .sync
            .local_changed(text(&format!("message {i}")), || false);
        assert_eq!(decision, LocalDecision::Broadcast);
        let written = nth_write(&b.clipboard, i, Duration::from_secs(2))
            .await
            .expect("not delivered");
        worst = worst.max(written - start);
    }
    eprintln!("network latency over 20 copies: worst {worst:?}");
    assert!(worst < Duration::from_millis(100), "worst {worst:?}");
    assert_eq!(b.clipboard.current(), Some(text("message 20")));
}

#[tokio::test(flavor = "multi_thread")]
async fn end_to_end_latency_with_polling() {
    for (poll, budget) in [(200, 300), (50, 100)] {
        let (mut a, mut b) = paired_pair().await;
        a.watch(Duration::from_millis(poll));
        b.watch(Duration::from_millis(poll));
        tokio::time::sleep(Duration::from_millis(poll * 2)).await;

        let mut worst = Duration::ZERO;
        for i in 1..=10 {
            let start = Instant::now();
            a.clipboard.copy(text(&format!("copy {i}")));
            let written = nth_write(&b.clipboard, i, Duration::from_secs(2))
                .await
                .expect("not delivered");
            worst = worst.max(written - start);
        }
        eprintln!("poll {poll}ms: worst copy-to-paste latency over 10 copies {worst:?}");
        assert!(
            worst < Duration::from_millis(budget),
            "poll {poll}ms: worst {worst:?}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn received_content_is_not_echoed_back() {
    let (mut a, mut b) = paired_pair().await;
    a.watch(Duration::from_millis(20));
    b.watch(Duration::from_millis(20));
    let mut b_events = b.sync.subscribe();

    a.clipboard.copy(text("ping"));
    nth_write(&b.clipboard, 1, Duration::from_secs(2))
        .await
        .unwrap();
    assert!(matches!(
        next_event(&mut b_events).await,
        ClipboardEvent::Received { .. }
    ));

    // Many poll cycles later, b has sent nothing and a received nothing back.
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(a.clipboard.writes().is_empty(), "echoed back to sender");
    assert!(b_events.try_recv().is_err(), "b sent something");

    // b copying something new still syncs normally.
    b.clipboard.copy(text("pong"));
    nth_write(&a.clipboard, 1, Duration::from_secs(2))
        .await
        .unwrap();
    assert_eq!(a.clipboard.current(), Some(text("pong")));
}

/// A clipboard that changes what it stores, the way real platforms do
/// (CRLF line endings on Windows, color conversion of images on macOS).
#[derive(Clone)]
struct AlteringClipboard(MemoryClipboard);

impl ClipboardBackend for AlteringClipboard {
    fn read(&mut self, include_image: bool) -> Result<Option<ClipboardContent>, BackendError> {
        self.0.read(include_image)
    }

    fn write(&mut self, content: &ClipboardContent) -> Result<(), BackendError> {
        let altered = match content {
            ClipboardContent::Text(t) => ClipboardContent::Text(t.replace('\n', "\r\n")),
            ClipboardContent::Image(img) => {
                let mut img = img.clone();
                img.rgba[0] ^= 1;
                ClipboardContent::Image(img)
            }
            other => other.clone(),
        };
        self.0.write(&altered)
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn platform_alterations_do_not_cause_ping_pong() {
    let (mut a, mut b) = paired_pair().await;
    a.watch(Duration::from_millis(20));
    b.watch_with(
        Duration::from_millis(20),
        AlteringClipboard(b.clipboard.clone()),
    );

    a.clipboard.copy(text("line one\nline two"));
    nth_write(&b.clipboard, 1, Duration::from_secs(2))
        .await
        .unwrap();
    a.clipboard.copy(image(32, 32, 1));
    nth_write(&b.clipboard, 2, Duration::from_secs(2))
        .await
        .unwrap();

    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(
        a.clipboard.writes().is_empty(),
        "altered content bounced back"
    );
    assert_eq!(b.clipboard.writes().len(), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn urls_and_images_sync() {
    let (mut a, mut b) = paired_pair().await;
    a.watch(Duration::from_millis(20));
    b.watch(Duration::from_millis(20));

    a.clipboard.copy(text("https://freelife.dev/docs"));
    nth_write(&b.clipboard, 1, Duration::from_secs(2))
        .await
        .unwrap();
    assert!(
        matches!(b.clipboard.current(), Some(ClipboardContent::Url(u)) if u == "https://freelife.dev/docs")
    );

    let screenshot = image(640, 480, 7);
    a.clipboard.copy(screenshot.clone());
    nth_write(&b.clipboard, 2, Duration::from_secs(5))
        .await
        .unwrap();
    assert_eq!(b.clipboard.current(), Some(screenshot));
}

#[tokio::test(flavor = "multi_thread")]
async fn sensitive_content_is_never_sent() {
    let (mut a, mut b) = paired_pair().await;
    a.watch(Duration::from_millis(20));
    b.watch(Duration::from_millis(20));
    let mut events = a.sync.subscribe();

    a.clipboard
        .copy_marked_sensitive(text("correct horse battery staple"));
    assert!(matches!(
        next_event(&mut events).await,
        ClipboardEvent::Skipped {
            reason: SensitiveReason::MarkedByApp,
            ..
        }
    ));

    a.clipboard.copy(text("x7#Kq9!mZ2pL"));
    assert!(matches!(
        next_event(&mut events).await,
        ClipboardEvent::Skipped {
            reason: SensitiveReason::PasswordLike,
            ..
        }
    ));

    a.clipboard.copy(text(
        "-----BEGIN RSA PRIVATE KEY-----\nMIIE\n-----END RSA PRIVATE KEY-----",
    ));
    assert!(matches!(
        next_event(&mut events).await,
        ClipboardEvent::Skipped {
            reason: SensitiveReason::PrivateKey,
            ..
        }
    ));

    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(b.clipboard.writes().is_empty());

    // With the heuristic off, only explicitly marked content is held back.
    let mut settings = a.sync.settings();
    settings.block_secrets = false;
    a.sync.set_settings(settings).await.unwrap();
    a.clipboard.copy(text("x7#Kq9!mZ2pLnew"));
    nth_write(&b.clipboard, 1, Duration::from_secs(2))
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn settings_gate_both_directions() {
    let (mut a, mut b) = paired_pair().await;
    a.watch(Duration::from_millis(20));
    b.watch(Duration::from_millis(20));

    // Sender off: nothing leaves.
    let off = ClipboardSettings {
        enabled: false,
        ..ClipboardSettings::default()
    };
    a.sync.set_settings(off.clone()).await.unwrap();
    a.clipboard.copy(text("one"));
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(b.clipboard.writes().is_empty());

    // Receiver off: nothing is applied.
    a.sync
        .set_settings(ClipboardSettings::default())
        .await
        .unwrap();
    b.sync.set_settings(off).await.unwrap();
    a.clipboard.copy(text("two"));
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(b.clipboard.writes().is_empty());

    // Trusted devices only, with b not on a's list: still nothing.
    b.sync
        .set_settings(ClipboardSettings::default())
        .await
        .unwrap();
    let mut trusted_only = ClipboardSettings {
        trusted_only: true,
        ..ClipboardSettings::default()
    };
    a.sync.set_settings(trusted_only.clone()).await.unwrap();
    a.clipboard.copy(text("three"));
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(b.clipboard.writes().is_empty());

    // Put b on the list: syncing resumes.
    trusted_only.trusted_devices.insert(b.node.device().id);
    a.sync.set_settings(trusted_only).await.unwrap();
    a.clipboard.copy(text("four"));
    nth_write(&b.clipboard, 1, Duration::from_secs(2))
        .await
        .unwrap();
    assert_eq!(b.clipboard.current(), Some(text("four")));
}

#[tokio::test(flavor = "multi_thread")]
async fn clipboard_at_startup_is_not_sent() {
    let (mut a, mut b) = paired_pair().await;
    a.clipboard.copy(text("already there before launch"));
    a.watch(Duration::from_millis(20));
    b.watch(Duration::from_millis(20));

    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(b.clipboard.writes().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn settings_persist() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(freelife_clipboard::SETTINGS_FILE);
    let a = Device::start("a").await;
    let sync = ClipboardSync::new(a.node.clone(), Some(path.clone())).await;
    let mut events = sync.subscribe();

    let mut settings = sync.settings();
    settings.trusted_only = true;
    settings.sync_images = false;
    sync.set_settings(settings.clone()).await.unwrap();
    assert!(matches!(
        next_event(&mut events).await,
        ClipboardEvent::SettingsChanged { .. }
    ));

    let reloaded = ClipboardSync::new(a.node.clone(), Some(path)).await;
    assert_eq!(reloaded.settings(), settings);
}

#[tokio::test(flavor = "multi_thread")]
async fn thousand_clipboard_updates_are_delivered_without_drops() {
    const UPDATES: usize = 1_000;
    let (a, b) = paired_pair().await;
    let mut messages = b.node.subscribe_messages();
    let started = Instant::now();

    for index in 0..UPDATES {
        let payload = index.to_string().into_bytes();
        let delivered = a
            .node
            .send_to_many(
                [b.node.device().id],
                freelife_core::FreeLifeMessage::ClipboardUpdate {
                    content_type: freelife_core::ClipboardContentType::Text,
                    payload: payload.clone(),
                },
                Duration::from_secs(5),
            )
            .await;
        assert_eq!(delivered, 1, "update {index} wasn't delivered to the connection");
        let (from, message) = tokio::time::timeout(Duration::from_secs(60), messages.recv())
            .await
            .expect("clipboard update load exceeded 60 seconds")
            .expect("clipboard receiver dropped an update");
        assert_eq!(from, a.node.device().id);
        assert_eq!(
            message,
            freelife_core::FreeLifeMessage::ClipboardUpdate {
                content_type: freelife_core::ClipboardContentType::Text,
                payload,
            },
            "clipboard update {index} was reordered or altered"
        );
    }

    assert!(
        started.elapsed() < Duration::from_secs(60),
        "1000 clipboard updates took {:?}, slower than 1000/minute",
        started.elapsed()
    );
}
