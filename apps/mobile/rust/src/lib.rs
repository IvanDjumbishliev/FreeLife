//! Rust side of the Flutter app, exposed to Dart via flutter_rust_bridge.
//!
//! `flutter_rust_bridge_codegen generate` scans `api` and writes the Dart
//! bindings to `lib/src/rust/` plus `src/frb_generated.rs` here (adding the
//! `mod frb_generated;` line to this file).

pub mod api;
