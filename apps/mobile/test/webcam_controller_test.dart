import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:freelife/platform/camera_channel.dart';
import 'package:freelife/webcam_controller.dart';

class RecordingEngine implements CameraEngine {
  final frames = <(Uint8List, bool, int)>[];
  final formats = <StreamFormat>[];
  int stops = 0;
  bool sourceStarted = false;

  @override
  Future<void> startSource() async => sourceStarted = true;

  @override
  void setStreamInfo(StreamFormat format) => formats.add(format);

  @override
  void pushFrame(
    Uint8List data, {
    required bool keyframe,
    required int timestampUs,
  }) => frames.add((data, keyframe, timestampUs));

  @override
  void stopStream() => stops++;
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  const channelName = 'test/camera-controller';
  final messenger =
      TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger;
  const channel = MethodChannel(channelName);

  late RecordingEngine engine;
  late List<String> calls;
  late List<String> errors;
  late WebcamController controller;
  Object? startResult;

  setUp(() {
    engine = RecordingEngine();
    calls = [];
    errors = [];
    startResult = {'width': 1920, 'height': 1080, 'fps': 30};
    messenger.setMockMethodCallHandler(channel, (call) async {
      calls.add(call.method);
      if (call.method == 'start') {
        final result = startResult;
        if (result is PlatformException) {
          throw result;
        }
        return result;
      }
      return null;
    });
    controller = WebcamController(
      engine: engine,
      channel: CameraChannel(channel),
      onError: errors.add,
    );
  });

  tearDown(() async {
    await controller.dispose();
    messenger.setMockMethodCallHandler(channel, null);
  });

  Future<void> nativeSends(String method, Object? args) =>
      messenger.handlePlatformMessage(
        channelName,
        const StandardMethodCodec().encodeMethodCall(MethodCall(method, args)),
        (_) {},
      );

  test('starting configures the stream and forwards frames to Rust', () async {
    await controller.init();
    expect(engine.sourceStarted, isTrue);

    await controller.start();
    expect(controller.active.value, isTrue);
    expect(engine.formats.single.width, 1920);
    expect(engine.formats.single.fps, 30);

    final bytes = Uint8List.fromList([0, 0, 0, 1, 0x65]);
    await nativeSends('encodedFrame', {
      'bytes': bytes,
      'keyframe': true,
      'timestampUs': 1700000000000000,
    });
    await Future<void>.delayed(Duration.zero);
    final (sent, keyframe, timestamp) = engine.frames.single;
    expect(sent, orderedEquals(bytes));
    expect((keyframe, timestamp), (true, 1700000000000000));
  });

  test('frames are not forwarded while the webcam is off', () async {
    await controller.init();
    await nativeSends('encodedFrame', {
      'bytes': Uint8List(4),
      'keyframe': false,
      'timestampUs': 1,
    });
    await Future<void>.delayed(Duration.zero);
    expect(engine.frames, isEmpty);
  });

  test('a missing permission is reported and the switch stays off', () async {
    await controller.init();
    startResult = PlatformException(
      code: 'PERMISSION_NEEDED',
      message: 'Allow camera access',
    );

    await controller.start();
    expect(controller.active.value, isFalse);
    expect(errors, ['Allow camera access']);
  });

  test('the notification stopping the stream turns the switch off', () async {
    await controller.init();
    await controller.start();

    await nativeSends('streamStopped', null);
    await Future<void>.delayed(Duration.zero);
    expect(controller.active.value, isFalse);
    expect(engine.stops, 1);
  });

  test('a camera error stops the switch and says why', () async {
    await controller.init();
    await controller.start();

    await nativeSends('streamError', {
      'message': 'The camera was disconnected',
    });
    await Future<void>.delayed(Duration.zero);
    expect(controller.active.value, isFalse);
    expect(errors, ['The camera was disconnected']);
  });
}
