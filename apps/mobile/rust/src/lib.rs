//! Rust side of the Flutter app, exposed to Dart via flutter_rust_bridge.
//!
//! `flutter_rust_bridge_codegen generate` (run from `apps/mobile`) reads
//! `api` and writes the Dart bindings to `lib/src/rust/` and the glue to
//! `src/frb_generated.rs`.

pub mod api;
mod frb_generated;
