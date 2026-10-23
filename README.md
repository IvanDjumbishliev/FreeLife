# 🔗 FreeLife

### Your devices. One ecosystem. Zero lock-in.

**FreeLife** is an open-source, cross-platform device ecosystem that brings seamless device continuity to everyone — not just users locked into a single brand. Connect Android with Linux, Windows with Android, and eventually iOS with your desktop through one unified experience.

Share files. Sync your clipboard. Turn your phone into a webcam. Manage notifications. Continue tasks across devices — all designed around local connectivity, privacy, and freedom of choice.

**Open Source · Privacy First · Cross-Platform · Community Driven**

> 🚧 **v0.1 Alpha** — FreeLife is an evolving project. Platform support, feature availability, and security guarantees depend on implementation and testing.

---

## ✨ The Vision

Apple has shown how powerful a connected device ecosystem can be. But seamless continuity shouldn't require expensive hardware or devices from the same manufacturer.

FreeLife aims to bring that experience to everyone, regardless of their operating system or hardware choices.

| | |
|---|---|
| 🌍 **Cross-platform** | Connect devices across supported operating systems. |
| 🔒 **Privacy-first** | Prioritize local communication and end-to-end encryption. |
| 📡 **LAN-first** | Discover and connect nearby devices without unnecessary cloud dependencies. |
| 🆓 **Free and open source** | Build, modify, self-host, and contribute to the ecosystem. |
| 🧩 **Modular by design** | Create new integrations and extend functionality through plug-in modules. |

## 🚀 Core Features

### 📡 AirShare — Wireless File Transfer

Move files between your phone and computer without cables, accounts, or cloud uploads.

- Transfer photos, videos, documents, screenshots, and other files.
- Discover nearby devices automatically using mDNS.
- Track transfer progress with chunked file transfers.
- Use encrypted connections for transfers between paired devices.

### 📋 Universal Clipboard

Copy on one device and paste on another.

- Synchronize text, URLs, and supported image formats.
- Support bidirectional clipboard sharing.
- Aim for near-instant updates over your local network.

### 📷 ContinuityCamera — Your Phone as a Webcam

Turn your phone into a flexible camera for your computer.

- Use your phone's camera as a webcam over Wi-Fi or USB.
- Target compatibility with video conferencing, streaming, and recording applications.
- Integrate with Linux through V4L2 and Windows through DirectShow, subject to platform-specific implementation.

### 🔔 Notification Bridge

Bring your phone's notifications to your desktop.

- View supported Android notifications from your computer.
- Dismiss notifications and synchronize supported interactions.
- Explore inline replies where the source application and platform permit them.

### 🔄 Handoff — Pick Up Where You Left Off

Move your workflow between devices without starting over.

- Send active browser tabs from your phone to your desktop.
- Use browser extensions and Android share targets to initiate handoffs.
- Discover paired devices and route handoff requests locally.
- Explore optional encrypted relay support for connections outside your local network.

---

## 🛠️ Technology Stack

FreeLife is designed around a shared Rust core, with dedicated interfaces for mobile and desktop devices.

| Component | Technology | Purpose |
|---|---|---|
| Core engine | Rust | Shared networking and device communication logic |
| Mobile app | Flutter | Cross-platform mobile interface and integrations |
| Desktop app | Tauri + React | Lightweight desktop experience |
| Mobile bridge | `flutter_rust_bridge` | Connect Flutter with the Rust core |
| Device discovery | mDNS | Automatic discovery on local networks |
| Secure transport | TLS 1.3 | Protect communication between paired devices |
| Camera streaming | WebRTC | Low-latency media transport |
| Browser extension | Manifest V3 | Browser-based handoff functionality |
| Optional relay | Axum | Self-hostable relay for out-of-network handoffs |

The goal is to reuse core functionality wherever possible while allowing each operating system to integrate with its native capabilities.

## 📱 Platform Compatibility

FreeLife aims to support a broad range of devices, with availability varying by feature and operating system.

| Platform | Status |
|---|---|
| 🤖 Android | Primary mobile target |
| 🐧 Linux | Primary desktop target |
| 🪟 Windows | Primary desktop target |
| 🍎 macOS | Planned |
| 📱 iOS | Planned, with platform-specific limitations |
| 🌐 Chrome | Browser extension target |
| 🦊 Firefox | Browser extension target |

**Important:** These are project targets, not a guarantee that every feature is currently implemented or tested on every platform. Consult the project issues and release notes for verified compatibility.

## 🔐 Security & Privacy

