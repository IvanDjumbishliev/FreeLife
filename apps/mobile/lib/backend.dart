import 'src/rust/api/node.dart' as rust;
import 'src/rust/api/node.dart'
    show
        DeviceSummary,
        NearbyDevice,
        PairedDevice,
        PairingUpdate,
        TransferUpdate;

export 'src/rust/api/node.dart'
    show
        DeviceSummary,
        NearbyDevice,
        PairedDevice,
        PairingUpdate,
        TransferDirection,
        TransferKind,
        TransferUpdate;

/// Everything the UI needs from the Rust node. [RustBackend] is the real
/// thing; tests substitute a fake.
abstract interface class FreeLifeBackend {
  Future<DeviceSummary> start({
    required String dataDir,
    required String downloadDir,
    required String deviceName,
  });

  List<PairedDevice> pairedDevices();

  Future<List<NearbyDevice>> discover();

  /// Resolves once both users have confirmed the code (see [pairingEvents]).
  Future<DeviceSummary> pair(String deviceId);

  bool respondPairing(int requestId, bool accept);

  Future<bool> unpair(String deviceId);

  Stream<PairingUpdate> pairingEvents();

  /// Every transfer in both directions.
  Stream<TransferUpdate> transferEvents();

  /// Updates for this one transfer; closes after it completes or fails.
  Stream<TransferUpdate> sendFile(String deviceId, String path);
}

/// Calls into `apps/mobile/rust` through flutter_rust_bridge.
/// `RustLib.init()` must have completed first.
class RustBackend implements FreeLifeBackend {
  const RustBackend();

  @override
  Future<DeviceSummary> start({
    required String dataDir,
    required String downloadDir,
    required String deviceName,
  }) => rust.startNode(
    dataDir: dataDir,
    downloadDir: downloadDir,
    deviceName: deviceName,
  );

  @override
  List<PairedDevice> pairedDevices() => rust.pairedDevices();

  @override
  Future<List<NearbyDevice>> discover() =>
      rust.discoverDevices(timeoutMs: 2500);

  @override
  Future<DeviceSummary> pair(String deviceId) =>
      rust.pairDevice(deviceId: deviceId);

  @override
  bool respondPairing(int requestId, bool accept) =>
      rust.respondPairing(requestId: requestId, accept: accept);

  @override
  Future<bool> unpair(String deviceId) => rust.unpairDevice(deviceId: deviceId);

  @override
  Stream<PairingUpdate> pairingEvents() => rust.pairingEvents();

  @override
  Stream<TransferUpdate> transferEvents() => rust.transferEvents();

  @override
  Stream<TransferUpdate> sendFile(String deviceId, String path) =>
      rust.sendFile(deviceId: deviceId, path: path);
}
