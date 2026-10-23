import 'package:flutter/material.dart';

import 'backend.dart';
import 'home_page.dart';
import 'platform/airshare_channel.dart';
import 'platform/device_channel.dart';
import 'src/rust/frb_generated.dart';

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();
  await RustLib.init();
  runApp(
    FreeLifeApp(backend: const RustBackend(), airshare: AirShareChannel()),
  );
}

class FreeLifeApp extends StatelessWidget {
  const FreeLifeApp({
    super.key,
    required this.backend,
    required this.airshare,
    this.device = const DeviceChannel(),
  });

  final FreeLifeBackend backend;
  final AirShareChannel airshare;
  final DeviceChannel device;

  @override
  Widget build(BuildContext context) {
    const seed = Color(0xFF2563EB);
    return MaterialApp(
      title: 'FreeLife',
      theme: ThemeData(colorScheme: ColorScheme.fromSeed(seedColor: seed)),
      darkTheme: ThemeData(
        colorScheme: ColorScheme.fromSeed(
          seedColor: seed,
          brightness: Brightness.dark,
        ),
      ),
      home: HomePage(backend: backend, device: device, airshare: airshare),
    );
  }
}
