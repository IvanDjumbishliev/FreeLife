import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:freelife/backend.dart';
import 'package:freelife/main.dart';
import 'package:freelife/clipboard_sync.dart';
import 'package:freelife/platform/airshare_channel.dart';
import 'package:freelife/platform/clipboard_channel.dart';
import 'package:freelife/src/rust/api/clipboard.dart' as rust;
import 'package:freelife/platform/device_channel.dart';

const laptop = DeviceSummary(
  id: 'laptop-id',
  name: 'ThinkPad',
  os: 'Linux',
  fingerprint: 'aaaa-bbbb-cccc-dddd',
);

TransferUpdate update({
  String id = 't1',
  TransferKind kind = TransferKind.progress,
  TransferDirection direction = TransferDirection.send,
  int bytes = 0,
  int total = 100,
  int speed = 0,
  int? eta,
  String? savedPath,
  String? error,
}) => TransferUpdate(
  id: id,
  kind: kind,
  direction: direction,
  peerId: laptop.id,
  peerName: laptop.name,
  filename: 'photo.jpg',
  bytesSent: bytes,
  totalBytes: total,
  speedBps: speed,
  etaSecs: eta,
  savedPath: savedPath,
  error: error,
);

class FakeClipboardEngine implements ClipboardEngine {
  @override
  bool enabled = false;

  @override
  rust.LocalClipDecision localText(
    String text, {
    required bool markedSensitive,
  }) => rust.LocalClipDecision.broadcast;

  @override
  Future<rust.LocalClipDecision> localPng(
    Uint8List png, {
    required bool markedSensitive,
  }) async => rust.LocalClipDecision.broadcast;

  @override
  Stream<rust.ClipboardReceived> received() => const Stream.empty();

  @override
  Future<void> setEnabled(bool enabled) async => this.enabled = enabled;
}

class FakeBackend implements FreeLifeBackend {
  List<PairedDevice> paired = [
    const PairedDevice(device: laptop, connected: true),
  ];
  final transfers = StreamController<TransferUpdate>.broadcast();
  final pairing = StreamController<PairingUpdate>.broadcast();
  final sent = <(String, String)>[];
  final responses = <(int, bool)>[];
  Map<String, String>? startArgs;

  @override
  Future<DeviceSummary> start({
    required String dataDir,
    required String downloadDir,
    required String deviceName,
  }) async {
    startArgs = {'data': dataDir, 'download': downloadDir, 'name': deviceName};
    return DeviceSummary(
      id: 'me',
      name: deviceName,
      os: 'Android',
      fingerprint: '1111-2222-3333-4444',
    );
  }

  @override
  List<PairedDevice> pairedDevices() => paired;

  @override
  Future<List<NearbyDevice>> discover() async => [
    const NearbyDevice(device: laptop, paired: false),
  ];

  @override
  Future<DeviceSummary> pair(String deviceId) async => laptop;

  @override
  bool respondPairing(int requestId, bool accept) {
    responses.add((requestId, accept));
    return true;
  }

  @override
  Future<bool> unpair(String deviceId) async => true;

  @override
  Stream<PairingUpdate> pairingEvents() => pairing.stream;

  @override
  Stream<TransferUpdate> transferEvents() => transfers.stream;

  @override
  Stream<TransferUpdate> sendFile(String deviceId, String path) {
    sent.add((deviceId, path));
    return const Stream.empty();
  }
}

/// Mocks both platform channels; records airshare calls.
class FakePlatform {
  FakePlatform({this.launchShared = const []});

  final List<String> launchShared;
  final calls = <MethodCall>[];
  late final AirShareChannel airshare;
  static const _airshare = MethodChannel(AirShareChannel.channelName);
  static const _device = MethodChannel(DeviceChannel.channelName);

  void install() {
    final messenger =
        TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger;
    messenger.setMockMethodCallHandler(_device, (call) async {
      if (call.method == 'getDeviceInfo') {
        return {'manufacturer': 'Acme', 'model': 'Phone 1', 'osVersion': '15'};
      }
      return null;
    });
    airshare = AirShareChannel();
    messenger.setMockMethodCallHandler(_airshare, (call) async {
      calls.add(call);
      return switch (call.method) {
        'getStorageDirs' => {'dataDir': '/data', 'downloadDir': '/dl/FreeLife'},
        'takeSharedFiles' => launchShared,
        _ => null,
      };
    });
  }

  /// Simulate the native side forwarding a share intent.
  Future<void> share(List<String> paths) async {
    final messenger =
        TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger;
    await messenger.handlePlatformMessage(
      AirShareChannel.channelName,
      const StandardMethodCodec().encodeMethodCall(
        MethodCall('sharedFiles', paths),
      ),
      (_) {},
    );
  }

  void uninstall() {
    final messenger =
        TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger;
    messenger.setMockMethodCallHandler(_airshare, null);
    messenger.setMockMethodCallHandler(_device, null);
  }
}

