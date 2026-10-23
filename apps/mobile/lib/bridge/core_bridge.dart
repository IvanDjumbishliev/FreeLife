/// Access to the shared Rust core (`apps/mobile/rust`).
///
/// Running `flutter_rust_bridge_codegen generate` creates the real bindings
/// in `lib/src/rust/`. Until those are generated and wired up, the app uses
/// [StubCoreBridge] so it builds and runs without the Rust toolchain.
abstract interface class CoreBridge {
  int protocolVersion();
  String greet(String name);
}

/// Mirrors the Rust stubs in `rust/src/api/simple.rs`.
class StubCoreBridge implements CoreBridge {
  const StubCoreBridge();

  @override
  int protocolVersion() => 1;

  @override
  String greet(String name) => 'Hello, $name! (stub bridge)';
}
