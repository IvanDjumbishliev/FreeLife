import 'dart:async';

import 'package:flutter/services.dart';

class StorageDirs {
  const StorageDirs({required this.dataDir, required this.downloadDir});

  /// App-private directory for the identity key and trusted devices.
  final String dataDir;

  /// Where received files are saved.
  final String downloadDir;
}

/// Dart side of the `dev.freelife/airshare` method channel: share-sheet
/// intents, notifications, storage paths and the mDNS multicast lock.
///
/// Implemented in `android/app/src/main/kotlin/dev/freelife/mobile/AirShareChannel.kt`.
class AirShareChannel {
  AirShareChannel([MethodChannel? channel])
    : _channel = channel ?? const MethodChannel(channelName) {
    _channel.setMethodCallHandler(_onNativeCall);
  }

  static const channelName = 'dev.freelife/airshare';

  final MethodChannel _channel;
  final _shared = StreamController<List<String>>.broadcast();
  final _sharedUrls = StreamController<String>.broadcast();

  /// Files shared to FreeLife while the app is running (copied to local
  /// paths by the native side).
  Stream<List<String>> get sharedFiles => _shared.stream;
  Stream<String> get sharedUrls => _sharedUrls.stream;

  Future<StorageDirs> storageDirs() async {
    final map = await _channel.invokeMapMethod<String, String>(
      'getStorageDirs',
    );
    return StorageDirs(
      dataDir: map!['dataDir']!,
      downloadDir: map['downloadDir']!,
    );
  }

  /// Files shared before the app was running (the launch intent). Each
  /// call returns them once.
  Future<List<String>> takeSharedFiles() async =>
      await _channel.invokeListMethod<String>('takeSharedFiles') ?? const [];

  Future<List<String>> takeSharedUrls() async =>
      await _channel.invokeListMethod<String>('takeSharedUrls') ?? const [];

  /// "File received from [device]: [filename]. Tap to open."
  Future<void> showFileReceived({
    required String device,
    required String filename,
    required String path,
  }) => _channel.invokeMethod('showFileReceivedNotification', {
    'device': device,
    'filename': filename,
    'path': path,
  });

  Future<void> requestNotificationPermission() =>
      _channel.invokeMethod('requestNotificationPermission');

  /// Android drops multicast packets unless an app holds a multicast lock,
  /// which breaks mDNS discovery.
  Future<void> acquireMulticastLock() =>
      _channel.invokeMethod('acquireMulticastLock');

  Future<Object?> _onNativeCall(MethodCall call) async {
    if (call.method == 'sharedFiles') {
      _shared.add(List<String>.from(call.arguments as List));
    } else if (call.method == 'sharedUrls') {
      for (final url in List<String>.from(call.arguments as List)) {
        _sharedUrls.add(url);
      }
    }
    return null;
  }

  void dispose() {
    _channel.setMethodCallHandler(null);
    _shared.close();
    _sharedUrls.close();
  }
}
