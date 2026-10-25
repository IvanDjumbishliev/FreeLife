use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::{BackendError, ClipboardBackend, ClipboardSync, RemoteClip};

#[derive(Debug, Clone)]
pub struct WatcherConfig {
    /// How often to check for local text changes. Detection adds up to this
    /// much latency (half of it on average) before a copy is sent.
    pub poll_interval: Duration,
    /// How often to check for image changes. Slower, because reading an
    /// image means copying all its pixels.
    pub image_poll_interval: Duration,
}

impl Default for WatcherConfig {
    fn default() -> Self {
        Self {
            // The worst-case copy-to-paste delay is roughly this interval plus
            // ~1 ms of network time, so it has to be well under 100 ms.
            poll_interval: Duration::from_millis(50),
            image_poll_interval: Duration::from_secs(1),
        }
    }
}

enum Command {
    Write(RemoteClip),
    Stop,
}

/// Polls the system clipboard on a dedicated thread and sends changes
/// through [`ClipboardSync`]. Content received from peers is written on
/// the same thread, so reads and writes never race. A write wakes the
/// thread at once rather than waiting for the next poll.
pub struct ClipboardWatcher {
    commands: mpsc::Sender<Command>,
    thread: Option<JoinHandle<()>>,
    receiver: tokio::task::JoinHandle<()>,
}

impl ClipboardWatcher {
    /// Start watching. `make_backend` runs on the watcher thread (some
    /// clipboard handles must stay on the thread that created them).
    pub fn spawn<B, F>(
        sync: Arc<ClipboardSync>,
        make_backend: F,
        config: WatcherConfig,
    ) -> std::io::Result<Self>
    where
        B: ClipboardBackend,
        F: FnOnce() -> Result<B, BackendError> + Send + 'static,
    {
        let (commands, rx) = mpsc::channel();
        let thread_sync = sync.clone();
        // Returns once the starting clipboard is recorded as the baseline, so
        // a copy made right after `spawn` returns is detected as a change.
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let thread = std::thread::Builder::new()
            .name("freelife-clipboard".into())
            .spawn(move || run(thread_sync, make_backend, config, rx, ready_tx))?;
        let _ = ready_rx.recv();

        let to_thread = commands.clone();
        let receiver = sync.spawn_receiver(move |clip| {
            let _ = to_thread.send(Command::Write(clip));
        });
        Ok(Self {
            commands,
            thread: Some(thread),
            receiver,
        })
    }

    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        self.receiver.abort();
        let _ = self.commands.send(Command::Stop);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for ClipboardWatcher {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn run<B, F>(
    sync: Arc<ClipboardSync>,
    make_backend: F,
    config: WatcherConfig,
    rx: mpsc::Receiver<Command>,
    ready: mpsc::SyncSender<()>,
) where
    B: ClipboardBackend,
    F: FnOnce() -> Result<B, BackendError>,
{
    let mut backend = match make_backend() {
        Ok(b) => b,
        Err(e) => {
            tracing::error!(error = %e, "clipboard unavailable; clipboard sync is off");
            let _ = ready.send(());
            return;
        }
    };
    if let Ok(Some(content)) = backend.read(true) {
        sync.set_baseline(&content);
    }
    let _ = ready.send(());

    let mut last_image_poll = Instant::now();
    loop {
        match rx.recv_timeout(config.poll_interval) {
            Ok(Command::Write(clip)) => {
                apply(&mut backend, &sync, clip);
                continue;
            }
            Ok(Command::Stop) | Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {}
        }

        let include_image = last_image_poll.elapsed() >= config.image_poll_interval;
        if include_image {
            last_image_poll = Instant::now();
        }
        match backend.read(include_image) {
            Ok(Some(content)) => {
                sync.local_changed(content, || backend.is_marked_sensitive());
            }
            Ok(None) => {}
            Err(e) => tracing::debug!(error = %e, "clipboard read failed"),
        }
    }
}

fn apply<B: ClipboardBackend>(backend: &mut B, sync: &ClipboardSync, clip: RemoteClip) {
    if let Err(e) = backend.write(&clip.content) {
        tracing::warn!(hash = %clip.hash, error = %e, "could not write received clipboard");
        return;
    }
    // Remember what the platform actually stored (see `ClipboardSync::applied`).
    let stored = match backend.read(true) {
        Ok(Some(content)) => content.hash(),
        _ => clip.hash,
    };
    sync.applied(stored);
}
