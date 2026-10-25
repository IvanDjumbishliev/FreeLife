import 'dart:async';

import 'package:flutter/services.dart';

/// One H.264 access unit from the phone's camera.
class EncodedFrame {
  const EncodedFrame({
    required this.bytes,
    required this.keyframe,
    required this.timestampUs,
  });

  final Uint8List bytes;
  final bool keyframe;

  /// Capture time, Unix epoch microseconds.
  final int timestampUs;
}

/// The format the encoder produces.
class StreamFormat {
  const StreamFormat({
    required this.width,
    required this.height,
    required this.fps,
  });

  final int width;
  final int height;
  final int fps;
}

/// Thrown when the stream can't start. [needsPermission] means the user has
/// to grant camera access first.
class WebcamUnavailable implements Exception {
  const WebcamUnavailable(this.message, {this.needsPermission = false});

  final String message;
  final bool needsPermission;

  @override
  String toString() => message;
}

/// Dart side of the `dev.freelife/camera` method channel. Implemented in
/// `android/app/src/main/kotlin/dev/freelife/mobile/CameraChannel.kt`.
class CameraChannel {
  CameraChannel([MethodChannel? channel])
    : _channel = channel ?? const MethodChannel(channelName) {
    _channel.setMethodCallHandler(_onNativeCall);
  }

  static const channelName = 'dev.freelife/camera';

  final MethodChannel _channel;
  final _frames = StreamController<EncodedFrame>.broadcast();
  final _errors = StreamController<String>.broadcast();
  final _stopped = StreamController<void>.broadcast();

  Stream<EncodedFrame> get frames => _frames.stream;

  /// The camera failed while streaming; the stream has stopped.
  Stream<String> get errors => _errors.stream;

  /// The stream ended, including when the user taps the notification to stop it.
  Stream<void> get stopped => _stopped.stream;

  Future<StreamFormat> start() async {
    try {
      final map = await _channel.invokeMapMethod<String, int>('start');
      return StreamFormat(
        width: map!['width']!,
        height: map['height']!,
        fps: map['fps']!,
      );
    } on PlatformException catch (e) {
      throw WebcamUnavailable(
        e.message ?? 'The camera couldn\'t start',
        needsPermission: e.code == 'PERMISSION_NEEDED',
      );
    }
  }

  Future<void> stop() => _channel.invokeMethod('stop');

  Future<Object?> _onNativeCall(MethodCall call) async {
    switch (call.method) {
      case 'encodedFrame':
        final args = Map<Object?, Object?>.from(call.arguments as Map);
        _frames.add(
          EncodedFrame(
            bytes: args['bytes'] as Uint8List,
            keyframe: args['keyframe'] == true,
            timestampUs: args['timestampUs'] as int,
          ),
        );
      case 'streamError':
        final args = Map<Object?, Object?>.from(call.arguments as Map);
        _errors.add(args['message'] as String? ?? 'The camera failed');
      case 'streamStopped':
        _stopped.add(null);
    }
    return null;
  }

  void dispose() {
    _channel.setMethodCallHandler(null);
    _frames.close();
    _errors.close();
    _stopped.close();
  }
}
