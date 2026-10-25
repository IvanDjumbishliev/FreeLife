import 'dart:async';

import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:freelife/clipboard_sync.dart';
import 'package:freelife/platform/clipboard_channel.dart';
import 'package:freelife/src/rust/api/clipboard.dart' as rust;

class FakeEngine implements ClipboardEngine {
  final localTexts = <(String, bool)>[];
  final localPngs = <(Uint8List, bool)>[];
  final _received = StreamController<rust.ClipboardReceived>();

  @override
  bool enabled = true;

  @override
  rust.LocalClipDecision localText(
    String text, {
    required bool markedSensitive,
  }) {
    localTexts.add((text, markedSensitive));
    return rust.LocalClipDecision.broadcast;
  }

  @override
  Future<rust.LocalClipDecision> localPng(
    Uint8List png, {
    required bool markedSensitive,
  }) async {
    localPngs.add((png, markedSensitive));
    return rust.LocalClipDecision.broadcast;
  }

  @override
  Stream<rust.ClipboardReceived> received() => _received.stream;

  void receive(rust.ClipboardReceived clip) => _received.add(clip);

  @override
  Future<void> setEnabled(bool value) async => enabled = value;
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  const channelName = 'test/clipboard';
  final messenger =
      TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger;
  const channel = MethodChannel(channelName);

  late FakeEngine engine;
  late List<MethodCall> calls;
  late List<String> announced;
  late bool writeSucceeds;
  late ClipboardSyncController controller;

  setUp(() {
    engine = FakeEngine();
    calls = [];
    announced = [];
    writeSucceeds = true;
    messenger.setMockMethodCallHandler(channel, (call) async {
      calls.add(call);
      return switch (call.method) {
        'writeText' || 'writePng' => writeSucceeds,
        _ => null,
      };
    });
    controller = ClipboardSyncController(
      engine: engine,
      channel: ClipboardChannel(channel),
      onReceived: announced.add,
    );
  });

  tearDown(() async {
    await controller.dispose();
    messenger.setMockMethodCallHandler(channel, null);
  });

  Future<void> nativeSends(String method, Map<String, Object?> args) =>
      messenger.handlePlatformMessage(
        channelName,
        const StandardMethodCodec().encodeMethodCall(MethodCall(method, args)),
        (_) {},
      );

  test('local text goes to the engine with the sensitive flag', () async {
    await controller.start();
    await nativeSends('localClip', {
      'kind': 'text',
      'text': 'hello',
      'markedSensitive': false,
    });
    await nativeSends('localClip', {
      'kind': 'text',
      'text': 'pw',
      'markedSensitive': true,
    });
    await Future<void>.delayed(Duration.zero);

    expect(engine.localTexts, [('hello', false), ('pw', true)]);
  });

  test('local images are passed as PNG bytes', () async {
    await controller.start();
    final png = Uint8List.fromList([0x89, 0x50, 0x4e, 0x47]);
    await nativeSends('localClip', {
      'kind': 'png',
      'bytes': png,
      'markedSensitive': false,
    });
    await Future<void>.delayed(Duration.zero);

    expect(engine.localPngs.single.$1, png);
  });

  test('received text is written to the clipboard and announced', () async {
    await controller.start();
    engine.receive(
      const rust.ClipboardReceived(
        deviceName: 'ThinkPad',
        kind: rust.ClipKind.text,
        text: 'from laptop',
        png: null,
      ),
    );
    await Future<void>.delayed(Duration.zero);
    await Future<void>.delayed(Duration.zero);

    final write = calls.singleWhere((c) => c.method == 'writeText');
    expect(write.arguments, {'text': 'from laptop', 'device': 'ThinkPad'});
    expect(announced, ['ThinkPad']);
  });

  test('received images are written as PNG', () async {
    await controller.start();
    final png = Uint8List.fromList([1, 2, 3]);
    engine.receive(
      rust.ClipboardReceived(
        deviceName: 'Mac',
        kind: rust.ClipKind.image,
        text: null,
        png: png,
      ),
    );
    await Future<void>.delayed(Duration.zero);
    await Future<void>.delayed(Duration.zero);

    expect(calls.singleWhere((c) => c.method == 'writePng').arguments, {
      'bytes': png,
      'device': 'Mac',
    });
    expect(announced, ['Mac']);
  });

  test('a refused write is not announced as synced', () async {
    writeSucceeds = false;
    await controller.start();
    engine.receive(
      const rust.ClipboardReceived(
        deviceName: 'Phone',
        kind: rust.ClipKind.text,
        text: 'queued',
        png: null,
      ),
    );
    await Future<void>.delayed(Duration.zero);
    await Future<void>.delayed(Duration.zero);

    expect(calls.any((c) => c.method == 'writeText'), isTrue);
    expect(announced, isEmpty);
  });

  test('turning sync off stops the foreground service', () async {
    await controller.start();
    expect(calls.map((c) => c.method), contains('startSyncService'));

    await controller.setEnabled(false);
    expect(engine.enabled, isFalse);
    expect(calls.last.method, 'stopSyncService');
  });
}