Your devices should work together without handing your data to an unnecessary third party.

FreeLife is designed with the following principles:

- **Local-first communication:** Prefer direct connections over cloud infrastructure.
- **Encrypted transport:** Use TLS 1.3 for supported communication channels.
- **No mandatory account:** Avoid requiring a centralized identity service.
- **Minimal data retention:** Keep file and clipboard content off relay storage wherever possible.
- **User-controlled connectivity:** Make device pairing and connection behavior transparent.

Security is an ongoing process. End-to-end encryption, key management, relay behavior, and individual platform integrations must be verified against the actual implementation before being considered production-ready.

## 🗂️ Repository Structure

```
FreeLife/
├── apps/
│   ├── mobile/          # Flutter app (Android + iOS)
│   │   ├── android/     #   Android host project + platform channel (MainActivity.kt)
│   │   ├── lib/         #   Dart UI, platform channel + Rust bridge wrappers
│   │   └── rust/        #   Rust crate exposed to Dart via flutter_rust_bridge
│   └── desktop/         # Tauri 2 + React (Vite) app (Windows, Linux, macOS)
│       ├── src/         #   React frontend
│       └── src-tauri/   #   Rust backend + Tauri commands
├── packages/
│   ├── core/            # freelife-core: identity, mDNS discovery, pairing, wire protocol, TLS sessions
│   ├── discovery/       # freelife-discovery: re-exports freelife_core::discovery
│   └── transfer/        # freelife-transfer: chunked file + clipboard transfer
├── relay/               # freelife-relay: optional self-hostable Axum relay
├── extension/           # Manifest V3 browser extension (Chrome + Firefox)
├── .github/workflows/   # CI: Rust tests, Android APK, Tauri Windows/Linux builds
└── Cargo.toml           # Cargo workspace (packages/* + relay)
```

The root Cargo workspace contains the shared crates and the relay. The two app crates (`apps/desktop/src-tauri`, `apps/mobile/rust`) are built separately because they need platform toolchains, but they depend on the shared crates by path.

## 🧑‍💻 Getting Started

### Prerequisites

