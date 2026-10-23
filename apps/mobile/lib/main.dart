import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import 'bridge/core_bridge.dart';
import 'platform/device_channel.dart';

void main() {
  WidgetsFlutterBinding.ensureInitialized();
  // TODO: once bindings are generated, call `await RustLib.init();` here
  // and swap StubCoreBridge for the generated API.
  runApp(const FreeLifeApp());
}

class FreeLifeApp extends StatelessWidget {
  const FreeLifeApp({
    super.key,
    this.bridge = const StubCoreBridge(),
    this.device = const DeviceChannel(),
  });

  final CoreBridge bridge;
  final DeviceChannel device;

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'FreeLife',
      theme: ThemeData(
        colorScheme: ColorScheme.fromSeed(seedColor: const Color(0xFF2563EB)),
      ),
      darkTheme: ThemeData(
        colorScheme: ColorScheme.fromSeed(
          seedColor: const Color(0xFF2563EB),
          brightness: Brightness.dark,
        ),
      ),
      home: HomePage(bridge: bridge, device: device),
    );
  }
}

class HomePage extends StatefulWidget {
  const HomePage({super.key, required this.bridge, required this.device});

  final CoreBridge bridge;
  final DeviceChannel device;

  @override
  State<HomePage> createState() => _HomePageState();
}

class _HomePageState extends State<HomePage> {
  NativeDeviceInfo? _info;
  int? _battery;
  String? _error;

  @override
  void initState() {
    super.initState();
    _load();
  }

  Future<void> _load() async {
    try {
      final info = await widget.device.getDeviceInfo();
      final battery = await widget.device.getBatteryLevel();
      if (!mounted) return;
      setState(() {
        _info = info;
        _battery = battery;
      });
    } on PlatformException catch (e) {
      if (mounted) setState(() => _error = e.message);
    } on MissingPluginException {
      // No native implementation on this platform yet (e.g. iOS).
      if (mounted) setState(() => _error = 'Platform channel not available');
    }
  }

  @override
  Widget build(BuildContext context) {
    final info = _info;
    return Scaffold(
      appBar: AppBar(title: const Text('FreeLife')),
      body: ListView(
        padding: const EdgeInsets.all(16),
        children: [
          ListTile(
            leading: const Icon(Icons.phone_android),
            title: Text(info?.displayName ?? 'This device'),
            subtitle: Text(
              _error ??
                  (info == null
                      ? 'Loading…'
                      : 'OS ${info.osVersion} · battery ${_battery ?? '?'}%'),
            ),
          ),
          ListTile(
            leading: const Icon(Icons.hub),
            title: const Text('Protocol version'),
            trailing: Text('${widget.bridge.protocolVersion()}'),
          ),
          const ListTile(
            leading: Icon(Icons.devices),
            title: Text('Nearby devices'),
            subtitle: Text('Discovery coming soon'),
          ),
        ],
      ),
    );
  }
}
