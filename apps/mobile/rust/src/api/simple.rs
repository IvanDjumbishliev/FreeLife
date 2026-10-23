//! Functions callable from Dart. Mirrored by `StubCoreBridge` in
//! `lib/bridge/core_bridge.dart` until the generated bindings are wired in.

#[flutter_rust_bridge::frb(sync)]
pub fn protocol_version() -> u16 {
    freelife_core::PROTOCOL_VERSION
}

#[flutter_rust_bridge::frb(sync)]
pub fn greet(name: String) -> String {
    format!("Hello, {name}!")
}

#[flutter_rust_bridge::frb(init)]
pub fn init_app() {
    flutter_rust_bridge::setup_default_user_utils();
}
