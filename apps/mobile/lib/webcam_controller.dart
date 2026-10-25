import 'dart:async';

import 'package:flutter/foundation.dart';

import 'platform/camera_channel.dart';
import 'src/rust/api/camera.dart' as rust;

/// The Rust camera source, as the controller uses it. Tests substitute a fake.
abstract interface class CameraEngine {
  Future<void> startSource();

  void setStreamInfo(StreamFormat format);

  void pushFrame(
    Uint8List data, {
    required bool keyframe,
    required int timestampUs,
  });

  void stopStream();
}

class RustCameraEngine implements CameraEngine {
  const RustCameraEngine();

  @override
  Future<void> startSource() => rust.cameraSourceStart();

  @override
  void setStreamInfo(StreamFormat format) => rust.cameraSetStreamInfo(
    width: format.width,
    height: format.height,
    fps: format.fps,
  );

  @override
  void pushFrame(
    Uint8List data, {
    required bool keyframe,
    required int timestampUs,
  }) => rust.cameraPushFrame(
    data: data,
    keyframe: keyframe,
    timestampUs: timestampUs,
  );

  @override
  void stopStream() => rust.cameraStop();
}

/// The "Start Webcam" switch: runs the phone's camera and forwards its
/// encoded frames to paired computers that subscribe.
class WebcamController {
  WebcamController({required this.engine, required this.channel, this.onError});

  final CameraEngine engine;
  final CameraChannel channel;

  /// Called with a message the user should see (permission needed, camera failed).
  final void Function(String message)? onError;

  /// Whether the webcam is on.
  final ValueNotifier<bool> active = ValueNotifier(false);

  /// Frame size and rate of the current stream, once it's running.
  StreamFormat? format;

  final _subscriptions = <StreamSubscription<Object?>>[];
  bool _sourceStarted = false;

  /// Subscribe to native events. Call once, after the node has started.
  Future<void> init() async {
    if (!_sourceStarted) {
      await engine.startSource();
      _sourceStarted = true;
    }
    if (_subscriptions.isNotEmpty) return;
    _subscriptions
      ..add(channel.frames.listen(_onFrame))
      ..add(
        channel.errors.listen((message) {
          active.value = false;
          onError?.call(message);
        }),
      )
      ..add(
        channel.stopped.listen((_) {
          active.value = false;
          engine.stopStream();
        }),
      );
  }

  Future<void> start() async {
    try {
      final stream = await channel.start();
      format = stream;
      engine.setStreamInfo(stream);
      active.value = true;
    } on WebcamUnavailable catch (e) {
      onError?.call(e.message);
    }
  }

  Future<void> stop() async {
    active.value = false;
    await channel.stop();
    engine.stopStream();
  }

  void _onFrame(EncodedFrame frame) {
    if (!active.value) return;
    engine.pushFrame(
      frame.bytes,
      keyframe: frame.keyframe,
      timestampUs: frame.timestampUs,
    );
  }

  Future<void> dispose() async {
    for (final s in _subscriptions) {
      await s.cancel();
    }
    _subscriptions.clear();
    active.dispose();
  }
}
