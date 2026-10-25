import 'dart:async';
import 'dart:typed_data';

import 'platform/clipboard_channel.dart';
import 'src/rust/api/clipboard.dart' as rust;

/// The Rust clipboard engine, as the controller uses it. Tests substitute a fake.
abstract interface class ClipboardEngine {
  rust.LocalClipDecision localText(
    String text, {
    required bool markedSensitive,
  });

  Future<rust.LocalClipDecision> localPng(
    Uint8List png, {
    required bool markedSensitive,
  });

  Stream<rust.ClipboardReceived> received();

  bool get enabled;

  Future<void> setEnabled(bool enabled);
}

class RustClipboardEngine implements ClipboardEngine {
  const RustClipboardEngine();

  @override
  rust.LocalClipDecision localText(
    String text, {
    required bool markedSensitive,
  }) => rust.clipboardLocalText(text: text, markedSensitive: markedSensitive);

  @override
  Future<rust.LocalClipDecision> localPng(
    Uint8List png, {
    required bool markedSensitive,
  }) => rust.clipboardLocalPng(png: png, markedSensitive: markedSensitive);

  @override
  Stream<rust.ClipboardReceived> received() => rust.clipboardReceivedEvents();

  @override
  bool get enabled => rust.clipboardEnabled();

  @override
  Future<void> setEnabled(bool enabled) =>
      rust.setClipboardEnabled(enabled: enabled);
}

/// Connects the Android clipboard to the Rust engine in both directions.
///
/// Local copies go to the engine, which sends them to paired devices (and
/// refuses secrets). Received content is written to the Android clipboard.
class ClipboardSyncController {
  ClipboardSyncController({
    required this.engine,
    required this.channel,
    this.onReceived,
  });

  final ClipboardEngine engine;
  final ClipboardChannel channel;

  /// Called with the sender's name after a received clip is on the clipboard.
  final void Function(String deviceName)? onReceived;

  StreamSubscription<LocalCopy>? _copies;
  StreamSubscription<rust.ClipboardReceived>? _received;

  /// Start syncing. Call once the node has started.
  Future<void> start() async {
    _copies ??= channel.copies.listen(_onLocalCopy);
    _received ??= engine.received().listen(_onRemote);
    if (engine.enabled) {
      await channel.startSyncService();
    }
  }

  Future<void> setEnabled(bool enabled) async {
    await engine.setEnabled(enabled);
    if (enabled) {
      await channel.startSyncService();
    } else {
      await channel.stopSyncService();
    }
  }

  Future<void> stop() async {
    await _copies?.cancel();
    await _received?.cancel();
    _copies = null;
    _received = null;
    await channel.stopSyncService();
  }

  void _onLocalCopy(LocalCopy copy) {
    final text = copy.text;
    final png = copy.png;
    if (text != null) {
      engine.localText(text, markedSensitive: copy.markedSensitive);
    } else if (png != null) {
      // Decoding and sending happen on the Rust runtime.
      engine.localPng(png, markedSensitive: copy.markedSensitive);
    }
  }

  Future<void> _onRemote(rust.ClipboardReceived clip) async {
    final wrote = switch (clip.kind) {
      rust.ClipKind.image when clip.png != null => await channel.writePng(
        clip.png!,
        device: clip.deviceName,
      ),
      _ when clip.text != null => await channel.writeText(
        clip.text!,
        device: clip.deviceName,
      ),
      _ => false,
    };
    if (wrote) {
      onReceived?.call(clip.deviceName);
    }
  }

  Future<void> dispose() async {
    await stop();
    channel.dispose();
  }
}

/// What the settings screen says about Android's limits.
const clipboardLimitations = <String>[
  'Android 10 and later only let apps read the clipboard while they are on '
      'screen. Copies made in other apps reach your devices when you tap '
      '"Sync clipboard" in the FreeLife notification.',
  'Android 13 and later show a "Copied" message whenever FreeLife puts '
      'something on the clipboard. Android draws this itself, and apps cannot '
      'turn it off.',
];
