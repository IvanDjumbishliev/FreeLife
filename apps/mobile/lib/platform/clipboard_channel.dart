import 'dart:async';

import 'package:flutter/services.dart';

/// A copy on this device, as reported by the native side.
class LocalCopy {
  const LocalCopy.text(this.text, {this.markedSensitive = false}) : png = null;
  const LocalCopy.png(this.png, {this.markedSensitive = false}) : text = null;

  final String? text;
  final Uint8List? png;

  /// The platform's "sensitive" flag (Android 13+).
  final bool markedSensitive;
}

/// Dart side of the `dev.freelife/clipboard` method channel. Implemented in
/// `android/app/src/main/kotlin/dev/freelife/mobile/ClipboardChannel.kt`.
class ClipboardChannel {
  ClipboardChannel([MethodChannel? channel])
    : _channel = channel ?? const MethodChannel(channelName) {
    _channel.setMethodCallHandler(_onNativeCall);
  }

  static const channelName = 'dev.freelife/clipboard';

  final MethodChannel _channel;
  final _copies = StreamController<LocalCopy>.broadcast();

  /// Copies made while the app is on screen, or read from the sync notification.
  Stream<LocalCopy> get copies => _copies.stream;

  /// Put text on the clipboard. Returns false if Android refused the write
  /// (the app wasn't focused); a tap-to-copy notification is shown then.
  Future<bool> writeText(String text, {required String device}) async =>
      await _channel.invokeMethod<bool>('writeText', {
        'text': text,
        'device': device,
      }) ??
      false;

  Future<bool> writePng(Uint8List png, {required String device}) async =>
      await _channel.invokeMethod<bool>('writePng', {
        'bytes': png,
        'device': device,
      }) ??
      false;

  /// Keep sync running in the background (foreground service with the
  /// "Sync clipboard" notification).
  Future<void> startSyncService() => _channel.invokeMethod('startSyncService');

  Future<void> stopSyncService() => _channel.invokeMethod('stopSyncService');

  /// Read the current clipboard now (e.g. after the user taps "Sync clipboard").
  Future<void> readNow() => _channel.invokeMethod('readNow');

  Future<Object?> _onNativeCall(MethodCall call) async {
    if (call.method != 'localClip') return null;
    final args = Map<Object?, Object?>.from(call.arguments as Map);
    final marked = args['markedSensitive'] == true;
    final copy = switch (args['kind']) {
      'text' => LocalCopy.text(args['text'] as String, markedSensitive: marked),
      'png' => LocalCopy.png(
        args['bytes'] as Uint8List,
        markedSensitive: marked,
      ),
      _ => null,
    };
    if (copy != null) _copies.add(copy);
    return null;
  }

  void dispose() {
    _channel.setMethodCallHandler(null);
    _copies.close();
  }
}