| Working on | You need |
|---|---|
| Rust crates / relay | [Rust](https://rustup.rs) (stable) |
| Desktop app | Rust, Node.js 20+, [pnpm](https://pnpm.io), and the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/) for your OS |
| Mobile app | [Flutter](https://docs.flutter.dev/get-started/install) (stable), Android SDK, JDK 17 |
| Browser extension | Chrome/Chromium or Firefox 142+ |

```bash
git clone https://github.com/IvanDjumbishliev/FreeLife.git
cd FreeLife
```

### Rust core, discovery & transfer

```bash
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings

# Opt-in test that uses real multicast on your network:
cargo test -p freelife-core -- --ignored mdns
```

How devices connect (details in the module docs of `packages/core/src/`):

1. **Discovery**: each device advertises `_freelife._tcp` on port `7432` via mDNS, with its id, name, OS and Ed25519 public key in the TXT record.
2. **Identity**: an Ed25519 key generated on first run and stored in the OS keychain. On Linux this needs a Secret Service provider (GNOME Keyring or KWallet). The device id is derived from the public key.
3. **Pairing**: a commit/reveal X25519 exchange. Both screens show a 6-digit code; once the user confirms it matches, each side signs the transcript and stores the other's key in `trusted_devices.json`.
4. **Sessions**: TLS 1.3 with each side's raw Ed25519 key pinned (RFC 7250, no certificates), carrying length-prefixed bincode `FreeLifeMessage`s. Sessions send a keep-alive ping every 30 s.
5. **AirShare** (`packages/transfer`): a `FileTransferRequest` carrying the file's SHA-256, then 256 KB `FileChunk`s. The receiver writes chunks at their offsets (so order doesn't matter), verifies the checksum, and only then moves the file into the download folder. `freelife_transfer::node::Node` ties everything together for the apps.

### Relay server

```bash
cargo run -p freelife-relay
# Listens on 0.0.0.0:8787 by default; override with FREELIFE_RELAY_ADDR.
curl localhost:8787/health
```

### Desktop app (Tauri + React)

```bash
cd apps/desktop
pnpm install
pnpm tauri dev      # run with hot reload
pnpm tauri build    # produce installers in src-tauri/target/release/bundle/
```

On Debian/Ubuntu, install the webview dependencies first:

```bash
sudo apt install libwebkit2gtk-4.1-dev build-essential curl wget file \
  libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev
```

Tauri commands live in `src-tauri/src/commands.rs`; their typed TypeScript wrappers are in `src/lib/api.ts`. Events pushed to the UI (transfer progress, pairing codes) are defined in `src-tauri/src/events.rs`.

To try AirShare, run the app on two machines on the same network. Click **Scan**, **Pair** a device, and confirm the 6-digit code on both. Then select the device and drop files onto the window. Received files go to `~/Downloads/FreeLife/`.

### Mobile app (Flutter)

```bash
cd apps/mobile
flutter pub get
flutter run                 # on a connected device or emulator
flutter build apk           # Android APK
```

You also need the Rust Android targets. The Gradle build compiles `rust/` for each ABI through cargokit (`rust_builder/`):

```bash
rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android
```

Two Android platform channels connect Dart and Kotlin:

- `dev.freelife/device` (`MainActivity.kt` ↔ `lib/platform/device_channel.dart`): device info, battery, clipboard.
- `dev.freelife/airshare` (`AirShareChannel.kt` ↔ `lib/platform/airshare_channel.dart`): "Share via FreeLife" intents, received-file notifications, storage paths, and the Wi-Fi multicast lock mDNS needs.

Received files are saved to the app's own Downloads folder (`Android/data/dev.freelife.mobile/files/Download/FreeLife`). Tapping the notification opens them.

**iOS:** the `ios/` host project isn't committed yet. Generate it with `flutter create --platforms=ios .` from `apps/mobile` (this leaves existing files untouched).

**Rust bridge:** Dart talks to `rust/src/api/node.rs` through generated bindings in `lib/src/rust/` (committed). After changing the Rust API, regenerate them:

```bash
cargo install flutter_rust_bridge_codegen --version 2.13.0
flutter_rust_bridge_codegen generate    # rewrites lib/src/rust/ and rust/src/frb_generated.rs
```

The `flutter_rust_bridge` versions in `pubspec.yaml` and `rust/Cargo.toml` must match exactly.

### Browser extension

- **Chrome:** open `chrome://extensions`, enable *Developer mode*, click *Load unpacked*, and select `extension/`.
- **Firefox:** open `about:debugging#/runtime/this-firefox`, click *Load Temporary Add-on*, and select `extension/manifest.json`.

Lint with `npx web-ext lint --source-dir extension`.

## 🤝 Contributing

**FreeLife is built for everyone, and everyone can help shape it.**

Whether you're a Rust developer, mobile engineer, designer, tester, technical writer, or simply someone who wants devices to work better together, your contributions are welcome.

- 🐛 **Report bugs:** Open an issue with your operating system, device details, version, and reproduction steps.
- 💡 **Suggest features:** Share your ideas and discuss proposed improvements before implementing major changes.
- ⚙️ **Build integrations:** Help improve platform support, device discovery, and modular functionality.
- 📖 **Improve documentation:** Make installation guides, explanations, and translations clearer.
- 🧪 **Test compatibility:** Help identify which device combinations and features work reliably.

### Workflow

1. Fork the repository and create a branch from `main` (`feat/clipboard-images`, `fix/discovery-timeout`, …).
2. Keep pull requests focused. Open an issue first for large changes or protocol changes.
3. Make sure the checks for the parts you touched pass locally:
   - Rust: `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`
   - Desktop: `pnpm build` in `apps/desktop`
   - Mobile: `flutter analyze` and `flutter test` in `apps/mobile`
4. Write clear commit messages in the imperative mood ("Add mDNS timeout option").
5. Open a pull request describing what changed and why. CI must be green before merge.

### Protocol changes

`packages/core` defines the wire protocol shared by every client. Any breaking change to `FreeLifeMessage`, the pairing messages, or framing must bump `PROTOCOL_VERSION` and be called out in the PR description. New `FreeLifeMessage` variants go at the end: bincode encodes the variant index, so reordering breaks older peers.

## 📄 License

FreeLife is distributed under the **MIT License**.

You are free to use, copy, modify, distribute, sublicense, and sell copies of the software, provided the required copyright and license notices are retained.

See [`LICENSE`](LICENSE) for the complete license text.

---

## 🌍 The Future Is Interconnected

Your phone shouldn't need to match your laptop brand. Your operating system shouldn't limit your workflow. And useful device integration shouldn't be reserved for expensive ecosystems.

**FreeLife is about connecting the devices you already own — on your terms.**

⭐ Star the project, share your ideas, report issues, or contribute to building a more open and connected digital world.

**FreeLife — Your devices. Your choice. One ecosystem.**
