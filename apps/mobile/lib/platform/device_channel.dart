import 'package:flutter/services.dart';

/// Information reported by the host OS through the platform channel.
class NativeDeviceInfo {
  const NativeDeviceInfo({
    required this.manufacturer,
    required this.model,
    required this.osVersion,
  });

  factory NativeDeviceInfo.fromMap(Map<Object?, Object?> map) =>
      NativeDeviceInfo(
        manufacturer: map['manufacturer'] as String? ?? 'unknown',
        model: map['model'] as String? ?? 'unknown',
        osVersion: map['osVersion'] as String? ?? 'unknown',
      );

  final String manufacturer;
  final String model;
  final String osVersion;

  String get displayName => '$manufacturer $model';
}

/// Dart side of the `dev.freelife/device` method channel.
///
/// The Android implementation lives in
/// `android/app/src/main/kotlin/dev/freelife/mobile/MainActivity.kt`.
/// Keep method names in sync with it.
class DeviceChannel {
  const DeviceChannel([this._channel = const MethodChannel(channelName)]);

  static const channelName = 'dev.freelife/device';

  final MethodChannel _channel;

  Future<NativeDeviceInfo> getDeviceInfo() async {
    final map = await _channel.invokeMapMethod<Object?, Object?>(
      'getDeviceInfo',
    );
    return NativeDeviceInfo.fromMap(map ?? const {});
  }

  /// Battery percentage (0-100), or null if the platform can't report it.
  Future<int?> getBatteryLevel() =>
      _channel.invokeMethod<int>('getBatteryLevel');

  Future<String?> getClipboardText() =>
      _channel.invokeMethod<String>('getClipboardText');

  Future<void> setClipboardText(String text) =>
      _channel.invokeMethod<void>('setClipboardText', {'text': text});
}
