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

## 🧑‍💻 Getting Started

### Prerequisites

Development requirements depend on the component you're working on. The intended toolchain includes:

- Rust and Cargo
- Node.js and pnpm
- Flutter and the Dart SDK
- Tauri prerequisites for your operating system

### Clone the repository

```bash
git clone https://github.com/YOUR_USERNAME/FreeLife.git
cd FreeLife
```

### Build the Rust core

```bash
cd core
cargo build
```

### Run the desktop application

From the desktop application's directory:

```bash
pnpm install
pnpm tauri dev
```

### Run the mobile application

From the Flutter application's directory:

```bash
flutter pub get
flutter run
```

Adjust directory names and commands to match the current repository structure. Consult the project documentation for component-specific setup, pairing, and troubleshooting instructions.

## 🤝 Contributing

**FreeLife is built for everyone, and everyone can help shape it.**

Whether you're a Rust developer, mobile engineer, designer, tester, technical writer, or simply someone who wants devices to work better together, your contributions are welcome.

- 🐛 **Report bugs:** Open an issue with your operating system, device details, version, and reproduction steps.
- 💡 **Suggest features:** Share your ideas and discuss proposed improvements before implementing major changes.
- ⚙️ **Build integrations:** Help improve platform support, device discovery, and modular functionality.
- 📖 **Improve documentation:** Make installation guides, explanations, and translations clearer.
- 🧪 **Test compatibility:** Help identify which device combinations and features work reliably.

For contribution guidelines, development setup, and community standards, see the repository's documentation.

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
