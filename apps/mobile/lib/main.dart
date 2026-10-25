import 'package:flutter/material.dart';

import 'backend.dart';
import 'clipboard_sync.dart';
import 'home_page.dart';
import 'platform/airshare_channel.dart';
import 'platform/clipboard_channel.dart';
import 'platform/device_channel.dart';
import 'src/rust/frb_generated.dart';

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();
  await RustLib.init();
  final messengerKey = GlobalKey<ScaffoldMessengerState>();
  final clipboard = ClipboardSyncController(
    engine: const RustClipboardEngine(),
    channel: ClipboardChannel(),
    onReceived: (device) => messengerKey.currentState?.showSnackBar(
      SnackBar(content: Text('Clipboard synced from $device')),
    ),
  );
  runApp(
    FreeLifeApp(
      backend: const RustBackend(),
      airshare: AirShareChannel(),
      clipboard: clipboard,
      messengerKey: messengerKey,
    ),
  );
}

class FreeLifeApp extends StatelessWidget {
  const FreeLifeApp({
    super.key,
    required this.backend,
    required this.airshare,
    required this.clipboard,
    this.device = const DeviceChannel(),
    this.messengerKey,
  });

  final FreeLifeBackend backend;
  final AirShareChannel airshare;
  final ClipboardSyncController clipboard;
  final DeviceChannel device;
  final GlobalKey<ScaffoldMessengerState>? messengerKey;

  @override
  Widget build(BuildContext context) {
    const seed = Color(0xFF2563EB);
    return MaterialApp(
      title: 'FreeLife',
      scaffoldMessengerKey: messengerKey,
      theme: ThemeData(colorScheme: ColorScheme.fromSeed(seedColor: seed)),
      darkTheme: ThemeData(
        colorScheme: ColorScheme.fromSeed(
          seedColor: seed,
          brightness: Brightness.dark,
        ),
      ),
      home: HomePage(
        backend: backend,
        device: device,
        airshare: airshare,
        clipboard: clipboard,
      ),
    );
  }
}