/// Stream events land in a microtask after the current frame; the second
/// pump renders the resulting setState.
Future<void> deliver(WidgetTester tester) async {
  await tester.pump();
  await tester.pump();
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  late FakeBackend backend;
  late FakePlatform platform;

  Future<void> pumpApp(
    WidgetTester tester, {
    List<String> launchShared = const [],
  }) async {
    platform = FakePlatform(launchShared: launchShared)..install();
    addTearDown(platform.uninstall);
    // The home page is taller than the default 800x600 test surface.
    tester.view.devicePixelRatio = 1.0;
    tester.view.physicalSize = const Size(800, 2400);
    addTearDown(tester.view.resetPhysicalSize);
    await tester.pumpWidget(
      FreeLifeApp(
        backend: backend,
        airshare: platform.airshare,
        clipboard: ClipboardSyncController(
          engine: FakeClipboardEngine(),
          channel: ClipboardChannel(const MethodChannel('test/clipboard')),
        ),
      ),
    );
    await tester.pumpAndSettle();
  }

  setUp(() => backend = FakeBackend());

  testWidgets('starts the node with native paths and lists paired devices', (
    tester,
  ) async {
    await pumpApp(tester);

    expect(backend.startArgs, {
      'data': '/data',
      'download': '/dl/FreeLife',
      'name': 'Acme Phone 1',
    });
    expect(find.text('Acme Phone 1'), findsOneWidget);
    expect(find.text('ThinkPad'), findsOneWidget);
    final methods = platform.calls.map((c) => c.method);
    expect(
      methods,
      containsAll(['requestNotificationPermission', 'acquireMulticastLock']),
    );
  });

  testWidgets('shows progress with speed and ETA', (tester) async {
    await pumpApp(tester);

    backend.transfers.add(
      update(
        bytes: 25 * 1024 * 1024,
        total: 100 * 1024 * 1024,
        speed: 5 * 1024 * 1024,
        eta: 15,
      ),
    );
    await deliver(tester);

    expect(find.text('photo.jpg'), findsOneWidget);
    expect(find.text('To ThinkPad'), findsOneWidget);
    expect(
      find.text('25.0 MB of 100 MB · 5.0 MB/s · 15s left'),
      findsOneWidget,
    );
    final bar = tester.widget<LinearProgressIndicator>(
      find.byType(LinearProgressIndicator),
    );
    expect(bar.value, closeTo(0.25, 1e-9));

    backend.transfers.add(
      update(kind: TransferKind.complete, bytes: 100, total: 100),
    );
    await deliver(tester);
    expect(find.byType(LinearProgressIndicator), findsNothing);
    expect(find.text('Sent 100 B'), findsOneWidget);
    expect(find.text('Sent photo.jpg to ThinkPad'), findsOneWidget); // snackbar
  });

  testWidgets('received files raise a notification', (tester) async {
    await pumpApp(tester);

    backend.transfers.add(
      update(
        kind: TransferKind.complete,
        direction: TransferDirection.receive,
        bytes: 100,
        savedPath: '/dl/FreeLife/photo.jpg',
      ),
    );
    await deliver(tester);

    final notify = platform.calls.singleWhere(
      (c) => c.method == 'showFileReceivedNotification',
    );
    expect(notify.arguments, {
      'device': 'ThinkPad',
      'filename': 'photo.jpg',
      'path': '/dl/FreeLife/photo.jpg',
    });
    expect(find.text('Saved · 100 B'), findsOneWidget);
  });

  testWidgets('shared files open the device picker and send', (tester) async {
    await pumpApp(tester);

    await platform.share(['/cache/a.pdf']);
    await tester.pumpAndSettle();

    expect(find.text('Send a.pdf to…'), findsOneWidget);
    await tester.tap(
      find.descendant(
        of: find.byType(BottomSheet),
        matching: find.text('ThinkPad'),
      ),
    );
    await tester.pumpAndSettle();

    expect(backend.sent, [('laptop-id', '/cache/a.pdf')]);
    expect(find.byType(BottomSheet), findsNothing);
  });

  testWidgets('files shared at launch are offered once the node starts', (
    tester,
  ) async {
    await pumpApp(tester, launchShared: ['/cache/x.png', '/cache/y.png']);
    expect(find.text('Send 2 files to…'), findsOneWidget);
  });

  testWidgets('pairing prompt shows the code and reports the answer', (
    tester,
  ) async {
    await pumpApp(tester);

    backend.pairing.add(
      const PairingUpdate(requestId: 7, device: laptop, code: '042917'),
    );
    await deliver(tester);
    expect(find.text('Pair with ThinkPad?'), findsOneWidget);
    expect(find.text('042 917'), findsOneWidget);

    await tester.tap(find.text('Codes match'));
    await tester.pump();
    expect(backend.responses, [(7, true)]);
    expect(find.text('Pair with ThinkPad?'), findsNothing);
  });

  testWidgets('dismissed pairing prompt disappears', (tester) async {
    await pumpApp(tester);

    backend.pairing.add(
      const PairingUpdate(requestId: 3, device: laptop, code: '111111'),
    );
    await deliver(tester);
    expect(find.text('Pair with ThinkPad?'), findsOneWidget);

    backend.pairing.add(const PairingUpdate(requestId: 3));
    await deliver(tester);
    expect(find.text('Pair with ThinkPad?'), findsNothing);
    expect(backend.responses, isEmpty);
  });

  testWidgets('scanning lists nearby devices with a pair button', (
    tester,
  ) async {
    backend.paired = [];
    await pumpApp(tester);

    await tester.tap(find.widgetWithText(TextButton, 'Scan'));
    await tester.pumpAndSettle();
    expect(find.widgetWithText(FilledButton, 'Pair'), findsOneWidget);
  });
}
